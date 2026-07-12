use std::fs;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use clap::ValueEnum;
use crate::SHOULD_QUIT;

pub struct DaemonGuard {
    pid_path: PathBuf,
}

impl DaemonGuard {
    fn pid_path() -> PathBuf {
        config_dir().join("argusd.pid")
    }

    pub fn daemonize() -> Result<Self, String> {
        let pid_path = Self::pid_path();

        let pid = unsafe { libc::fork() };
        if pid < 0 {
            return Err("fork failed".into());
        }
        if pid > 0 {
            unsafe { libc::_exit(0); }
        }

        unsafe { libc::setsid(); }
        if unsafe { libc::fork() } > 0 {
            unsafe { libc::_exit(0); }
        }

        let my_pid = std::process::id();
        if let Some(parent) = pid_path.parent() {
            fs::create_dir_all(parent).ok();
        }
        fs::write(&pid_path, my_pid.to_string()).map_err(|e| format!("failed to write PID file: {e}"))?;

        redirect_stdio();

        let cleanup_path = pid_path.clone();
        std::thread::spawn(move || {
            while !SHOULD_QUIT.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            fs::remove_file(&cleanup_path).ok();
        });

        Ok(Self { pid_path })
    }

    pub fn print_service(template: ServiceTemplate) {
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/usr/local/bin/argusd"));
        match template {
            ServiceTemplate::Launchd => print_launchd_plist(&exe),
            ServiceTemplate::Systemd => print_systemd_unit(&exe),
        }
    }

    pub fn stop() {
        let pid_path = Self::pid_path();
        let pid_str = match fs::read_to_string(&pid_path) {
            Ok(s) => s.trim().to_string(),
            Err(_) => {
                eprintln!("argusd: no PID file found at {}", pid_path.display());
                std::process::exit(1);
            }
        };
        let pid: i32 = match pid_str.parse() {
            Ok(n) => n,
            Err(_) => {
                eprintln!("argusd: invalid PID in {}", pid_path.display());
                std::process::exit(1);
            }
        };

        unsafe { libc::kill(pid, libc::SIGTERM) };
        eprintln!("argusd: sent SIGTERM to pid {pid}");

        for _ in 0..50 {
            unsafe { libc::kill(pid, 0) };
            let alive = std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH);
            if !alive {
                fs::remove_file(&pid_path).ok();
                eprintln!("argusd: stopped");
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        eprintln!("argusd: process {pid} did not exit, sending SIGKILL");
        unsafe { libc::kill(pid, libc::SIGKILL) };
        fs::remove_file(&pid_path).ok();
    }
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        fs::remove_file(&self.pid_path).ok();
    }
}

fn redirect_stdio() {
    if let Ok(null) = std::fs::File::open("/dev/null") {
        let fd = null.as_raw_fd();
        unsafe {
            libc::dup2(fd, libc::STDIN_FILENO);
            libc::dup2(fd, libc::STDOUT_FILENO);
            libc::dup2(fd, libc::STDERR_FILENO);
        }
    }
}

#[derive(Clone, ValueEnum)]
pub enum ServiceTemplate {
    Launchd,
    Systemd,
}

fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("argus")
}

fn print_launchd_plist(exe: &PathBuf) {
    let exe = exe.display();
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.argus.daemon</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
    </array>
    <key>KeepAlive</key>
    <true/>
    <key>RunAtLoad</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/tmp/argusd.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/argusd.log</string>
</dict>
</plist>
"#
    );
    println!("{plist}");
    eprintln!("---");
    eprintln!("Install: mkdir -p ~/Library/LaunchAgents && argusd --generate-service launchd > ~/Library/LaunchAgents/com.argus.daemon.plist && launchctl load ~/Library/LaunchAgents/com.argus.daemon.plist");
}

fn print_systemd_unit(exe: &PathBuf) {
    let exe = exe.display();
    let unit = format!(
        r#"[Unit]
Description=Argus Daemon
After=network.target

[Service]
ExecStart={exe}
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
"#
    );
    println!("{unit}");
    eprintln!("---");
    eprintln!("Install: sudo tee /etc/systemd/system/argusd.service <<< \"$(argusd --generate-service systemd)\" && sudo systemctl daemon-reload && sudo systemctl enable --now argusd");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_dir_has_argus() {
        let dir = config_dir();
        assert!(dir.ends_with("argus"));
    }

    #[test]
    fn test_pid_path_ends_correctly() {
        let path = DaemonGuard::pid_path();
        assert_eq!(path.file_name().unwrap(), "argusd.pid");
        assert!(path.ends_with("argus/argusd.pid"));
    }
}