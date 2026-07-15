use std::path::PathBuf;

use globset::{Glob, GlobBuilder};
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct WatchDir {
    pub path: PathBuf,
    pub include: Option<Glob>,
    pub exclude: Option<Glob>,
}

impl WatchDir {
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn matches(&self, event_path: &std::path::Path) -> bool {
        let rel = match event_path.strip_prefix(&self.path) {
            Ok(r) => r,
            Err(_) => return false,
        };
        let rel_str = rel.to_string_lossy();

        if let Some(ref exclude) = self.exclude {
            if exclude.compile_matcher().is_match(rel_str.as_ref()) {
                return false;
            }
        }
        if let Some(ref include) = self.include {
            if !include.compile_matcher().is_match(rel_str.as_ref()) {
                return false;
            }
        }
        true
    }
}

/// Serde helper: deserialize either a plain path string or a structured object.
#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
enum WatchDirSource {
    Plain(PathBuf),
    Structured {
        path: PathBuf,
        include: Option<String>,
        exclude: Option<String>,
    },
}

impl TryFrom<WatchDirSource> for WatchDir {
    type Error = String;

    fn try_from(source: WatchDirSource) -> Result<Self, Self::Error> {
        match source {
            WatchDirSource::Plain(path) => Ok(WatchDir {
                path,
                include: None,
                exclude: None,
            }),
            WatchDirSource::Structured {
                path,
                include,
                exclude,
            } => {
                let compile_glob = |s: String| -> Result<Glob, String> {
                    GlobBuilder::new(&s)
                        .case_insensitive(true)
                        .build()
                        .map_err(|e| format!("invalid glob '{s}': {e}"))
                };
                let include = include.map(compile_glob).transpose()?;
                let exclude = exclude.map(compile_glob).transpose()?;
                Ok(WatchDir {
                    path,
                    include,
                    exclude,
                })
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub watch_dirs: Vec<WatchDir>,
    pub debounce_seconds: u64,
    pub uds_path: String,
    pub snapshot_retention: SnapshotRetention,
    pub delta_retention_days: u64,
    pub consolidation: ConsolidationConfig,
    pub log_level: Option<String>,
    pub log_enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SnapshotRetention {
    #[serde(default = "default_hourly_retention")]
    pub hourly_retention_days: u64,
    #[serde(default = "default_daily_retention")]
    pub daily_retention_days: u64,
}

impl Default for SnapshotRetention {
    fn default() -> Self {
        Self {
            hourly_retention_days: default_hourly_retention(),
            daily_retention_days: default_daily_retention(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConsolidationConfig {
    #[serde(default = "default_sibling_threshold")]
    pub sibling_threshold: u64,
    #[serde(default = "default_consolidation_interval")]
    pub interval_minutes: u64,
}

impl Default for ConsolidationConfig {
    fn default() -> Self {
        Self {
            sibling_threshold: default_sibling_threshold(),
            interval_minutes: default_consolidation_interval(),
        }
    }
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            watch_dirs: vec![
                WatchDir {
                    path: PathBuf::from("/Users/lex/Downloads"),
                    include: None,
                    exclude: None,
                },
                WatchDir {
                    path: PathBuf::from("/Users/lex/Desktop"),
                    include: None,
                    exclude: None,
                },
            ],
            debounce_seconds: default_debounce_seconds(),
            uds_path: default_uds_path(),
            snapshot_retention: SnapshotRetention::default(),
            delta_retention_days: default_delta_retention_days(),
            consolidation: ConsolidationConfig::default(),
            log_level: None,
            log_enabled: false,
        }
    }
}

const fn default_debounce_seconds() -> u64 {
    10
}

fn default_uds_path() -> String {
    argus_core::DEFAULT_UDS_PATH.to_string()
}

const fn default_hourly_retention() -> u64 {
    7
}

const fn default_daily_retention() -> u64 {
    30
}

const fn default_delta_retention_days() -> u64 {
    30
}

const fn default_sibling_threshold() -> u64 {
    500
}

const fn default_consolidation_interval() -> u64 {
    60
}

fn config_path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("argus")
        .join("config.toml")
}

#[derive(Deserialize)]
struct RawDaemonConfig {
    #[serde(default)]
    watch_dirs: Vec<WatchDirSource>,
    #[serde(default = "default_debounce_seconds")]
    debounce_seconds: u64,
    #[serde(default = "default_uds_path")]
    uds_path: String,
    #[serde(default)]
    snapshot_retention: SnapshotRetention,
    #[serde(default = "default_delta_retention_days")]
    delta_retention_days: u64,
    #[serde(default)]
    consolidation: ConsolidationConfig,
    #[serde(default)]
    log_level: Option<String>,
    #[serde(default)]
    log_enabled: bool,
}

impl TryFrom<RawDaemonConfig> for DaemonConfig {
    type Error = String;

    fn try_from(raw: RawDaemonConfig) -> Result<Self, Self::Error> {
        let watch_dirs: Result<Vec<WatchDir>, _> = raw
            .watch_dirs
            .into_iter()
            .map(WatchDir::try_from)
            .collect();
        Ok(DaemonConfig {
            watch_dirs: watch_dirs?,
            debounce_seconds: raw.debounce_seconds,
            uds_path: raw.uds_path,
            snapshot_retention: raw.snapshot_retention,
            delta_retention_days: raw.delta_retention_days,
            consolidation: raw.consolidation,
            log_level: raw.log_level,
            log_enabled: raw.log_enabled,
        })
    }
}

#[derive(Deserialize)]
struct RawConfig {
    daemon: Option<RawDaemonConfig>,
}

pub fn load_config() -> DaemonConfig {
    let path = config_path();
    load_config_from_path(&path)
}

pub fn load_config_from(path: &str) -> DaemonConfig {
    load_config_from_path(&PathBuf::from(path))
}

fn load_config_from_path(path: &PathBuf) -> DaemonConfig {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return DaemonConfig::default(),
    };
    match toml::from_str::<RawConfig>(&content) {
        Ok(raw) => match raw.daemon {
            Some(raw_daemon) => match DaemonConfig::try_from(raw_daemon) {
                Ok(cfg) => cfg,
                Err(e) => {
                    tracing::warn!("invalid watch_dirs in config {path:?}: {e}, using defaults");
                    DaemonConfig::default()
                }
            },
            None => DaemonConfig::default(),
        },
        Err(e) => {
            tracing::warn!("failed to parse config {path:?}: {e}, using defaults");
            DaemonConfig::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use globset::GlobBuilder;

    fn glob(pattern: &str) -> Glob {
        GlobBuilder::new(pattern)
            .case_insensitive(true)
            .build()
            .unwrap()
    }

    #[test]
    fn test_default_config() {
        let config = DaemonConfig::default();
        assert_eq!(config.debounce_seconds, 10);
        assert_eq!(config.uds_path, argus_core::DEFAULT_UDS_PATH);
        assert!(!config.watch_dirs.is_empty());
        assert_eq!(config.delta_retention_days, 30);
        assert_eq!(config.consolidation.sibling_threshold, 500);
        assert_eq!(config.consolidation.interval_minutes, 60);
        assert!(config.log_level.is_none());
        assert!(!config.log_enabled);
    }

    #[test]
    fn test_config_log_level() {
        let toml_str = r#"
[daemon]
log_level = "debug"
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        let raw_daemon = raw.daemon.unwrap();
        let config = DaemonConfig::try_from(raw_daemon).unwrap();
        assert_eq!(config.log_level, Some("debug".to_string()));
        assert!(!config.log_enabled);
    }

    #[test]
    fn test_config_log_enabled() {
        let toml_str = r#"
[daemon]
log_enabled = true
log_level = "warn"
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        let raw_daemon = raw.daemon.unwrap();
        let config = DaemonConfig::try_from(raw_daemon).unwrap();
        assert!(config.log_enabled);
        assert_eq!(config.log_level, Some("warn".to_string()));
    }

    #[test]
    fn test_watch_dir_plain_string() {
        let toml_str = r#"
[daemon]
watch_dirs = ["/home/user/downloads"]
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        let raw_daemon = raw.daemon.unwrap();
        let config = DaemonConfig::try_from(raw_daemon).unwrap();
        assert_eq!(config.watch_dirs.len(), 1);
        assert_eq!(config.watch_dirs[0].path, PathBuf::from("/home/user/downloads"));
        assert!(config.watch_dirs[0].include.is_none());
        assert!(config.watch_dirs[0].exclude.is_none());
    }

    #[test]
    fn test_watch_dir_structured_with_filters() {
        let toml_str = r#"
[daemon]
watch_dirs = [
    { path = "/var/log", include = "*.log", exclude = "*.gz" },
]
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        let raw_daemon = raw.daemon.unwrap();
        let config = DaemonConfig::try_from(raw_daemon).unwrap();
        assert_eq!(config.watch_dirs.len(), 1);
        assert_eq!(config.watch_dirs[0].path, PathBuf::from("/var/log"));
        assert!(config.watch_dirs[0].include.is_some());
        assert!(config.watch_dirs[0].exclude.is_some());
    }

    #[test]
    fn test_watch_dir_mixed_plain_and_structured() {
        let toml_str = r#"
[daemon]
watch_dirs = [
    "/home/user/docs",
    { path = "/home/user/downloads", include = "*.pdf" },
]
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        let raw_daemon = raw.daemon.unwrap();
        let config = DaemonConfig::try_from(raw_daemon).unwrap();
        assert_eq!(config.watch_dirs.len(), 2);
        assert_eq!(config.watch_dirs[1].path, PathBuf::from("/home/user/downloads"));
        assert!(config.watch_dirs[1].include.is_some());
    }

    #[test]
    fn test_watch_dir_matches_include_glob() {
        let dir = WatchDir {
            path: PathBuf::from("/downloads"),
            include: Some(glob("*.pdf")),
            exclude: None,
        };
        assert!(dir.matches(PathBuf::from("/downloads/report.pdf").as_path()));
        assert!(!dir.matches(PathBuf::from("/downloads/file.txt").as_path()));
        assert!(!dir.matches(PathBuf::from("/other/file.pdf").as_path()));
    }

    #[test]
    fn test_watch_dir_matches_exclude_glob() {
        let dir = WatchDir {
            path: PathBuf::from("/logs"),
            include: None,
            exclude: Some(glob("*.gz")),
        };
        assert!(dir.matches(PathBuf::from("/logs/app.log").as_path()));
        assert!(!dir.matches(PathBuf::from("/logs/app.log.gz").as_path()));
    }

    #[test]
    fn test_watch_dir_include_and_exclude() {
        let dir = WatchDir {
            path: PathBuf::from("/data"),
            include: Some(glob("*.{txt,csv}")),
            exclude: Some(glob("*.tmp")),
        };
        assert!(dir.matches(PathBuf::from("/data/records.csv").as_path()));
        assert!(!dir.matches(PathBuf::from("/data/records.tmp").as_path()));
        assert!(!dir.matches(PathBuf::from("/data/records.jpg").as_path()));
    }

    #[test]
    fn test_watch_dir_outside_path() {
        let dir = WatchDir {
            path: PathBuf::from("/watch"),
            include: None,
            exclude: None,
        };
        assert!(!dir.matches(PathBuf::from("/other/file.txt").as_path()));
    }

    #[test]
    fn test_watch_dir_ci_matches_substring() {
        let dir = WatchDir {
            path: PathBuf::from("/home"),
            include: Some(glob("*jetbrains*")),
            exclude: None,
        };
        assert!(dir.matches(PathBuf::from("/home/Applications/JetBrains/idea.log").as_path()));
        assert!(dir.matches(PathBuf::from("/home/.cache/JetBrains").as_path()));
        assert!(!dir.matches(PathBuf::from("/home/Downloads/file.txt").as_path()));
    }

    #[test]
    fn test_invalid_glob_errors() {
        let toml_str = r#"
[daemon]
watch_dirs = [
    { path = "/tmp", include = "[invalid" },
]
"#;
        let raw: RawConfig = toml::from_str(toml_str).unwrap();
        let raw_daemon = raw.daemon.unwrap();
        let result = DaemonConfig::try_from(raw_daemon);
        assert!(result.is_err());
    }
}