//! Fork-only terminal-cleanup presence pin (work#1478): the pure
//! resolution/summarizing helpers of the terminal cleanup hook, plus the
//! serde contract of the `terminal_cleanup` declaration on the dense
//! `WorkflowNamespace` (absent when undeclared, defaulted when a stored
//! spec predates the key). The full sandbox-exec path is proven by the
//! fabro-test workbench probe (docker-gated), not here.

use chrono::Utc;
use fabro_types::settings::WorkflowNamespace;
use fabro_types::{RunId, RunProjection};

use super::run_terminal_cleanup::{
    HandledRuns, declared_terminal_cleanup, mark_handled, stderr_reason, stdout_summary,
};

/// A minimal terminal-shaped projection whose workflow declaration is
/// `terminal_cleanup`.
fn projection_with_cleanup(terminal_cleanup: Option<&str>) -> RunProjection {
    let mut spec = fabro_types::test_support::test_run_spec();
    spec.settings.workflow.terminal_cleanup = terminal_cleanup.map(str::to_string);
    RunProjection::new(String::new(), spec, Utc::now())
}

#[test]
fn declared_terminal_cleanup_is_none_when_undeclared() {
    let run_state = projection_with_cleanup(None);
    assert!(declared_terminal_cleanup(&run_state).is_none());
}

#[test]
fn declared_terminal_cleanup_is_none_when_blank() {
    for blank in ["", "   ", "\t\n "] {
        let run_state = projection_with_cleanup(Some(blank));
        assert!(
            declared_terminal_cleanup(&run_state).is_none(),
            "blank declaration {blank:?} must be the permanent no-op"
        );
    }
}

#[test]
fn declared_terminal_cleanup_returns_the_trimmed_command() {
    let run_state = projection_with_cleanup(Some("  nu .fabro/scripts/run-cleanup.nu\n"));
    assert_eq!(
        declared_terminal_cleanup(&run_state).as_deref(),
        Some("nu .fabro/scripts/run-cleanup.nu")
    );
}

#[test]
fn mark_handled_admits_each_run_exactly_once() {
    let handled = HandledRuns::default();
    let run_id = RunId::new();
    assert!(mark_handled(&handled, run_id));
    // The once-only gate: a terminal run's later signals never revisit it.
    assert!(!mark_handled(&handled, run_id));
    // A different run is still admitted.
    assert!(mark_handled(&handled, RunId::new()));
}

#[test]
fn stdout_summary_takes_the_first_trimmed_line() {
    assert_eq!(
        stdout_summary(b"  freed 4096 KiB\nsecond line\n"),
        "freed 4096 KiB"
    );
    assert_eq!(stdout_summary(b"\n\nleading blank"), "");
    assert_eq!(stdout_summary(b""), "");
}

#[test]
fn stdout_summary_is_bounded() {
    let long = "x".repeat(600);
    let summary = stdout_summary(format!("{long}\n").as_bytes());
    assert_eq!(summary.chars().count(), 200);
}

#[test]
fn stderr_reason_takes_the_last_non_empty_line() {
    assert_eq!(
        stderr_reason(b"first\nrun-cleanup: /opt/target: permission denied\n\n"),
        "run-cleanup: /opt/target: permission denied"
    );
    assert_eq!(stderr_reason(b"  \n"), "");
    assert_eq!(stderr_reason(b""), "");
}

#[test]
fn stderr_reason_is_bounded() {
    let long = "y".repeat(600);
    let reason = stderr_reason(format!("\n{long}").as_bytes());
    assert_eq!(reason.chars().count(), 200);
}

#[test]
fn workflow_namespace_serde_omits_undeclared_and_defaults_absent() {
    // Byte-stability: an undeclared workflow serializes without the key.
    let undeclared = serde_json::to_value(WorkflowNamespace::default())
        .expect("serialize undeclared workflow namespace");
    assert!(
        undeclared.get("terminal_cleanup").is_none(),
        "undeclared terminal_cleanup must not serialize: {undeclared}"
    );

    // A stored spec from before the key existed stays deserializable.
    let legacy: WorkflowNamespace =
        serde_json::from_str(r#"{"graph":"workflow.fabro","metadata":{}}"#)
            .expect("legacy workflow namespace deserializes");
    assert!(legacy.terminal_cleanup.is_none());

    // A declared command round-trips.
    let declared = WorkflowNamespace {
        terminal_cleanup: Some("nu cleanup.nu".into()),
        ..Default::default()
    };
    let value = serde_json::to_value(&declared).expect("serialize declared");
    assert_eq!(value["terminal_cleanup"], "nu cleanup.nu");
}
