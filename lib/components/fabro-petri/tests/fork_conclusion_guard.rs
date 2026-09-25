//! Fork presence pin (fabro-b00c, user decision 2026-09-25 — engine-side):
//! a green conclusion over a recorded leg failure is refused. The guard
//! lives in the projection fold's conclusion path; these tests pin the
//! decision rule. A merge resolution that drops them dropped the feature
//! — restore it, never relax the test.

use fabro_petri::projection::fork_taxonomy::refuses_green_conclusion;
use fabro_types::{RunStatus, SuccessReason};

fn completed() -> RunStatus {
    RunStatus::Succeeded {
        reason: SuccessReason::Completed,
    }
}

/// The rule: a plain Completed success with a recorded failure is refused —
/// the observed shape of every catch-all-green pass (status=success WITH
/// failure recorded, fabro-b00c instances 1-7).
#[test]
fn a_recorded_failure_refuses_a_completed_green() {
    assert!(refuses_green_conclusion(
        &completed(),
        Some("bad_output after 2 repair turns")
    ));
    assert!(refuses_green_conclusion(
        &completed(),
        Some("stage envelope violation in `implementer`")
    ));
}

/// A clean success with no recorded failure stays green.
#[test]
fn a_clean_completed_green_stands() {
    assert!(!refuses_green_conclusion(&completed(), None));
    assert!(!refuses_green_conclusion(&completed(), Some("")));
}

/// Explicit escapes stay green: PublishBlocked set its own success reason
/// (delivery incomplete, work green); the boundary success is an explicit
/// x.kind exit decision. Neither is downgraded by the guard.
#[test]
fn explicit_green_escapes_are_not_refused() {
    for reason in [SuccessReason::PublishBlocked, SuccessReason::Boundary] {
        let status = RunStatus::Succeeded { reason };
        assert!(!refuses_green_conclusion(
            &status,
            Some("a recorded failure rides along")
        ));
    }
}

/// Failures and parks are not the guard's business: only green conclusions
/// are refused.
#[test]
fn non_success_statuses_pass_through_untouched() {
    let failed = RunStatus::Failed {
        reason: fabro_types::FailureReason::WorkflowError,
    };
    assert!(!refuses_green_conclusion(&failed, Some("already failed")));
    let blocked = RunStatus::Blocked {
        blocked_reason: fabro_types::BlockedReason::QuotaRateLimit,
    };
    assert!(!refuses_green_conclusion(&blocked, Some("parked")));
}
