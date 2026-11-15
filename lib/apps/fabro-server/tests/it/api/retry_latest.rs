//! Retry-on-latest (work#1320): the door's guards and the Part B
//! resolution pins — the successor creation MATERIALIZES the workflow
//! from the target branch's tip (the automation fire-time path),
//! never resolves a version by scanning runs. The full green path
//! (faulty version → fix → re-drive → successor green with carried
//! work) stays the bench facet in fabro-test — the item's
//! closes-when proof.

use std::collections::HashMap;
use std::sync::Arc;

use axum::http::{Request, StatusCode};
use fabro_server::test_support::{
    TestAppStateBuilder, TestAutomationRunMaterializer, build_test_router, test_store_bundle,
};
use fabro_static::EnvVars;
use fabro_types::{
    DirtyStatus, FailureReason, GitContext, GitRunTarget, RunGraph, RunId, RunProjection, RunSpec,
    RunStatus, RunTarget, WorkflowSettings,
};
use serde_json::Value;
use tower::ServiceExt;

use crate::helpers::api;

const SOURCE_SHA: &str = "1111111111111111111111111111111111111111";
const TIP_SHA: &str = "2222222222222222222222222222222222222222";
const OLD_VERSION: &str = "a5300000000000000000000000000000000000000000000000000000000000000";
const ORIGIN_URL: &str = "/nonexistent/retry-latest-origin.git";

async fn retry_latest(
    app: &axum::Router,
    run_id: &str,
    body: Option<Value>,
) -> axum::response::Response {
    let request = match body {
        Some(body) => Request::builder()
            .method(axum::http::Method::POST)
            .uri(api(&format!("/runs/{run_id}/retry-latest")))
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(
                serde_json::to_vec(&body).expect("retry-latest body should serialize"),
            )),
        None => Request::builder()
            .method(axum::http::Method::POST)
            .uri(api(&format!("/runs/{run_id}/retry-latest")))
            .body(axum::body::Body::empty()),
    }
    .expect("retry-latest request should build");
    app.clone()
        .oneshot(request)
        .await
        .expect("call should answer")
}

/// A terminal failed source run the projector's way: git-backed, with
/// slug, version and pinned starting commit under the caller's control.
async fn seed_failed_run(
    store: &Arc<fabro_store::Database>,
    run_id: RunId,
    workflow_slug: Option<&str>,
    workflow_version_id: Option<String>,
) {
    let projection = RunProjection::new(
        "seeded failed run".to_string(),
        RunSpec {
            run_id,
            settings: WorkflowSettings::default(),
            graph: RunGraph::new("test"),
            graph_source: None,
            workflow_slug: workflow_slug.map(str::to_string),
            workflow_version_id: workflow_version_id.and_then(|id| id.parse().ok()),
            target: Some(RunTarget::Git(GitRunTarget {
                repo:   "denkhaus/fabro".to_string(),
                branch: "denkhaus".to_string(),
                tag:    None,
                sha:    Some(SOURCE_SHA.to_string()),
            })),
            automation: None,
            source_directory: None,
            labels: HashMap::new(),
            provenance: fabro_types::test_support::test_run_provenance(),
            definition_blob: None,
            spec_blob: None,
            git: Some(GitContext {
                origin_url: ORIGIN_URL.to_string(),
                branch:     "denkhaus".to_string(),
                sha:        Some(SOURCE_SHA.to_string()),
                dirty:      DirtyStatus::Clean,
            }),
            fork_source_ref: None,
            admission: fabro_types::PetriAdmission::default(),
        },
        chrono::Utc::now(),
    );
    let mut projection = projection;
    projection.status = RunStatus::Failed {
        reason: FailureReason::WorkflowError,
    };
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

/// A state + router with the test materializer attached; the store
/// handle is the seeding and projection-read door.
fn retry_latest_app(
    materializer: TestAutomationRunMaterializer,
) -> (
    axum::Router,
    Arc<fabro_store::Database>,
    TestAutomationRunMaterializer,
    tempfile::TempDir,
) {
    let temp_dir = tempfile::tempdir().expect("retry-latest test tempdir");
    let (store, artifact_store) = test_store_bundle();
    let state = TestAppStateBuilder::new()
        .active_config_path(temp_dir.path().join("settings.toml"))
        .vault_path(temp_dir.path().join("secrets.json"))
        .vault_entries([(EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
        .automation_materializer(materializer.clone())
        .store_bundle(Arc::clone(&store), artifact_store)
        .build();
    (build_test_router(state), store, materializer, temp_dir)
}

/// The successor's stored projection: the tip-checkout and version
/// pins read what the create path persisted, not what the response
/// claims.
async fn stored_projection(store: &Arc<fabro_store::Database>, run_id: &str) -> RunProjection {
    let row: (String,) =
        sqlx::query_as("SELECT projection_json FROM petri_projection WHERE run_id = ?")
            .bind(run_id)
            .fetch_one(&store.run_summary_store().pool())
            .await
            .expect("successor projection row");
    serde_json::from_str(&row.0).expect("successor projection parses")
}

fn git_target(projection: &RunProjection) -> GitRunTarget {
    match &projection.spec.target {
        Some(RunTarget::Git(target)) => target.clone(),
        other => panic!("expected a git target, got {other:?}"),
    }
}

#[tokio::test]
async fn materializes_the_workflow_from_the_branch_tip_instead_of_scanning_runs() {
    let materializer = TestAutomationRunMaterializer::succeed(GitRunTarget {
        repo:   "denkhaus/fabro".to_string(),
        branch: "denkhaus".to_string(),
        tag:    None,
        sha:    Some(TIP_SHA.to_string()),
    });
    let (app, store, materializer, _temp) = retry_latest_app(materializer);
    let source = RunId::new();
    seed_failed_run(
        &store,
        source,
        Some("develop"),
        Some(OLD_VERSION.to_string()),
    )
    .await;

    let body = fabro_test::expect_axum_json(
        retry_latest(&app, &source.to_string(), None).await,
        StatusCode::CREATED,
        "retry-latest materialized fire",
    )
    .await;

    // The resolution input: the source's slug is the workflow selector,
    // and the target carries NO pinned sha — materialization reads the
    // branch tip (work#1320 Part B TARGET).
    let selections = materializer.captured_workflow_selections();
    assert_eq!(selections.len(), 1, "materialization fired exactly once");
    assert_eq!(selections[0].0, "develop");
    assert_eq!(
        selections[0].1,
        GitRunTarget {
            repo:   "denkhaus/fabro".to_string(),
            branch: "denkhaus".to_string(),
            tag:    None,
            sha:    None,
        },
        "the tip selector strips the source's pinned sha"
    );

    // The successor re-admits the MATERIALIZED version — not the
    // source's — and checks out the tip the materializer resolved.
    let version = body["workflow_version_id"]
        .as_str()
        .expect("workflow_version_id in response")
        .to_string();
    assert_ne!(version, OLD_VERSION);
    assert_eq!(body["carried_commits"], 0, "{body}");
    let successor = body["new_run_id"].as_str().expect("new_run_id").to_string();
    let projection = stored_projection(&store, &successor).await;
    assert_eq!(
        projection.spec.workflow_version_id.map(|id| id.to_string()),
        Some(version.clone()),
        "the successor admits the materialized version"
    );
    assert_eq!(
        git_target(&projection).sha.as_deref(),
        Some(TIP_SHA),
        "the successor's checkout is the tip, never the source's pinned sha"
    );
}

#[tokio::test]
async fn a_tip_equal_to_the_source_refuses_naming_both_versions() {
    // The materializer's tip IS the source's starting commit: the
    // branch did not move, nothing new exists to re-drive on.
    let materializer = TestAutomationRunMaterializer::succeed(GitRunTarget {
        repo:   "denkhaus/fabro".to_string(),
        branch: "denkhaus".to_string(),
        tag:    None,
        sha:    Some(SOURCE_SHA.to_string()),
    });
    let (app, store, materializer, _temp) = retry_latest_app(materializer);
    let source = RunId::new();
    seed_failed_run(
        &store,
        source,
        Some("develop"),
        Some(OLD_VERSION.to_string()),
    )
    .await;

    // Discover the can's version id: the first fire re-drives on it.
    let body = fabro_test::expect_axum_json(
        retry_latest(&app, &source.to_string(), None).await,
        StatusCode::CREATED,
        "retry-latest materialized fire",
    )
    .await;
    let tip_version = body["workflow_version_id"]
        .as_str()
        .expect("workflow_version_id in response")
        .to_string();

    // The source now names the tip's version AND the tip is its own
    // starting commit — the plain-retry refusal holds, naming both.
    let second = RunId::new();
    seed_failed_run(&store, second, Some("develop"), Some(tip_version.clone())).await;
    let detail = fabro_test::expect_axum_text(
        retry_latest(&app, &second.to_string(), None).await,
        StatusCode::CONFLICT,
        "retry-latest equal-tip refusal",
    )
    .await;
    assert!(
        detail.contains(&tip_version),
        "the refusal names the tip's version: {detail}"
    );
    assert!(
        detail.contains(SOURCE_SHA),
        "the refusal names the shared commit: {detail}"
    );
    assert!(
        detail.contains("plain retry"),
        "the refusal points at the plain retry: {detail}"
    );
    assert_eq!(materializer.captured_workflow_selections().len(), 2);
}

#[tokio::test]
async fn an_equal_version_on_a_moved_tip_proceeds_on_the_new_checkout() {
    // The 2026-11-15 lab finding (work#1320): the bundle carries the
    // graph, prompts and environment, never the stage scripts — a
    // script fix moves the TIP, not the version. An equal version on a
    // MOVED tip re-drives: the successor re-admits the same version on
    // the new checkout, where the fixed script lives.
    let materializer = TestAutomationRunMaterializer::succeed(GitRunTarget {
        repo:   "denkhaus/fabro".to_string(),
        branch: "denkhaus".to_string(),
        tag:    None,
        sha:    Some(TIP_SHA.to_string()),
    });
    let (app, store, _materializer, _temp) = retry_latest_app(materializer);
    let source = RunId::new();
    seed_failed_run(
        &store,
        source,
        Some("develop"),
        Some(OLD_VERSION.to_string()),
    )
    .await;

    // Discover the can's version, then seed a source that ALREADY ran
    // it — with the tip moved (TIP_SHA, not the source's SOURCE_SHA).
    let body = fabro_test::expect_axum_json(
        retry_latest(&app, &source.to_string(), None).await,
        StatusCode::CREATED,
        "retry-latest materialized fire",
    )
    .await;
    let tip_version = body["workflow_version_id"]
        .as_str()
        .expect("workflow_version_id in response")
        .to_string();

    let second = RunId::new();
    seed_failed_run(&store, second, Some("develop"), Some(tip_version.clone())).await;
    let body = fabro_test::expect_axum_json(
        retry_latest(&app, &second.to_string(), None).await,
        StatusCode::CREATED,
        "an equal version on a moved tip proceeds",
    )
    .await;
    assert_eq!(
        body["workflow_version_id"].as_str(),
        Some(tip_version.as_str()),
        "the successor re-admits the same version"
    );
    let successor = body["new_run_id"].as_str().expect("new_run_id").to_string();
    let projection = stored_projection(&store, &successor).await;
    assert_eq!(
        git_target(&projection).sha.as_deref(),
        Some(TIP_SHA),
        "the successor's checkout is the moved tip"
    );
}

#[tokio::test]
async fn an_explicit_version_bypasses_materialization_and_keeps_the_source_target() {
    let materializer = TestAutomationRunMaterializer::succeed(GitRunTarget {
        repo:   "denkhaus/fabro".to_string(),
        branch: "denkhaus".to_string(),
        tag:    None,
        sha:    Some(TIP_SHA.to_string()),
    });
    let (app, store, materializer, _temp) = retry_latest_app(materializer);
    let source = RunId::new();
    seed_failed_run(
        &store,
        source,
        Some("develop"),
        Some(OLD_VERSION.to_string()),
    )
    .await;

    // Discover and store the can's version through one materialized
    // fire, then re-drive a second source on it EXPLICITLY.
    let body = fabro_test::expect_axum_json(
        retry_latest(&app, &source.to_string(), None).await,
        StatusCode::CREATED,
        "retry-latest materialized fire",
    )
    .await;
    let stored_version = body["workflow_version_id"]
        .as_str()
        .expect("workflow_version_id in response")
        .to_string();
    let explicit_source = RunId::new();
    seed_failed_run(
        &store,
        explicit_source,
        Some("develop"),
        Some(OLD_VERSION.to_string()),
    )
    .await;

    let body = fabro_test::expect_axum_json(
        retry_latest(
            &app,
            &explicit_source.to_string(),
            Some(serde_json::json!({ "workflow_version_id": stored_version })),
        )
        .await,
        StatusCode::CREATED,
        "retry-latest explicit fire",
    )
    .await;
    assert_eq!(
        materializer.captured_workflow_selections().len(),
        1,
        "the explicit fire materialized nothing"
    );

    // The fast path keeps today's semantics: the source's pinned
    // target, an explicit version, no tip resolution.
    let successor = body["new_run_id"].as_str().expect("new_run_id").to_string();
    let projection = stored_projection(&store, &successor).await;
    assert_eq!(
        git_target(&projection).sha.as_deref(),
        Some(SOURCE_SHA),
        "the explicit fast path re-drives the source's pinned target"
    );
}

#[tokio::test]
async fn a_workflow_missing_at_the_tip_refuses_with_the_chain() {
    let materializer = TestAutomationRunMaterializer::fail_workflow_not_found();
    let (app, store, materializer, _temp) = retry_latest_app(materializer);
    let source = RunId::new();
    seed_failed_run(
        &store,
        source,
        Some("gone-from-tip"),
        Some(OLD_VERSION.to_string()),
    )
    .await;

    let detail = fabro_test::expect_axum_text(
        retry_latest(&app, &source.to_string(), None).await,
        StatusCode::CONFLICT,
        "retry-latest refusal",
    )
    .await;
    assert!(
        detail.contains("automation workflow was not found"),
        "the refusal carries the materialization chain: {detail}"
    );
    assert_eq!(
        materializer.captured_workflow_selections().len(),
        1,
        "the refusal came from materialization, not a run scan"
    );
}

#[tokio::test]
async fn a_source_without_a_slug_refuses_before_materialization() {
    let materializer = TestAutomationRunMaterializer::fail_workflow_not_found();
    let (app, store, materializer, _temp) = retry_latest_app(materializer);
    let source = RunId::new();
    seed_failed_run(&store, source, None, None).await;

    let detail = fabro_test::expect_axum_text(
        retry_latest(&app, &source.to_string(), None).await,
        StatusCode::CONFLICT,
        "retry-latest refusal",
    )
    .await;
    assert!(
        detail.contains("names no workflow slug"),
        "the refusal names the missing slug: {detail}"
    );
    assert!(
        materializer.captured_workflow_selections().is_empty(),
        "no materialization without a selector"
    );
}

#[tokio::test]
async fn an_unknown_run_is_not_found() {
    let app = crate::helpers::test_app_with_no_providers();
    // A valid ULID-shaped id that exists in no store (Crockford base32
    // alphabet, ULID length): the parse passes, the lookup refuses.
    fabro_test::expect_axum_status(
        retry_latest(&app, "01M7CRT3ST0000000000000000", None).await,
        StatusCode::NOT_FOUND,
        "retry-latest on an unknown run",
    )
    .await;
}

#[tokio::test]
async fn a_store_unknown_run_refuses_without_a_successor() {
    let app = crate::helpers::test_app_with_no_providers();
    fabro_test::expect_axum_status_in(
        retry_latest(&app, "01M7CRT3ST0000000000000001", None).await,
        &[StatusCode::NOT_FOUND, StatusCode::CONFLICT],
        "retry-latest on a store-unknown run",
    )
    .await;
}
