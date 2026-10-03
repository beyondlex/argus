use std::path::PathBuf;

use chrono::{DateTime, Utc};

use super::audit::AuditOp;
use super::cleaner::{CleanItem, CleanReport};
use super::dir_size;
use super::safety::{classify_risk, RiskLevel};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    NodeModules,
    Target,
    Build,
    Dist,
    Venv,
    NextCache,
    Terraform,
}

impl ArtifactKind {
    pub fn dir_name(&self) -> &'static str {
        match self {
            ArtifactKind::NodeModules => "node_modules",
            ArtifactKind::Target => "target",
            ArtifactKind::Build => "build",
            ArtifactKind::Dist => "dist",
            ArtifactKind::Venv => "venv",
            ArtifactKind::NextCache => ".next",
            ArtifactKind::Terraform => ".terraform",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ArtifactKind::NodeModules => "node_modules",
            ArtifactKind::Target => "target (Rust)",
            ArtifactKind::Build => "build",
            ArtifactKind::Dist => "dist",
            ArtifactKind::Venv => "venv (Python)",
            ArtifactKind::NextCache => ".next (Next.js)",
            ArtifactKind::Terraform => ".terraform",
        }
    }
}

pub static ALL_ARTIFACT_KINDS: &[ArtifactKind] = &[
    ArtifactKind::NodeModules,
    ArtifactKind::Target,
    ArtifactKind::Build,
    ArtifactKind::Dist,
    ArtifactKind::Venv,
    ArtifactKind::NextCache,
    ArtifactKind::Terraform,
];

#[derive(Debug, Clone)]
pub struct Artifact {
    pub path: PathBuf,
    pub kind: ArtifactKind,
    pub size: u64,
    pub last_modified: DateTime<Utc>,
    pub project_name: String,
    pub age_days: u64,
}

fn default_search_roots() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut roots = Vec::new();
    if let Some(ref h) = home {
        let candidates = ["Projects", "GitHub", "dev", "Work", "Documents", "Desktop"];
        for c in &candidates {
            let p = h.join(c);
            if p.exists() {
                roots.push(p);
            }
        }
    }
    let current = std::env::current_dir().ok();
    if let Some(cwd) = current {
        if !roots.contains(&cwd) {
            roots.push(cwd);
        }
    }
    roots
}

/// How deep below a search root artifact directories may sit. The original
/// single-level check (`root/<project>/<kind>`) missed nested workspaces —
/// `~/Projects/work/app` with its `target/` was invisible to purge. 4 covers
/// up to three project levels without walking whole projects; matched
/// artifact dirs are never descended into, so `node_modules` trees are not
/// re-walked at their (huge) depth.
const ARTIFACT_MAX_DEPTH: usize = 4;

pub fn find_artifacts(roots: &[PathBuf]) -> Result<Vec<Artifact>, String> {
    let search_roots = if roots.is_empty() {
        default_search_roots()
    } else {
        roots.to_vec()
    };

    let mut artifacts = Vec::new();
    let now = Utc::now();

    for root in &search_roots {
        if !root.exists() {
            continue;
        }
        // Iterative depth-bounded walk. entry.file_type() does not follow
        // symlinks: a `link -> /some/huge/dir` entry is neither matched as an
        // artifact nor descended into (same symlink discipline as dir_size).
        let mut stack = vec![(root.clone(), 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            let read_dir = match std::fs::read_dir(&dir) {
                Ok(r) => r,
                Err(_) => continue,
            };
            for entry in read_dir.flatten() {
                let Ok(ft) = entry.file_type() else {
                    continue;
                };
                if !ft.is_dir() {
                    continue;
                }
                let path = entry.path();
                let Some(kind) = ALL_ARTIFACT_KINDS
                    .iter()
                    .find(|k| path.file_name().is_some_and(|n| n == k.dir_name()))
                else {
                    if depth < ARTIFACT_MAX_DEPTH {
                        stack.push((path, depth + 1));
                    }
                    continue;
                };

                let size = dir_size(&path);
                let modified = std::fs::metadata(&path)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .map(|t| {
                        let secs = t
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs() as i64;
                        DateTime::from_timestamp(secs, 0).unwrap_or(now)
                    })
                    .unwrap_or(now);
                let age_days = (now - modified).num_days().max(0) as u64;

                // The enclosing directory is the project (e.g. `…/my-app/target`
                // → "my-app"), matching the flat layout's project naming.
                let project_name = path
                    .parent()
                    .and_then(|p| p.file_name())
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();

                artifacts.push(Artifact {
                    path,
                    kind: *kind,
                    size,
                    last_modified: modified,
                    project_name,
                    age_days,
                });
            }
        }
    }

    artifacts.sort_by_key(|a| std::cmp::Reverse(a.size));
    Ok(artifacts)
}

pub fn remove_artifacts(artifacts: &[Artifact]) -> Result<CleanReport, String> {
    let items: Vec<CleanItem> = artifacts
        .iter()
        .map(|a| CleanItem {
            path: a.path.clone(),
            size: a.size,
            risk: classify_risk(&a.path).max(RiskLevel::Low),
            target_id: format!("purge-{}", a.kind.dir_name()),
        })
        .collect();

    // Shared check + trash + audit loop; the private copy used to word
    // errors and log audit entries slightly differently than exec_clean.
    Ok(super::cleaner::exec_items(&items, AuditOp::Purge))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_artifact_kind_dir_names() {
        assert_eq!(ArtifactKind::NodeModules.dir_name(), "node_modules");
        assert_eq!(ArtifactKind::Target.dir_name(), "target");
        assert_eq!(ArtifactKind::Venv.dir_name(), "venv");
    }

    #[test]
    fn test_find_artifacts_in_temp() {
        let tmp = std::env::temp_dir().join("_argus_purge_test");
        let _ = fs::create_dir_all(&tmp);
        let proj_dir = tmp.join("my-test-project");
        let target = proj_dir.join("target");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("some.o"), b"test").unwrap();
        let node = proj_dir.join("node_modules");
        fs::create_dir_all(&node).unwrap();
        fs::write(node.join("pkg.js"), b"test").unwrap();

        let artifacts = find_artifacts(std::slice::from_ref(&tmp)).unwrap();
        assert!(artifacts.len() >= 2);

        let rust_target = artifacts.iter().find(|a| a.kind == ArtifactKind::Target);
        assert!(rust_target.is_some());
        assert_eq!(rust_target.unwrap().project_name, "my-test-project");

        let nm = artifacts
            .iter()
            .find(|a| a.kind == ArtifactKind::NodeModules);
        assert!(nm.is_some());

        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn test_find_artifacts_nonexistent_root() {
        let artifacts = find_artifacts(&[PathBuf::from("/_nonexistent_root_99")]).unwrap();
        assert!(artifacts.is_empty());
    }

    /// Nested workspaces must be found: the old scan only looked at
    /// `root/<project>/<kind>`, so `root/work/my-app/target` was invisible.
    #[test]
    fn test_find_artifacts_nested_workspace() {
        let tmp = std::env::temp_dir().join("_argus_purge_nested");
        let _ = fs::remove_dir_all(&tmp);
        let app = tmp.join("work").join("my-app");
        fs::create_dir_all(app.join("target")).unwrap();
        fs::write(app.join("target").join("a.o"), b"test").unwrap();

        let artifacts = find_artifacts(std::slice::from_ref(&tmp)).unwrap();
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].kind, ArtifactKind::Target);
        assert_eq!(artifacts[0].project_name, "my-app");

        let _ = fs::remove_dir_all(&tmp);
    }

    /// A matched artifact directory is reported once and not descended into:
    /// a `node_modules` inside `node_modules` must not produce a second
    /// artifact for the same tree.
    #[test]
    fn test_find_artifacts_no_self_nesting() {
        let tmp = std::env::temp_dir().join("_argus_purge_nesting");
        let _ = fs::remove_dir_all(&tmp);
        let nm = tmp.join("proj").join("node_modules");
        fs::create_dir_all(nm.join("dep").join("node_modules")).unwrap();

        let artifacts = find_artifacts(std::slice::from_ref(&tmp)).unwrap();
        assert_eq!(artifacts.len(), 1, "outer node_modules only");
        assert_eq!(artifacts[0].path, nm);

        let _ = fs::remove_dir_all(&tmp);
    }

    /// Symlinked directories must be neither matched nor followed: a link
    /// pointing at a tree outside the root must not make its artifacts
    /// visible (same discipline as `dir_size` — links are not walked).
    #[cfg(unix)]
    #[test]
    fn test_find_artifacts_skips_symlinked_dirs() {
        use std::os::unix::fs::symlink;
        let outside = std::env::temp_dir().join("_argus_purge_symlink_outside");
        let _ = fs::remove_dir_all(&outside);
        fs::create_dir_all(outside.join("real").join("target")).unwrap();

        let tmp = std::env::temp_dir().join("_argus_purge_symlink");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        symlink(outside.join("real"), tmp.join("alias")).unwrap();

        let artifacts = find_artifacts(std::slice::from_ref(&tmp)).unwrap();
        assert!(
            artifacts.is_empty(),
            "artifact reachable only through a symlinked dir must not be reported"
        );

        let _ = fs::remove_dir_all(&tmp);
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn test_artifact_kind_labels() {
        assert_eq!(ArtifactKind::Target.label(), "target (Rust)");
        assert_eq!(ArtifactKind::NodeModules.label(), "node_modules");
    }
}
