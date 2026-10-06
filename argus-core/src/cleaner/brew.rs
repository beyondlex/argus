use std::collections::{HashMap, HashSet};
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
    // `brew --prefix` is a Ruby script (~100ms). It is called from the TUI's
    // message-handling path (BrewScanComplete / enter_brew_ai_review), where
    // a blocking spawn stalled the UI on every brew scan completion; the
    // prefix cannot change within a process lifetime, so cache it.
    static PREFIX: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    PREFIX
        .get_or_init(|| {
            let out = Command::new(brew_bin())
                .arg("--prefix")
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
            PathBuf::from(out.unwrap_or_else(|| "/opt/homebrew".to_string()))
        })
        .clone()
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

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Preloaded shell-history state shared by every package in one scan.
/// Each package used to re-read both history files (multi-MB for long-lived
/// shells); after that was fixed, `last_used` still re-scanned every line per
/// package — O(packages × history lines), seconds of CPU for heavy histories.
/// One pass at load time indexes each command's first token instead.
///
/// Token matching mirrors `command_matches_pkg` exactly:
/// - exact token == pkg (A) or == pkg's base name, i.e. the part before '@' (B)
///   → looked up in `timed_exact` / `plain_exact`
/// - token with trailing digits stripped == base (`python3` vs `python`) (C)
///   → looked up in `timed_stripped` / `plain_stripped`
struct HistorySnapshot {
    /// First token → latest timestamped use (zsh extended format).
    timed_exact: HashMap<String, DateTime<Utc>>,
    /// Digit-stripped token → latest timestamped use (case C).
    timed_stripped: HashMap<String, DateTime<Utc>>,
    /// Per history file, in load order: untimestamped lines carry no per-line
    /// time, so the first file with a matching plain line contributes its
    /// mtime (unchanged fallback behavior).
    plain: Vec<PlainFileHistory>,
}

struct PlainFileHistory {
    mtime: Option<DateTime<Utc>>,
    exact: HashSet<String>,
    stripped: HashSet<String>,
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
        Self::from_files(files)
    }

    /// Index the history in one pass. Exposed for tests.
    fn from_files(files: Vec<(String, Option<DateTime<Utc>>)>) -> Self {
        let mut timed_exact: HashMap<String, DateTime<Utc>> = HashMap::new();
        let mut timed_stripped: HashMap<String, DateTime<Utc>> = HashMap::new();
        let mut plain = Vec::new();

        for (content, mtime) in files {
            let mut plain_file = PlainFileHistory {
                mtime,
                exact: HashSet::new(),
                stripped: HashSet::new(),
            };
            for line in content.lines() {
                if let Some(dt) = zsh_entry_timestamp(line) {
                    // ": <ts>:0;<cmd>" — index the command's first token.
                    let cmd = line.split_once(';').map(|(_, r)| r).unwrap_or("");
                    if let Some(token) = first_token(cmd) {
                        index_timed(&mut timed_exact, &mut timed_stripped, token, dt);
                    }
                    continue;
                }
                let trimmed = line.trim_start();
                if trimmed.starts_with(':') {
                    // Malformed zsh entry (unparseable timestamp): the old
                    // scan matched neither branch for these lines.
                    continue;
                }
                if let Some(token) = first_token(trimmed) {
                    plain_file.exact.insert(token.to_string());
                    if let Some(base) = digit_stripped(token) {
                        plain_file.stripped.insert(base.to_string());
                    }
                }
            }
            plain.push(plain_file);
        }

        Self {
            timed_exact,
            timed_stripped,
            plain,
        }
    }

    /// Latest timestamp among history entries whose command matches `pkg_name`,
    /// or the mtime of the first history file with any untimestamped match.
    fn last_used(&self, pkg_name: &str) -> Option<DateTime<Utc>> {
        let base = pkg_name.split('@').next().unwrap_or(pkg_name);
        let latest = [
            self.timed_exact.get(pkg_name),
            self.timed_exact.get(base),
            // Empty-stripped tokens are never indexed, so a bare digit query
            // cannot alias anything.
            self.timed_stripped.get(base),
        ]
        .into_iter()
        .flatten()
        .copied()
        .max();
        if latest.is_some() {
            return latest;
        }
        for file in &self.plain {
            if file.exact.contains(pkg_name)
                || file.exact.contains(base)
                || file.stripped.contains(base)
            {
                return file.mtime;
            }
        }
        None
    }
}

fn zsh_entry_timestamp(line: &str) -> Option<DateTime<Utc>> {
    // The zsh field is "<ts>:0" — everything after the first ':' is the event
    // id. Parsing "ts:0" as i64 used to fail on every line, silently
    // disabling this whole branch.
    let ts = line
        .strip_prefix(": ")
        .and_then(|rest| rest.split(';').next())
        .and_then(|field| field.split(':').next())?
        .trim()
        .parse::<i64>()
        .ok()?;
    Utc.timestamp_opt(ts, 0).single()
}

fn first_token(command: &str) -> Option<&str> {
    command.split_whitespace().next().filter(|t| !t.is_empty())
}

fn digit_stripped(token: &str) -> Option<&str> {
    let base = token.trim_end_matches(|c: char| c.is_ascii_digit());
    (!base.is_empty()).then_some(base)
}

fn index_timed(
    exact: &mut HashMap<String, DateTime<Utc>>,
    stripped: &mut HashMap<String, DateTime<Utc>>,
    token: &str,
    dt: DateTime<Utc>,
) {
    upsert_max(exact, token, dt);
    if let Some(base) = digit_stripped(token) {
        upsert_max(stripped, base, dt);
    }
}

/// Keep the latest timestamp when a token appears in several entries.
fn upsert_max(map: &mut HashMap<String, DateTime<Utc>>, key: &str, dt: DateTime<Utc>) {
    map.entry(key.to_string())
        .and_modify(|e| {
            if dt > *e {
                *e = dt;
            }
        })
        .or_insert(dt);
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

/// Whether a `.app` bundle stem plausibly belongs to a cask name. Inputs
/// differ in case and separator conventions ("Google Chrome" vs
/// "google-chrome"). Matching is token-level with digit extension allowed
/// (`iterm` matches `iterm2` like the shell-history rule), and *every* app
/// name word must be accounted for by a cask token — review #64: the old
/// `cask.contains(stem.to_lowercase())` matched `Chromium.app` for the
/// `google-chrome` cask (any "chrome" substring did), and lowercased only
/// the stem, so a cask name carrying uppercase could never match at all.
/// A missed match falls through to the keg atime and then "never used",
/// so false negatives push toward uninstall suggestions — the token rule
/// keeps recall for the real-world name pairs (space vs dash, trailing
/// version digits) while dropping cross-product false positives.
fn app_stem_matches_cask(stem: &str, cask: &str) -> bool {
    let stem_lc = stem.to_lowercase();
    if stem_lc.replace(' ', "-") == cask.to_lowercase() {
        return true;
    }
    let cask_lc = cask.to_lowercase();
    let cask_tokens: Vec<&str> = cask_lc.split('-').collect();
    stem_lc.split_whitespace().all(|stem_token| {
        cask_tokens.iter().any(|cask_token| {
            *cask_token == stem_token
                || (cask_token.starts_with(stem_token)
                    && cask_token[stem_token.len()..]
                        .bytes()
                        .all(|b| b.is_ascii_digit()))
        })
    })
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
                if app_stem_matches_cask(stem, pkg_name) {
                    let output = match Command::new("mdls")
                        .arg("-name")
                        .arg("kMDItemLastUsedDate")
                        .arg("-raw")
                        .arg(&p)
                        .output()
                    {
                        Ok(o) => o,
                        // A transient spawn failure (e.g. fd pressure) must
                        // skip this app, not give up on the whole search.
                        Err(_) => continue,
                    };
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

/// Ordering behind [`sort_oldest_first`], exposed so filtered/partial views
/// (TUI panel sort, CLI subset re-sort) sort identically to the full scan
/// without each growing a private copy of the closure.
pub fn compare_oldest_first(a: &BrewPackage, b: &BrewPackage) -> std::cmp::Ordering {
    match (&a.last_used, &b.last_used) {
        (None, None) => b.size.cmp(&a.size),
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(a_dt), Some(b_dt)) => a_dt.cmp(b_dt),
    }
}

/// Display ordering shared by the scanner and the CLI: least-recently-used
/// first, never-used packages before all used ones (sized descending within
/// each group). One definition — the CLI used to re-sort its filtered list
/// with a private copy of this closure that could drift from the scan order.
pub fn sort_oldest_first(packages: &mut [BrewPackage]) {
    packages.sort_by(compare_oldest_first);
}

/// Human-relative "last used" label shared by the CLI list and the TUI brew
/// panel: never / today / yesterday / Nd ago / Nmo ago / Ny ago. Both clients
/// shipped identical private copies that had to be fixed in lockstep.
pub fn format_last_used_relative(last_used: Option<DateTime<Utc>>) -> String {
    let Some(dt) = last_used else {
        return "never".to_string();
    };
    let days = (Utc::now() - dt).num_days();
    match days {
        0 => "today".to_string(),
        1 => "yesterday".to_string(),
        d if d < 30 => format!("{d}d ago"),
        d if d < 365 => format!("{}mo ago", d / 30),
        d => format!("{}y ago", d / 365),
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

    sort_oldest_first(&mut packages);

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

    fn pkg(name: &str, last_used: Option<DateTime<Utc>>, size: u64) -> BrewPackage {
        BrewPackage {
            name: name.into(),
            package_type: BrewPackageType::Formula,
            version: String::new(),
            size,
            installed_date: None,
            last_used,
            description: String::new(),
            dependents: 0,
            dependents_names: Vec::new(),
        }
    }

    /// Never-used packages come first (largest first within the group),
    /// then used ones oldest-first. The CLI re-applies this rule to its
    /// filtered subset; both callers must agree.
    #[test]
    fn test_sort_oldest_first() {
        let t = |secs: i64| Utc.timestamp_opt(secs, 0).single();
        let mut packages = vec![
            pkg("recent", t(1_700_000_100), 10),
            pkg("never-big", None, 500),
            pkg("old", t(1_600_000_000), 10),
            pkg("never-small", None, 5),
        ];
        sort_oldest_first(&mut packages);
        let names: Vec<&str> = packages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["never-big", "never-small", "old", "recent"]);
    }

    /// The shared comparator must agree with `sort_oldest_first`: filtered
    /// views sort their subsets with it directly.
    #[test]
    fn test_compare_oldest_first_matches_sort() {
        let t = |secs: i64| Utc.timestamp_opt(secs, 0).single();
        let a = pkg("a", t(1_600_000_000), 10);
        let b = pkg("b", t(1_700_000_000), 10);
        let never_big = pkg("n", None, 500);
        let never_small = pkg("m", None, 5);

        assert_eq!(
            compare_oldest_first(&never_big, &never_small),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            compare_oldest_first(&never_big, &a),
            std::cmp::Ordering::Less
        );
        assert_eq!(compare_oldest_first(&a, &b), std::cmp::Ordering::Less);
        assert_eq!(compare_oldest_first(&a, &a), std::cmp::Ordering::Equal);
    }

    /// One relative-time formatter for CLI and TUI: the boundaries
    /// (never / today / yesterday / <30d / <1y / older) must hold.
    #[test]
    fn test_format_last_used_relative() {
        assert_eq!(format_last_used_relative(None), "never");
        let now = Utc::now();
        assert_eq!(format_last_used_relative(Some(now)), "today");
        assert_eq!(
            format_last_used_relative(Some(now - chrono::Duration::hours(30))),
            "yesterday"
        );
        assert_eq!(
            format_last_used_relative(Some(now - chrono::Duration::days(10))),
            "10d ago"
        );
        assert_eq!(
            format_last_used_relative(Some(now - chrono::Duration::days(60))),
            "2mo ago"
        );
        assert_eq!(
            format_last_used_relative(Some(now - chrono::Duration::days(730))),
            "2y ago"
        );
    }

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

    /// Brute-force line-by-line matcher kept as the reference for the
    /// token-index equivalence test. Semantics: first token == pkg (A),
    /// == pkg's base name before '@' (B), or trailing digits stripped ==
    /// base (`python3` vs `python`) (C).
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

    #[test]
    fn test_command_matches_pkg() {
        assert!(command_matches_pkg("wget https://example.com", "wget"));
        assert!(command_matches_pkg("python3 script.py", "python@3.11"));
        assert!(!command_matches_pkg("ls -la", "wget"));
        assert!(command_matches_pkg("python -V", "python@3.11"));
    }

    #[test]
    fn test_history_snapshot_zsh_extended_format() {
        let hist = HistorySnapshot::from_files(vec![(
            ": 1700000000:0;wget https://example.com\n: 1690000000:0;ls -la\n".into(),
            None,
        )]);
        let dt = hist.last_used("wget").expect("zsh timestamp");
        assert_eq!(dt.timestamp(), 1_700_000_000);
        // Several timestamped entries for one command: the latest wins.
        let multi = HistorySnapshot::from_files(vec![(
            ": 1700000000:0;wget a\n: 1710000000:0;wget b\n".into(),
            None,
        )]);
        assert_eq!(multi.last_used("wget").unwrap().timestamp(), 1_710_000_000);
        assert!(hist.last_used("curl").is_none());
    }

    #[test]
    fn test_history_snapshot_plain_format_uses_mtime() {
        let mtime = Utc.timestamp_opt(1_690_000_000, 0).single();
        let hist = HistorySnapshot::from_files(vec![("wget file.tar.gz\n".into(), mtime)]);
        let dt = hist.last_used("wget").expect("mtime fallback");
        assert_eq!(dt.timestamp(), 1_690_000_000);
    }

    #[test]
    fn test_history_snapshot_empty_or_unmatched() {
        let hist = HistorySnapshot::from_files(Vec::new());
        assert!(hist.last_used("wget").is_none());
    }

    /// The one-pass token index must agree with brute-force line scanning for
    /// every package name: same Optional timestamp, or the same mtime
    /// fallback. Guards the three matching cases (exact pkg, exact base,
    /// digit-stripped base) against drift.
    #[test]
    fn test_history_index_matches_brute_force_scan() {
        let mtime = Utc.timestamp_opt(1_680_000_000, 0).single();
        let content = ": 1700000000:0;wget https://example.com\n\
                       : 1700000100:0;python3 -m pip install x\n\
                       : 1700000200:0;python -V\n\
                       : 1700000300:0;python@3.11 --version\n\
                       ls -la\n\
                       : 1700000400:0;git status\n\
                       : malformed;wget should-not-parse\n\
                       wget2 file.bin\n\
                       : 1700000500:0;vimrc_helper\n\
                       : 1700000600:0;cargo build --release\n\
                       : 1700000700:0;cargo3 test\n\
                       wget later.tar.gz\n";
        let files = vec![(content.to_string(), mtime)];
        let indexed = HistorySnapshot::from_files(files.clone());

        // Brute force: the old line-by-line scan (timestamped lines only —
        // this fixture's mtime fallback is exercised by the dedicated test).
        let brute_force = |pkg: &str| -> Option<DateTime<Utc>> {
            let mut latest = None;
            for (content, _) in &files {
                for line in content.lines() {
                    if let Some(dt) = zsh_entry_timestamp(line) {
                        let rest = line.split_once(';').map(|(_, r)| r).unwrap_or("");
                        if command_matches_pkg(rest, pkg) && latest.is_none_or(|l| dt > l) {
                            latest = Some(dt);
                        }
                    }
                }
            }
            latest
        };

        // Timestamped-comparable packages only: packages whose only matches
        // are plain lines fall through the brute-force helper (ts-only) here
        // and are asserted against the mtime fallback below.
        for pkg in [
            "wget",
            "python",
            "python3",
            "python@3.11",
            "git",
            "cargo",
            "cargo@3",
            "vimrc_helper",
            "vim",
            "curl",
        ] {
            assert_eq!(
                indexed.last_used(pkg),
                brute_force(pkg),
                "index diverged from brute force for pkg {pkg:?}"
            );
        }

        // The fixture's plain lines (`wget later.tar.gz`) map to the file
        // mtime fallback for wget — but wget also has timestamped entries,
        // and those take precedence in the normalized semantics.
        assert_eq!(
            indexed.last_used("wget").unwrap().timestamp(),
            1_700_000_000
        );
        // Plain-only package falls back to the file mtime.
        assert_eq!(indexed.last_used("wget2"), mtime);
    }

    #[test]
    fn test_brew_prefix_fallback() {
        let prefix = brew_prefix();
        assert!(!prefix.to_string_lossy().is_empty());
    }

    /// The cask→.app matcher must keep recall for real-world name pairs
    /// (case, spaces vs dashes, trailing version digits) while refusing
    /// cross-product matches between distinct products.
    #[test]
    fn test_app_stem_matches_cask() {
        // Exact and separator/case variants.
        assert!(app_stem_matches_cask("firefox", "firefox"));
        assert!(app_stem_matches_cask("Google Chrome", "google-chrome"));
        assert!(app_stem_matches_cask(
            "Visual Studio Code",
            "visual-studio-code"
        ));
        // Trailing digits extend a stem token (shell-history rule).
        assert!(app_stem_matches_cask("iTerm", "iterm2"));
        assert!(app_stem_matches_cask("iTerm2", "iterm2"));

        // Distinct products must not claim each other.
        assert!(!app_stem_matches_cask("Chromium", "google-chrome"));
        assert!(!app_stem_matches_cask("Chrome", "chromium"));
        assert!(!app_stem_matches_cask("Google Drive", "google-chrome"));
        // A longer app word that merely starts with the cask token.
        assert!(!app_stem_matches_cask("VLC Remote", "vlc"));
    }

    #[test]
    fn test_dir_size_nonexistent() {
        assert_eq!(dir_size(Path::new("/_nonexistent_xyz_")), 0);
    }
}
