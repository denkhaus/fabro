//! The merger's merge door over a mocked GitHub (ADR-0039 stage E,
//! denkhaus/work#1107): the happy path (squash merge, 200, the response
//! names the PR) and the conflict signal (409 `merge_conflict` — the
//! fast path parks a run for the agent pass on exactly that code). The
//! authorization arms are pinned externally (tests/it/api/orchestration)
//! where no GitHub plumbing is needed — every refusal precedes the
//! GitHub call.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use fabro_store::platform_records::{
    PlatformRecord, PlatformRecordKind, PullRequestCreatedRecord, RunOrchestrationRecord,
};
use fabro_types::{OrchestrationField, OrchestrationVerdict, RunId};
use httpmock::MockServer;
use serde_json::json;
use tower::ServiceExt;

use super::super::test_support::build_test_router;
use super::tests::create_github_token_app_state;
use crate::server::AppState;

/// Seed a run row with a workflow slug, a terminal status, and an
/// optional pull request already folded onto the projection (the
/// post-fold production state — the external IT precedent).
pub(super) async fn seed_run(
    state: &Arc<AppState>,
    run_id: &RunId,
    workflow_slug: &str,
    pull_request: Option<fabro_types::PullRequestLink>,
) {
    let mut projection = fabro_types::RunProjection::new(
        format!("{workflow_slug} run"),
        fabro_types::RunSpec {
            run_id:              *run_id,
            settings:            fabro_types::WorkflowSettings::default(),
            graph:               fabro_types::RunGraph::new("test"),
            graph_source:        None,
            workflow_slug:       Some(workflow_slug.to_string()),
            workflow_version_id: None,
            target:              None,
            automation:          None,
            source_directory:    None,
            labels:              std::collections::HashMap::new(),
            provenance:          fabro_types::test_support::test_run_provenance(),
            definition_blob:     None,
            spec_blob:           None,
            git:                 None,
            fork_source_ref:     None,
            admission:           fabro_types::PetriAdmission::default(),
        },
        chrono::Utc::now(),
    );
    projection.status = fabro_types::RunStatus::Succeeded {
        reason: fabro_types::SuccessReason::Completed,
    };
    projection.pull_request = pull_request;
    let pool = state.stores.run_summaries.pool();
    let mut tx = pool.begin().await.expect("seed tx");
    fabro_store::RunSummaryStore::write_petri_run_row_on_connection(&mut tx, run_id, &projection)
        .await
        .expect("run row seed writes");
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
    .expect("projection row seed");
    tx.commit().await.expect("seed tx commits");
}

/// The merged fixture: a develop run carrying PR #501 and the
/// adjudicator's merge:enabled verdict.
async fn seed_mergeable_target(state: &Arc<AppState>, target: &RunId, adjudicator: &RunId) {
    state
        .stores
        .run_summaries
        .platform_records()
        .append(
            target,
            &PlatformRecord::PullRequestCreated(PullRequestCreatedRecord {
                number:    501,
                owner:     "denkhaus".to_string(),
                repo:      "fabro".to_string(),
                html_url:  "https://github.com/denkhaus/fabro/pull/501".to_string(),
                head_sha:  None,
                draft:     false,
                operation: None,
            }),
            None,
        )
        .await
        .expect("PR record seed");
    state
        .stores
        .run_summaries
        .platform_records()
        .append(
            target,
            &PlatformRecord::RunOrchestration(RunOrchestrationRecord {
                field:   OrchestrationField::Merge,
                verdict: OrchestrationVerdict::Enabled,
                by_run:  Some(*adjudicator),
            }),
            None,
        )
        .await
        .expect("verdict seed");
}

async fn merge_call(
    state: &Arc<AppState>,
    worker: &RunId,
    target: &RunId,
) -> axum::http::Response<Body> {
    let app = build_test_router(Arc::clone(state));
    let token = state.test_issue_worker_token_with_run_tools(worker);
    app.oneshot(
        Request::builder()
            .method(Method::POST)
            .uri(format!("/api/v1/runs/{target}/merge"))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .expect("merge request builds"),
    )
    .await
    .expect("merge request runs")
}

async fn body_json(response: axum::http::Response<Body>) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("merge response body");
    serde_json::from_slice(&bytes).expect("merge response json")
}

/// The happy path: the merger merges the adjudicator-enabled run's PR
/// server-side (squash) and the response names the PR.
#[tokio::test]
async fn the_merger_door_merges_a_merge_enabled_pull_request() {
    let github = MockServer::start();
    let state = create_github_token_app_state(Some("ghu_test"), Some(github.base_url()));
    let merger = RunId::new();
    let adjudicator = RunId::new();
    let target = RunId::new();
    seed_run(&state, &merger, "merger", None).await;
    seed_run(&state, &adjudicator, "adjudicator", None).await;
    seed_run(
        &state,
        &target,
        "develop",
        Some(fabro_types::PullRequestLink {
            owner:  "denkhaus".to_string(),
            repo:   "fabro".to_string(),
            number: 501,
            state:  None,
        }),
    )
    .await;
    seed_mergeable_target(&state, &target, &adjudicator).await;

    let merge_mock = github.mock(|when, then| {
        when.method("PUT")
            .path("/repos/denkhaus/fabro/pulls/501/merge")
            .header("authorization", "Bearer ghu_test")
            .json_body(json!({"merge_method": "squash"}));
        then.status(200).json_body(json!({"sha": "abc1234"}));
    });

    let response = merge_call(&state, &merger, &target).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the merger door merges the enabled run's PR"
    );
    let body = body_json(response).await;
    assert_eq!(body["merged"], json!(true), "the response reports merged");
    assert_eq!(
        body["pull_request"], "https://github.com/denkhaus/fabro/pull/501",
        "the response names the merged PR: {body}"
    );
    assert_eq!(merge_mock.calls(), 1, "exactly one GitHub merge call");
    // The merged outcome lands on the stored link (denkhaus/work#1173):
    // one `pull_request.merged` platform record, folded onto the
    // projection's `pull_request.state` — the field the run list and the
    // board's PR chip read.
    assert_eq!(
        merged_records_of(&state, &target).await,
        1,
        "the merge door records the merged outcome exactly once"
    );
    let projection = super::run_records::projection(&state, target)
        .await
        .expect("the projection reads")
        .expect("the target run's projection exists");
    assert_eq!(
        projection.pull_request.as_ref().and_then(|link| link.state),
        Some(fabro_types::PullRequestState::Merged),
        "the folded link carries the merged state for the run list"
    );
}

/// The conflict signal: a declined merge answers 409 with the
/// `merge_conflict` code — the fast path's park trigger. Both GitHub
/// decline statuses carry it: 409 (a real conflict) and 405 (not
/// mergeable — the method or PR state refuses). The 405 arm is the
/// #1117 pin: the TYPED variant routes it, never a prose match.
#[tokio::test]
async fn a_conflicted_merge_answers_409_with_the_conflict_code() {
    for status in [409u16, 405] {
        let github = MockServer::start();
        let state = create_github_token_app_state(Some("ghu_test"), Some(github.base_url()));
        let merger = RunId::new();
        let adjudicator = RunId::new();
        let target = RunId::new();
        seed_run(&state, &merger, "merger", None).await;
        seed_run(&state, &adjudicator, "adjudicator", None).await;
        seed_run(
            &state,
            &target,
            "develop",
            Some(fabro_types::PullRequestLink {
                owner:  "denkhaus".to_string(),
                repo:   "fabro".to_string(),
                number: 501,
                state:  None,
            }),
        )
        .await;
        seed_mergeable_target(&state, &target, &adjudicator).await;

        github.mock(move |when, then| {
            when.method("PUT")
                .path("/repos/denkhaus/fabro/pulls/501/merge");
            then.status(status)
                .json_body(json!({"message": "declined"}));
        });

        let response = merge_call(&state, &merger, &target).await;
        assert_eq!(
            response.status(),
            StatusCode::CONFLICT,
            "a merge decline (status {status}) answers 409"
        );
        let body = body_json(response).await;
        assert_eq!(
            body["errors"][0]["code"], "merge_conflict",
            "the conflict code names the park trigger (status {status}): {body}"
        );
    }
}

/// Idempotence (work#1127): a re-drive of an ALREADY-MERGED pull
/// request answers 200 immediately — the explicit state read
/// short-circuits before the merge call. A crash between merge and the
/// closable mark must not strand a merged run as a phantom conflict
/// packet. No PUT mock exists: a merge call would hit the mock
/// server's unmatched 404 and fail the 200 assertion.
#[tokio::test]
async fn a_re_drive_of_an_already_merged_pull_request_answers_200() {
    let github = MockServer::start();
    let state = create_github_token_app_state(Some("ghu_test"), Some(github.base_url()));
    let merger = RunId::new();
    let adjudicator = RunId::new();
    let target = RunId::new();
    seed_run(&state, &merger, "merger", None).await;
    seed_run(&state, &adjudicator, "adjudicator", None).await;
    seed_run(
        &state,
        &target,
        "develop",
        Some(fabro_types::PullRequestLink {
            owner:  "denkhaus".to_string(),
            repo:   "fabro".to_string(),
            number: 501,
            state:  None,
        }),
    )
    .await;
    seed_mergeable_target(&state, &target, &adjudicator).await;

    let state_mock = github.mock(|when, then| {
        when.method("GET")
            .path("/repos/denkhaus/fabro/pulls/501")
            .header("authorization", "Bearer ghu_test");
        then.status(200).json_body(json!({
            "number": 501,
            "title": "the merged run's pull request",
            "state": "closed",
            "draft": false,
            "merged": true,
            "merged_at": "2026-11-04T20:00:00Z",
            "mergeable": true,
            "additions": 10,
            "deletions": 2,
            "changed_files": 2,
            "html_url": "https://github.com/denkhaus/fabro/pull/501",
            "user": {"login": "fabro-run"},
            "head": {"ref": "fabro/run/01TARGET"},
            "base": {"ref": "denkhaus"},
            "created_at": "2026-11-04T19:00:00Z",
            "updated_at": "2026-11-04T20:00:00Z"
        }));
    });

    let response = merge_call(&state, &merger, &target).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a re-drive of an already-merged PR answers 200 (no phantom conflict)"
    );
    let body = body_json(response).await;
    assert_eq!(body["merged"], json!(true), "the response reports merged");
    assert_eq!(
        body["pull_request"], "https://github.com/denkhaus/fabro/pull/501",
        "the response names the PR: {body}"
    );
    assert_eq!(
        state_mock.calls(),
        1,
        "exactly one state read short-circuits the merge call"
    );
    // The idempotent path still records the merged outcome exactly once
    // (denkhaus/work#1173): a second re-drive of the settled link adds
    // no further record.
    assert_eq!(
        merged_records_of(&state, &target).await,
        1,
        "the idempotent re-drive records the merged state once"
    );
    let response = merge_call(&state, &merger, &target).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the second re-drive still answers 200"
    );
    assert_eq!(
        merged_records_of(&state, &target).await,
        1,
        "a re-drive of a settled merged link records nothing new"
    );
}

async fn merged_records_of(state: &Arc<AppState>, target: &RunId) -> usize {
    state
        .stores
        .run_summaries
        .platform_records()
        .read_kind(target, PlatformRecordKind::PullRequestMerged)
        .await
        .expect("the merged records read")
        .len()
}
