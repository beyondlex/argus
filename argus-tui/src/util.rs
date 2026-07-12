use std::path::{Path, PathBuf};
use std::time::Duration;

use ratatui::style::{Color, Style};
use ratatui::text::Span;

/// Map common key names to symbolic representations
pub fn key_symbol(key: &'static str) -> &'static str {
    match key {
        "Tab" => "⇥",
        "S-Tab" => "⇧⇥",
        "Esc" => "⎋",
        "Enter" => "⏎",
        _ => key,
    }
}

/// Create a styled key hint pair: `key` (green) `label` (DarkGray)
pub fn key_hint(key: &'static str, label: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(
            format!(" {}", key_symbol(key)),
            Style::default().fg(Color::Green),
        ),
        Span::styled(format!(" {} ", label), Style::default().fg(Color::DarkGray)),
    ]
}

/// Create a sequence of key hint pairs separated by double spaces
pub fn key_hints(hints: &[(&'static str, &str)]) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (i, (key, label)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        spans.extend(key_hint(key, label));
    }
    spans
}

/// Format bytes into human-readable string (e.g., "1.5 GB", "800 KB")
pub fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;

    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }

    if unit_idx == 0 {
        format!("{} {}", bytes, UNITS[unit_idx])
    } else {
        format!("{:.2} {}", size, UNITS[unit_idx])
    }
}

/// Format delta as signed human-readable string
pub fn format_delta(delta: i64) -> String {
    if delta >= 0 {
        format!("+{}", format_size(delta as u64))
    } else {
        format!("-{}", format_size(delta.unsigned_abs()))
    }
}

/// Decide which size label a tree node should display.
///
/// Rules:
/// - Files always show real size.
/// - Ordinary directories without scan data show `?`.
/// - Output is always right-aligned to 11 characters.
pub fn display_size_label(is_dir: bool, has_scan_data: bool, current_size: u64) -> String {
    let s = if is_dir && !has_scan_data {
        "?".to_string()
    } else {
        format_size(current_size)
    };
    format!("{:>11}", s)
}

/// Format a count with thousands separators.
pub fn format_count(count: u64) -> String {
    let digits = count.to_string();
    let mut out = String::new();
    for (idx, ch) in digits.chars().rev().enumerate() {
        if idx > 0 && idx % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

/// Format a duration as seconds with two decimal places.
pub fn format_duration(duration: Duration) -> String {
    format!("{:.2}s", duration.as_secs_f64())
}

/// Display a path relative to $HOME when possible.
pub fn display_path(path: &Path) -> String {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return path.display().to_string();
    };

    if let Ok(relative) = path.strip_prefix(&home) {
        if relative.as_os_str().is_empty() {
            "~".to_string()
        } else {
            format!("~/{}", relative.display())
        }
    } else {
        path.display().to_string()
    }
}

/// Get the default snapshot directory
pub fn default_snapshots_dir() -> std::path::PathBuf {
    dirs_config_path().join("argus").join("snapshots")
}

/// Get the default config file path
pub fn default_config_path() -> std::path::PathBuf {
    dirs_config_path().join("argus").join("config.toml")
}

/// Get the config directory (~/.config/argus or XDG_CONFIG_HOME)
fn dirs_config_path() -> std::path::PathBuf {
    if let Ok(val) = std::env::var("XDG_CONFIG_HOME") {
        std::path::PathBuf::from(val)
    } else if let Ok(home) = std::env::var("HOME") {
        std::path::PathBuf::from(home).join(".config")
    } else {
        std::path::PathBuf::from(".")
    }
}

/// Determine if a path is protected (system blacklist)
pub fn is_protected_path(path: &Path) -> bool {
    let protected: &[&str] = if cfg!(target_os = "macos") {
        &[
            "/System",
            "/System/Volumes",
            "/usr/bin",
            "/usr/lib",
            "/bin",
            "/sbin",
            "/etc",
            "/var/db",
        ]
    } else {
        &[
            "/boot", "/etc", "/dev", "/proc", "/sys", "/usr/bin", "/usr/lib", "/bin", "/sbin",
            "/lib", "/lib64",
        ]
    };

    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let canonical_str = canonical.to_string_lossy();
    protected
        .iter()
        .any(|p| canonical_str == *p || canonical_str.starts_with(&format!("{}/", p)))
}

/// Extract unit suffix from a formatted size string (e.g., "1.23 MB" -> "MB")
pub fn extract_unit(s: &str) -> &str {
    s.trim().split_whitespace().last().unwrap_or("B")
}

/// Map unit to color for positive delta display (entire string colored).
/// B→Gray, KB→Yellow, MB→Orange, GB→Red
pub fn delta_unit_color(unit: &str) -> Color {
    match unit {
        "B" => Color::Gray,
        "KB" => Color::Yellow,
        "MB" => Color::Rgb(255, 165, 0),
        "GB" => Color::Red,
        _ => Color::Gray,
    }
}

/// Map unit to color for filesize display (unit part only).
/// B→Green, KB→Yellow, MB→Orange, GB→Red
pub fn filesize_unit_color(unit: &str) -> Color {
    match unit {
        "B" => Color::Green,
        "KB" => Color::Yellow,
        "MB" => Color::Rgb(255, 165, 0),
        "GB" => Color::Red,
        _ => Color::Green,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_size_label_file_shows_real_size() {
        assert_eq!(display_size_label(false, false, 1024), "    1.00 KB");
    }

    #[test]
    fn test_display_size_label_unscanned_dir_shows_dash() {
        assert_eq!(display_size_label(true, false, 0), "          ?");
    }

    #[test]
    fn test_display_size_label_scanned_dir_shows_size() {
        assert_eq!(display_size_label(true, true, 2048), "    2.00 KB");
    }

    #[test]
    fn test_format_count_inserts_commas() {
        assert_eq!(format_count(123456), "123,456");
    }

    #[test]
    fn test_format_duration_formats_seconds() {
        assert_eq!(format_duration(Duration::from_millis(32_450)), "32.45s");
    }
}
