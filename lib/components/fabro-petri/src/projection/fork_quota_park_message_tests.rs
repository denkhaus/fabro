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

/// The conclusion's view of a projection: each node at its final visit.
fn final_visits_of(projection: &RunProjection) -> Vec<super::coordinator::FinalVisit> {
    super::coordinator::final_visit_stages(projection)
}

/// One stage row the helper plants: node, visit, first event seq,
/// outcome, failure reason.
type StageRow<'a> = (&'a str, u32, u32, Option<StageOutcome>, Option<&'a str>);

fn projection_with_stages(stages: &[StageRow<'_>]) -> RunProjection {
    let mut projection = RunProjection::new(
        "pin".to_string(),
        types_support::test_run_spec(),
        chrono::Utc::now(),
    );
    for (node, visit, seq, outcome, failure_reason) in stages {
        let stage = projection.stage_entry(node, *visit, first_event_seq(*seq));
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
            1,
            10,
            Some(StageOutcome::Failed {
                retry_requested: false,
            }),
            Some(""),
        ),
        // The window-closed stage, later in execution order.
        (
            "reviewer",
            1,
            30,
            Some(StageOutcome::Failed {
                retry_requested: false,
            }),
            Some(window),
        ),
        // A later success never participates.
        ("exit", 1, 50, Some(StageOutcome::Succeeded), None),
    ]);
    assert_eq!(
        super::coordinator::first_failed_stage_failure(&final_visits_of(&projection)).as_deref(),
        Some(window),
        "the fallback names the first NON-EMPTY failed-stage reason"
    );
    // No failed stage with a reason: no message, no park.
    let green = projection_with_stages(&[
        ("planner", 1, 10, Some(StageOutcome::Succeeded), None),
        ("exit", 1, 20, Some(StageOutcome::Succeeded), None),
    ]);
    assert_eq!(
        super::coordinator::first_failed_stage_failure(&final_visits_of(&green)),
        None
    );
}

/// fabro-29e3: a leg is a NODE, not a visit — a failure the same node's
/// later successful visit superseded (the loop lane's gate-bounce cycle:
/// tester red, implementer repairs, tester green) never reaches the
/// conclusion. The run 01M50BH4 shape: every final visit green, the
/// engine's invocation-level failure still carrying the tester@1 echo.
#[test]
fn a_superseded_visit_failure_never_feeds_the_park_source() {
    let window = "Usage limit reached for 5 hour. Your limit will reset at 2026-10-15 23:04:33";
    let projection = projection_with_stages(&[
        (
            "tester",
            1,
            10,
            Some(StageOutcome::Failed {
                retry_requested: false,
            }),
            Some(window),
        ),
        ("implementer", 1, 20, Some(StageOutcome::Succeeded), None),
        ("tester", 2, 30, Some(StageOutcome::Succeeded), None),
        ("exit", 1, 40, Some(StageOutcome::Succeeded), None),
    ]);
    assert_eq!(
        super::coordinator::first_failed_stage_failure(&final_visits_of(&projection)),
        None,
        "a recovered gate bounce is not a leg failure"
    );
    // The same node failing again after a green visit fails for real:
    // visit 3 is the final visit and no later visit supersedes it.
    let relapsed = projection_with_stages(&[
        ("tester", 1, 10, Some(StageOutcome::Succeeded), None),
        (
            "tester",
            2,
            20,
            Some(StageOutcome::Failed {
                retry_requested: false,
            }),
            Some(window),
        ),
    ]);
    assert_eq!(
        super::coordinator::first_failed_stage_failure(&final_visits_of(&relapsed)).as_deref(),
        Some(window),
        "a node whose FINAL visit failed still names its failure"
    );
}

/// Presence pin on the wiring (the include_str! precedent from the fork's
/// gate tests): the stages' final outcomes must still lead the park's
/// message source, with the recorded message covering only a failed final
/// visit whose reason is empty — dropping either half resurrects a
/// green-lie (01M4NSBK) or parks a recovered bounce (01M50BH4).
#[test]
fn the_park_message_source_leads_with_the_stages_final_outcomes() {
    let source = include_str!("coordinator.rs");
    assert!(
        source.contains("let park_message = first_failed_stage_failure(&final_visits).or_else("),
        "the park's message source must lead with the stages (fabro-29e3)"
    );
    assert!(
        source.contains(".then(|| failure_message.clone())"),
        "a failed final visit keeps the recorded message as its voice"
    );
}
