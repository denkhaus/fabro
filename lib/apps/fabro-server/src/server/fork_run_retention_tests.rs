//! Fork-only run-retention supervisor tests (fabro-b1a4): the config →
//! request seam the SCHEDULED pass sends (the manual CLI builds the same
//! shape through its own args), the enabled/disabled pass behavior over a
//! real isolated `AppState`, and the full delete chain for a terminal run
//! older than the configured age.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{Duration as ChronoDuration, Utc};
use fabro_config::Storage;
use fabro_types::RunId;

use super::fork_run_retention::{self, RetentionPassStats};
use super::tests::test_app_state_with_isolated_storage;
use super::*;

fn set_retention(state: &AppState, max_run_age_days: u64) {
    let mut settings = (*state.server_settings()).clone();
    settings.server.retention.max_run_age_days = max_run_age_days;
    *state.server_settings.write().expect("settings lock") = Arc::new(settings);
}

/// The scheduled pass asks for exactly this: the configured age in the
/// CLI's own `older_than` grammar, execution (not dry-run), no other
/// filter widening the blast radius.
#[test]
fn retention_request_names_configured_age_for_execution() {
    let request = fork_run_retention::retention_request(14).expect("enabled age builds a request");

    assert_eq!(request.older_than.as_deref(), Some("14d"));
    assert!(!request.dry_run);
    assert!(request.before.is_none());
    assert!(request.workflow.is_none());
    assert!(!request.orphans);
    assert!(request.labels.is_empty());
}

/// `0` disables retention: no request, no plan, no deletion.
#[test]
fn retention_request_is_none_when_disabled() {
    assert!(fork_run_retention::retention_request(0).is_none());
}

#[tokio::test]
async fn retention_pass_is_a_noop_when_disabled() {
    let state = test_app_state_with_isolated_storage();
    set_retention(&state, 0);

    let stats = fork_run_retention::run_retention_pass(&state)
        .await
        .expect("disabled pass must not fail");

    assert!(stats.is_none(), "disabled retention must not plan");
}

#[tokio::test]
async fn retention_pass_plans_an_empty_store_with_the_configured_age() {
    let state = test_app_state_with_isolated_storage();
    set_retention(&state, 14);

    let stats = fork_run_retention::run_retention_pass(&state)
        .await
        .expect("enabled pass over an empty store must not fail")
        .expect("enabled configuration must report stats");

    assert_eq!(
        stats,
        RetentionPassStats {
            planned:       0,
            deleted_count: 0,
            freed_bytes:   0,
        },
        "an empty store yields an empty plan"
    );
}

/// Full chain: a terminal run older than the configured age is planned
/// AND executed away — store row and scratch dir gone. This is the
/// retention semantics the manual CLI shares (fabro-b1a4).
#[tokio::test]
async fn retention_pass_deletes_terminal_runs_older_than_configured_age() {
    let state = test_app_state_with_isolated_storage();
    set_retention(&state, 14);
    let old = Utc::now() - ChronoDuration::days(15);
    // The age filter keys on the run's own clock: RunInfo.start_time_dt
    // derives from the RunId's embedded ULID timestamp (run_lookup), so
    // the fixture id must itself be 15 days old.
    let run_id = RunId::from(ulid::Ulid::from_parts(
        u64::try_from(old.timestamp_millis()).expect("post-1970 timestamp"),
        0,
    ));

    let mut projection = fabro_types::RunProjection::new(
        "old terminal run".to_string(),
        fabro_types::RunSpec {
            run_id,
            settings: fabro_types::WorkflowSettings::default(),
            graph: fabro_types::RunGraph::new("retention"),
            graph_source: None,
            workflow_slug: None,
            workflow_version_id: None,
            target: None,
            automation: None,
            source_directory: None,
            labels: HashMap::default(),
            provenance: fabro_types::test_support::test_run_provenance(),
            definition_blob: None,
            spec_blob: None,
            git: None,
            fork_source_ref: None,
            admission: fabro_types::PetriAdmission::default(),
        },
        old,
    );
    projection.status = fabro_types::RunStatus::Succeeded {
        reason: fabro_types::SuccessReason::Completed,
    };

    let pool = state.stores.run_summaries.pool();
    let mut tx = pool.begin().await.expect("seed tx");
    fabro_store::RunSummaryStore::write_petri_run_row_on_connection(&mut tx, &run_id, &projection)
        .await
        .expect("seed run row");
    tx.commit().await.expect("seed commit");

    let storage = Storage::new(state.server_storage_dir());
    let scratch_root = storage.run_scratch(&run_id).root().to_path_buf();
    tokio::fs::create_dir_all(&scratch_root)
        .await
        .expect("scratch dir");
    tokio::fs::write(scratch_root.join("marker"), b"payload")
        .await
        .expect("scratch file");

    let before = state
        .stores
        .run_summaries
        .list_all(Utc::now())
        .await
        .expect("list before");
    assert_eq!(before.len(), 1, "the old terminal run is stored");

    let stats = fork_run_retention::run_retention_pass(&state)
        .await
        .expect("pass must not fail")
        .expect("enabled configuration must report stats");

    assert_eq!(stats.deleted_count, 1, "the old run is deleted");
    assert_eq!(stats.planned, 1);

    let after = state
        .stores
        .run_summaries
        .list_all(Utc::now())
        .await
        .expect("list after");
    assert!(after.is_empty(), "the run row is gone");
    assert!(!scratch_root.exists(), "the run scratch dir is reclaimed");
}
