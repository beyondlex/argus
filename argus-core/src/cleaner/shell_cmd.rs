use std::process::{Child, Stdio};
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
fn run_with_timeout(target: &ShellCmdTarget) -> Result<std::process::Output, String> {
    let mut child = std::process::Command::new(&target.command)
        .args(&target.args)
        // Output is buffered by us, not the terminal; avoid the child
        // inheriting our stdio so long output cannot block on the pipe.
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to run {}: {e}", target.command))?;

    let deadline = Instant::now() + Duration::from_secs(target.timeout_secs.max(1));
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| format!("wait {}: {e}", target.command))?
        {
            return finish_output(child, status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "{} timed out after {}s",
                target.command, target.timeout_secs
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn finish_output(
    mut child: Child,
    status: std::process::ExitStatus,
) -> Result<std::process::Output, String> {
    use std::io::Read;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_end(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_end(&mut stderr);
    }
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

pub fn exec_all_shell_cmds(targets: &[ShellCmdTarget]) -> Vec<ShellCmdResult> {
    targets.iter().map(try_exec_shell_cmd).collect()
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
}
