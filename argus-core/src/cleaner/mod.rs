pub mod audit;
pub mod brew;
pub mod categories;
#[allow(clippy::module_inception)]
pub mod cleaner;
pub mod purge;
pub mod safety;
#[cfg(feature = "shell-cmds")]
pub mod shell_cmd;
pub mod uninstaller;

use std::path::Path;

/// Recursively sum logical file sizes below `path` (bytes, not disk usage).
/// Symlinks are skipped entirely — they are followed in several of the
/// module's scans (Library trees, brew kegs) and counting their targets would
/// double-book shared data.
///
/// Single shared implementation: four private copies of this function had
/// drifted across purge/categories/brew/uninstaller. Exported so clients
/// (TUI detail panels) reuse it instead of growing yet another copy.
pub fn dir_size(path: &Path) -> u64 {
    // Entry point must not follow a symlink either: `Path::is_dir`/`is_file`
    // stat through links, so a `link -> /some/huge/dir` argument counted the
    // whole target subtree despite the walk below skipping every symlink.
    let Ok(top) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if top.is_symlink() {
        return 0;
    }
    if top.is_file() {
        return top.len();
    }
    if !top.is_dir() {
        return 0;
    }

    let mut total = 0u64;
    let mut dirs = vec![path.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(read_dir) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read_dir.flatten() {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                dirs.push(entry.path());
            } else if ft.is_file() {
                total += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dir_size_sums_files_skips_symlinks() {
        let tmp = std::env::temp_dir().join("_argus_dir_size_test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("sub")).unwrap();
        std::fs::write(tmp.join("a.bin"), vec![0u8; 100]).unwrap();
        std::fs::write(tmp.join("sub").join("b.bin"), vec![0u8; 50]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(tmp.join("a.bin"), tmp.join("link.bin")).unwrap();

        assert_eq!(dir_size(&tmp), 150);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_dir_size_nonexistent_is_zero() {
        assert_eq!(dir_size(Path::new("/_nonexistent_xyz_99")), 0);
    }

    #[test]
    fn test_dir_size_single_file() {
        let tmp = std::env::temp_dir().join("_argus_dir_size_file");
        std::fs::write(&tmp, b"hello").unwrap();
        assert_eq!(dir_size(&tmp), 5);
        let _ = std::fs::remove_file(&tmp);
    }

    /// A symlink argument must count as 0, whatever it points at: the walk
    /// skips symlink entries, but the old entry checks (`is_file`/`is_dir`)
    /// stat *through* the top-level link and summed the whole target.
    #[cfg(unix)]
    #[test]
    fn test_dir_size_symlink_argument_counts_zero() {
        use std::os::unix::fs::symlink;
        let tmp = std::env::temp_dir().join("_argus_dir_size_symlink");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("real")).unwrap();
        std::fs::write(tmp.join("real").join("big.bin"), vec![0u8; 4096]).unwrap();

        let dir_link = tmp.join("dir-link");
        symlink(tmp.join("real"), &dir_link).unwrap();
        assert_eq!(dir_size(&dir_link), 0, "symlink to dir must count 0");

        let file_link = tmp.join("file-link");
        symlink(tmp.join("real").join("big.bin"), &file_link).unwrap();
        assert_eq!(dir_size(&file_link), 0, "symlink to file must count 0");

        // The real directory behind the links still measures normally.
        assert_eq!(dir_size(&tmp.join("real")), 4096);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
