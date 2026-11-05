//! Fork presence pin (denkhaus/work#1072): the `human_needed` terminal
//! status kind survives on the fork. The deadlock-exit family parks a
//! run as a decision request — the board gains a decision inbox, and
//! the adjudicator routes these runs to `merge: parked` until the
//! operator answers through the work-item comment channel. These tests
//! exist only on the fork; a merge resolution that drops them dropped
//! the feature — restore it, never relax the test.

use fabro_types::{RunStatus, RunStatusKind, TerminalStatus};

/// The kind exists, serializes snake_case on the wire, and boards in
/// the decision column (rank 5, ahead of the outcome columns — the API
/// `BoardColumn` enum mirrors this order).
#[test]
fn human_needed_kind_exists_and_boards_in_the_decision_column() {
    assert_eq!(
        serde_json::to_string(&RunStatusKind::HumanNeeded).unwrap(),
        "\"human_needed\""
    );
    assert_eq!(RunStatusKind::HumanNeeded.board_rank(), 5);
    // The outcome columns sit behind the decision column.
    assert!(RunStatusKind::HumanNeeded.board_rank() < RunStatusKind::Succeeded.board_rank());
}

/// The status is terminal and immutable, with a terminal projection:
/// poll loops and finalization treat it as done, and no lifecycle event
/// moves it except the `* -> Dead` escape hatch.
#[test]
fn human_needed_status_is_terminal_and_frozen() {
    let status = RunStatus::HumanNeeded;
    assert!(status.is_terminal());
    assert!(status.is_immutable());
    assert!(!status.is_active());
    assert_eq!(status.terminal_status(), Some(TerminalStatus::HumanNeeded));
    assert_eq!(status.to_string(), "human_needed");
    assert_eq!(
        serde_json::to_string(&status).unwrap(),
        r#"{"kind":"human_needed"}"#
    );
}
