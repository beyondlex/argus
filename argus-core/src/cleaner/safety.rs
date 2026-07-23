use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RiskLevel {
    Safe,
    Low,
    Medium,
    High,
}

impl RiskLevel {
    pub fn requires_typed_confirmation(&self) -> bool {
        matches!(self, RiskLevel::Medium | RiskLevel::High)
    }

    pub fn label(&self) -> &'static str {
        match self {
            RiskLevel::Safe => "safe",
            RiskLevel::Low => "low",
            RiskLevel::Medium => "medium",
            RiskLevel::High => "high",
        }
    }

    pub fn from_label(s: &str) -> Option<Self> {
        match s {
            "safe" => Some(RiskLevel::Safe),
            "low" => Some(RiskLevel::Low),
            "medium" => Some(RiskLevel::Medium),
            "high" => Some(RiskLevel::High),
            _ => None,
        }
    }
}

#[cfg(target_os = "macos")]
static MACOS_PROTECTED_PREFIXES: &[&str] = &[
    "/System",
    "/System/Volumes",
    "/usr/bin",
    "/usr/lib",
    "/bin",
    "/sbin",
    "/etc",
    "/private/etc",
    "/var/db",
    "/private/var/db",
];

#[cfg(target_os = "linux")]
static LINUX_PROTECTED_PREFIXES: &[&str] = &[
    "/boot", "/etc", "/dev", "/proc", "/sys", "/usr/bin", "/usr/lib", "/bin", "/sbin", "/lib",
    "/lib64",
];

fn protected_prefixes() -> &'static [&'static str] {
    #[cfg(target_os = "macos")]
    {
        MACOS_PROTECTED_PREFIXES
    }
    #[cfg(target_os = "linux")]
    {
        LINUX_PROTECTED_PREFIXES
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        &[]
    }
}

pub fn is_protected(path: &Path) -> bool {
    let canonical = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => path.to_path_buf(),
    };
    let path_str = canonical.to_string_lossy();
    for prefix in protected_prefixes() {
        if path_str == *prefix || path_str.starts_with(&format!("{}/", prefix)) {
            return true;
        }
    }
    false
}

fn user_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub fn classify_risk(path: &Path) -> RiskLevel {
    if is_protected(path) {
        return RiskLevel::High;
    }
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let path_str = canonical.to_string_lossy();

    let home = match user_home() {
        Some(h) => h,
        None => return RiskLevel::Medium,
    };
    let home_str = home.to_string_lossy();

    if !path_str.starts_with(home_str.as_ref()) {
        if path_str.starts_with("/var/tmp")
            || path_str.starts_with("/tmp")
            || path_str.starts_with("/Library")
        {
            return RiskLevel::Medium;
        }
        return RiskLevel::Low;
    }

    let under_library = path_str.contains("/Library/");
    if under_library {
        if path_str.contains("/Caches")
            || path_str.contains("/Logs")
            || path_str.contains("/Temp")
            || path_str.contains("/Trash")
        {
            return RiskLevel::Low;
        }
        if path_str.contains("/Application Support") || path_str.contains("/Preferences") {
            return RiskLevel::Medium;
        }
        return RiskLevel::Low;
    }

    if path_str.contains("/.Trash") {
        return RiskLevel::Safe;
    }

    RiskLevel::Safe
}

pub fn check_deletion_allowed(path: &Path) -> Result<(), String> {
    if is_protected(path) {
        return Err(format!("path is protected: {}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_protected_system_paths() {
        assert!(is_protected(Path::new("/System")));
        assert!(is_protected(Path::new("/System/Library")));
        assert!(is_protected(Path::new("/usr/bin")));
        assert!(!is_protected(Path::new("/tmp")));
        assert!(!is_protected(Path::new("/var/folders")));
    }

    #[test]
    fn test_non_protected_user_paths() {
        assert!(!is_protected(Path::new("/Users/test/Downloads")));
    }

    #[test]
    fn test_risk_user_cache_is_low() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        let p = Path::new(&home).join("Library/Caches/com.example.app");
        assert_eq!(classify_risk(&p), RiskLevel::Low);
    }

    #[test]
    fn test_risk_trash_is_safe() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        let p = Path::new(&home).join(".Trash");
        let risk = classify_risk(&p);
        assert!(risk == RiskLevel::Safe || risk == RiskLevel::Low);
    }

    #[test]
    fn test_risk_system_is_high() {
        assert_eq!(classify_risk(Path::new("/System")), RiskLevel::High);
    }

    #[test]
    fn test_check_deletion_allowed_rejects_protected() {
        assert!(check_deletion_allowed(Path::new("/System")).is_err());
        assert!(check_deletion_allowed(Path::new("/tmp")).is_ok());
    }

    #[test]
    fn test_risk_level_labels() {
        assert_eq!(RiskLevel::Safe.label(), "safe");
        assert_eq!(RiskLevel::High.label(), "high");
        assert_eq!(RiskLevel::from_label("medium"), Some(RiskLevel::Medium));
        assert_eq!(RiskLevel::from_label("unknown"), None);
    }

    #[test]
    fn test_requires_confirmation() {
        assert!(!RiskLevel::Safe.requires_typed_confirmation());
        assert!(!RiskLevel::Low.requires_typed_confirmation());
        assert!(RiskLevel::Medium.requires_typed_confirmation());
        assert!(RiskLevel::High.requires_typed_confirmation());
    }
}
