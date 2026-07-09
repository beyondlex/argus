use std::path::Path;

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
    } else if let Some(home) = std::env::var("HOME").ok() {
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

/// Count total files in a directory tree
pub fn count_files<F>(
    node: &F,
    is_dir_fn: fn(&F) -> bool,
    children_fn: fn(&F) -> &std::collections::HashMap<String, F>,
) -> u64 {
    let mut count = 0u64;
    if !is_dir_fn(node) {
        count += 1;
    }
    for child in children_fn(node).values() {
        count += count_files(child, is_dir_fn, children_fn);
    }
    count
}

/// Count files in a FileNode tree
pub fn count_file_nodes(node: &argus_core::FileNode) -> u64 {
    count_files(node, |n| n.is_dir, |n| &n.children)
}

/// Count files in a DiffNode tree
pub fn count_diff_nodes(node: &argus_core::DiffNode) -> u64 {
    count_files(node, |n| n.is_dir, |n| &n.children)
}
