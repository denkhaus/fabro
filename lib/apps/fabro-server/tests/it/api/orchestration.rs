//! HTTP-level integration tests for the orchestration doors (ADR-0039
//! stage D, denkhaus/work#1107): the verdict write door's authorization
//! matrix, the adjudication queue's anti-join, and the run-observations
//! read.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use fabro_server::server::{RouterOptions, build_router_with_options};
use fabro_server::test_support::{build_test_router, test_auth_mode};
use fabro_types::{RunId, RunStatus, SuccessReason};
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::helpers::{api, response_json};

fn orchestration_app() -> (
    axum::Router,
    Arc<fabro_server::server::AppState>,
    Arc<fabro_store::Database>,
) {
    let settings = crate::helpers::test_settings();
    let (store, artifact_store) = fabro_server::test_support::test_store_bundle();
    let state = fabro_server::test_support::test_app_state_with_store(
        settings.server_settings,
        settings.manifest_run_defaults,
        5,
        Arc::clone(&store),
        artifact_store,
    );
    let app = build_router_with_options(state.clone(), &test_auth_mode(), RouterOptions::default());
    (app, state, store)
}

/// Seed a run row the projector's way, with a workflow slug and a
/// terminal status.
async fn seed_run(
    store: &Arc<fabro_store::Database>,
    run_id: RunId,
    workflow_slug: &str,
    terminal: bool,
) {
    let status = if terminal {
        RunStatus::Succeeded {
            reason: SuccessReason::Completed,
        }
    } else {
        RunStatus::Running
    };
    let mut projection = fabro_types::RunProjection::new(
        format!("{workflow_slug} run"),
        fabro_types::RunSpec {
            run_id,
            settings: fabro_types::WorkflowSettings::default(),
            graph: fabro_types::RunGraph::new("test"),
            graph_source: None,
            workflow_slug: Some(workflow_slug.to_string()),
            workflow_version_id: None,
            target: None,
            automation: None,
            source_directory: None,
            labels: HashMap::new(),
            provenance: fabro_types::test_support::test_run_provenance(),
            definition_blob: None,
            spec_blob: None,
            git: None,
            fork_source_ref: None,
            admission: fabro_types::PetriAdmission::default(),
        },
        chrono::Utc::now(),
    );
    projection.status = status;
    let pool = store.run_summary_store().pool();
    let mut tx = pool.begin().await.expect("seed tx");
    fabro_store::RunSummaryStore::write_petri_run_row_on_connection(&mut tx, &run_id, &projection)
        .await
        .expect("seed run row");
    let projection_json = serde_json::to_string(&projection).expect("projection json");
    sqlx::query(
        "INSERT INTO petri_projection (run_id, projection_json, fold_json, positions_json, \
         stream_seq, updated_at_ms) VALUES (?, ?, '{}', '{}', 0, 0) \
         ON CONFLICT(run_id) DO UPDATE SET projection_json = excluded.projection_json",
    )
    .bind(run_id.to_string())
    .bind(projection_json)
    .execute(&mut *tx)
    .await
    .expect("seed projection row");
    tx.commit().await.expect("seed commit");
}

/// One mark call as a workflow worker of `writer`.
async fn mark_call(
    app: &axum::Router,
    token: &str,
    target: &RunId,
    body: Value,
) -> axum::http::Response<Body> {
    let request = Request::builder()
        .method(Method::POST)
        .uri(api(&format!("/runs/{target}/orchestration")))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&body).expect("mark body serializes"),
        ))
        .expect("mark request builds");
    app.clone()
        .oneshot(request)
        .await
        .expect("mark request runs")
}

/// The mark door's authorization matrix (the stage-D grill): only the
/// field's own workflow writes its verdict — the adjudicator owns
/// `merge`, the merger owns `closable` — a foreign workflow is refused,
/// a run cannot mark itself, and a user principal has no write at all.
#[tokio::test]
async fn the_mark_door_enforces_one_writer_per_field() {
    let (app, state, store) = orchestration_app();
    let adjudicator = RunId::new();
    let merger = RunId::new();
    let developer = RunId::new();
    let target = RunId::new();
    seed_run(&store, adjudicator, "adjudicator", true).await;
    seed_run(&store, merger, "merger", true).await;
    seed_run(&store, developer, "develop", true).await;
    seed_run(&store, target, "develop", true).await;
    let adjudicator_token = state.test_issue_worker_token_with_run_tools(&adjudicator);
    let merger_token = state.test_issue_worker_token_with_run_tools(&merger);
    let developer_token = state.test_issue_worker_token_with_run_tools(&developer);

    // The adjudicator writes merge — the happy path.
    let response = mark_call(
        &app,
        &adjudicator_token,
        &target,
        json!({"field": "merge", "verdict": "enabled"}),
    )
    .await;
    let body = response_json(response, StatusCode::OK, "adjudicator marks merge").await;
    assert_eq!(body["run_id"].as_str(), Some(target.to_string().as_str()));
    assert_eq!(body["field"].as_str(), Some("merge"));
    assert_eq!(body["verdict"].as_str(), Some("enabled"));
    assert_eq!(
        body["by_run"].as_str(),
        Some(adjudicator.to_string().as_str()),
        "the response names the WRITER run"
    );

    // The adjudicator cannot write the merger's field.
    let response = mark_call(
        &app,
        &adjudicator_token,
        &target,
        json!({"field": "closable", "verdict": "enabled"}),
    )
    .await;
    let body = response_json(
        response,
        StatusCode::FORBIDDEN,
        "adjudicator marks closable",
    )
    .await;
    assert!(
        body["errors"][0]["detail"]
            .as_str()
            .expect("refusal detail")
            .contains("writer per field"),
        "the field-ownership refusal teaches the rule: {body}"
    );

    // The merger cannot write the adjudicator's field.
    let response = mark_call(
        &app,
        &merger_token,
        &target,
        json!({"field": "merge", "verdict": "refused"}),
    )
    .await;
    response_json(response, StatusCode::FORBIDDEN, "merger marks merge").await;

    // A workflow off the allowlist has no door at all.
    let response = mark_call(
        &app,
        &developer_token,
        &target,
        json!({"field": "merge", "verdict": "enabled"}),
    )
    .await;
    let body = response_json(response, StatusCode::FORBIDDEN, "developer marks merge").await;
    assert!(
        body["errors"][0]["detail"]
            .as_str()
            .expect("refusal detail")
            .contains("allowlist"),
        "the allowlist refusal names the boundary: {body}"
    );

    // A run cannot mark itself.
    let response = mark_call(
        &app,
        &adjudicator_token,
        &adjudicator,
        json!({"field": "merge", "verdict": "enabled"}),
    )
    .await;
    response_json(response, StatusCode::FORBIDDEN, "adjudicator marks itself").await;
}

/// A user principal has no verdict write: the door is for workflow runs
/// (the gears), and the run view serves users their reads.
#[tokio::test]
async fn the_mark_door_refuses_user_principals() {
    let (store, artifact_store) = fabro_server::test_support::test_store_bundle();
    let settings = crate::helpers::test_settings();
    let state = fabro_server::test_support::test_app_state_with_store(
        settings.server_settings,
        settings.manifest_run_defaults,
        5,
        Arc::clone(&store),
        artifact_store,
    );
    let app = build_test_router(state);
    let target = RunId::new();
    seed_run(&store, target, "develop", true).await;
    let request = Request::builder()
        .method(Method::POST)
        .uri(api(&format!("/runs/{target}/orchestration")))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({"field": "merge", "verdict": "enabled"}))
                .expect("mark body serializes"),
        ))
        .expect("mark request builds");
    let response = app.oneshot(request).await.expect("request runs");
    let body = response_json(response, StatusCode::FORBIDDEN, "user marks a verdict").await;
    assert!(
        body["errors"][0]["detail"]
            .as_str()
            .expect("refusal detail")
            .contains("Only workflow runs"),
        "the user refusal names who may write: {body}"
    );
}

/// The queue's anti-join over HTTP: terminal runs without a verdict on
/// the field, minus the verdicted, minus the non-terminal — and the
/// marked run leaves the merge queue while the closable field stays
/// pending.
#[tokio::test]
async fn the_queue_lists_terminal_runs_minus_the_field_verdicts() {
    let (app, state, store) = orchestration_app();
    let adjudicator = RunId::new();
    let unadjudicated = RunId::new();
    let verdicted = RunId::new();
    let live = RunId::new();
    seed_run(&store, adjudicator, "adjudicator", true).await;
    seed_run(&store, unadjudicated, "develop", true).await;
    seed_run(&store, verdicted, "develop", true).await;
    seed_run(&store, live, "develop", false).await;
    let token = state.test_issue_worker_token_with_run_tools(&adjudicator);

    // verdicted carries merge = refused already.
    let response = mark_call(
        &app,
        &token,
        &verdicted,
        json!({"field": "merge", "verdict": "refused"}),
    )
    .await;
    response_json(response, StatusCode::OK, "seed verdict").await;

    let queue_ids = |body: &Value| {
        body["data"]
            .as_array()
            .expect("queue entries")
            .iter()
            .map(|entry| {
                entry["run"]["id"]
                    .as_str()
                    .expect("run id inside the entry")
                    .to_string()
            })
            .collect::<Vec<_>>()
    };
    let read_queue = |field: &str| {
        let request = Request::builder()
            .method(Method::GET)
            .uri(api(&format!("/runs/queue?field={field}")))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .expect("queue request builds");
        let app = app.clone();
        async move {
            let response = app.oneshot(request).await.expect("queue request runs");
            response_json(response, StatusCode::OK, "queue read").await
        }
    };

    let merge_queue = read_queue("merge").await;
    let ids = queue_ids(&merge_queue);
    assert!(
        ids.contains(&unadjudicated.to_string()),
        "the unadjudicated terminal run queues: {ids:?}"
    );
    assert!(
        !ids.contains(&verdicted.to_string()),
        "the verdicted run left the queue: {ids:?}"
    );
    assert!(
        !ids.contains(&live.to_string()),
        "a non-terminal run never queues: {ids:?}"
    );

    // The closable field is still pending for the verdicted run: the
    // merge verdict does not free the merger's queue.
    let closable_queue = read_queue("closable").await;
    let closable_ids = queue_ids(&closable_queue);
    assert!(
        closable_ids.contains(&verdicted.to_string()),
        "the merge verdict does not free the closable queue: {closable_ids:?}"
    );

    // Marking the unadjudicated run removes it from the merge queue.
    let response = mark_call(
        &app,
        &token,
        &unadjudicated,
        json!({"field": "merge", "verdict": "enabled"}),
    )
    .await;
    response_json(response, StatusCode::OK, "mark the queued run").await;
    let after = read_queue("merge").await;
    let after_ids = queue_ids(&after);
    assert!(
        !after_ids.contains(&unadjudicated.to_string()),
        "the marked run left the merge queue: {after_ids:?}"
    );
}

/// The verdict-filtered queue (stage E): `verdict=enabled` flips the
/// join — exactly the runs settled on that verdict list, the closing
/// pass's worklist. Parked and never-verdicted runs stay out.
#[tokio::test]
async fn the_queue_with_a_verdict_filter_lists_exactly_the_settled_runs() {
    let (app, state, store) = orchestration_app();
    let merger = RunId::new();
    let merged = RunId::new();
    let parked = RunId::new();
    let fresh = RunId::new();
    seed_run(&store, merger, "merger", true).await;
    seed_run(&store, merged, "develop", true).await;
    seed_run(&store, parked, "develop", true).await;
    seed_run(&store, fresh, "develop", true).await;
    let token = state.test_issue_worker_token_with_run_tools(&merger);

    // The merger merged `merged` (closable = enabled); a conflict parked
    // `parked` (closable = parked); `fresh` carries no verdict at all.
    for (target, verdict) in [(merged, "enabled"), (parked, "parked")] {
        let response = mark_call(
            &app,
            &token,
            &target,
            json!({"field": "closable", "verdict": verdict}),
        )
        .await;
        response_json(response, StatusCode::OK, "seed closable verdict").await;
    }

    let request = Request::builder()
        .method(Method::GET)
        .uri(api(
            "/runs/queue?field=closable&verdict=enabled&workflow=develop",
        ))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .expect("closing-pass queue request builds");
    let response = app.oneshot(request).await.expect("queue request runs");
    let body = response_json(response, StatusCode::OK, "closing-pass queue read").await;
    let ids = body["data"]
        .as_array()
        .expect("queue entries")
        .iter()
        .map(|entry| {
            entry["run"]["id"]
                .as_str()
                .expect("run id inside the entry")
                .to_string()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![merged.to_string()],
        "the closing-pass worklist holds exactly the closable-enabled run — parked and \
         never-verdicted runs stay out: {body}"
    );
}

/// The observations read door: a run's observation records in seq order
/// with the category that routes adjudication.
#[tokio::test]
async fn the_observations_read_lists_a_runs_records() {
    let (app, state, store) = orchestration_app();
    let run = RunId::new();
    seed_run(&store, run, "develop", true).await;
    let records = store.run_summary_store().platform_records();
    for (category, text) in [
        (
            fabro_types::ObservationCategory::Painpoint,
            "fact=patch-visibility verdict=not-visible base=abc1234 item=denkhaus/work#1",
        ),
        (
            fabro_types::ObservationCategory::Note,
            "fact=patch-visibility verdict=visible base=abc1234 item=denkhaus/work#1",
        ),
    ] {
        records
            .append(
                &run,
                &fabro_store::platform_records::PlatformRecord::ObservationRecorded(
                    fabro_store::platform_records::ObservationRecordedRecord {
                        category,
                        text: text.to_string(),
                        execution: 3,
                        firing: 1,
                    },
                ),
                None,
            )
            .await
            .expect("observation seed");
    }

    let token = state.test_issue_worker_token_with_run_tools(&run);
    let request = Request::builder()
        .method(Method::GET)
        .uri(api(&format!("/runs/{run}/observations")))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .expect("observations request builds");
    let response = app.oneshot(request).await.expect("request runs");
    let body = response_json(response, StatusCode::OK, "observations read").await;
    let observations = body["observations"].as_array().expect("observations");
    assert_eq!(observations.len(), 2, "both records list: {body}");
    assert_eq!(observations[0]["category"].as_str(), Some("painpoint"));
    assert_eq!(observations[0]["execution"].as_i64(), Some(3));
    assert_eq!(observations[0]["firing"].as_i64(), Some(1));
    assert!(
        observations[0]["text"]
            .as_str()
            .expect("text")
            .starts_with("fact=patch-visibility"),
        "the fact head rides the text: {body}"
    );
    assert_eq!(observations[1]["category"].as_str(), Some("note"));
}

/// Seed the work project through the user surface (the run_channel
/// precedent): the claims the decoration test grants need it.
async fn seed_work_project(state: &Arc<fabro_server::server::AppState>) {
    let app = fabro_server::test_support::build_test_router(Arc::clone(state));
    let request = Request::builder()
        .method(Method::PUT)
        .uri(api("/projects/fabro"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "displayName": "Fabro",
                "trackerProvider": "github",
                "trackerRepo": "denkhaus/work",
                "targetRepo": "denkhaus/fabro",
                "projectLabel": "project:fabro",
                "laneLabels": {"loop": "lane:loop"},
                "claimLabelPrefix": "fabro:run:",
                "claimCommentEnabled": false
            }))
            .expect("project fixture serializes"),
        ))
        .expect("project request builds");
    let response = app.oneshot(request).await.expect("project upsert runs");
    response_json(response, StatusCode::OK, "PUT /projects/fabro (user seed)").await;
}

/// Grant one claim for `run` on `item` through the user surface and
/// return the claim id (the run_channel precedent).
async fn seed_claim(
    state: &Arc<fabro_server::server::AppState>,
    run: &RunId,
    item: &str,
) -> String {
    let app = fabro_server::test_support::build_test_router(Arc::clone(state));
    let payload = json!({"itemKey": item, "runId": run.to_string(), "lane": "loop"});
    let request = Request::builder()
        .method(Method::POST)
        .uri(api("/projects/fabro/claims"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&payload).expect("claim fixture serializes"),
        ))
        .expect("claim request builds");
    let response = app.oneshot(request).await.expect("claim seed runs");
    let body = response_json(response, StatusCode::OK, "POST /projects/fabro/claims").await;
    body["claim"]["claimId"]
        .as_str()
        .expect("the seeded claim id")
        .to_string()
}

/// The part-2 queue semantics over HTTP (ADR-0039 stage D part 2):
/// entries carry the work item of the run's latest claim, a PARKED
/// latest verdict keeps the run queued with the escalation visible in
/// the entry's verdict, and the workflow filter narrows the worklist to
/// the named workflow's runs.
#[tokio::test]
async fn the_queue_decorates_work_items_keeps_parked_and_filters_by_workflow() {
    let (app, state, store) = orchestration_app();
    let adjudicator = RunId::new();
    let claimed = RunId::new();
    let parked = RunId::new();
    let smoke = RunId::new();
    seed_run(&store, adjudicator, "adjudicator", true).await;
    seed_run(&store, claimed, "develop", true).await;
    seed_run(&store, parked, "develop", true).await;
    seed_run(&store, smoke, "smoke", true).await;
    seed_work_project(&state).await;
    seed_claim(&state, &claimed, "denkhaus/work#1").await;
    seed_claim(&state, &parked, "denkhaus/work#2").await;
    let token = state.test_issue_worker_token_with_run_tools(&adjudicator);

    // The adjudicator parks one run: an open operator escalation.
    let response = mark_call(
        &app,
        &token,
        &parked,
        json!({"field": "merge", "verdict": "parked"}),
    )
    .await;
    response_json(response, StatusCode::OK, "seed parked verdict").await;

    let read_queue = |query: &str| {
        let request = Request::builder()
            .method(Method::GET)
            .uri(api(&format!("/runs/queue?{query}")))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .expect("queue request builds");
        let app = app.clone();
        async move {
            let response = app.oneshot(request).await.expect("queue request runs");
            response_json(response, StatusCode::OK, "queue read").await
        }
    };
    let entry_of = |body: &Value, run: &RunId| {
        body["data"]
            .as_array()
            .expect("queue entries")
            .iter()
            .find(|entry| entry["run"]["id"].as_str() == Some(run.to_string().as_str()))
            .cloned()
            .unwrap_or_else(|| panic!("run {run} is not queued: {body}"))
    };

    let queue = read_queue("field=merge").await;
    let claimed_entry = entry_of(&queue, &claimed);
    assert_eq!(
        claimed_entry["work_item"].as_str(),
        Some("denkhaus/work#1"),
        "the entry carries the run's claimed work item: {claimed_entry}"
    );
    assert_eq!(
        claimed_entry["project"].as_str(),
        Some("fabro"),
        "the entry carries the claim's project — queue consumers must not hardcode one \
         (work#1113): {claimed_entry}"
    );
    assert_eq!(
        claimed_entry["verdict"].as_str(),
        Some("pending"),
        "a never-adjudicated run reads pending: {claimed_entry}"
    );
    let parked_entry = entry_of(&queue, &parked);
    assert_eq!(
        parked_entry["verdict"].as_str(),
        Some("parked"),
        "the parked run stays queued with its escalation visible: {parked_entry}"
    );
    let smoke_entry = entry_of(&queue, &smoke);
    assert_eq!(
        smoke_entry["work_item"].as_str(),
        None,
        "an unclaimed run decorates with no work item: {smoke_entry}"
    );
    assert_eq!(
        smoke_entry["project"].as_str(),
        None,
        "an unclaimed run decorates with no project: {smoke_entry}"
    );

    // The workflow filter narrows the worklist: smoke stays out.
    let develop_queue = read_queue("field=merge&workflow=develop").await;
    let develop_ids = develop_queue["data"]
        .as_array()
        .expect("queue entries")
        .iter()
        .map(|entry| entry["run"]["id"].as_str().expect("run id").to_string())
        .collect::<Vec<_>>();
    assert!(
        !develop_ids.contains(&smoke.to_string()),
        "the workflow filter keeps other workflows out: {develop_ids:?}"
    );
    assert!(
        develop_ids.contains(&parked.to_string()) && develop_ids.contains(&claimed.to_string()),
        "the develop worklist holds both develop runs: {develop_ids:?}"
    );
}
