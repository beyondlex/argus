use std::fs;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::SHOULD_QUIT;
use clap::ValueEnum;

pub struct DaemonGuard {
    pid_path: PathBuf,
}

impl DaemonGuard {
    fn pid_path() -> PathBuf {
        config_dir().join("argusd.pid")
    }

    /// Claim single-instance ownership before any watcher starts.
    ///
    /// Two concurrent daemons would each insert an event per filesystem
    /// change (double-counted deltas) and the second would steal the UDS
    /// socket file from under the first. The PID file doubles as the lock:
    /// it is claimed for foreground runs too, and `Drop` releases it.
    ///
    /// Known trade-off: after a crash the file can name a recycled PID and
    /// produce a false "already running"; run `argusd stop` / delete the
    /// file to recover (same PID-reuse caveat as `stop()`).
    pub fn acquire(daemon: bool) -> Result<Self, String> {
        if let Some(pid) = Self::running_daemon_pid() {
            return Err(format!(
                "argusd is already running (pid {pid}); stop it first with `argusd stop`"
            ));
        }
        if daemon {
            return Self::daemonize();
        }
        let pid_path = Self::pid_path();
        if let Some(parent) = pid_path.parent() {
            fs::create_dir_all(parent).ok();
        }
        fs::write(&pid_path, std::process::id().to_string())
            .map_err(|e| format!("failed to write PID file: {e}"))?;
        Ok(Self { pid_path })
    }

    /// PID of the live daemon named by the PID file, if any.
    /// A stale file whose PID no longer exists is treated as not running.
    pub fn running_daemon_pid() -> Option<i32> {
        let pid: i32 = fs::read_to_string(Self::pid_path())
            .ok()?
            .trim()
            .parse()
            .ok()?;
        if pid_alive(pid) {
            Some(pid)
        } else {
            None
        }
    }

    fn daemonize() -> Result<Self, String> {
        let pid_path = Self::pid_path();

        let pid = unsafe { libc::fork() };
        if pid < 0 {
            return Err("fork failed".into());
        }
        if pid > 0 {
            unsafe {
                libc::_exit(0);
            }
        }

        unsafe {
            libc::setsid();
        }
        if unsafe { libc::fork() } > 0 {
            unsafe {
                libc::_exit(0);
            }
        }

        let my_pid = std::process::id();
        if let Some(parent) = pid_path.parent() {
            fs::create_dir_all(parent).ok();
        }
        fs::write(&pid_path, my_pid.to_string())
            .map_err(|e| format!("failed to write PID file: {e}"))?;

        println!("argusd: daemon started (pid {my_pid})");
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
        let exe =
            std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/usr/local/bin/argusd"));
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

/// Existence probe: signal 0 delivers nothing but reports ESRCH when the
/// process does not exist. EPERM (process exists, not ours) counts as alive.
fn pid_alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) };
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

fn print_launchd_plist(exe: &Path) {
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

fn print_systemd_unit(exe: &Path) {
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

    #[test]
    fn test_pid_alive_detects_live_and_reaped_process() {
        assert!(pid_alive(std::process::id() as i32));

        // A fully reaped child must report ESRCH.
        let mut child = std::process::Command::new("true")
            .spawn()
            .expect("spawn true");
        let pid = child.id() as i32;
        child.wait().expect("reap child");
        assert!(!pid_alive(pid));
    }
}
