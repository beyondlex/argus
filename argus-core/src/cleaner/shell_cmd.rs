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

pub fn try_exec_shell_cmd(target: &ShellCmdTarget) -> ShellCmdResult {
    let result = std::process::Command::new(&target.command)
        .args(&target.args)
        .output();

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
        Err(e) => ShellCmdResult {
            id: target.id.clone(),
            label: target.label.clone(),
            success: false,
            output: String::new(),
            error: Some(format!("{} not found: {e}", target.command)),
        },
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
}