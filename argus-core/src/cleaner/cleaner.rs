use std::path::PathBuf;

use super::audit::{log_operation, AuditEntry, AuditOp};
use super::categories::{scan_target_size, CleanTarget};
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
        let (size, existing_paths) =
            scan_target_size(target).map_err(|e| format!("scan target {}: {e}", target.id))?;
        if size > 0 && !existing_paths.is_empty() {
            for p in existing_paths {
                let risk = classify_risk(&p).max(target.risk);
                items.push(CleanItem {
                    path: p,
                    size,
                    risk,
                    target_id: target.id.clone(),
                });
            }
            total_bytes += size;
        }
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

pub fn exec_clean(items: &[CleanItem], _force: bool) -> Result<CleanReport, String> {
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
}
