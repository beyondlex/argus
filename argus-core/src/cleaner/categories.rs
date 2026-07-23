use std::path::{Path, PathBuf};

use super::safety::RiskLevel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetCategory {
    AppCache,
    BrowserCache,
    DevTools,
    SystemLogs,
    TempFiles,
    Trash,
}

impl TargetCategory {
    pub fn label(&self) -> &'static str {
        match self {
            TargetCategory::AppCache => "App Cache",
            TargetCategory::BrowserCache => "Browser Cache",
            TargetCategory::DevTools => "Developer Tools",
            TargetCategory::SystemLogs => "System Logs",
            TargetCategory::TempFiles => "Temp Files",
            TargetCategory::Trash => "Trash",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CleanTarget {
    pub id: String,
    pub label: String,
    pub paths: Vec<PathBuf>,
    pub risk: RiskLevel,
    pub category: TargetCategory,
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn library_cache() -> Option<PathBuf> {
    home_dir().map(|h| h.join("Library/Caches"))
}

fn library_logs() -> Option<PathBuf> {
    home_dir().map(|h| h.join("Library/Logs"))
}

#[cfg(target_os = "macos")]
pub fn default_clean_targets() -> Vec<CleanTarget> {
    let mut targets = Vec::new();

    if let Some(cache) = library_cache() {
        targets.push(CleanTarget {
            id: "user-app-cache".into(),
            label: "User App Cache".into(),
            paths: vec![cache.clone()],
            risk: RiskLevel::Low,
            category: TargetCategory::AppCache,
        });
    }

    if let Some(logs) = library_logs() {
        targets.push(CleanTarget {
            id: "system-logs".into(),
            label: "System Logs".into(),
            paths: vec![logs.clone()],
            risk: RiskLevel::Low,
            category: TargetCategory::SystemLogs,
        });
    }

    if let Some(home) = home_dir() {
        targets.push(CleanTarget {
            id: "browser-cache-chrome".into(),
            label: "Chrome Cache".into(),
            paths: vec![
                home.join("Library/Caches/Google/Chrome"),
                home.join("Library/Caches/com.google.Chrome"),
            ],
            risk: RiskLevel::Low,
            category: TargetCategory::BrowserCache,
        });

        targets.push(CleanTarget {
            id: "browser-cache-safari".into(),
            label: "Safari Cache".into(),
            paths: vec![
                home.join("Library/Caches/com.apple.Safari"),
                home.join("Library/Caches/com.apple.WebKit.WebContent"),
            ],
            risk: RiskLevel::Low,
            category: TargetCategory::BrowserCache,
        });

        targets.push(CleanTarget {
            id: "browser-cache-firefox".into(),
            label: "Firefox Cache".into(),
            paths: vec![
                home.join("Library/Caches/Mozilla"),
                home.join("Library/Caches/Firefox"),
            ],
            risk: RiskLevel::Low,
            category: TargetCategory::BrowserCache,
        });

        targets.push(CleanTarget {
            id: "dev-xcode".into(),
            label: "Xcode Derived Data".into(),
            paths: vec![
                home.join("Library/Developer/Xcode/DerivedData"),
                home.join("Library/Developer/Xcode/Archives"),
                home.join("Library/Developer/CoreSimulator/Caches"),
            ],
            risk: RiskLevel::Low,
            category: TargetCategory::DevTools,
        });

        targets.push(CleanTarget {
            id: "dev-node".into(),
            label: "Node.js Cache".into(),
            paths: vec![
                home.join(".npm/_cacache"),
                home.join("Library/Caches/node-gyp"),
            ],
            risk: RiskLevel::Low,
            category: TargetCategory::DevTools,
        });

        targets.push(CleanTarget {
            id: "dev-rust".into(),
            label: "Rust Cache".into(),
            paths: vec![
                home.join(".cargo/registry/cache"),
                home.join(".cargo/git/db"),
            ],
            risk: RiskLevel::Low,
            category: TargetCategory::DevTools,
        });

        targets.push(CleanTarget {
            id: "temp-files".into(),
            label: "Temp Files".into(),
            paths: vec![
                PathBuf::from("/tmp"),
                home.join("Library/Caches/com.apple.helpd"),
            ],
            risk: RiskLevel::Low,
            category: TargetCategory::TempFiles,
        });

        targets.push(CleanTarget {
            id: "trash".into(),
            label: "Trash".into(),
            paths: vec![home.join(".Trash"), PathBuf::from("/Users/Shared/Trash")],
            risk: RiskLevel::Safe,
            category: TargetCategory::Trash,
        });
    }

    targets
}

#[cfg(not(target_os = "macos"))]
pub fn default_clean_targets() -> Vec<CleanTarget> {
    Vec::new()
}

pub fn scan_target_size(target: &CleanTarget) -> Result<(u64, Vec<PathBuf>), std::io::Error> {
    let mut total = 0u64;
    let mut existing = Vec::new();
    for p in &target.paths {
        if p.exists() {
            existing.push(p.clone());
            total += dir_size(p)?;
        }
    }
    Ok((total, existing))
}

fn dir_size(path: &Path) -> Result<u64, std::io::Error> {
    let mut total = 0u64;
    if path.is_file() {
        return Ok(std::fs::metadata(path)?.len());
    }
    if path.is_dir() {
        let mut dirs = vec![path.to_path_buf()];
        while let Some(dir) = dirs.pop() {
            let read_dir = match std::fs::read_dir(&dir) {
                Ok(r) => r,
                Err(_) => continue,
            };
            for entry in read_dir.flatten() {
                let ft = match entry.file_type() {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                if ft.is_symlink() {
                    continue;
                }
                if ft.is_dir() {
                    dirs.push(entry.path());
                } else if ft.is_file() {
                    total += match entry.metadata() {
                        Ok(m) => m.len(),
                        Err(_) => 0,
                    };
                }
            }
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_targets_have_ids() {
        let targets = default_clean_targets();
        for t in &targets {
            assert!(!t.id.is_empty(), "target id empty: {:?}", t.label);
            assert!(!t.paths.is_empty(), "target {:?} has no paths", t.id);
        }
    }

    #[test]
    fn test_target_category_labels() {
        assert_eq!(TargetCategory::AppCache.label(), "App Cache");
        assert_eq!(TargetCategory::Trash.label(), "Trash");
    }

    #[test]
    fn test_dir_size_nonexistent() {
        let p = Path::new("/nonexistent_path_xyz");
        assert_eq!(dir_size(p).unwrap_or(0), 0);
    }

    #[test]
    fn test_dir_size_file() {
        let tmp = std::env::temp_dir();
        let f = tmp.join("_test_cleaner_size");
        std::fs::write(&f, b"hello").unwrap();
        assert_eq!(dir_size(&f).unwrap(), 5);
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn test_scan_target_size_nonexistent() {
        let target = CleanTarget {
            id: "nonexistent".into(),
            label: "Nonexistent".into(),
            paths: vec![PathBuf::from("/_nonexistent_path_xyz99/")],
            risk: RiskLevel::Safe,
            category: TargetCategory::TempFiles,
        };
        let (size, paths) = scan_target_size(&target).unwrap();
        assert_eq!(size, 0);
        assert!(paths.is_empty());
    }
}
