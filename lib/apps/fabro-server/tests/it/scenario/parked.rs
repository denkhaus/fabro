//! Needs-human park visibility through the runs list door (work#1508).
//!
//! A needs-human park is a stall BY DESIGN (ADR-0045: the operator must
//! disposition it) — an invisible park stalls forever. The fork.43 round
//! observed a parked run invisible in the production UI while
//! `fabro ps -a` showed it plainly. These pins hold the exposure: a
//! parked run appears through the board's default listing and through
//! the board's Needs human column query.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use fabro_server::test_support::build_test_router;
use fabro_types::{RunId, RunProjection, RunSpec, RunStatus, RunStatusKind};
use tower::ServiceExt;

use crate::helpers::{api, response_json};

fn parked_app() -> (axum::Router, Arc<fabro_store::Database>) {
    let (store, artifact_store) = fabro_server::test_support::test_store_bundle();
    let state = fabro_server::test_support::test_app_state_with_store(
        crate::helpers::test_settings().server_settings,
        crate::helpers::test_settings().manifest_run_defaults,
        5,
        Arc::clone(&store),
        artifact_store,
    );
    let app = build_test_router(state);
    (app, store)
}

/// Seed a run the projector's way: a projection carrying `status`.
async fn seed_run(store: &Arc<fabro_store::Database>, run_id: RunId, status: RunStatus) {
    let mut projection = RunProjection::new(
        "already landed park (needs a human)".to_string(),
        RunSpec {
            run_id,
            settings: fabro_types::WorkflowSettings::default(),
            graph: fabro_types::RunGraph::new("test"),
            graph_source: None,
            workflow_slug: Some("develop".to_string()),
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

/// The board's exact request shape: the default listing with no status
/// filter. A parked run must appear in it — the columns view buckets
/// whatever this returns.
#[tokio::test]
async fn a_needs_human_park_appears_in_the_default_runs_listing() {
    let (app, store) = parked_app();
    let parked_run = RunId::new();
    seed_run(&store, parked_run, RunStatus::HumanNeeded).await;

    let req = Request::builder()
        .method("GET")
        .uri(api("/runs"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = response_json(resp, StatusCode::OK, "GET /api/v1/runs (default)").await;
    let ids = listed_ids(&body);
    assert!(
        ids.contains(&parked_run.to_string()),
        "the parked run must appear in the default listing, got {ids:?}"
    );
}

/// The Needs human column's door: `status=human_needed`. The board's
/// column buckets what this returns.
#[tokio::test]
async fn a_needs_human_park_appears_through_the_needs_human_column_query() {
    let (app, store) = parked_app();
    let parked_run = RunId::new();
    seed_run(&store, parked_run, RunStatus::HumanNeeded).await;

    let req = Request::builder()
        .method("GET")
        .uri(api("/runs?status=human_needed"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = response_json(resp, StatusCode::OK, "GET /api/v1/runs?status=human_needed").await;
    let ids = listed_ids(&body);
    assert!(
        ids.contains(&parked_run.to_string()),
        "the parked run must appear through status=human_needed, got {ids:?}"
    );
    assert!(
        ids.iter().all(|id| id == &parked_run.to_string()),
        "status=human_needed answers only the parked run, got {ids:?}"
    );
}

/// The response rows carry the parked kind so the board can bucket the
/// run into its Needs human column.
#[tokio::test]
async fn the_parked_run_row_carries_the_human_needed_kind() {
    let (app, store) = parked_app();
    let parked_run = RunId::new();
    seed_run(&store, parked_run, RunStatus::HumanNeeded).await;

    let req = Request::builder()
        .method("GET")
        .uri(api("/runs"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = response_json(resp, StatusCode::OK, "GET /api/v1/runs (kind pin)").await;
    let row = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"].as_str() == Some(parked_run.to_string().as_str()))
        .expect("the parked run row")
        .clone();
    assert_eq!(
        row["lifecycle"]["status"]["kind"].as_str(),
        Some(RunStatusKind::HumanNeeded.to_string().as_str()),
        "the parked row carries the human_needed kind: {row}"
    );
}

/// Archive the seeded park through the archive door: the disposition's
/// first half (the fork.43 incident archived the park before the
/// operator looked again).
async fn archive_park(app: axum::Router, run_id: &RunId) {
    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/archive")))
        .body(Body::empty())
        .expect("archive request builds");
    let resp = app.oneshot(req).await.expect("archive call");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "POST /api/v1/runs/{run_id}/archive"
    );
}

/// The archived park stays a board citizen (work#1508): archiving a
/// needs-human park is bookkeeping — the disposition (the item close,
/// the verdict release) is still the operator's, so the park remains
/// exposed through the default listing under the archival overlay.
/// The fork.43 incident: an archived park invisible in the UI while
/// `fabro ps -a` showed it plainly.
#[tokio::test]
async fn an_archived_needs_human_park_still_appears_in_the_default_runs_listing() {
    let (app, store) = parked_app();
    let parked_run = RunId::new();
    seed_run(&store, parked_run, RunStatus::HumanNeeded).await;
    archive_park(app.clone(), &parked_run).await;

    let req = Request::builder()
        .method("GET")
        .uri(api("/runs"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = response_json(resp, StatusCode::OK, "GET /api/v1/runs (archived park)").await;
    let ids = listed_ids(&body);
    assert!(
        ids.contains(&parked_run.to_string()),
        "the archived park must stay exposed in the default listing, got {ids:?}"
    );
    // And it is honestly marked archived.
    let row = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"].as_str() == Some(parked_run.to_string().as_str()))
        .expect("the archived park row");
    assert_eq!(
        row["lifecycle"]["archived"].as_bool(),
        Some(true),
        "the archived park keeps its overlay flag: {row}"
    );
}

/// The Needs human column answers parks of ANY archive state — the
/// waiting state is the column's subject (ADR-0045), not the overlay.
#[tokio::test]
async fn an_archived_needs_human_park_still_appears_through_the_needs_human_column_query() {
    let (app, store) = parked_app();
    let parked_run = RunId::new();
    seed_run(&store, parked_run, RunStatus::HumanNeeded).await;
    archive_park(app.clone(), &parked_run).await;

    let req = Request::builder()
        .method("GET")
        .uri(api("/runs?status=human_needed"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = response_json(
        resp,
        StatusCode::OK,
        "GET /api/v1/runs?status=human_needed (archived park)",
    )
    .await;
    let ids = listed_ids(&body);
    assert!(
        ids.contains(&parked_run.to_string()),
        "the archived park must answer the Needs human column query, got {ids:?}"
    );
}

/// The archive overlay keeps hiding everything that is NOT a park: a
/// plain archived run stays out of the default listing (the existing
/// contract, pinned beside the park exposure so the exception cannot
/// grow silently).
#[tokio::test]
async fn an_archived_non_park_run_stays_hidden_in_the_default_runs_listing() {
    let (app, store) = parked_app();
    let finished_run = RunId::new();
    seed_run(&store, finished_run, RunStatus::Succeeded {
        reason: fabro_types::SuccessReason::Completed,
    })
    .await;
    archive_park(app.clone(), &finished_run).await;

    let req = Request::builder()
        .method("GET")
        .uri(api("/runs"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = response_json(resp, StatusCode::OK, "GET /api/v1/runs (non-park)").await;
    let ids = listed_ids(&body);
    assert!(
        !ids.contains(&finished_run.to_string()),
        "a plain archived run stays hidden, got {ids:?}"
    );
}

fn listed_ids(body: &serde_json::Value) -> Vec<String> {
    let rows = body["data"]
        .as_array()
        .expect("the paginated envelope carries data");
    rows.iter()
        .map(|row| {
            row["id"]
                .as_str()
                .expect("every run row carries its id")
                .to_string()
        })
        .collect()
}
