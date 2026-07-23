use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use super::audit::{log_operation, AuditEntry, AuditOp};
use super::cleaner::{CleanItem, CleanReport};
use super::safety::{check_deletion_allowed, RiskLevel};

#[derive(Debug, Clone)]
pub struct AppInfo {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    pub last_used: Option<DateTime<Utc>>,
    pub is_from_app_store: bool,
}

#[derive(Debug, Clone)]
pub struct AppLeftovers {
    pub app: AppInfo,
    pub leftover_paths: Vec<PathBuf>,
    pub total_leftover_bytes: u64,
}

const APP_DIRS: &[&str] = &[
    "/Applications",
    "/Applications/Utilities",
    "/System/Applications",
];

const LEFTOVER_RELATIVE_PATHS: &[&str] = &[
    "Library/Application Support",
    "Library/Caches",
    "Library/Preferences",
    "Library/Logs",
    "Library/WebKit",
    "Library/Cookies",
    "Library/Saved Application State",
    "Library/Containers",
];

fn bundle_id_for_app(app_path: &Path) -> Option<String> {
    let plist_path = app_path.join("Contents/Info.plist");
    if !plist_path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(&plist_path).ok()?;

    if let Some(start) = content.find("<key>CFBundleIdentifier</key>") {
        let after = &content[start + 30..];
        if let Some(val_start) = after.find("<string>") {
            let from_val = &after[val_start + 8..];
            if let Some(val_end) = from_val.find("</string>") {
                return Some(from_val[..val_end].to_string());
            }
        }
    }
    None
}

fn app_name_from_path(app_path: &Path) -> String {
    app_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Unknown".to_string())
}

fn app_size(app_path: &Path) -> u64 {
    let mut total = 0u64;
    let mut dirs = vec![app_path.to_path_buf()];
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
    total
}

pub fn find_installed_apps() -> Result<Vec<AppInfo>, String> {
    let mut apps = Vec::new();
    for dir_str in APP_DIRS {
        let dir = Path::new(dir_str);
        if !dir.exists() {
            continue;
        }
        let read_dir = match std::fs::read_dir(dir) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("app") {
                continue;
            }
            let name = app_name_from_path(&path);
            let size = app_size(&path);
            let id = bundle_id_for_app(&path).unwrap_or_else(|| format!("unknown.{}", name));
            apps.push(AppInfo {
                id,
                name,
                path,
                size,
                last_used: None,
                is_from_app_store: false,
            });
        }
    }
    apps.sort_by(|a, b| b.size.cmp(&a.size));
    Ok(apps)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub fn find_leftovers(app: &AppInfo) -> Result<AppLeftovers, String> {
    let home = home_dir().ok_or_else(|| "HOME not set".to_string())?;
    let mut leftovers = Vec::new();
    let mut total = 0u64;

    let bundle_id = &app.id;
    let app_name = &app.name;

    for rel in LEFTOVER_RELATIVE_PATHS {
        let base = home.join(rel);

        for candidate_name in &[bundle_id.as_str(), app_name.as_str()] {
            let p = base.join(candidate_name);
            if p.exists() {
                let size = dir_size(&p);
                leftovers.push(p);
                total += size;
            }
        }
    }

    let app_support = home.join("Library/Application Support");
    if let Ok(read_dir) = std::fs::read_dir(&app_support) {
        for entry in read_dir.flatten() {
            let p = entry.path();
            if leftovers.contains(&p) {
                continue;
            }
            let fname = p
                .file_name()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let matches = fname.contains(&app_name.to_lowercase())
                || fname.contains(&bundle_id.to_lowercase().replace('.', ""));
            if matches && p.exists() {
                let size = dir_size(&p);
                leftovers.push(p);
                total += size;
            }
        }
    }

    Ok(AppLeftovers {
        app: app.clone(),
        leftover_paths: leftovers,
        total_leftover_bytes: total,
    })
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    if path.is_file() {
        return std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
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
    total
}

pub fn uninstall_app(app: &AppInfo, remove_leftovers: bool) -> Result<CleanReport, String> {
    let mut items = vec![CleanItem {
        path: app.path.clone(),
        size: app.size,
        risk: RiskLevel::Low,
        target_id: "uninstall".into(),
    }];

    if remove_leftovers {
        let leftovers = find_leftovers(app)?;
        for p in leftovers.leftover_paths {
            let size = dir_size(&p);
            items.push(CleanItem {
                path: p,
                size,
                risk: RiskLevel::Low,
                target_id: "uninstall-leftover".into(),
            });
        }
    }

    let mut report = CleanReport {
        total_attempted: items.len() as u64,
        total_succeeded: 0,
        total_failed: 0,
        freed_bytes: 0,
        errors: Vec::new(),
    };

    for item in &items {
        if let Err(e) = check_deletion_allowed(&item.path) {
            report.total_failed += 1;
            report.errors.push((item.path.clone(), e));
            continue;
        }
        match trash::delete(&item.path) {
            Ok(()) => {
                report.total_succeeded += 1;
                report.freed_bytes += item.size;
            }
            Err(e) => {
                report.total_failed += 1;
                report
                    .errors
                    .push((item.path.clone(), format!("trash: {e}")));
            }
        }
    }

    let entry = AuditEntry {
        timestamp: chrono::Utc::now(),
        operation: AuditOp::Uninstall,
        paths: items.iter().map(|i| i.path.clone()).collect(),
        total_bytes: report.freed_bytes,
        success: report.total_failed == 0,
        error: if report.total_failed > 0 {
            Some(format!("{} failures", report.total_failed))
        } else {
            None
        },
    };
    let _ = log_operation(&entry);

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_apps_returns_list() {
        let apps = find_installed_apps().unwrap();
        for app in &apps {
            assert!(!app.name.is_empty());
            assert!(app.path.to_string_lossy().ends_with(".app"));
        }
    }

    #[test]
    fn test_app_name_from_path() {
        let p = Path::new("/Applications/Firefox.app");
        assert_eq!(app_name_from_path(p), "Firefox");
    }

    #[test]
    fn test_dir_size_nonexistent() {
        assert_eq!(dir_size(Path::new("/_nonexistent_xyz_")), 0);
    }

    #[test]
    fn test_find_leftovers_nonexistent_app() {
        let app = AppInfo {
            id: "com.nonexistent.xyz".into(),
            name: "NonexistentAppXYZ".into(),
            path: PathBuf::from("/Applications/NonexistentAppXYZ.app"),
            size: 0,
            last_used: None,
            is_from_app_store: false,
        };
        let leftovers = find_leftovers(&app).unwrap();
        assert!(leftovers.leftover_paths.is_empty());
        assert_eq!(leftovers.total_leftover_bytes, 0);
    }
}
