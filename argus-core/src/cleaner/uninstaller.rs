use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use super::audit::AuditOp;
use super::cleaner::{CleanItem, CleanReport};
use super::dir_size;
use super::safety::RiskLevel;

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

const BUNDLE_ID_KEY: &str = "<key>CFBundleIdentifier</key>";

fn bundle_id_for_app(app_path: &Path) -> Option<String> {
    let plist_path = app_path.join("Contents/Info.plist");
    if !plist_path.exists() {
        return None;
    }
    if let Ok(content) = std::fs::read_to_string(&plist_path) {
        if let Some(id) = bundle_id_from_plist_xml(&content) {
            return Some(id);
        }
    }
    // Binary plists fail the UTF-8 read (Xcode, Pages, Tunnelblick, …) and
    // used to fall through to "unknown.<name>", losing bundle-id-based
    // leftover detection for exactly the big vendor apps whose containers
    // are named by the dotted id. plutil is the OS's plist converter — the
    // same spawn-per-app cost class as the existing mdls call, and only
    // reached for plists the plain scan could not read.
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("plutil")
            .args(["-convert", "xml1", "-o", "-"])
            .arg(&plist_path)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        bundle_id_from_plist_xml(&String::from_utf8_lossy(&output.stdout))
    }
    #[cfg(not(target_os = "macos"))]
    None
}

/// Scan plist XML (plain on disk, or plutil-converted) for the
/// CFBundleIdentifier string value. The offset must skip exactly the key
/// tag — a hardcoded +30 used to eat one byte past '>', so a minified plist
/// ("<key>…</key><string>…") failed the "<string>" lookup below.
fn bundle_id_from_plist_xml(content: &str) -> Option<String> {
    if let Some(start) = content.find(BUNDLE_ID_KEY) {
        let after = &content[start + BUNDLE_ID_KEY.len()..];
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
    // Same walk as the shared dir_size (symlink dirs not followed, symlink
    // files not counted); the private copy had drifted from it.
    super::dir_size(app_path)
}

fn last_used_date(app_path: &Path) -> Option<DateTime<Utc>> {
    // Try Spotlight metadata first (macOS)
    let output = std::process::Command::new("mdls")
        .arg("-name")
        .arg("kMDItemLastUsedDate")
        .arg("-raw")
        .arg(app_path)
        .output()
        .ok()?;
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let trimmed = stdout.trim();
        if !trimmed.is_empty() && trimmed != "(null)" {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(trimmed) {
                return Some(dt.with_timezone(&Utc));
            }
        }
    }
    // Fallback: app bundle mtime
    let meta = std::fs::metadata(app_path).ok()?;
    if let Ok(mtime) = meta.modified() {
        let duration = mtime.duration_since(std::time::UNIX_EPOCH).ok()?;
        Some(DateTime::from_timestamp(duration.as_secs() as i64, 0).unwrap_or_default())
    } else {
        None
    }
}

/// Walk the application directories and collect bundle info.
/// `with_details` controls the expensive parts — `app_size` (full bundle
/// walk) and `last_used_date` (one `mdls` spawn per app, seconds across a
/// few hundred apps). The orphan scan only needs names and bundle ids, so
/// it skips both instead of paying for data it discards.
fn collect_app_bundles(
    progress: Option<std::sync::mpsc::Sender<String>>,
    with_details: bool,
) -> Vec<AppInfo> {
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
            if let Some(ref tx) = progress {
                let _ = tx.send(path.display().to_string());
            }
            let size = if with_details { app_size(&path) } else { 0 };
            let id = bundle_id_for_app(&path).unwrap_or_else(|| format!("unknown.{}", name));
            let last_used = if with_details {
                last_used_date(&path)
            } else {
                None
            };
            apps.push(AppInfo {
                id,
                name,
                path,
                size,
                last_used,
                is_from_app_store: false,
            });
        }
    }
    apps
}

pub fn find_installed_apps(
    progress: Option<std::sync::mpsc::Sender<String>>,
) -> Result<Vec<AppInfo>, String> {
    let mut apps = collect_app_bundles(progress, true);
    apps.sort_by_key(|a| std::cmp::Reverse(a.size));
    Ok(apps)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Whether an `Application Support` entry belongs to `app`.
///
/// Inputs are lowercase. The old rule substring-matched the app name, so a
/// short name (`Go`) claimed unrelated directories (`Google Drive`); names
/// must now match exactly. Bundle ids stay substring-matched in both the
/// dotted and the dots-stripped form — vendors use both on disk, and an id
/// (`com.google.drive`) is specific enough that a substring cannot over-match.
fn app_support_entry_matches(
    fname_lower: &str,
    app_name_lower: &str,
    bundle_id_lower: &str,
) -> bool {
    if fname_lower == app_name_lower {
        return true;
    }
    if bundle_id_lower.is_empty() {
        return false;
    }
    fname_lower.contains(bundle_id_lower) || fname_lower.contains(&bundle_id_lower.replace('.', ""))
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
        let name_lc = app_name.to_lowercase();
        let id_lc = bundle_id.to_lowercase();
        for entry in read_dir.flatten() {
            let p = entry.path();
            if leftovers.contains(&p) {
                continue;
            }
            let fname = p
                .file_name()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if app_support_entry_matches(&fname, &name_lc, &id_lc) && p.exists() {
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

#[derive(Debug, Clone)]
pub struct OrphanedData {
    pub paths: Vec<PathBuf>,
    pub total_bytes: u64,
    pub item_count: usize,
    /// Number of installed apps the orphan classification considered.
    /// Callers previously ran a second full app scan (one `mdls` spawn per
    /// app) just to display this count.
    pub installed_app_count: usize,
}

pub fn find_orphaned_data() -> Result<OrphanedData, String> {
    let home = home_dir().ok_or_else(|| "HOME not set".to_string())?;
    // Names and bundle ids only: sizes and Spotlight last-used dates are
    // irrelevant here and cost a full bundle walk plus one mdls per app.
    let apps = collect_app_bundles(None, false);
    find_orphaned_data_in(&home, &apps)
}

/// Injectable core of [`find_orphaned_data`]: home dir and the installed-app
/// list are parameters so tests run against a temp `HOME` — the old test
/// called the real function, whose `dir_size` pass over every unknown entry
/// under the developer's actual `~/Library` kept a test binary spinning for
/// tens of minutes.
fn find_orphaned_data_in(home: &Path, apps: &[AppInfo]) -> Result<OrphanedData, String> {
    let known_names: Vec<String> = apps
        .iter()
        .flat_map(|a| {
            // Both id spellings: sandbox containers and vendor dirs on disk
            // keep the dotted bundle id ("com.vendor.app"), while some
            // vendors strip the dots. Matching only the stripped form used
            // to flag every sandbox container whose app name was not a
            // substring of the id as orphaned data.
            vec![
                a.name.to_lowercase(),
                a.id.to_lowercase(),
                a.id.to_lowercase().replace('.', ""),
            ]
        })
        .collect();

    let mut orphaned = Vec::new();
    let mut total = 0u64;

    for rel in LEFTOVER_RELATIVE_PATHS {
        let base = home.join(rel);
        if !base.exists() {
            continue;
        }
        let read_dir = match std::fs::read_dir(&base) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for entry in read_dir.flatten() {
            let p = entry.path();
            let fname = p
                .file_name()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();

            if fname.starts_with('.') {
                continue;
            }

            let is_known = known_names.iter().any(|k| {
                let kc = k.to_lowercase();
                let fc = fname.to_lowercase();
                fc == kc || fc.contains(&kc) || kc.contains(&fc)
            });

            if !is_known {
                let size = dir_size(&p);
                orphaned.push(p);
                total += size;
            }
        }
    }

    let count = orphaned.len();
    Ok(OrphanedData {
        paths: orphaned,
        total_bytes: total,
        item_count: count,
        installed_app_count: apps.len(),
    })
}

pub fn uninstall_app(app: &AppInfo, remove_leftovers: bool) -> Result<CleanReport, String> {
    let leftover_paths = if remove_leftovers {
        find_leftovers(app)?.leftover_paths
    } else {
        Vec::new()
    };
    uninstall_app_with_leftovers(app, &leftover_paths)
}

/// Uninstall `app` plus exactly the given leftover paths.
///
/// The TUI confirm panel lets users deselect individual leftovers; the bool
/// variant re-ran `find_leftovers` here and trashed everything the scan
/// found, silently ignoring that selection (and repeating the per-path
/// `dir_size` walk the panel had just paid for).
pub fn uninstall_app_with_leftovers(
    app: &AppInfo,
    leftover_paths: &[PathBuf],
) -> Result<CleanReport, String> {
    let mut items = vec![CleanItem {
        path: app.path.clone(),
        size: app.size,
        risk: RiskLevel::Low,
        target_id: "uninstall".into(),
    }];

    for p in leftover_paths {
        items.push(CleanItem {
            path: p.clone(),
            size: dir_size(p),
            risk: RiskLevel::Low,
            target_id: "uninstall-leftover".into(),
        });
    }

    // Shared check + trash + audit loop (same as clean and purge).
    Ok(super::cleaner::exec_items(&items, AuditOp::Uninstall))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    #[test]
    fn test_bundle_id_minified_plist() {
        // No whitespace between key and string tags: the scan must skip
        // exactly the key tag, not one byte past it.
        let tmp = std::env::temp_dir().join("_argus_bundle_id_test.app/Contents");
        fs::create_dir_all(&tmp).unwrap();
        fs::write(
            tmp.join("Info.plist"),
            "<plist><dict><key>CFBundleIdentifier</key><string>com.example.min</string></dict></plist>",
        )
        .unwrap();
        let id = bundle_id_for_app(tmp.parent().unwrap());
        assert_eq!(id.as_deref(), Some("com.example.min"));
        let _ = fs::remove_dir_all(tmp.parent().unwrap().parent().unwrap());
    }

    /// A binary plist (invalid UTF-8) must still yield the bundle id: the
    /// plain-text read fails, and the macOS plutil conversion rescues it.
    /// Xcode/Pages/Tunnelblick all ship binary plists and used to degrade to
    /// "unknown.<name>", losing every bundle-id-named leftover.
    #[cfg(target_os = "macos")]
    #[test]
    fn test_bundle_id_binary_plist_via_plutil() {
        use std::process::Command;
        let tmp = std::env::temp_dir().join("_argus_bundle_id_bin.app/Contents");
        fs::create_dir_all(&tmp).unwrap();
        let plist = tmp.join("Info.plist");
        fs::write(
            &plist,
            "<plist><dict><key>CFBundleIdentifier</key><string>com.example.binary</string></dict></plist>",
        )
        .unwrap();
        let converted = Command::new("plutil")
            .args(["-convert", "binary1", "-o", &plist.to_string_lossy()])
            .arg(&plist)
            .status()
            .expect("run plutil");
        assert!(converted.success(), "plutil must be able to binarize");

        // Sanity: the file is now invalid UTF-8, i.e. the plain read fails.
        assert!(fs::read_to_string(&plist).is_err());

        let id = bundle_id_for_app(tmp.parent().unwrap());
        assert_eq!(id.as_deref(), Some("com.example.binary"));
        let _ = fs::remove_dir_all(tmp.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn test_find_apps_returns_list() {
        // Cheap listing only (Info.plist reads): the `with_details` variant
        // additionally walks every real app bundle for sizes and spawns one
        // mdls per app — minutes of work the assertions below never use.
        let apps = collect_app_bundles(None, false);
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

    /// Application Support entries match by exact app name or bundle id
    /// (dotted and stripped). A short app name must not claim unrelated
    /// directories by substring: `Go` used to match `Google Drive`.
    #[test]
    fn test_app_support_entry_match_rules() {
        // Exact name match always wins (inputs are pre-lowercased by the
        // caller — `find_leftovers` lowercases before calling).
        assert!(app_support_entry_matches(
            "firefox",
            "firefox",
            "org.mozilla.firefox"
        ));
        assert!(app_support_entry_matches(
            "google drive",
            "google drive",
            "x"
        ));

        // Bundle id substring, dotted and dots-stripped vendor forms.
        assert!(app_support_entry_matches(
            "org.videolan.vlc",
            "vlc",
            "org.videolan.vlc"
        ));
        assert!(app_support_entry_matches(
            "orgvideolanvlc",
            "vlc",
            "org.videolan.vlc"
        ));
        assert!(app_support_entry_matches(
            "com.google.drive.cache",
            "google drive",
            "com.google.drive"
        ));

        // The over-matching cases the old contains() rule claimed:
        assert!(
            !app_support_entry_matches("google drive", "go", "com.golang.go"),
            "short name must not substring-match"
        );
        assert!(
            !app_support_entry_matches("goland", "go", "com.jetbrains.goland"),
            "prefix-at-boundary is still a different app"
        );
        assert!(
            !app_support_entry_matches("unrelated", "app", "com.vendor.app2"),
            "name substring inside a longer dir name no longer matches"
        );

        // Empty bundle id (no plist → "unknown.<name>" is still an id, but a
        // defensive empty must not panic or match).
        assert!(!app_support_entry_matches("anything", "app", ""));
    }

    /// Orphan classification must run against the injected temp home: entries
    /// matching an installed app (name or bundle id) are known, everything
    /// else in the leftover dirs is orphaned.
    #[test]
    fn test_find_orphaned_data_classifies_against_injected_apps() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let app_support = home.join("Library/Application Support");
        fs::create_dir_all(&app_support).unwrap();
        fs::create_dir_all(home.join("Library/Caches")).unwrap();

        // Known: name match, bundle-id match (dots stripped), and the raw
        // dotted id — the standard sandbox-container naming.
        fs::write(app_support.join("Firefox"), "x").unwrap();
        fs::write(app_support.join("com.example.KnownApp"), "x").unwrap();
        // Case-insensitive match on the bundle id.
        fs::write(app_support.join("COM.EXAMPLE.KNOWNAPP"), "x").unwrap();
        // A container directory named by the raw dotted bundle id must stay
        // known even when the app name is not a substring of the id.
        let containers = home.join("Library/Containers");
        fs::create_dir_all(&containers).unwrap();
        fs::write(containers.join("com.pixelmatorteam.pixelmator.x"), "x").unwrap();
        // Unknown: orphaned.
        fs::write(app_support.join("OrphanJunk"), "hello").unwrap();
        fs::write(home.join("Library/Caches/LeftoverData"), "yy").unwrap();

        let apps = vec![
            AppInfo {
                id: "com.mozilla.firefox".into(),
                name: "Firefox".into(),
                path: PathBuf::from("/Applications/Firefox.app"),
                size: 0,
                last_used: None,
                is_from_app_store: false,
            },
            AppInfo {
                id: "com.example.KnownApp".into(),
                name: "KnownApp".into(),
                path: PathBuf::from("/Applications/KnownApp.app"),
                size: 0,
                last_used: None,
                is_from_app_store: false,
            },
            AppInfo {
                id: "com.pixelmatorteam.pixelmator.x".into(),
                name: "Pixelmator Pro".into(),
                path: PathBuf::from("/Applications/Pixelmator Pro.app"),
                size: 0,
                last_used: None,
                is_from_app_store: false,
            },
        ];

        let data = find_orphaned_data_in(home, &apps).unwrap();
        assert_eq!(data.installed_app_count, 3);
        assert_eq!(data.item_count, 2, "known entries must not be orphaned");
        assert_eq!(data.total_bytes, 5 + 2);
        let names: Vec<String> = data
            .paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(names.contains(&"OrphanJunk".to_string()));
        assert!(names.contains(&"LeftoverData".to_string()));
    }

    /// A nonexistent leftover base directory is skipped, not an error.
    #[test]
    fn test_find_orphaned_data_missing_home_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let data = find_orphaned_data_in(tmp.path(), &[]).unwrap();
        assert_eq!(data.item_count, 0);
        assert_eq!(data.total_bytes, 0);
        assert_eq!(data.installed_app_count, 0);
    }
}
