use std::process::Stdio;
use std::time::{Duration, Instant};

use super::audit::{log_operation, AuditEntry, AuditOp};

#[derive(Debug, Clone)]
pub struct ShellCmdTarget {
    pub id: String,
    pub label: String,
    pub command: String,
    pub args: Vec<String>,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone)]
pub struct ShellCmdResult {
    pub id: String,
    pub label: String,
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
}

pub fn default_shell_cmd_targets() -> Vec<ShellCmdTarget> {
    vec![
        ShellCmdTarget {
            id: "brew-cleanup".into(),
            label: "Homebrew Cleanup".into(),
            command: "brew".into(),
            args: vec!["cleanup".into()],
            timeout_secs: 120,
        },
        ShellCmdTarget {
            id: "brew-autoremove".into(),
            label: "Homebrew Autoremove".into(),
            command: "brew".into(),
            args: vec!["autoremove".into()],
            timeout_secs: 120,
        },
        ShellCmdTarget {
            id: "docker-prune".into(),
            label: "Docker Build Cache".into(),
            command: "docker".into(),
            args: vec!["builder".into(), "prune".into(), "-f".into()],
            timeout_secs: 300,
        },
    ]
}

/// Run the command and collect its output, aborting on timeout.
///
/// `Command::output()` has no timeout support in std and the declared
/// `timeout_secs` used to be silently ignored — a wedged `docker builder
/// prune` blocked the cleanup flow forever. Poll `try_wait` instead and kill
/// the process when the budget is exhausted.
///
/// The pipes must be drained *while* the child runs, not after exit: once the
/// 64 KiB pipe buffer fills, a chatty child (`brew cleanup` prints every
/// keg) blocks on write and never exits, so polling `try_wait` alone would
/// kill it at the deadline even though it was making progress.
fn run_with_timeout(target: &ShellCmdTarget) -> Result<std::process::Output, String> {
    use std::sync::mpsc;

    let mut child = std::process::Command::new(&target.command)
        .args(&target.args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to run {}: {e}", target.command))?;

    // One drain thread per pipe; each forwards bytes into a channel-owned
    // buffer so the child can never block on a full pipe.
    fn drain<R: std::io::Read + Send + 'static>(
        pipe: Option<R>,
    ) -> (
        std::thread::JoinHandle<()>,
        std::sync::mpsc::Receiver<Vec<u8>>,
    ) {
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let handle = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut pipe) = pipe {
                let mut chunk = [0u8; 8192];
                // Read to EOF (child exit or kill closes the pipe).
                while let Ok(n) = pipe.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
            }
            let _ = tx.send(buf);
        });
        (handle, rx)
    }
    let (out_handle, out_rx) = drain(child.stdout.take());
    let (err_handle, err_rx) = drain(child.stderr.take());

    let deadline = Instant::now() + Duration::from_secs(target.timeout_secs.max(1));
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => {
                // Pipes are drained by the threads; reaping is best-effort.
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("wait {}: {e}", target.command));
            }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            // Drop the drain threads' blocking reads by closing our ends via
            // process exit of the pipes; join with a detach to avoid hanging.
            return Err(format!(
                "{} timed out after {}s",
                target.command, target.timeout_secs
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    };

    // Child has exited, so both pipes see EOF and the joins return promptly.
    let stdout = out_rx.recv().unwrap_or_default();
    let stderr = err_rx.recv().unwrap_or_default();
    let _ = out_handle.join();
    let _ = err_handle.join();
    // Fully reap the child after draining the pipes.
    let _ = child.wait();
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

pub fn try_exec_shell_cmd(target: &ShellCmdTarget) -> ShellCmdResult {
    let result = run_with_timeout(target);

    match result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let success = output.status.success();
            let error = if !success && !stderr.is_empty() {
                Some(stderr)
            } else {
                None
            };

            let entry = AuditEntry {
                timestamp: chrono::Utc::now(),
                operation: AuditOp::Clean,
                paths: Vec::new(),
                total_bytes: 0,
                success,
                error: error.clone(),
            };
            let _ = log_operation(&entry);

            ShellCmdResult {
                id: target.id.clone(),
                label: target.label.clone(),
                success,
                output: stdout,
                error,
            }
        }
        Err(e) => {
            let entry = AuditEntry {
                timestamp: chrono::Utc::now(),
                operation: AuditOp::Clean,
                paths: Vec::new(),
                total_bytes: 0,
                success: false,
                error: Some(e.clone()),
            };
            let _ = log_operation(&entry);
            ShellCmdResult {
                id: target.id.clone(),
                label: target.label.clone(),
                success: false,
                output: String::new(),
                error: Some(e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_shell_cmd_targets_have_ids() {
        let targets = default_shell_cmd_targets();
        for t in &targets {
            assert!(!t.id.is_empty(), "target id empty: {:?}", t.label);
            assert!(!t.command.is_empty());
        }
    }

    #[test]
    fn test_try_exec_shell_cmd_echo() {
        let target = ShellCmdTarget {
            id: "test-echo".into(),
            label: "Test Echo".into(),
            command: "echo".into(),
            args: vec!["hello".into()],
            timeout_secs: 5,
        };
        let result = try_exec_shell_cmd(&target);
        assert!(result.success);
        assert_eq!(result.output, "hello");
    }

    /// Declared timeouts must actually fire: the field used to be silently
    /// ignored, so a wedged command blocked the cleanup flow forever.
    #[cfg(unix)]
    #[test]
    fn test_try_exec_shell_cmd_timeout_kills() {
        let start = std::time::Instant::now();
        let target = ShellCmdTarget {
            id: "test-timeout".into(),
            label: "Test Timeout".into(),
            command: "sleep".into(),
            args: vec!["30".into()],
            timeout_secs: 1,
        };
        let result = try_exec_shell_cmd(&target);
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("timed out"));
        assert!(start.elapsed() < std::time::Duration::from_secs(10));
    }

    /// Output larger than the 64 KiB pipe buffer must not wedge the run: the
    /// pipes used to stay unread while polling, so a chatty child blocked on
    /// write and was killed at the deadline despite finishing its work.
    #[cfg(unix)]
    #[test]
    fn test_try_exec_shell_cmd_large_output_completes() {
        let target = ShellCmdTarget {
            id: "test-chatty".into(),
            label: "Test Chatty".into(),
            command: "sh".into(),
            args: vec!["-c".into(), "head -c 262144 /dev/zero | tr '\\0' x".into()],
            timeout_secs: 10,
        };
        let result = try_exec_shell_cmd(&target);
        assert!(result.success, "error: {:?}", result.error);
        assert_eq!(result.output.len(), 262_144);
    }
}
