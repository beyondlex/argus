use std::path::{Path, PathBuf};

use super::audit::{log_operation, AuditEntry, AuditOp};
use super::categories::CleanTarget;
use super::dir_size;
use super::safety::{check_deletion_allowed, classify_risk, RiskLevel};

#[derive(Debug, Clone)]
pub struct CleanItem {
    pub path: PathBuf,
    pub size: u64,
    pub risk: RiskLevel,
    pub target_id: String,
}

#[derive(Debug, Clone)]
pub struct CleanPlan {
    pub targets: Vec<CleanTarget>,
    pub total_bytes: u64,
    pub items: Vec<CleanItem>,
}

#[derive(Debug, Clone)]
pub struct CleanReport {
    pub total_attempted: u64,
    pub total_succeeded: u64,
    pub total_failed: u64,
    pub freed_bytes: u64,
    pub errors: Vec<(PathBuf, String)>,
}

impl CleanPlan {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

pub fn plan_clean(targets: &[CleanTarget]) -> Result<CleanPlan, String> {
    let mut items = Vec::new();

    for target in targets {
        let mut existing_paths = Vec::new();
        for p in &target.paths {
            if p.exists() {
                existing_paths.push(p.clone());
            }
        }
        if existing_paths.is_empty() {
            continue;
        }
        // Per-path size: assigning the target total to every path item used
        // to inflate each item's label and multiply freed_bytes by the number
        // of existing paths once exec_clean summed them.
        for p in existing_paths {
            let size = dir_size(&p);
            let risk = classify_risk(&p).max(target.risk);
            items.push(CleanItem {
                path: p,
                size,
                risk,
                target_id: target.id.clone(),
            });
        }
    }

    // Drop paths equal to or nested inside an earlier item's path. Targets
    // legitimately overlap (`~/Library/Logs` umbrella vs its
    // `DiagnosticReports`/`PowerManagement` sub-targets); selecting both must
    // count and delete the subtree once, not twice with a guaranteed ENOENT
    // on the second, already-removed path. The outermost (earliest-listed)
    // path wins; `Path::starts_with` compares component-wise, so sibling
    // names like `/logs-extra` never match.
    let mut deduped: Vec<CleanItem> = Vec::with_capacity(items.len());
    for item in items {
        if !deduped.iter().any(|kept| item.path.starts_with(&kept.path)) {
            deduped.push(item);
        }
    }
    let items = deduped;

    let total_bytes: u64 = items.iter().map(|i| i.size).sum();

    Ok(CleanPlan {
        targets: targets.to_vec(),
        total_bytes,
        items,
    })
}

pub fn dry_clean(targets: &[CleanTarget]) -> Result<CleanPlan, String> {
    plan_clean(targets)
}

fn move_to_trash(path: &Path) -> Result<(), String> {
    check_deletion_allowed(path).map_err(|e| e.to_string())?;
    trash::delete(path).map_err(|e| format!("trash error for {}: {e}", path.display()))
}

/// Move the given items to the trash and write an audit entry.
///
/// There is deliberately no "force" or "dry-run" flag: previews are produced
/// by [`plan_clean`], and execution always deletes. An ignored `_force`
/// parameter used to exist here and misled a caller into passing a dry-run
/// flag through it — deleting for real while the UI advertised a preview.
pub fn exec_clean(items: &[CleanItem]) -> Result<CleanReport, String> {
    Ok(exec_items(items, AuditOp::Clean))
}

/// Shared trash-delete loop for clean / purge / uninstall. The three private
/// copies used to drift in error wording and audit metadata.
pub(crate) fn exec_items(items: &[CleanItem], op: AuditOp) -> CleanReport {
    let mut report = CleanReport {
        total_attempted: items.len() as u64,
        total_succeeded: 0,
        total_failed: 0,
        freed_bytes: 0,
        errors: Vec::new(),
    };

    for item in items {
        match move_to_trash(&item.path) {
            Ok(()) => {
                report.total_succeeded += 1;
                report.freed_bytes += item.size;
            }
            Err(e) => {
                report.total_failed += 1;
                report.errors.push((item.path.clone(), e));
            }
        }
    }

    let entry = AuditEntry {
        timestamp: chrono::Utc::now(),
        operation: op,
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

    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cleaner::categories::TargetCategory;

    fn temp_target() -> CleanTarget {
        CleanTarget {
            id: "test-temp".into(),
            label: "Test Temp".into(),
            paths: vec![std::env::temp_dir()],
            risk: RiskLevel::Safe,
            category: TargetCategory::TempFiles,
        }
    }

    #[test]
    fn test_dry_clean_returns_plan() {
        let target = temp_target();
        let plan = dry_clean(&[target]).unwrap();
        assert!(!plan.is_empty() || plan.total_bytes == 0);
    }

    #[test]
    fn test_plan_clean_nonexistent() {
        let target = CleanTarget {
            id: "nonexistent".into(),
            label: "Nonexistent".into(),
            paths: vec![PathBuf::from("/_xyz_nonexistent_test_99/")],
            risk: RiskLevel::Safe,
            category: TargetCategory::TempFiles,
        };
        let plan = plan_clean(&[target]).unwrap();
        assert!(plan.is_empty());
        assert_eq!(plan.total_bytes, 0);
    }

    /// A target with several existing paths must report each path's own size:
    /// assigning the target total to every item inflated per-item labels and
    /// multiplied freed_bytes by the number of paths when exec_clean summed
    /// the items.
    #[test]
    fn test_plan_clean_per_path_sizes_not_duplicated() {
        let dir = std::env::temp_dir().join("_argus_plan_clean_sizes");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a")).unwrap();
        std::fs::create_dir_all(dir.join("b")).unwrap();
        std::fs::write(dir.join("a").join("x.bin"), vec![0u8; 100]).unwrap();
        std::fs::write(dir.join("b").join("y.bin"), vec![0u8; 300]).unwrap();

        let target = CleanTarget {
            id: "multi-path".into(),
            label: "Multi Path".into(),
            paths: vec![dir.join("a"), dir.join("b")],
            risk: RiskLevel::Safe,
            category: TargetCategory::TempFiles,
        };

        let plan = plan_clean(&[target]).unwrap();
        assert_eq!(plan.items.len(), 2);
        let sizes: Vec<u64> = plan.items.iter().map(|i| i.size).collect();
        assert_eq!(sizes, vec![100, 300]);
        assert_eq!(plan.total_bytes, 400);

        // exec_clean's freed-bytes sum over the plan items equals the plan total.
        let items_sum: u64 = plan.items.iter().map(|i| i.size).sum();
        assert_eq!(items_sum, plan.total_bytes);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Overlapping targets (an umbrella dir and a path inside it, or the same
    /// path in two targets) must plan the subtree once: one item, one size.
    /// Both used to appear, double-counting total_bytes and guaranteeing an
    /// ENOENT failure on the second exec.
    #[test]
    fn test_plan_clean_dedupes_nested_and_duplicate_paths() {
        let dir = std::env::temp_dir().join("_argus_plan_clean_dedup");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("logs/nested")).unwrap();
        std::fs::write(dir.join("logs").join("a.bin"), vec![0u8; 100]).unwrap();
        std::fs::write(dir.join("logs/nested").join("b.bin"), vec![0u8; 200]).unwrap();

        let umbrella = CleanTarget {
            id: "umbrella".into(),
            label: "Umbrella".into(),
            paths: vec![dir.join("logs")],
            risk: RiskLevel::Safe,
            category: TargetCategory::TempFiles,
        };
        let nested = CleanTarget {
            id: "nested".into(),
            label: "Nested".into(),
            paths: vec![dir.join("logs/nested")],
            risk: RiskLevel::Safe,
            category: TargetCategory::TempFiles,
        };
        let duplicate = CleanTarget {
            id: "duplicate".into(),
            label: "Duplicate".into(),
            paths: vec![dir.join("logs")],
            risk: RiskLevel::Safe,
            category: TargetCategory::TempFiles,
        };

        let plan = plan_clean(&[umbrella, nested, duplicate]).unwrap();
        assert_eq!(plan.items.len(), 1, "nested and duplicate paths must dedup");
        assert_eq!(plan.items[0].path, dir.join("logs"));
        assert_eq!(plan.total_bytes, 300);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Sibling paths sharing a name prefix must both survive dedup
    /// (`/logs` vs `/logs-extra` are different subtrees).
    #[test]
    fn test_plan_clean_keeps_sibling_prefix_paths() {
        let dir = std::env::temp_dir().join("_argus_plan_clean_sibling");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("logs")).unwrap();
        std::fs::create_dir_all(dir.join("logs-extra")).unwrap();
        std::fs::write(dir.join("logs").join("a.bin"), vec![0u8; 10]).unwrap();
        std::fs::write(dir.join("logs-extra").join("b.bin"), vec![0u8; 20]).unwrap();

        let t1 = CleanTarget {
            id: "logs".into(),
            label: "Logs".into(),
            paths: vec![dir.join("logs")],
            risk: RiskLevel::Safe,
            category: TargetCategory::TempFiles,
        };
        let t2 = CleanTarget {
            id: "logs-extra".into(),
            label: "Logs Extra".into(),
            paths: vec![dir.join("logs-extra")],
            risk: RiskLevel::Safe,
            category: TargetCategory::TempFiles,
        };

        let plan = plan_clean(&[t1, t2]).unwrap();
        assert_eq!(plan.items.len(), 2);
        assert_eq!(plan.total_bytes, 30);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
