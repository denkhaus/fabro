//! Fork-only run-stall watchdog tests (fabro-d4f2): the crossing-window
//! selection — a `running` run warns once as its silence passes the
//! 90-minute threshold; recent runs, terminal runs and already-crossed
//! runs do not warn again.

use std::collections::HashMap;

use chrono::{Duration as ChronoDuration, Utc};
use fabro_types::RunId;

use super::tests::test_app_state_with_isolated_storage;
use super::{fork_run_stall_watchdog, *};

/// Seed one run row in the given status with `last_event_at_ms` offset
/// from now, through the store's own write path.
async fn seed_run_row(
    state: &AppState,
    run_id: &RunId,
    status: fabro_types::RunStatus,
    last_event_offset: ChronoDuration,
) {
    let now = Utc::now();
    let mut projection = fabro_types::RunProjection::new(
        format!("{status:?} fixture"),
        fabro_types::RunSpec {
            run_id:              *run_id,
            settings:            fabro_types::WorkflowSettings::default(),
            graph:               fabro_types::RunGraph::new("stall"),
            graph_source:        None,
            workflow_slug:       None,
            workflow_version_id: None,
            target:              None,
            automation:          None,
            source_directory:    None,
            labels:              HashMap::default(),
            provenance:          fabro_types::test_support::test_run_provenance(),
            definition_blob:     None,
            spec_blob:           None,
            git:                 None,
            fork_source_ref:     None,
            admission:           fabro_types::PetriAdmission::default(),
        },
        now,
    );
    projection.status = status;

    let pool = state.stores.run_summaries.pool();
    let mut tx = pool.begin().await.expect("seed tx");
    fabro_store::RunSummaryStore::write_petri_run_row_on_connection(&mut tx, run_id, &projection)
        .await
        .expect("seed run row");
    sqlx::query("UPDATE runs SET last_event_at_ms = ? WHERE id = ?")
        .bind((now + last_event_offset).timestamp_millis())
        .bind(run_id.to_string())
        .execute(&mut *tx)
        .await
        .expect("seed last_event_at_ms");
    tx.commit().await.expect("seed commit");
}

/// The incident shape: silence just past 90 minutes warns.
#[tokio::test]
async fn stall_watchdog_warns_runs_crossing_the_threshold() {
    let state = test_app_state_with_isolated_storage();
    let run_id = RunId::new();
    seed_run_row(
        &state,
        &run_id,
        fabro_types::RunStatus::Running,
        ChronoDuration::minutes(-92),
    )
    .await;

    let warned = fork_run_stall_watchdog::stall_watchdog_pass(&state, Utc::now())
        .await
        .expect("watchdog pass must not fail");

    assert_eq!(warned, 1, "the silent running run is selected");
}

/// Recent silence and terminal status never warn.
#[tokio::test]
async fn stall_watchdog_ignores_recent_and_terminal_runs() {
    let state = test_app_state_with_isolated_storage();
    let recent = RunId::new();
    seed_run_row(
        &state,
        &recent,
        fabro_types::RunStatus::Running,
        ChronoDuration::minutes(-30),
    )
    .await;
    let terminal = RunId::new();
    seed_run_row(
        &state,
        &terminal,
        fabro_types::RunStatus::Succeeded {
            reason: fabro_types::SuccessReason::Completed,
        },
        ChronoDuration::minutes(-92),
    )
    .await;

    let warned = fork_run_stall_watchdog::stall_watchdog_pass(&state, Utc::now())
        .await
        .expect("watchdog pass must not fail");

    assert_eq!(
        warned, 0,
        "healthy running runs and silent terminal runs are not selected"
    );
}

/// Once-per-episode: the pass after the crossing pass warns nothing —
/// the run left the window (its last event is now older than the
/// window's far edge).
#[tokio::test]
async fn stall_watchdog_warns_once_per_episode() {
    let state = test_app_state_with_isolated_storage();
    let run_id = RunId::new();
    // Silence at -95min: inside the first pass's window [t0-105, t0-90),
    // older than the second pass's far edge at (t0+15)-105 = t0-90.
    seed_run_row(
        &state,
        &run_id,
        fabro_types::RunStatus::Running,
        ChronoDuration::minutes(-95),
    )
    .await;

    // Anchor both passes on explicit clock values: pass 1 at t0, pass 2
    // one interval later.
    let t0 = Utc::now();
    let first = fork_run_stall_watchdog::stall_watchdog_pass(&state, t0)
        .await
        .expect("first pass");
    let t1 = t0 + ChronoDuration::minutes(15);
    let second = fork_run_stall_watchdog::stall_watchdog_pass(&state, t1)
        .await
        .expect("second pass");

    assert_eq!(first, 1, "the crossing pass warns");
    assert_eq!(second, 0, "the follow-up pass stays silent");
}
