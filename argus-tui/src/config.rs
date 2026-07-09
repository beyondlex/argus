use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

/// TUI configuration loaded from config.toml
#[derive(Debug, Clone)]
pub struct TuiConfig {
    pub keybindings: Keybindings,
    pub theme: Theme,
    pub browsing: BrowsingConfig,
}

#[derive(Debug, Clone, Default)]
pub struct BrowsingConfig {
    pub auto_scan_on_start: bool,
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

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            keybindings: Keybindings::default(),
            theme: Theme::default(),
            browsing: BrowsingConfig::default(),
        }
    }
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

    config
}
