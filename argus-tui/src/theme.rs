use ratatui::style::Color;

/// Semantic color theme for the entire TUI.
/// `dark()` / `light()` constructors for two modes.
/// Resolved at startup from config + terminal-light detection.
#[derive(Debug, Clone)]
pub struct ColorTheme {
    // Backgrounds
    pub bg: Color,
    pub popup_bg: Color,
    pub selected_bg: Color,
    pub match_bg: Color,
    pub selection_fg: Color,
    pub focus_bg: Color,
    pub focus_fg: Color,

    // Text hierarchy
    pub text: Color,
    pub text_secondary: Color,
    pub text_tertiary: Color,
    pub text_highlight: Color,

    // Semantic accents
    pub accent: Color,
    pub success: Color,
    pub danger: Color,
    pub warning: Color,

    // File types
    pub symlink: Color,
    pub hidden: Color,

    // Search
    pub search_match_selected_bg: Color,

    // UI elements
    pub spinner: Color,
    pub scrollbar: Color,
    pub border_unfocused: Color,
    pub popup_border_normal: Color,

    // Size units
    pub unit_b: Color,
    pub unit_kb: Color,
    pub unit_mb: Color,
    pub unit_gb: Color,
}

impl ColorTheme {
    /// Dark theme (default for dark terminals)
    pub fn dark() -> Self {
        Self {
            bg: Color::Black,
            popup_bg: Color::Black,
            selected_bg: Color::Black,
            match_bg: Color::Cyan,
            selection_fg: Color::White,
            focus_bg: Color::LightYellow,
            focus_fg: Color::Black,
            text: Color::White,
            text_secondary: Color::Gray,
            text_tertiary: Color::DarkGray,
            text_highlight: Color::Yellow,
            accent: Color::Cyan,
            success: Color::Green,
            danger: Color::Red,
            warning: Color::Yellow,
            symlink: Color::Magenta,
            hidden: Color::DarkGray,
            search_match_selected_bg: Color::Green,
            spinner: Color::Magenta,
            scrollbar: Color::DarkGray,
            border_unfocused: Color::DarkGray,
            popup_border_normal: Color::White,
            unit_b: Color::Green,
            unit_kb: Color::Yellow,
            unit_mb: Color::Rgb(255, 165, 0),
            unit_gb: Color::Red,
        }
    }

    /// Light theme (for light terminals)
    pub fn light() -> Self {
        Self {
            bg: Color::White,
            popup_bg: Color::White,
            selected_bg: Color::Rgb(220, 235, 255),
            match_bg: Color::Rgb(200, 230, 255),
            selection_fg: Color::Black,
            focus_bg: Color::Rgb(200, 220, 255),
            focus_fg: Color::Black,
            text: Color::Black,
            text_secondary: Color::Rgb(80, 80, 80),
            text_tertiary: Color::Rgb(140, 140, 140),
            text_highlight: Color::Rgb(180, 120, 0),
            accent: Color::Rgb(0, 100, 180),
            success: Color::Rgb(0, 130, 0),
            danger: Color::Rgb(180, 0, 0),
            warning: Color::Rgb(180, 120, 0),
            symlink: Color::Rgb(160, 50, 160),
            hidden: Color::Rgb(140, 140, 140),
            search_match_selected_bg: Color::Rgb(180, 230, 180),
            spinner: Color::Rgb(160, 50, 160),
            scrollbar: Color::Rgb(180, 180, 180),
            border_unfocused: Color::Rgb(180, 180, 180),
            popup_border_normal: Color::Rgb(80, 80, 80),
            unit_b: Color::Rgb(0, 130, 0),
            unit_kb: Color::Rgb(180, 120, 0),
            unit_mb: Color::Rgb(200, 100, 0),
            unit_gb: Color::Rgb(180, 0, 0),
        }
    }
}

/// Detect terminal luminosity and return appropriate theme.
/// Falls back to dark when unknown.
pub fn detect_theme(config_scheme: &str) -> ColorTheme {
    match config_scheme {
        "dark" => return ColorTheme::dark(),
        "light" => return ColorTheme::light(),
        _ => {}
    }

    // luma: 0=black(dark), 1=white(light). Threshold 0.5.
    match terminal_light::luma() {
        Ok(luma) if luma > 0.5 => ColorTheme::light(),
        _ => ColorTheme::dark(),
    }
}
