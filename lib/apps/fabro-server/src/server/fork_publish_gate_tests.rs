//! Fork-only pin: automatic pull requests exist for success-terminated
//! runs only (user decision A, 2026-10-11; revisor harvest finding 3 of
//! run 01M4KMWYRQDJ54ZSS08NJNH74N): a quota-park SoftStop can rewrite a
//! run to FAILED while its conclusion stays a boundary success, and PR
//! #391 opened auto-merge-enabled 6s after exactly that failed exit —
//! for work that never reached tester/reviewer. The pushed run branch
//! stays the salvage instrument; the manual endpoint's force override
//! is the deliberate salvage-PR path.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use fabro_store::platform_records::{PlatformRecordKind, RunLifecycleKind};
use fabro_types::{FailureReason, RunId, RunStatus, SuccessReason, WorkflowPath};
use serde_json::json;
use tower::ServiceExt;

use super::handler::pull_requests::RunPrInputs;
use super::{AppState, run_publish, run_records};

const MINIMAL_DOT: &str = r#"digraph Test {
    graph [goal="Test"]
    start [shape=Mdiamond]
    exit  [shape=Msquare]
    start -> exit
}"#;

/// The run's projection, driven to `terminal` the way a real run's
/// lifecycle records drive it: the fold rejects status jumps, so the
/// walk is Submitted -> Runnable -> Starting -> Running -> terminal.
async fn seed_terminal_projection(
    state: &AppState,
    run_id: RunId,
    terminal: (RunLifecycleKind, RunStatus),
) {
    for (kind, status) in [
        (RunLifecycleKind::Runnable, RunStatus::Runnable),
        (RunLifecycleKind::Starting, RunStatus::Starting),
        (RunLifecycleKind::Running, RunStatus::Running),
        terminal,
    ] {
        run_records::lifecycle(state, run_id, run_records::transition(kind, status))
            .await
            .unwrap();
    }
}

async fn pull_request_requests(state: &AppState, run_id: RunId) -> usize {
    state
        .stores
        .run_summaries
        .platform_records()
        .read_kind(&run_id, PlatformRecordKind::PullRequestRequested)
        .await
        .expect("pull request requests read")
        .len()
}

async fn projection(state: &AppState, run_id: RunId) -> Arc<fabro_store::RunProjection> {
    run_records::projection(state, run_id)
        .await
        .unwrap()
        .expect("the run's projection")
}

/// Create a run with `[run.pull_request] enabled` the way the develop
/// lane's runs are created: a git target on the Docker-class default
/// environment, dry-run so nothing executes, and an explicit PR model so
/// the request does not depend on the test catalog's ready providers.
async fn run_with_pull_requests_enabled(app: &axum::Router, state: &AppState) -> RunId {
    let mut files = std::collections::BTreeMap::from([(
        WorkflowPath::new("workflow.fabro").unwrap(),
        MINIMAL_DOT.to_string(),
    )]);
    files.insert(
        WorkflowPath::new("workflow.toml").unwrap(),
        "_version = 1\n[run.pull_request]\nenabled = true\nmodel = \"acme-large\"\n".to_string(),
    );
    let version = fabro_types::WorkflowVersion::new(
        WorkflowPath::new("workflow.fabro").unwrap(),
        files,
        std::collections::BTreeMap::new(),
    )
    .unwrap();
    let version = fabro_workflow_version::ValidatedWorkflowVersion::new(version).unwrap();
    let workflow_version_id =
        fabro_workflow_version::WorkflowVersionStore::new(state.store_ref().blobs())
            .put(&version)
            .await
            .unwrap();

    let request = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/runs")
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "workflow_version_id": workflow_version_id,
                "target": {
                    "kind": "git",
                    "repo": "fabro-sh/fabro",
                    "branch": "main"
                },
                "args": { "dry_run": true }
            }))
            .unwrap(),
        ))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let body = fabro_test::expect_axum_json(response, StatusCode::CREATED, file!()).await;
    body["id"]
        .as_str()
        .expect("the created run carries an id")
        .parse::<RunId>()
        .unwrap()
}

/// The PR-#391 class at the automatic path: a run that ended FAILED (a
/// quota-park soft stop) never requests a pull request.
#[tokio::test]
async fn a_failed_run_never_requests_an_automatic_pull_request() {
    let state = crate::test_support::test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = run_with_pull_requests_enabled(&app, &state).await;

    seed_terminal_projection(
        &state,
        run_id,
        (RunLifecycleKind::Failed, RunStatus::Failed {
            reason: FailureReason::SoftStop,
        }),
    )
    .await;
    assert!(matches!(
        projection(&state, run_id).await.status,
        RunStatus::Failed { .. }
    ));

    run_publish::request_pull_request_if_enabled(&state, &run_id).await;
    assert_eq!(
        pull_request_requests(&state, run_id).await,
        0,
        "a failed run must not request a pull request"
    );
}

/// The control: a success-terminated run still requests its pull request
/// — a boundary success is a success.
#[tokio::test]
async fn a_succeeded_run_still_requests_its_pull_request() {
    let state = crate::test_support::test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = run_with_pull_requests_enabled(&app, &state).await;

    seed_terminal_projection(
        &state,
        run_id,
        (RunLifecycleKind::Succeeded, RunStatus::Succeeded {
            reason: SuccessReason::Boundary,
        }),
    )
    .await;
    assert!(matches!(
        projection(&state, run_id).await.status,
        RunStatus::Succeeded { .. }
    ));

    run_publish::request_pull_request_if_enabled(&state, &run_id).await;
    assert_eq!(
        pull_request_requests(&state, run_id).await,
        1,
        "a success-terminated run still requests its pull request"
    );
}

/// The supervisor / manual seam: extraction refuses a failed run with the
/// typed code, and `force` is the deliberate salvage-PR override.
#[tokio::test]
async fn extraction_refuses_a_failed_run_and_force_overrides() {
    let state = crate::test_support::test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = run_with_pull_requests_enabled(&app, &state).await;

    seed_terminal_projection(
        &state,
        run_id,
        (RunLifecycleKind::Failed, RunStatus::Failed {
            reason: FailureReason::SoftStop,
        }),
    )
    .await;
    let failed = projection(&state, run_id).await;
    match RunPrInputs::extract(&failed, false) {
        Ok(_) => panic!("a failed run must be refused"),
        Err(err) => assert_eq!(err.code(), Some("run_not_successful")),
    }

    // force passes the status gate (the run then fails the metadata
    // checks the fixture cannot satisfy — proving the gate let it pass).
    match RunPrInputs::extract(&failed, true) {
        Ok(_) => {}
        Err(err) => assert_ne!(
            err.code(),
            Some("run_not_successful"),
            "force must pass the status gate"
        ),
    }
}
