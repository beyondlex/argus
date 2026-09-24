use std::path::PathBuf;

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
    let mut total_bytes = 0u64;

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
        let mut target_total = 0u64;
        for p in existing_paths {
            let size = dir_size(&p);
            let risk = classify_risk(&p).max(target.risk);
            items.push(CleanItem {
                path: p,
                size,
                risk,
                target_id: target.id.clone(),
            });
            target_total += size;
        }
        total_bytes += target_total;
    }

    Ok(CleanPlan {
        targets: targets.to_vec(),
        total_bytes,
        items,
    })
}

pub fn dry_clean(targets: &[CleanTarget]) -> Result<CleanPlan, String> {
    plan_clean(targets)
}

fn move_to_trash(path: &PathBuf) -> Result<(), String> {
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
        operation: AuditOp::Clean,
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
}
