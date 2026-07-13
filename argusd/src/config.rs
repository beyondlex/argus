use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct DaemonConfig {
    #[serde(default)]
    pub watch_dirs: Vec<PathBuf>,
    #[serde(default = "default_debounce_seconds")]
    pub debounce_seconds: u64,
    #[serde(default = "default_uds_path")]
    pub uds_path: String,
    #[serde(default)]
    pub snapshot_retention: SnapshotRetention,
    #[serde(default = "default_delta_retention_days")]
    pub delta_retention_days: u64,
    #[serde(default)]
    pub consolidation: ConsolidationConfig,
    #[serde(default)]
    pub log_level: Option<String>,
    #[serde(default)]
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
                PathBuf::from("/Users/lex/Downloads"),
                PathBuf::from("/Users/lex/Desktop"),
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
struct RawConfig {
    daemon: Option<DaemonConfig>,
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
        Ok(raw) => raw.daemon.unwrap_or_default(),
        Err(e) => {
            tracing::warn!("failed to parse config {path:?}: {e}, using defaults");
            DaemonConfig::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let config = raw.daemon.unwrap();
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
        let config = raw.daemon.unwrap();
        assert!(config.log_enabled);
        assert_eq!(config.log_level, Some("warn".to_string()));
    }
}
