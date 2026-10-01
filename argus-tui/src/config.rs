use std::path::Path;

use serde::Deserialize;

/// TUI configuration loaded from config.toml
///
/// Only options that are actually wired up are parsed here. Planned but
/// unimplemented sections (`[keybindings]` remapping, `[labels]` custom
/// mappings, `[theme].colors`, `[ignore]`) are intentionally absent —
/// parsing options the program ignores reads as if they worked.
#[derive(Debug, Clone, Default)]
pub struct TuiConfig {
    pub theme: Theme,
    pub browsing: BrowsingConfig,
    pub daemon: DaemonAccessConfig,
    pub ai: AiConfig,
}

#[derive(Debug, Clone)]
pub struct AiConfig {
    pub enabled: bool,
    pub api_url: String,
    pub api_key: String,
    pub model: String,
    pub language: String,
    /// Prompt-side budget: batches estimated above this are split into chunks.
    pub max_tokens_per_request: usize,
    /// Response-side cap sent as the API request's `max_tokens`. Independent
    /// of the prompt budget: a batch response carries several text fields per
    /// path, and reusing the small prompt budget truncated large batches.
    pub max_response_tokens: usize,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_url: String::new(),
            api_key: String::new(),
            model: "gpt-4o".into(),
            language: "en-US".into(),
            max_tokens_per_request: 4096,
            max_response_tokens: 8192,
        }
    }
}

impl AiConfig {
    /// Convert to core's AiConfig for API calls.
    pub fn to_core_config(&self) -> argus_core::AiConfig {
        argus_core::AiConfig {
            api_url: self.api_url.clone(),
            api_key: self.api_key.clone(),
            model: self.model.clone(),
            language: self.language.clone(),
            max_tokens_per_request: self.max_tokens_per_request,
            max_response_tokens: self.max_response_tokens,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DaemonAccessConfig {
    pub uds_path: String,
    /// How often (in seconds) to check for daemon availability when not connected.
    /// 0 disables periodic detection.
    pub detection_interval_secs: u64,
}

impl Default for DaemonAccessConfig {
    fn default() -> Self {
        Self {
            uds_path: argus_core::DEFAULT_UDS_PATH.to_string(),
            detection_interval_secs: 5,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct BrowsingConfig {
    pub auto_scan_on_start: bool,
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub color_scheme: String,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            color_scheme: "system".into(),
        }
    }
}

/// Raw TOML config structure for deserialization
#[derive(Debug, Deserialize)]
struct RawConfig {
    theme: Option<RawTheme>,
    browsing: Option<RawBrowsing>,
    daemon: Option<RawDaemon>,
    ai: Option<RawAi>,
}

#[derive(Debug, Deserialize)]
struct RawAi {
    enabled: Option<bool>,
    api_url: Option<String>,
    api_key: Option<String>,
    model: Option<String>,
    language: Option<String>,
    max_tokens_per_request: Option<usize>,
    max_response_tokens: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct RawDaemon {
    uds_path: Option<String>,
    detection_interval_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RawBrowsing {
    auto_scan_on_start: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct RawTheme {
    color_scheme: Option<String>,
}

/// Load TUI config from config.toml. Returns default if file doesn't exist.
pub fn load_config(path: &Path) -> TuiConfig {
    if !path.exists() {
        return TuiConfig::default();
    }

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return TuiConfig::default(),
    };

    let raw: RawConfig = match toml::from_str(&content) {
        Ok(r) => r,
        Err(e) => {
            // Printed before the TUI enters the alternate screen, so the line
            // is still in the terminal scrollback after quitting. A silent
            // fallback made typos like `[daemons]` disable every custom
            // option without a trace (review note #40).
            eprintln!(
                "argus: failed to parse config {}: {e}, using defaults",
                path.display()
            );
            return TuiConfig::default();
        }
    };

    let mut config = TuiConfig::default();

    if let Some(th) = raw.theme {
        if let Some(v) = th.color_scheme {
            config.theme.color_scheme = v;
        }
    }

    if let Some(b) = raw.browsing {
        if let Some(v) = b.auto_scan_on_start {
            config.browsing.auto_scan_on_start = v;
        }
    }

    if let Some(d) = raw.daemon {
        if let Some(v) = d.uds_path {
            config.daemon.uds_path = v;
        }
        if let Some(v) = d.detection_interval_secs {
            config.daemon.detection_interval_secs = v;
        }
    }

    if let Some(a) = raw.ai {
        if let Some(v) = a.enabled {
            config.ai.enabled = v;
        }
        if let Some(v) = a.api_url {
            config.ai.api_url = v;
        }
        if let Some(v) = a.api_key {
            config.ai.api_key = v;
        }
        if let Some(v) = a.model {
            config.ai.model = v;
        }
        if let Some(v) = a.language {
            config.ai.language = v;
        }
        if let Some(v) = a.max_tokens_per_request {
            config.ai.max_tokens_per_request = v;
        }
        if let Some(v) = a.max_response_tokens {
            config.ai.max_response_tokens = v;
        }
    }

    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = TuiConfig::default();
        assert_eq!(config.theme.color_scheme, "system");
        assert!(!config.browsing.auto_scan_on_start);
        assert_eq!(config.daemon.uds_path, argus_core::DEFAULT_UDS_PATH);
        assert_eq!(config.daemon.detection_interval_secs, 5);
        assert_eq!(config.ai.language, "en-US");
        assert!(!config.ai.enabled);
        assert!(config.ai.api_url.is_empty());
        assert!(config.ai.api_key.is_empty());
        assert_eq!(config.ai.model, "gpt-4o");
        assert_eq!(config.ai.max_tokens_per_request, 4096);
        assert_eq!(config.ai.max_response_tokens, 8192);
    }

    #[test]
    fn test_load_config_file_not_found_returns_default() {
        let path = Path::new("/nonexistent/path/config.toml");
        let config = load_config(path);
        assert_eq!(config.theme.color_scheme, "system");
    }

    #[test]
    fn test_load_config_empty_toml_returns_default() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "").unwrap();
        let config = load_config(&path);
        assert_eq!(config.theme.color_scheme, "system");
    }

    /// Legacy sections that were parsed but never wired up must be ignored
    /// without breaking the sections that do work.
    #[test]
    fn test_load_config_full_config() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r##"
[keybindings]
move_up = "w"

[theme]
color_scheme = "dark"
colors.growth_high = "#FF0000"

[browsing]
auto_scan_on_start = true

[daemon]
uds_path = "/tmp/argus.sock"
detection_interval_secs = 0

[labels]
custom_mappings = [{ pattern = "*.pyc", label = "python-bytecode" }]
"##,
        )
        .unwrap();
        let config = load_config(&path);
        assert_eq!(config.theme.color_scheme, "dark");
        assert!(config.browsing.auto_scan_on_start);
        assert_eq!(config.daemon.uds_path, "/tmp/argus.sock");
        assert_eq!(config.daemon.detection_interval_secs, 0);
    }

    #[test]
    fn test_load_config_invalid_toml_returns_default() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[[[invalid toml").unwrap();
        let config = load_config(&path);
        assert_eq!(config.theme.color_scheme, "system");
    }

    #[test]
    fn test_load_config_browsing_partial() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[browsing]
auto_scan_on_start = true
"#,
        )
        .unwrap();
        let config = load_config(&path);
        assert!(config.browsing.auto_scan_on_start);
    }

    #[test]
    fn test_default_ai_config() {
        let config = TuiConfig::default();
        assert_eq!(config.ai.language, "en-US");
        assert!(!config.ai.enabled);
        assert!(config.ai.api_url.is_empty());
        assert_eq!(config.ai.model, "gpt-4o");
        assert_eq!(config.ai.max_tokens_per_request, 4096);
        assert_eq!(config.ai.max_response_tokens, 8192);
    }

    /// The prompt budget and the response cap are independent knobs: the
    /// response cap used to reuse the prompt budget, truncating large batch
    /// responses (several text fields per path) and burning retries.
    #[test]
    fn test_load_config_ai_response_tokens_independent() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[ai]
enabled = true
api_url = "https://api.openai.com/v1/chat/completions"
api_key = "sk-xxx"
model = "gpt-4o-mini"
language = "zh-CN"
max_tokens_per_request = 2048
max_response_tokens = 16384
"#,
        )
        .unwrap();
        let config = load_config(&path);
        assert_eq!(config.ai.max_tokens_per_request, 2048);
        assert_eq!(config.ai.max_response_tokens, 16384);

        let core = config.ai.to_core_config();
        assert_eq!(core.max_tokens_per_request, 2048);
        assert_eq!(core.max_response_tokens, 16384);
    }

    #[test]
    fn test_load_config_ai_full() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[ai]
enabled = true
api_url = "https://api.openai.com/v1/chat/completions"
api_key = "sk-xxx"
model = "gpt-4o-mini"
language = "zh-CN"
max_tokens_per_request = 2048
"#,
        )
        .unwrap();
        let config = load_config(&path);
        assert!(config.ai.enabled);
        assert_eq!(
            config.ai.api_url,
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(config.ai.api_key, "sk-xxx");
        assert_eq!(config.ai.model, "gpt-4o-mini");
        assert_eq!(config.ai.language, "zh-CN");
        assert_eq!(config.ai.max_tokens_per_request, 2048);
    }
}
