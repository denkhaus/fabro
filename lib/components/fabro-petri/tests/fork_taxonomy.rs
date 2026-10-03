//! Fork presence pin (fabro-6655, W1-3): the publish-blocked taxonomy
//! survives on Petri's projection fold. These tests exist only on the
//! fork; a merge resolution that drops them dropped the feature — restore
//! it, never relax the test.

use chrono::Utc;
use fabro_types::{RunStatus, SuccessReason};

/// The taxonomy vocabulary itself: both fork success reasons exist and
/// serialize snake_case on the wire (Slack renders the reason string).
#[test]
fn fork_success_reasons_exist_and_serialize() {
    assert_eq!(
        serde_json::to_string(&SuccessReason::PublishBlocked).unwrap(),
        "\"publish_blocked\""
    );
    assert_eq!(
        serde_json::to_string(&SuccessReason::Boundary).unwrap(),
        "\"boundary\""
    );
    // A green publish-blocked run stays SUCCESS-shaped: the graph's work
    // succeeded; only delivery is incomplete (fabro-67e5's core invariant).
    let status = RunStatus::Succeeded {
        reason: SuccessReason::PublishBlocked,
    };
    assert!(status.is_terminal());
    assert!(matches!(status, RunStatus::Succeeded {
        reason: SuccessReason::PublishBlocked,
    }));
}

/// The classification helpers used by the fold seams are pure and decide
/// on the recorded pull-request creation state alone.
#[test]
fn publish_creation_failed_classifies_on_error_presence() {
    // exercised through lib tests in projection; this pin guards the
    // public shape (no panic, terminal status preserved).
    let status = RunStatus::Succeeded {
        reason: SuccessReason::PublishBlocked,
    };
    let _ = Utc::now();
    assert!(format!("{status}").contains("publish_blocked"));
}
