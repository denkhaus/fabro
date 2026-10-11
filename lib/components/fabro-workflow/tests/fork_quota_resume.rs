//! Fork presence pin: a quota-parked run is resumable (fabro-2c4a).
//!
//! A run whose provider usage window closed in flight parks as
//! `Blocked { QuotaRateLimit }` (the fork's coordinator, fabro-2e7b /
//! fabro-6ac5) and the line's recovery promises a resume — but retry,
//! rewind and fork refused every `Blocked` status through
//! `ensure_terminal`, so the promised resume had no path at all. These
//! assertions pin the resume contract in a fork-owned file: an upstream
//! merge can never silently drop it.

use chrono::Utc;
use fabro_types::test_support::test_run_provenance;
use fabro_types::{
    BlockedReason, FailureReason, PetriAdmission, RunGraph, RunProjection, RunSpec, RunStatus,
    WorkflowSettings, fixtures,
};
use fabro_workflow::operations;

fn source(status: RunStatus) -> RunProjection {
    let mut projection = RunProjection::new(
        "Parked source".to_string(),
        RunSpec {
            run_id:              fixtures::RUN_1,
            settings:            WorkflowSettings::default(),
            graph:               RunGraph::new("source"),
            graph_source:        Some("digraph source { start -> exit }".to_string()),
            workflow_slug:       Some("source".to_string()),
            workflow_version_id: None,
            target:              None,
            automation:          None,
            source_directory:    None,
            labels:              std::collections::HashMap::new(),
            provenance:          test_run_provenance(),
            definition_blob:     None,
            spec_blob:           None,
            git:                 None,
            fork_source_ref:     None,
            admission:           PetriAdmission::default(),
        },
        Utc::now(),
    );
    projection.status = status;
    projection
}

#[test]
fn a_quota_park_is_terminal_enough_to_resume() {
    let parked = source(RunStatus::Blocked {
        blocked_reason: BlockedReason::QuotaRateLimit,
    });
    operations::ensure_terminal(&parked, &fixtures::RUN_1, "retry")
        .expect("a quota park must be retryable — retry IS the resume");
    operations::ensure_terminal(&parked, &fixtures::RUN_1, "rewind")
        .expect("a quota park must be rewindable");
}

#[test]
fn an_active_block_is_not_a_park() {
    let waiting = source(RunStatus::Blocked {
        blocked_reason: BlockedReason::HumanInputRequired,
    });
    assert!(
        operations::ensure_terminal(&waiting, &fixtures::RUN_1, "retry").is_err(),
        "a run waiting for an answer must not be retryable"
    );
}

#[test]
fn the_park_resumes_the_interrupted_last_stage() {
    assert!(
        operations::reruns_last(RunStatus::Blocked {
            blocked_reason: BlockedReason::QuotaRateLimit,
        }),
        "the window closed in flight, so the last checkpointed stage is the work to finish"
    );
    assert!(!operations::reruns_last(RunStatus::Blocked {
        blocked_reason: BlockedReason::HumanInputRequired,
    }));
    assert!(operations::reruns_last(RunStatus::Failed {
        reason: FailureReason::SoftStop,
    }));
}

#[test]
fn fabro_2c4a_resume_seam_exists() {
    let status = include_str!("../../../foundation/fabro-types/src/status.rs");
    assert!(
        status.contains("pub fn is_resumable_park"),
        "the is_resumable_park predicate must exist in fabro-types"
    );
    assert!(
        status.contains("QuotaRateLimit"),
        "the quota park reason must stay named"
    );
    let fork = include_str!("../src/operations/fork.rs");
    assert!(
        fork.contains("current.is_resumable_park()"),
        "ensure_terminal must accept the resumable park"
    );
}
