use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use super::audit::{log_operation, AuditEntry, AuditOp};
use super::cleaner::{CleanItem, CleanReport};
use super::safety::{check_deletion_allowed, classify_risk, RiskLevel};

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
        let read_dir = match std::fs::read_dir(root) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let dir_name = path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();

            for kind in ALL_ARTIFACT_KINDS {
                let target_dir = path.join(kind.dir_name());
                if target_dir.exists() && target_dir.is_dir() {
                    let size = dir_size(&target_dir);
                    let modified = std::fs::metadata(&target_dir)
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

                    artifacts.push(Artifact {
                        path: target_dir,
                        kind: *kind,
                        size,
                        last_modified: modified,
                        project_name: dir_name.clone(),
                        age_days,
                    });
                }
            }
        }
    }

    artifacts.sort_by(|a, b| b.size.cmp(&a.size));
    Ok(artifacts)
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
        operation: AuditOp::Purge,
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

        let artifacts = find_artifacts(&[tmp.clone()]).unwrap();
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

    #[test]
    fn test_artifact_kind_labels() {
        assert_eq!(ArtifactKind::Target.label(), "target (Rust)");
        assert_eq!(ArtifactKind::NodeModules.label(), "node_modules");
    }
}
