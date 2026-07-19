use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

/// TUI configuration loaded from config.toml
#[derive(Debug, Clone, Default)]
pub struct TuiConfig {
    pub keybindings: Keybindings,
    pub theme: Theme,
    pub browsing: BrowsingConfig,
    pub daemon: DaemonAccessConfig,
    pub labels: LabelConfig,
    pub ai: AiConfig,
}

#[derive(Debug, Clone)]
pub struct AiConfig {
    pub language: String,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            language: "en-US".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DaemonAccessConfig {
    pub uds_path: String,
}

impl Default for DaemonAccessConfig {
    fn default() -> Self {
        Self {
            uds_path: argus_core::DEFAULT_UDS_PATH.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LabelConfig {
    pub custom_mappings: Vec<LabelMapping>,
}

#[derive(Debug, Clone)]
pub struct LabelMapping {
    pub pattern: String,
    pub label: String,
}

impl Default for LabelConfig {
    fn default() -> Self {
        Self {
            custom_mappings: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct BrowsingConfig {
    pub auto_scan_on_start: bool,
}

impl Default for BrowsingConfig {
    fn default() -> Self {
        Self {
            auto_scan_on_start: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Keybindings {
    pub move_up: String,
    pub move_down: String,
    pub enter_dir: String,
    pub leave_dir: String,
    pub sort_toggle: String,
    pub delete_item: String,
    pub focus_panel: String,
    pub quit: String,
    pub help: String,
    pub scan: String,
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub color_scheme: String,
    pub colors: HashMap<String, String>,
}

impl Default for Keybindings {
    fn default() -> Self {
        Self {
            move_up: "k".into(),
            move_down: "j".into(),
            enter_dir: "l".into(),
            leave_dir: "h".into(),
            sort_toggle: "o".into(),
            delete_item: "d".into(),
            focus_panel: "tab".into(),
            quit: "q".into(),
            help: "?".into(),
            scan: "s".into(),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        let mut colors = HashMap::new();
        colors.insert("growth_high".into(), "#FF4444".into());
        colors.insert("growth_medium".into(), "#FF8800".into());
        colors.insert("shrink_green".into(), "#44FF44".into());
        colors.insert("text_primary".into(), "#FFFFFF".into());

        Self {
            color_scheme: "system".into(),
            colors,
        }
    }
}

/// Raw TOML config structure for deserialization
#[derive(Debug, Deserialize)]
struct RawConfig {
    keybindings: Option<RawKeybindings>,
    theme: Option<RawTheme>,
    browsing: Option<RawBrowsing>,
    daemon: Option<RawDaemon>,
    labels: Option<RawLabels>,
    ai: Option<RawAi>,
}

#[derive(Debug, Deserialize)]
struct RawAi {
    language: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawLabels {
    custom_mappings: Option<Vec<RawLabelMapping>>,
}

#[derive(Debug, Deserialize)]
struct RawLabelMapping {
    pattern: String,
    label: String,
}

#[derive(Debug, Deserialize)]
struct RawDaemon {
    uds_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawBrowsing {
    auto_scan_on_start: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct RawKeybindings {
    move_up: Option<String>,
    move_down: Option<String>,
    enter_dir: Option<String>,
    leave_dir: Option<String>,
    sort_toggle: Option<String>,
    delete_item: Option<String>,
    focus_panel: Option<String>,
    quit: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawTheme {
    color_scheme: Option<String>,
    colors: Option<HashMap<String, String>>,
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
        Err(_) => return TuiConfig::default(),
    };

    let mut config = TuiConfig::default();

    if let Some(kb) = raw.keybindings {
        if let Some(v) = kb.move_up {
            config.keybindings.move_up = v;
        }
        if let Some(v) = kb.move_down {
            config.keybindings.move_down = v;
        }
        if let Some(v) = kb.enter_dir {
            config.keybindings.enter_dir = v;
        }
        if let Some(v) = kb.leave_dir {
            config.keybindings.leave_dir = v;
        }
        if let Some(v) = kb.sort_toggle {
            config.keybindings.sort_toggle = v;
        }
        if let Some(v) = kb.delete_item {
            config.keybindings.delete_item = v;
        }
        if let Some(v) = kb.focus_panel {
            config.keybindings.focus_panel = v;
        }
        if let Some(v) = kb.quit {
            config.keybindings.quit = v;
        }
    }

    if let Some(th) = raw.theme {
        if let Some(v) = th.color_scheme {
            config.theme.color_scheme = v;
        }
        if let Some(colors) = th.colors {
            config.theme.colors.extend(colors);
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
    }

    if let Some(l) = raw.labels {
        if let Some(mappings) = l.custom_mappings {
            config.labels.custom_mappings = mappings
                .into_iter()
                .map(|m| LabelMapping {
                    pattern: m.pattern,
                    label: m.label,
                })
                .collect();
        }
    }

    if let Some(a) = raw.ai {
        if let Some(v) = a.language {
            config.ai.language = v;
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
        assert_eq!(config.keybindings.move_up, "k");
        assert_eq!(config.keybindings.move_down, "j");
        assert_eq!(config.keybindings.enter_dir, "l");
        assert_eq!(config.keybindings.leave_dir, "h");
        assert_eq!(config.keybindings.quit, "q");
        assert_eq!(config.keybindings.help, "?");
        assert_eq!(config.theme.color_scheme, "system");
        assert!(config.theme.colors.contains_key("growth_high"));
        assert!(config.theme.colors.contains_key("text_primary"));
        assert!(!config.browsing.auto_scan_on_start);
        assert_eq!(config.daemon.uds_path, argus_core::DEFAULT_UDS_PATH);
        assert_eq!(config.ai.language, "en-US");
    }

    #[test]
    fn test_load_config_file_not_found_returns_default() {
        let path = Path::new("/nonexistent/path/config.toml");
        let config = load_config(path);
        assert_eq!(config.keybindings.move_up, "k");
    }

    #[test]
    fn test_load_config_empty_toml_returns_default() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "").unwrap();
        let config = load_config(&path);
        assert_eq!(config.keybindings.move_up, "k");
    }

    #[test]
    fn test_load_config_partial_keybindings() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[keybindings]
move_up = "w"
move_down = "s"
"#,
        )
        .unwrap();
        let config = load_config(&path);
        assert_eq!(config.keybindings.move_up, "w");
        assert_eq!(config.keybindings.move_down, "s");
        assert_eq!(config.keybindings.quit, "q"); // default
    }

    #[test]
    fn test_load_config_full_config() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r##"
[keybindings]
move_up = "w"
move_down = "s"
enter_dir = "right"
leave_dir = "left"
sort_toggle = "t"
delete_item = "x"
focus_panel = "f1"
quit = "ctrl-c"

[theme]
color_scheme = "dark"
colors.growth_high = "#FF0000"
colors.custom_key = "#00FF00"

[browsing]
auto_scan_on_start = true

[daemon]
uds_path = "/tmp/argus.sock"
"##,
        )
        .unwrap();
        let config = load_config(&path);
        assert_eq!(config.keybindings.move_up, "w");
        assert_eq!(config.keybindings.quit, "ctrl-c");
        assert_eq!(config.theme.color_scheme, "dark");
        assert_eq!(config.theme.colors.get("growth_high").unwrap(), "#FF0000");
        assert_eq!(config.theme.colors.get("custom_key").unwrap(), "#00FF00");
        assert!(config.browsing.auto_scan_on_start);
        assert_eq!(config.daemon.uds_path, "/tmp/argus.sock");
    }

    #[test]
    fn test_load_config_invalid_toml_returns_default() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[[[invalid toml").unwrap();
        let config = load_config(&path);
        assert_eq!(config.keybindings.move_up, "k");
    }

    #[test]
    fn test_load_config_theme_overrides_default_colors() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r##"
[theme]
colors.growth_high = "#112233"
"##,
        )
        .unwrap();
        let config = load_config(&path);
        assert_eq!(config.theme.colors.get("growth_high").unwrap(), "#112233");
        // other default colors still present
        assert_eq!(config.theme.colors.get("text_primary").unwrap(), "#FFFFFF");
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
    fn test_default_config_labels_empty() {
        let config = TuiConfig::default();
        assert!(config.labels.custom_mappings.is_empty());
    }

    #[test]
    fn test_load_config_labels_custom_mappings() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[labels]
custom_mappings = [
    { pattern = "*/.terraform/*", label = "iac-cache" },
    { pattern = "*.pyc", label = "python-bytecode" },
]
"#,
        )
        .unwrap();
        let config = load_config(&path);
        assert_eq!(config.labels.custom_mappings.len(), 2);
        assert_eq!(config.labels.custom_mappings[0].pattern, "*/.terraform/*");
        assert_eq!(config.labels.custom_mappings[0].label, "iac-cache");
        assert_eq!(config.labels.custom_mappings[1].pattern, "*.pyc");
        assert_eq!(config.labels.custom_mappings[1].label, "python-bytecode");
    }

    #[test]
    fn test_default_ai_language() {
        let config = TuiConfig::default();
        assert_eq!(config.ai.language, "en-US");
    }

    #[test]
    fn test_load_config_ai_language() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[ai]
language = "zh-CN"
"#,
        )
        .unwrap();
        let config = load_config(&path);
        assert_eq!(config.ai.language, "zh-CN");
    }
}
