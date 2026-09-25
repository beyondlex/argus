use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::{DateTime, TimeZone, Utc};

use super::audit::{log_operation, AuditEntry, AuditOp};
use super::cleaner::CleanReport;
use super::dir_size;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrewPackageType {
    Formula,
    Cask,
}

impl BrewPackageType {
    pub fn label(&self) -> &'static str {
        match self {
            BrewPackageType::Formula => "F",
            BrewPackageType::Cask => "C",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BrewPackage {
    pub name: String,
    pub package_type: BrewPackageType,
    pub version: String,
    pub size: u64,
    pub installed_date: Option<DateTime<Utc>>,
    pub last_used: Option<DateTime<Utc>>,
    pub description: String,
    pub dependents: usize,
    pub dependents_names: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrewSortMode {
    Time,
    Size,
    Name,
    Type,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrewFilterType {
    All,
    Formula,
    Cask,
}

fn brew_bin() -> PathBuf {
    // 常见的 brew 安装路径
    let candidates = ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"];
    for c in &candidates {
        if Path::new(c).exists() {
            return PathBuf::from(c);
        }
    }
    // fallback: 从 PATH 中查找
    PathBuf::from("brew")
}

pub fn brew_prefix() -> PathBuf {
    let out = Command::new(brew_bin())
        .arg("--prefix")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    PathBuf::from(out.unwrap_or_else(|| "/opt/homebrew".to_string()))
}

fn brew_list_json(package_type: &str) -> Vec<BrewInfo> {
    let out = Command::new(brew_bin())
        .args([
            "list",
            &format!("--{}", package_type),
            "--json",
            "--versions",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success());
    let Some(output) = out else {
        return Vec::new();
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_brew_list_json(&stdout, package_type)
}

fn parse_brew_list_json(json_str: &str, package_type: &str) -> Vec<BrewInfo> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) else {
        return Vec::new();
    };

    let key = if package_type == "formula" {
        "formulae"
    } else {
        "casks"
    };

    let Some(items) = v.get(key).and_then(|v| v.as_array()) else {
        return Vec::new();
    };

    let mut result = Vec::new();
    for item in items {
        let name = item
            .get("full_name")
            .or_else(|| item.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if name.is_empty() {
            continue;
        }

        let versions = item.get("versions");
        let version = versions
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|v| v.as_str())
            .or_else(|| {
                versions
                    .and_then(|v| v.as_object())
                    .and_then(|m| m.values().next())
                    .and_then(|v| v.as_str())
            })
            .map(|s| s.to_string())
            .unwrap_or_default();

        let installed_on = item
            .get("installed_on")
            .and_then(|v| v.as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        result.push(BrewInfo {
            name,
            version,
            installed_date: installed_on,
            ptype: package_type.to_string(),
        });
    }
    result
}

#[allow(dead_code)]
struct BrewInfo {
    name: String,
    version: String,
    installed_date: Option<DateTime<Utc>>,
    ptype: String,
}

#[allow(dead_code)]
fn brew_info_json(name: &str) -> Option<BrewDetailedInfo> {
    let out = Command::new(brew_bin())
        .args(["info", "--json=v2", name])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    parse_brew_info_json(&stdout)
}

#[allow(dead_code)]
fn parse_brew_info_json(json_str: &str) -> Option<BrewDetailedInfo> {
    let v: serde_json::Value = serde_json::from_str(json_str).ok()?;

    // formulae 和 casks 都可能在顶层数组中
    let item = v
        .get("formulae")
        .and_then(|a| a.as_array())
        .and_then(|arr| arr.first())
        .or_else(|| {
            v.get("casks")
                .and_then(|a| a.as_array())
                .and_then(|arr| arr.first())
        })?;

    let desc = item
        .get("desc")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let deps_count = item
        .get("dependencies")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    let aliases_count = item
        .get("build_dependencies")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    // 获取大小: installed bottle 的下载大小
    let size = item
        .get("bottle")
        .and_then(|b| b.get("stable"))
        .and_then(|b| b.get("files"))
        .and_then(|f| f.as_object())
        .and_then(|m| {
            // 取第一个平台的 size
            m.values().next()
        })
        .and_then(|f| f.get("size"))
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    Some(BrewDetailedInfo {
        desc,
        deps_count: deps_count + aliases_count,
        installed_bottle_size: size,
    })
}

#[allow(dead_code)]
struct BrewDetailedInfo {
    desc: String,
    deps_count: usize,
    installed_bottle_size: u64,
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Preloaded shell-history state shared by every package in one scan.
/// Each package used to re-read both history files (multi-MB for long-lived
/// shells), making a 100-package scan re-read hundreds of MB.
struct HistorySnapshot {
    /// `(content, mtime)` per readable history file.
    files: Vec<(String, Option<DateTime<Utc>>)>,
}

impl HistorySnapshot {
    fn load() -> Self {
        let mut files = Vec::new();
        if let Some(home) = home_dir() {
            for hist_path in [home.join(".zsh_history"), home.join(".bash_history")] {
                let mtime = std::fs::metadata(&hist_path)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| {
                        let dur = t.duration_since(std::time::UNIX_EPOCH).ok()?;
                        Utc.timestamp_opt(dur.as_secs() as i64, 0).single()
                    });
                // read_to_string fails on non-UTF-8 bytes (zsh keeps binary
                // garbage from pasted input); skip that file rather than
                // aborting the whole scan (`?` here used to drop every later
                // file too).
                if let Ok(content) = std::fs::read_to_string(&hist_path) {
                    files.push((content, mtime));
                }
            }
        }
        Self { files }
    }

    /// Latest timestamp among history entries whose command matches `pkg_name`.
    /// zsh extended format: ": <timestamp>:0;<command>"; plain: "  <command>".
    fn last_used(&self, pkg_name: &str) -> Option<DateTime<Utc>> {
        let mut latest: Option<DateTime<Utc>> = None;
        for (content, mtime) in &self.files {
            for line in content.lines().rev() {
                if let Some(ts_str) = line
                    .strip_prefix(": ")
                    .and_then(|rest| rest.split(';').next())
                    // The zsh field is "<ts>:0" — everything after the first
                    // ':' is the event id. Parsing "ts:0" as i64 used to fail
                    // on every line, silently disabling this whole branch.
                    .and_then(|field| field.split(':').next())
                {
                    if let Ok(ts) = ts_str.trim().parse::<i64>() {
                        if let Some(dt) = Utc.timestamp_opt(ts, 0).single() {
                            let line_rest = line.split_once(';').map(|(_, r)| r).unwrap_or("");
                            if command_matches_pkg(line_rest, pkg_name)
                                && latest.is_none_or(|l| dt > l)
                            {
                                latest = Some(dt);
                            }
                        }
                    }
                }
                let trimmed = line.trim_start();
                if !trimmed.starts_with(':') && command_matches_pkg(trimmed, pkg_name) {
                    // No per-entry timestamp: fall back to the file mtime.
                    if latest.is_none() {
                        latest = *mtime;
                    }
                }
            }
        }
        latest
    }
}

fn command_matches_pkg(command: &str, pkg_name: &str) -> bool {
    let cmd_first_token = command.split_whitespace().next().unwrap_or("");
    if cmd_first_token == pkg_name {
        return true;
    }
    let base_name = pkg_name.split('@').next().unwrap_or(pkg_name);
    if cmd_first_token == base_name {
        return true;
    }
    let cmd_base = cmd_first_token.trim_end_matches(|c: char| c.is_ascii_digit());
    !cmd_base.is_empty() && cmd_base == base_name
}

/// 获取包安装路径下二进制文件的最后访问时间（第一层，最多 100 个条目取最大值）
fn last_access_from_opt(prefix: &Path, pkg_name: &str) -> Option<DateTime<Utc>> {
    let opt_dir = prefix.join("opt").join(pkg_name);
    if !opt_dir.exists() {
        return None;
    }
    // 先检查 bin 目录下的可执行文件
    let bin_dir = opt_dir.join("bin");
    let target_dir = if bin_dir.exists() { &bin_dir } else { &opt_dir };
    let mut latest: Option<DateTime<Utc>> = None;
    let read_dir = std::fs::read_dir(target_dir).ok()?;
    for entry in read_dir.flatten().take(100) {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_dir() {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if let Ok(atime) = meta.accessed() {
            if let Ok(dur) = atime.duration_since(std::time::UNIX_EPOCH) {
                if let Some(dt) = Utc.timestamp_opt(dur.as_secs() as i64, 0).single() {
                    if latest.is_none_or(|l| dt > l) {
                        latest = Some(dt);
                    }
                }
            }
        }
    }
    latest
}

/// Spotlight: 对 cask 类型的 GUI 应用获取最后使用时间
fn spotlight_last_used(pkg_name: &str) -> Option<DateTime<Utc>> {
    // 尝试在常见应用路径下查找 .app
    let home = home_dir()?;
    let app_dirs = [PathBuf::from("/Applications"), home.join("Applications")];
    for app_dir in &app_dirs {
        if !app_dir.exists() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(app_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|s| s.to_str()) != Some("app") {
                    continue;
                }
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if stem.eq_ignore_ascii_case(pkg_name)
                    || stem.replace(' ', "-").eq_ignore_ascii_case(pkg_name)
                    || pkg_name.contains(&stem.to_lowercase())
                {
                    let output = Command::new("mdls")
                        .arg("-name")
                        .arg("kMDItemLastUsedDate")
                        .arg("-raw")
                        .arg(&p)
                        .output()
                        .ok()?;
                    if output.status.success() {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        let trimmed = stdout.trim();
                        if !trimmed.is_empty() && trimmed != "(null)" {
                            if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
                                return Some(dt.with_timezone(&Utc));
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// 综合判定包的最后使用时间：shell history → opt atime → spotlight
fn determine_last_used(
    pkg_name: &str,
    package_type: &BrewPackageType,
    history: &HistorySnapshot,
    prefix: &Path,
) -> Option<DateTime<Utc>> {
    // 1. Shell history (formula/CLI)
    if let Some(dt) = history.last_used(pkg_name) {
        return Some(dt);
    }
    // 2. 文件访问时间
    if let Some(dt) = last_access_from_opt(prefix, pkg_name) {
        return Some(dt);
    }
    // 3. Spotlight (cask/GUI)
    if *package_type == BrewPackageType::Cask {
        if let Some(dt) = spotlight_last_used(pkg_name) {
            return Some(dt);
        }
    }
    None
}

/// 获取 brew 包的 keg 目录大小
fn keg_size(prefix: &Path, name: &str, ptype: &BrewPackageType) -> u64 {
    let keg_dir = keg_path(prefix, name, ptype);
    dir_size(&keg_dir)
}

/// 获取 brew 包的 keg 目录路径
pub fn keg_path(prefix: &Path, name: &str, ptype: &BrewPackageType) -> PathBuf {
    match ptype {
        BrewPackageType::Formula => prefix.join("Cellar").join(name),
        BrewPackageType::Cask => prefix.join("Caskroom").join(name),
    }
}

/// 获取所有已安装的 brew 包，按 last_used 升序排列
pub fn list_brew_packages(progress: Option<std::sync::mpsc::Sender<String>>) -> Vec<BrewPackage> {
    let mut packages = Vec::new();

    if let Some(ref tx) = progress {
        let _ = tx.send("0/0 Preparing package list...".into());
    }

    let prefix = brew_prefix();
    // Load once per scan: per-package history re-reads dominated the runtime.
    let history = HistorySnapshot::load();
    let formulae = brew_list_json("formula");
    let casks = brew_list_json("cask");

    let total = formulae.len() + casks.len();
    let mut count = 0;

    for info in &formulae {
        count += 1;
        if let Some(ref tx) = progress {
            let _ = tx.send(format!("{}/{} {}", count, total, info.name));
        }

        let ptype = BrewPackageType::Formula;
        let size = keg_size(&prefix, &info.name, &ptype);
        let last_used = determine_last_used(&info.name, &ptype, &history, &prefix);

        packages.push(BrewPackage {
            name: info.name.clone(),
            package_type: ptype,
            version: info.version.clone(),
            size,
            installed_date: info.installed_date,
            last_used,
            description: String::new(),
            dependents: 0,
            dependents_names: Vec::new(),
        });
    }

    for info in &casks {
        count += 1;
        if let Some(ref tx) = progress {
            let _ = tx.send(format!("{}/{} {}", count, total, info.name));
        }

        let ptype = BrewPackageType::Cask;
        let size = keg_size(&prefix, &info.name, &ptype);
        let last_used = determine_last_used(&info.name, &ptype, &history, &prefix);

        packages.push(BrewPackage {
            name: info.name.clone(),
            package_type: ptype,
            version: info.version.clone(),
            size,
            installed_date: info.installed_date,
            last_used,
            description: String::new(),
            dependents: 0,
            dependents_names: Vec::new(),
        });
    }

    packages.sort_by(|a, b| match (&a.last_used, &b.last_used) {
        (None, None) => b.size.cmp(&a.size),
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(a_dt), Some(b_dt)) => a_dt.cmp(b_dt),
    });

    packages
}

/// Run `brew uses --installed <name>` to get accurate dependents for a single package.
pub fn brew_dependents_of(name: &str) -> Vec<String> {
    let out = Command::new(brew_bin())
        .args(["uses", "--installed", name])
        .output()
        .ok()
        .filter(|o| o.status.success());
    let Some(output) = out else {
        return Vec::new();
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim().to_string())
        .collect()
}

/// 卸载 brew 包
pub fn uninstall_brew_package(pkg: &BrewPackage) -> Result<CleanReport, String> {
    let mut cmd = Command::new(brew_bin());
    cmd.arg("uninstall").arg("--force").arg(&pkg.name);

    let output = cmd
        .output()
        .map_err(|e| format!("failed to run brew: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("brew uninstall failed: {}", stderr.trim()));
    }

    // 清理缓存
    let _ = Command::new(brew_bin())
        .args(["cleanup", &pkg.name])
        .output();

    let report = CleanReport {
        total_attempted: 1,
        total_succeeded: 1,
        total_failed: 0,
        freed_bytes: pkg.size,
        errors: Vec::new(),
    };

    let entry = AuditEntry {
        timestamp: Utc::now(),
        operation: AuditOp::Uninstall,
        paths: vec![PathBuf::from(format!("brew:{}", pkg.name))],
        total_bytes: pkg.size,
        success: true,
        error: None,
    };
    let _ = log_operation(&entry);

    Ok(report)
}

/// 获取 brew 缓存目录大小
pub fn brew_cache_size() -> u64 {
    let out = Command::new(brew_bin())
        .arg("--cache")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    let cache_dir =
        PathBuf::from(out.unwrap_or_else(|| "/opt/homebrew/Library/Caches/Homebrew".to_string()));
    dir_size(&cache_dir)
}

/// 清理 brew 缓存
pub fn clean_brew_cache() -> Result<u64, String> {
    let before = brew_cache_size();
    let output = Command::new(brew_bin())
        .arg("cleanup")
        .output()
        .map_err(|e| format!("failed to run brew cleanup: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("brew cleanup failed: {}", stderr.trim()));
    }
    let after = brew_cache_size();
    Ok(before.saturating_sub(after))
}

/// 检查 brew 是否可用
pub fn is_brew_available() -> bool {
    Command::new(brew_bin())
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_brew_bin_path() {
        let bin = brew_bin();
        assert!(!bin.to_string_lossy().is_empty());
    }

    #[test]
    fn test_parse_brew_list_json_empty() {
        let result = parse_brew_list_json("{}", "formula");
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_brew_list_json_formula() {
        let json = r#"{
            "formulae": [
                {"full_name": "wget", "versions": {"stable": "2.1.0"}, "installed_on": "2024-01-15T10:00:00Z"},
                {"full_name": "curl", "versions": {"stable": "8.4.0"}, "installed_on": "2023-06-01T08:30:00Z"}
            ]
        }"#;
        let result = parse_brew_list_json(json, "formula");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "wget");
        assert_eq!(result[0].version, "2.1.0");
        assert!(result[0].installed_date.is_some());
        assert_eq!(result[1].name, "curl");
    }

    #[test]
    fn test_parse_brew_list_json_cask() {
        let json = r#"{
            "casks": [
                {"full_name": "firefox", "versions": {"stable": "120.0"}, "installed_on": "2023-11-01T12:00:00Z"}
            ]
        }"#;
        let result = parse_brew_list_json(json, "cask");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "firefox");
    }

    #[test]
    fn test_command_matches_pkg() {
        assert!(command_matches_pkg("wget https://example.com", "wget"));
        assert!(command_matches_pkg("python3 script.py", "python@3.11"));
        assert!(!command_matches_pkg("ls -la", "wget"));
        assert!(command_matches_pkg("python -V", "python@3.11"));
    }

    #[test]
    fn test_history_snapshot_zsh_extended_format() {
        let hist = HistorySnapshot {
            files: vec![(
                ": 1700000000:0;wget https://example.com\n: 1690000000:0;ls -la\n".into(),
                None,
            )],
        };
        let dt = hist.last_used("wget").expect("zsh timestamp");
        assert_eq!(dt.timestamp(), 1_700_000_000);
        assert!(hist.last_used("curl").is_none());
    }

    #[test]
    fn test_history_snapshot_plain_format_uses_mtime() {
        let mtime = Utc.timestamp_opt(1_690_000_000, 0).single();
        let hist = HistorySnapshot {
            files: vec![("wget file.tar.gz\n".into(), mtime)],
        };
        let dt = hist.last_used("wget").expect("mtime fallback");
        assert_eq!(dt.timestamp(), 1_690_000_000);
    }

    #[test]
    fn test_history_snapshot_empty_or_unmatched() {
        let hist = HistorySnapshot { files: Vec::new() };
        assert!(hist.last_used("wget").is_none());
    }

    #[test]
    fn test_brew_prefix_fallback() {
        let prefix = brew_prefix();
        assert!(!prefix.to_string_lossy().is_empty());
    }

    #[test]
    fn test_dir_size_nonexistent() {
        assert_eq!(dir_size(Path::new("/_nonexistent_xyz_")), 0);
    }
}
