//! Fork-only pin: the Slack terminal notification's quota-park recovery
//! hint (revisor harvest, landed 2026-10-11). A `SoftStop` failure is the
//! engine-side quota park (ADR-0021): its message carries the window
//! reset prose, and the notification must turn that into a copy-pasteable
//! recovery path instead of leaving the operator to event forensics.
//! Keeping the pin in a fork-owned file means an upstream merge can never
//! silently drop it.

use fabro_store::platform_records::{RunLifecycleKind, RunLifecycleRecord};
use fabro_types::{FailureReason, RunId, RunStatus};

#[test]
fn a_soft_stop_failure_result_carries_the_recovery_hint() {
    let run_id = "01M4KMWYRQDJ54ZSS08NJNH74N".parse::<RunId>().unwrap();
    let parked = RunLifecycleRecord {
        transition: RunLifecycleKind::Failed,
        status: Some(RunStatus::Failed {
            reason: FailureReason::SoftStop,
        }),
        reason: Some(
            "provider usage window closed: zai 5h quota, will reset at 2026-10-11 05:04:33"
                .to_string(),
        ),
        ..RunLifecycleRecord::new(RunLifecycleKind::Failed)
    };
    let result = super::slack_lifecycle_failed_result(run_id, &parked);
    assert!(
        result.starts_with("soft_stop — provider usage window closed"),
        "{result}"
    );
    assert!(result.contains("will reset at"), "{result}");
    assert!(
        result.contains(&format!("recovery: fabro rewind {run_id}")),
        "{result}"
    );

    let hard = RunLifecycleRecord {
        transition: RunLifecycleKind::Failed,
        status: Some(RunStatus::Failed {
            reason: FailureReason::WorkflowError,
        }),
        reason: Some("boom".to_string()),
        ..RunLifecycleRecord::new(RunLifecycleKind::Failed)
    };
    let hard_result = super::slack_lifecycle_failed_result(run_id, &hard);
    assert_eq!(hard_result, "workflow_error — boom");
    assert!(!hard_result.contains("recovery"), "{hard_result}");
}
