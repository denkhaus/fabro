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

fn set_sandbox_reap(state: &AppState, floor_hours: u64) {
    let mut settings = (*state.server_settings()).clone();
    settings.server.retention.terminal_sandbox_age_hours = floor_hours;
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

//
// #1299 information clock: the triage gate on the record reap (user
// decision, grill 2026-11-13 — a run with ANY untriaged observation is
// not reapable; the gate is the no-loss guarantee until the sweep
// consumer lands).
//

/// Seeds a terminal run whose ULID clock is `days` days old (the age
/// filter keys on the id's embedded timestamp) and returns its id.
async fn seed_old_terminal_run(state: &AppState, days: i64) -> RunId {
    seed_old_terminal_run_with(state, days, |_| {}).await
}

/// The seed with a projection mutation applied before the row is
/// written (the sandbox-reap tests attach a sandbox instance this way).
async fn seed_old_terminal_run_with<F>(state: &AppState, days: i64, mutate: F) -> RunId
where
    F: FnOnce(&mut fabro_types::RunProjection),
{
    let old = Utc::now() - ChronoDuration::days(days);
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
    // The summary's completed_at (the reap arm's clock) derives from the
    // projection's conclusion record.
    projection.conclusion = Some(fabro_types::Conclusion {
        timestamp:            old,
        status:               fabro_types::StageOutcome::Succeeded,
        timing:               fabro_types::RunTiming::default(),
        failure:              None,
        final_git_commit_sha: None,
        stages:               Vec::new(),
        usage:                None,
        diff:                 fabro_types::RunDiff::default(),
        total_retries:        0,
    });
    mutate(&mut projection);

    let pool = state.stores.run_summaries.pool();
    let mut tx = pool.begin().await.expect("seed tx");
    fabro_store::RunSummaryStore::write_petri_run_row_on_connection(&mut tx, &run_id, &projection)
        .await
        .expect("seed run row");
    tx.commit().await.expect("seed commit");

    // The plan walks the storage run dirs; a run without a scratch dir is
    // invisible to it (the full-chain test's pattern).
    let storage = fabro_config::Storage::new(state.server_storage_dir());
    let scratch_root = storage.run_scratch(&run_id).root().to_path_buf();
    tokio::fs::create_dir_all(&scratch_root)
        .await
        .expect("scratch dir");
    tokio::fs::write(scratch_root.join("marker"), b"payload")
        .await
        .expect("scratch file");
    run_id
}

/// Seeds one painpoint observation on the run and returns its stored seq.
async fn seed_observation(state: &AppState, run_id: &RunId) -> u64 {
    let stored = state
        .stores
        .run_summaries
        .platform_records()
        .append(
            run_id,
            &fabro_store::platform_records::PlatformRecord::ObservationRecorded(
                fabro_store::platform_records::ObservationRecordedRecord {
                    category:  fabro_types::ObservationCategory::Painpoint,
                    text:      "seeded observation".to_string(),
                    execution: 0,
                    firing:    1,
                },
            ),
            None,
        )
        .await
        .expect("seed observation record");
    stored.seq
}

#[tokio::test]
async fn retention_pass_spares_a_run_with_an_untriaged_observation() {
    let state = test_app_state_with_isolated_storage();
    set_retention(&state, 14);
    let run_id = seed_old_terminal_run(&state, 15).await;
    seed_observation(&state, &run_id).await;

    let stats = fork_run_retention::run_retention_pass(&state)
        .await
        .expect("pass must not fail")
        .expect("enabled configuration must report stats");

    assert_eq!(
        stats.deleted_count, 0,
        "an untriaged observation gates the record reap (#1299)"
    );
    let after = state
        .stores
        .run_summaries
        .list_all(Utc::now())
        .await
        .expect("list after");
    assert_eq!(after.len(), 1, "the gated run row survives");
}

#[tokio::test]
async fn retention_pass_reaps_a_run_whose_every_observation_is_linked() {
    let state = test_app_state_with_isolated_storage();
    set_retention(&state, 14);
    state
        .work_store()
        .upsert_project(&fabro_work::WorkProject {
            project_id:               "fabro".to_owned(),
            display_name:             "Fabro".to_owned(),
            tracker_provider:         "github".to_owned(),
            tracker_repo:             "denkhaus/work".to_owned(),
            target_repo:              None,
            project_label:            "project:fabro".to_owned(),
            lane_labels:              std::collections::BTreeMap::new(),
            claim_label_prefix:       "fabro:run:".to_owned(),
            claim_comment_enabled:    false,
            backpressure_open_pr_cap: 0,
        })
        .await
        .expect("seed project");
    let run_id = seed_old_terminal_run(&state, 15).await;
    let seq = seed_observation(&state, &run_id).await;
    state
        .work_store()
        .link_observation("fabro", "denkhaus/work#1", &run_id, seq)
        .await
        .expect("seed link");

    let stats = fork_run_retention::run_retention_pass(&state)
        .await
        .expect("pass must not fail")
        .expect("enabled configuration must report stats");

    assert_eq!(
        stats.deleted_count, 1,
        "a fully triaged run reaps with the record clock"
    );
    let after = state
        .stores
        .run_summaries
        .list_all(Utc::now())
        .await
        .expect("list after");
    assert!(after.is_empty(), "the triaged run is gone");
}

/// Seeds the run's sandbox instance (retained: the arm's cursor).
fn seed_sandbox(projection: &mut fabro_types::RunProjection, retained: bool) {
    projection.sandbox = Some(fabro_types::RunSandbox::ready(
        fabro_types::RunSandboxPlan {
            provider: fabro_types::SandboxProviderKind::DOCKER,
            image:    None,
            snapshot: None,
        },
        fabro_types::RunSandboxInstance {
            provider:          fabro_types::SandboxProviderKind::DOCKER,
            image:             None,
            snapshot:          None,
            runtime:           fabro_types::RunSandboxRuntime {
                id:                "petri-env-test".to_string(),
                working_directory: "/workspace".to_string(),
                repo_cloned:       None,
                clone_origin_url:  None,
                clone_branch:      None,
                workspace_root:    None,
                repos_root:        None,
                primary_repo_link: None,
                primary_repo_path: None,
            },
            ready_duration_ms: None,
            retained:          Some(retained),
        },
    ));
}

//
// #1299 disk clock: the sandbox-reap arm (work#168's grilled design).
//

#[tokio::test]
async fn sandbox_reap_pass_is_a_noop_when_disabled() {
    let state = test_app_state_with_isolated_storage();
    set_sandbox_reap(&state, 0);

    let stats = fork_run_retention::reap_sandboxes_pass(&state)
        .await
        .expect("disabled arm must not fail");

    assert!(
        stats.is_none(),
        "terminal_sandbox_age_hours=0 disables the arm (and the brake with it)"
    );
}

#[tokio::test]
async fn sandbox_reap_pass_fails_open_per_run_when_the_provider_config_is_absent() {
    let state = test_app_state_with_isolated_storage();
    set_sandbox_reap(&state, 48);
    // The test state configures no docker provider: the per-run prune
    // cannot load credentials, WARNs, and the pass fails OPEN — the run
    // row, its records, and its sandbox cursor stay untouched.
    let _run_id = seed_old_terminal_run_with(&state, 15, |projection| {
        seed_sandbox(projection, true);
    })
    .await;

    let stats = fork_run_retention::reap_sandboxes_pass(&state)
        .await
        .expect("the pass itself must not fail")
        .expect("the enabled arm reports stats");

    assert_eq!(stats.candidates, 1, "the old terminal run is a candidate");
    assert_eq!(
        stats.failed, 1,
        "the absent provider config fails the run open"
    );
    assert_eq!(stats.reaped, 0);
    let after = state
        .stores
        .run_summaries
        .list_all(Utc::now())
        .await
        .expect("list after");
    assert_eq!(after.len(), 1, "the run row survives a failed reap");
}

#[tokio::test]
async fn sandbox_reap_pass_skips_young_terminal_runs_without_the_brake() {
    let state = test_app_state_with_isolated_storage();
    set_sandbox_reap(&state, 48);
    let run_id = seed_old_terminal_run_with(&state, 1, |projection| {
        seed_sandbox(projection, true);
    })
    .await;

    let stats = fork_run_retention::reap_sandboxes_pass(&state)
        .await
        .expect("the pass must not fail")
        .expect("the enabled arm reports stats");

    assert_eq!(
        stats.candidates, 0,
        "a 1h-old terminal run stays under the 48h floor"
    );
    assert_eq!(after_sandbox_retained(&state, &run_id).await, Some(true));
}

/// The stored row's sandbox retained flag — the arm's idempotency
/// cursor.
async fn after_sandbox_retained(state: &AppState, run_id: &RunId) -> Option<bool> {
    state
        .stores
        .run_summaries
        .list_all(Utc::now())
        .await
        .expect("list")
        .iter()
        .find(|run| run.id == *run_id)
        .and_then(|run| run.sandbox.as_ref())
        .and_then(fabro_types::RunSandbox::instance)
        .and_then(|instance| instance.retained)
}

//
// Removal reaper (fabro-d616 arm b) + deletion tombstone.
//

/// Seed one run row in the given status with `last_event_at_ms` offset
/// from now, through the store's own write path (plus the one column the
/// projector derives from folded events, which seeding does not have).
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
            graph:               fabro_types::RunGraph::new("reaper"),
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

/// The incident shape: a row the delete flow left in `removing` with no
/// event for over the stuck threshold is FINISHED — row gone, scratch
/// gone — by the reaper pass (fabro-d616 arm b).
#[tokio::test]
async fn removal_reaper_finishes_stuck_removing_rows() {
    let state = test_app_state_with_isolated_storage();
    let run_id = RunId::new();
    seed_run_row(
        &state,
        &run_id,
        fabro_types::RunStatus::Removing,
        ChronoDuration::minutes(-20),
    )
    .await;

    let finished = fork_run_retention::removal_reaper_pass(&state, Utc::now())
        .await
        .expect("reaper pass must not fail");

    assert_eq!(finished, 1, "the stuck removing row is finished");
    assert!(
        !state
            .stores
            .run_summaries
            .contains(&run_id)
            .await
            .expect("contains"),
        "the run row is gone"
    );
}

/// A delete that is merely slow (row fresh in `removing`) must not be
/// reaped out from under a live deletion.
#[tokio::test]
async fn removal_reaper_leaves_fresh_removing_rows() {
    let state = test_app_state_with_isolated_storage();
    let run_id = RunId::new();
    seed_run_row(
        &state,
        &run_id,
        fabro_types::RunStatus::Removing,
        ChronoDuration::minutes(-2),
    )
    .await;

    let finished = fork_run_retention::removal_reaper_pass(&state, Utc::now())
        .await
        .expect("reaper pass must not fail");

    assert_eq!(finished, 0, "a fresh removing row is not reaped");
    assert!(
        state
            .stores
            .run_summaries
            .contains(&run_id)
            .await
            .expect("contains"),
        "the run row is still there"
    );
}

/// THE fabro-d616 tombstone pin: after a delete, a LATE projector pass
/// (the resurrection vector of the 2026-10-19 incident — rm --force
/// returned success, a straggler projection re-inserted the row as
/// status=removing, the loop lane starved 10h) cannot bring the run
/// back. The upsert refuses what the tombstone guards.
#[tokio::test]
async fn deleted_runs_cannot_be_resurrected_by_a_late_projection_pass() {
    let state = test_app_state_with_isolated_storage();
    let run_id = RunId::new();
    seed_run_row(
        &state,
        &run_id,
        fabro_types::RunStatus::Removing,
        ChronoDuration::minutes(-1),
    )
    .await;

    state
        .stores
        .runs
        .delete_run(&run_id)
        .await
        .expect("delete writes the tombstone with the row removal");
    assert!(
        !state
            .stores
            .run_summaries
            .contains(&run_id)
            .await
            .expect("contains after delete"),
        "the row is gone"
    );

    // The late projector pass: same write path the projector uses, after
    // the delete completed.
    let pool = state.stores.run_summaries.pool();
    let mut tx = pool.begin().await.expect("late projection tx");
    let mut projection = fabro_types::RunProjection::new(
        "late resurrection attempt".to_string(),
        fabro_types::RunSpec {
            run_id,
            settings: fabro_types::WorkflowSettings::default(),
            graph: fabro_types::RunGraph::new("reaper"),
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
        Utc::now(),
    );
    projection.status = fabro_types::RunStatus::Running;
    fabro_store::RunSummaryStore::write_petri_run_row_on_connection(&mut tx, &run_id, &projection)
        .await
        .expect("late projector write must succeed as SQL");
    tx.commit().await.expect("late projection commit");

    assert!(
        !state
            .stores
            .run_summaries
            .contains(&run_id)
            .await
            .expect("contains after late pass"),
        "the tombstone refused the resurrection: a deleted run stays deleted"
    );
}
// ── Sandbox-reap arm (fabro-3377): candidate selection pins ────────────

/// A minimal `Run` summary for the pure reap filter: the given status,
/// terminal `completed_hours_ago`, sandbox instance with the given
/// retention.
fn reap_test_run(
    status: fabro_types::RunStatus,
    completed_hours_ago: i64,
    retained: Option<bool>,
) -> fabro_types::Run {
    use fabro_types::{
        AskFabro, RunId, RunLifecycle, RunLinks, RunOrigin, RunSandbox, RunSandboxInstance,
        RunSandboxPlan, RunSandboxRuntime, RunTimestamps, WorkflowRef,
    };

    let now = Utc::now();
    let id = RunId::new();
    let instance = RunSandboxInstance {
        provider: fabro_types::SandboxProviderKind::DOCKER,
        image: None,
        snapshot: None,
        runtime: RunSandboxRuntime {
            id:                "petri-test-l0".to_owned(),
            working_directory: "/workspace".to_owned(),
            repo_cloned:       None,
            clone_origin_url:  None,
            clone_branch:      None,
            workspace_root:    None,
            repos_root:        None,
            primary_repo_path: None,
            primary_repo_link: None,
        },
        ready_duration_ms: None,
        retained,
    };
    fabro_types::Run {
        id,
        parent_id: None,
        children_count: 0,
        claimed_seed_id: None,
        title: "reap pin".to_owned(),
        goal: "reap pin".to_owned(),
        workflow: WorkflowRef {
            slug:       None,
            name:       Some("Pin".to_owned()),
            graph_name: None,
            node_count: 0,
            edge_count: 0,
        },
        automation: None,
        repository: None,
        created_by: fabro_types::Principal::Worker { run_id: id },
        origin: RunOrigin::default(),
        labels: HashMap::default(),
        lifecycle: RunLifecycle {
            status,
            conclusion_failure: None,
            approval: None,
            pending_control: None,
            queue_position: None,
            error: None,
            archived: false,
            archived_at: None,
        },
        sandbox: Some(RunSandbox::ready(
            RunSandboxPlan {
                provider: fabro_types::SandboxProviderKind::DOCKER,
                image:    None,
                snapshot: None,
            },
            instance,
        )),
        workflow_version_id: None,
        models: Vec::new(),
        source_directory: None,
        timestamps: RunTimestamps {
            created_at:    now - ChronoDuration::hours(completed_hours_ago + 1),
            started_at:    Some(now - ChronoDuration::hours(completed_hours_ago + 1)),
            last_event_at: Some(now - ChronoDuration::hours(completed_hours_ago)),
            completed_at:  Some(now - ChronoDuration::hours(completed_hours_ago)),
        },
        timing: None,
        usage: lithos_llm::types::Usage::default(),
        size: fabro_types::RunSize::default(),
        ask_fabro: AskFabro::default(),
        diff: None,
        pull_request: None,
        current_question: None,
        superseded_by: None,
        retried_from: None,
        links: RunLinks { web: None },
    }
}

use fabro_types::{BlockedReason, FailureReason, RunStatus, SuccessReason};

#[test]
fn reap_candidates_take_terminal_runs_past_the_floor() {
    let now = Utc::now();
    let runs = vec![reap_test_run(
        RunStatus::Succeeded {
            reason: SuccessReason::Completed,
        },
        72,
        Some(true),
    )];

    let candidates = fork_run_retention::reap_candidates(&runs, now, 48, false);

    assert_eq!(
        candidates.len(),
        1,
        "succeeded 72h ago with a kept sandbox reaps"
    );
}

#[test]
fn reap_candidates_exclude_human_needed_and_park_at_any_age() {
    let now = Utc::now();
    let hitl = reap_test_run(RunStatus::HumanNeeded, 200, Some(true));
    let park = reap_test_run(
        RunStatus::Blocked {
            blocked_reason: BlockedReason::QuotaRateLimit,
        },
        200,
        Some(true),
    );
    let runs = vec![hitl, park];

    let candidates = fork_run_retention::reap_candidates(&runs, now, 48, false);

    assert!(
        candidates.is_empty(),
        "HITL and resumable park are protected at any age"
    );
}

#[test]
fn reap_candidates_skip_young_terminal_without_brake_but_take_them_with_it() {
    let now = Utc::now();
    let runs = vec![reap_test_run(
        RunStatus::Failed {
            reason: FailureReason::WorkflowError,
        },
        5,
        Some(true),
    )];

    assert!(
        fork_run_retention::reap_candidates(&runs, now, 48, false).is_empty(),
        "5h terminal stays under a 48h floor without the brake"
    );
    assert_eq!(
        fork_run_retention::reap_candidates(&runs, now, 48, true).len(),
        1,
        "the brake widens the age window"
    );
}

#[test]
fn reap_candidates_brake_never_widens_protections() {
    let now = Utc::now();
    let runs = vec![reap_test_run(RunStatus::HumanNeeded, 3, Some(true))];

    assert!(
        fork_run_retention::reap_candidates(&runs, now, 48, true).is_empty(),
        "HumanNeeded stays protected under the brake"
    );
}

#[test]
fn reap_candidates_skip_already_removed_or_absent_sandboxes() {
    let now = Utc::now();
    let removed = reap_test_run(
        RunStatus::Succeeded {
            reason: SuccessReason::Completed,
        },
        72,
        Some(false),
    );
    let mut absent = reap_test_run(
        RunStatus::Succeeded {
            reason: SuccessReason::Completed,
        },
        72,
        Some(true),
    );
    absent.sandbox = None;
    let runs = vec![removed, absent];

    assert!(
        fork_run_retention::reap_candidates(&runs, now, 48, false).is_empty(),
        "no point reaping an already-removed or absent sandbox"
    );
}

#[test]
fn reap_candidates_order_oldest_terminal_first() {
    let now = Utc::now();
    let young = reap_test_run(
        RunStatus::Succeeded {
            reason: SuccessReason::Completed,
        },
        50,
        Some(true),
    );
    let old = reap_test_run(RunStatus::Dead, 300, Some(true));
    let young_id = young.id;
    let old_id = old.id;
    let runs = vec![young, old];

    let candidates = fork_run_retention::reap_candidates(&runs, now, 48, false);

    assert_eq!(candidates.len(), 2);
    assert_eq!(
        candidates[0], old_id,
        "oldest terminal transition first (young.id {young_id} vs old.id {old_id})"
    );
}

#[test]
fn reap_candidates_floor_zero_disables_the_arm() {
    let now = Utc::now();
    let runs = vec![reap_test_run(
        RunStatus::Succeeded {
            reason: SuccessReason::Completed,
        },
        500,
        Some(true),
    )];

    assert!(
        fork_run_retention::reap_candidates(&runs, now, 0, true).is_empty(),
        "floor 0 disables the reap arm including the brake"
    );
}

#[test]
fn percent_used_rounds_down_and_saturates() {
    use fork_run_retention::percent_used;

    assert_eq!(percent_used(0, 0), None, "zero total has no percent");
    assert_eq!(percent_used(100, 0), Some(100), "full disk is 100");
    assert_eq!(percent_used(100, 100), Some(0), "empty disk is 0");
    assert_eq!(percent_used(1000, 250), Some(75), "75% exact");
    assert_eq!(
        percent_used(1000, 333),
        Some(66),
        "66.7% rounds down — the brake threshold must not fire early"
    );
    assert_eq!(
        percent_used(100, 150),
        Some(0),
        "available above total (reserved blocks) saturates to 0% used"
    );
}
