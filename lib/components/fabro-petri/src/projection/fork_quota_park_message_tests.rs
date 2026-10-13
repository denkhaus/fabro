//! Fork-only pin (fabro-71e6, sprint-35 arch finding N1): the quota
//! park's MESSAGE SOURCE falls back to the first failed stage's
//! failure_reason when the invocation-level failure is empty
//! (fabro-a73dea, run 01M4NSBKV6ZBPAFFEFND65HEP9: the loop graph's
//! designed `planner -> exit [outcome=failed]` arm consumed the failure,
//! the invocation-level message stayed empty, and the run concluded
//! green with the window closed — the green-lie the park exists to
//! prevent). The upstream pin sits in tests/projection.rs, which an
//! upstream merge rewrites freely; this file survives it.

use fabro_types::outcome::StageOutcome;
use fabro_types::{RunProjection, StageCompletion, first_event_seq, test_support as types_support};

fn projection_with_stages(
    stages: &[(&str, u32, Option<StageOutcome>, Option<&str>)],
) -> RunProjection {
    let mut projection = RunProjection::new(
        "pin".to_string(),
        types_support::test_run_spec(),
        chrono::Utc::now(),
    );
    for (node, seq, outcome, failure_reason) in stages {
        let stage = projection.stage_entry(node, 1, first_event_seq(*seq));
        if let Some(outcome) = outcome {
            stage.completion = Some(StageCompletion {
                outcome:        *outcome,
                notes:          None,
                failure_reason: failure_reason.map(str::to_string),
                timestamp:      chrono::Utc::now(),
            });
        }
    }
    projection
}

/// The park's fallback source reads the FIRST failed stage in execution
/// order, skipping empty reasons: an earlier failure with an empty reason
/// must not shadow the window message a later stage carries.
#[test]
fn the_fallback_reads_the_first_failed_stages_reason_in_execution_order() {
    let window = "Usage limit reached for 5 hour. Your limit will reset at 2026-10-11 05:04:33";
    let projection = projection_with_stages(&[
        // Earliest stage: failed but reasonless — not a message source.
        (
            "planner",
            10,
            Some(StageOutcome::Failed {
                retry_requested: false,
            }),
            Some(""),
        ),
        // The window-closed stage, later in execution order.
        (
            "reviewer",
            30,
            Some(StageOutcome::Failed {
                retry_requested: false,
            }),
            Some(window),
        ),
        // A later success never participates.
        ("exit", 50, Some(StageOutcome::Succeeded), None),
    ]);
    assert_eq!(
        super::coordinator::first_failed_stage_failure(&projection).as_deref(),
        Some(window),
        "the fallback names the first NON-EMPTY failed-stage reason"
    );
    // No failed stage with a reason: no message, no park.
    let green = projection_with_stages(&[
        ("planner", 10, Some(StageOutcome::Succeeded), None),
        ("exit", 20, Some(StageOutcome::Succeeded), None),
    ]);
    assert_eq!(super::coordinator::first_failed_stage_failure(&green), None);
}

/// Presence pin on the wiring (the include_str! precedent from the fork's
/// gate tests): the park's message source must still feed the fold through
/// the fallback — removing it resurrects the 01M4NSBK green-lie.
#[test]
fn the_park_message_source_still_feeds_the_fold_through_the_fallback() {
    let source = include_str!("coordinator.rs");
    assert!(
        source.contains(".or_else(|| first_failed_stage_failure(projection))"),
        "the park's message source must keep its stage-level fallback (run 01M4NSBK)"
    );
}
