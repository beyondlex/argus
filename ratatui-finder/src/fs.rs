use crate::FinderMode;
use std::path::{Component, Path};

/// Expand `~` to the user's home directory.
pub fn expand(path: &str) -> String {
    if path.starts_with("~/") || path == "~" {
        if let Ok(home) = std::env::var("HOME") {
            if path == "~" {
                return home;
            }
            return home + &path[1..];
        }
    }
    path.to_string()
}

/// Contract home directory prefix back to `~`.
pub fn contract(path: &str) -> String {
    if let Ok(home) = std::env::var("HOME") {
        if path.starts_with(&home) {
            if path == home {
                return "~".to_string();
            }
            return "~".to_string() + &path[home.len()..];
        }
    }
    path.to_string()
}

/// Return the parent directory path (always ends with '/').
pub fn parent(path: &str) -> String {
    let expanded = expand(path);
    let p = Path::new(&expanded);
    let parent = p.parent().unwrap_or(Path::new("/"));
    let mut result = parent.display().to_string();
    if !result.ends_with('/') {
        result.push('/');
    }
    contract(&result)
}

/// Return the basename (last component) of a path.
pub fn basename(path: &str) -> String {
    let expanded = expand(path);
    let p = Path::new(&expanded);
    p.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// Check if path is a directory.
pub fn is_dir(path: &str) -> bool {
    let expanded = expand(path);
    Path::new(&expanded).is_dir()
}

/// Check if path is the filesystem root.
pub fn is_root(path: &str) -> bool {
    let expanded = expand(path);
    let p = Path::new(&expanded);
    p.components().count() == 1 && p.components().next() == Some(Component::RootDir)
}

/// A raw directory item returned by `list()`.
#[derive(Debug, Clone)]
pub struct RawItem {
    pub name: String,
    pub is_dir: bool,
}

/// List directory contents, filtered by mode, sorted with directories first then alphabetically.
pub fn list(dir: &str, mode: FinderMode) -> Vec<RawItem> {
    let expanded = expand(dir);
    let path = Path::new(&expanded);

    let mut items: Vec<RawItem> = match std::fs::read_dir(path) {
        Ok(entries) => entries
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let name = entry.file_name().to_string_lossy().to_string();
                // Skip hidden files
                if name.starts_with('.') {
                    return None;
                }
                let is_dir = entry.file_type().ok()?.is_dir();
                match mode {
                    FinderMode::Dir => {
                        if !is_dir {
                            return None;
                        }
                    }
                    FinderMode::File => {
                        if is_dir {
                            return None;
                        }
                    }
                    FinderMode::Both => {}
                }
                Some(RawItem { name, is_dir })
            })
            .collect(),
        Err(_) => return Vec::new(),
    };

    // Sort: directories first, then by lowercase name
    items.sort_by(|a, b| {
        if a.is_dir != b.is_dir {
            if a.is_dir {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            }
        } else {
            a.name.to_lowercase().cmp(&b.name.to_lowercase())
        }
    });

    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_tilde() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(expand("~"), home);
        assert_eq!(expand("~/foo"), home + "/foo");
        assert_eq!(expand("/foo"), "/foo");
    }

    #[test]
    fn test_contract() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(contract(&home), "~");
        assert_eq!(contract(&format!("{home}/foo")), "~/foo");
        assert_eq!(contract("/other"), "/other");
    }

    #[test]
    fn test_parent() {
        assert_eq!(parent("~/a/b"), "~/a/");
        assert_eq!(parent("~/a"), "~/");
        let home = std::env::var("HOME").unwrap();
        let home_parent = std::path::Path::new(&home).parent().unwrap().display().to_string();
        assert_eq!(expand(&parent("~")), home_parent + "/");
    }

    #[test]
    fn test_basename() {
        assert_eq!(basename("~/a/b.txt"), "b.txt");
        assert_eq!(basename("~"), {
            let home = std::env::var("HOME").unwrap();
            Path::new(&home)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string()
        });
    }

    #[test]
    fn test_is_root() {
        assert!(is_root("/"));
        assert!(!is_root("~"));
        assert!(!is_root("/home"));
    }

    #[test]
    fn test_list_sorts_dirs_first() {
        use std::fs;
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path();
        fs::write(dir.join("a.txt"), "").unwrap();
        fs::create_dir(dir.join("b_dir")).unwrap();
        fs::write(dir.join("c.txt"), "").unwrap();
        fs::create_dir(dir.join("d_dir")).unwrap();

        let items = list(dir.to_str().unwrap(), FinderMode::Both);
        assert!(items.len() >= 4);
        // Directories should come first
        assert!(items[0].is_dir);
        assert!(items[1].is_dir);
        // Then files
        assert!(!items[2].is_dir);
        assert!(!items[3].is_dir);
    }
}