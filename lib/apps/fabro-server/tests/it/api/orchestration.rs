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

/// The scripted tracker for this file's work-item surfaces: the
/// observations read resolves linked item states through it (#9 open,
/// #10 closed); every other key resolves to nothing.
struct ObservationsFakeTracker;

#[async_trait::async_trait]
impl fabro_work::TrackerProvider for ObservationsFakeTracker {
    async fn list(
        &self,
        _project: &fabro_work::WorkProject,
        _filter: fabro_work::WorkItemFilter,
    ) -> anyhow::Result<Vec<fabro_work::WorkItem>> {
        Ok(Vec::new())
    }

    async fn get(
        &self,
        _project: &fabro_work::WorkProject,
        item_key: &str,
    ) -> anyhow::Result<Option<fabro_work::WorkItem>> {
        let state = match item_key {
            "denkhaus/work#9" => "open",
            "denkhaus/work#10" => "closed",
            _ => return Ok(None),
        };
        Ok(Some(fabro_work::WorkItem {
            item_key:   item_key.to_owned(),
            title:      "observations fixture item".to_owned(),
            body:       String::new(),
            state:      state.to_owned(),
            labels:     vec!["project:fabro".to_owned()],
            priority:   2,
            url:        String::new(),
            updated_at: None,
        }))
    }

    async fn create(
        &self,
        _project: &fabro_work::WorkProject,
        draft: fabro_work::WorkItemDraft,
    ) -> anyhow::Result<fabro_work::WorkItem> {
        Ok(fabro_work::WorkItem {
            item_key:   draft.title.clone(),
            title:      draft.title,
            body:       draft.body,
            state:      "open".to_owned(),
            labels:     draft.labels,
            priority:   2,
            url:        String::new(),
            updated_at: None,
        })
    }

    async fn update(
        &self,
        _project: &fabro_work::WorkProject,
        _item_key: &str,
        _title: Option<&str>,
        _body: Option<&str>,
    ) -> anyhow::Result<fabro_work::WorkItem> {
        anyhow::bail!("update is outside the observations IT's scope")
    }

    async fn close(
        &self,
        _project: &fabro_work::WorkProject,
        _item_key: &str,
        _reason: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn comment(
        &self,
        _project: &fabro_work::WorkProject,
        _item_key: &str,
        _body: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn project_claim(
        &self,
        _project: &fabro_work::WorkProject,
        _item_key: &str,
        _run_id: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn clear_claim(
        &self,
        _project: &fabro_work::WorkProject,
        _item_key: &str,
        _run_id: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

fn orchestration_app() -> (
    axum::Router,
    Arc<fabro_server::server::AppState>,
    Arc<fabro_store::Database>,
) {
    let settings = crate::helpers::test_settings();
    let (store, artifact_store) = fabro_server::test_support::test_store_bundle();
    let state = fabro_server::test_support::test_app_state_with_store_and_tracker(
        settings.server_settings,
        settings.manifest_run_defaults,
        5,
        Arc::clone(&store),
        artifact_store,
        Arc::new(ObservationsFakeTracker),
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

/// The other-field requirement (work#1126): `requires=merge=enabled`
/// on `field=closable` is the merger's worklist — exactly the
/// merge-enabled runs without a settled closable verdict. A
/// merge-pending or merge-refused run never reaches the merge door;
/// the malformed shapes refuse with teaching codes.
#[tokio::test]
async fn the_queue_requirement_lists_exactly_the_merger_worklist() {
    let (app, state, store) = orchestration_app();
    let adjudicator = RunId::new();
    let merger = RunId::new();
    let enabled = RunId::new();
    let pending = RunId::new();
    let refused = RunId::new();
    let drained = RunId::new();
    seed_run(&store, adjudicator, "adjudicator", true).await;
    seed_run(&store, merger, "merger", true).await;
    for run in [&enabled, &pending, &refused, &drained] {
        seed_run(&store, *run, "develop", true).await;
    }
    let token = state.test_issue_worker_token_with_run_tools(&adjudicator);
    let merger_token = state.test_issue_worker_token_with_run_tools(&merger);

    // merge:enabled on `enabled`; merge:refused on `refused`; `pending`
    // carries nothing; `drained` is merge:enabled AND closable:enabled
    // (the merger already processed it). The adjudicator writes merge;
    // the merger writes closable (field ownership at the door).
    for (target, field, verdict) in [
        (enabled, "merge", "enabled"),
        (refused, "merge", "refused"),
        (drained, "merge", "enabled"),
    ] {
        let response = mark_call(
            &app,
            &token,
            &target,
            json!({"field": field, "verdict": verdict}),
        )
        .await;
        response_json(response, StatusCode::OK, "seed merge verdict").await;
    }
    let response = mark_call(
        &app,
        &merger_token,
        &drained,
        json!({"field": "closable", "verdict": "enabled"}),
    )
    .await;
    response_json(response, StatusCode::OK, "seed closable verdict").await;

    let request = Request::builder()
        .method(Method::GET)
        .uri(api(
            "/runs/queue?field=closable&requires=merge=enabled&workflow=develop",
        ))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .expect("merger worklist request builds");
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("merger worklist request runs");
    let body = response_json(response, StatusCode::OK, "merger worklist read").await;
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
        vec![enabled.to_string()],
        "the merger worklist holds exactly the merge-enabled run without a settled closable \
         verdict — pending and refused never reach the merge door, the drained run stays \
         out: {body}"
    );

    // The teaching refusals: the same field, the verdict combination,
    // the malformed pair, the unknown halves.
    for (query, code) in [
        (
            "/runs/queue?field=closable&requires=closable=enabled",
            "queue_requires_same_field",
        ),
        (
            "/runs/queue?field=closable&verdict=enabled&requires=merge=enabled",
            "queue_requires_conflict",
        ),
        (
            "/runs/queue?field=closable&requires=merge",
            "queue_requires_malformed",
        ),
        (
            "/runs/queue?field=closable&requires=bogus=enabled",
            "queue_requires_unknown_field",
        ),
        (
            "/runs/queue?field=closable&requires=merge=parked",
            "queue_requires_not_settled",
        ),
    ] {
        let request = Request::builder()
            .method(Method::GET)
            .uri(api(query))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .expect("refusal request builds");
        let response = app
            .clone()
            .oneshot(request)
            .await
            .expect("refusal request runs");
        let body = response_json(response, StatusCode::BAD_REQUEST, "requirement refusal").await;
        assert_eq!(
            body["errors"][0]["code"].as_str().unwrap_or_default(),
            code,
            "the refusal carries its teaching code: {body}"
        );
    }
}

/// The merge door's refusal arms (stage E): every non-merger shape
/// refuses BEFORE any GitHub call — a plain worker, the adjudicator
/// (the content judge never integrates), a self-merge, a PR-less run,
/// and a run whose merge verdict is not enabled.
#[tokio::test]
async fn the_merge_door_refuses_every_non_merger_shape() {
    let (app, state, store) = orchestration_app();
    let merger = RunId::new();
    let adjudicator = RunId::new();
    let plain = RunId::new();
    let merged = RunId::new();
    let pr_less = RunId::new();
    seed_run(&store, merger, "merger", true).await;
    seed_run(&store, adjudicator, "adjudicator", true).await;
    seed_run(&store, plain, "develop", true).await;
    seed_run(&store, merged, "develop", true).await;
    seed_run(&store, pr_less, "develop", true).await;

    // The merged run carries a PR and merge = enabled (the adjudicator's
    // content verdict); the PR-less run carries neither.
    store
        .run_summary_store()
        .platform_records()
        .append(
            &merged,
            &fabro_store::platform_records::PlatformRecord::PullRequestCreated(
                fabro_store::platform_records::PullRequestCreatedRecord {
                    number:    501,
                    owner:     "denkhaus".to_string(),
                    repo:      "fabro".to_string(),
                    html_url:  "https://github.com/denkhaus/fabro/pull/501".to_string(),
                    head_sha:  None,
                    draft:     false,
                    operation: None,
                },
            ),
            None,
        )
        .await
        .unwrap();
    store
        .run_summary_store()
        .platform_records()
        .append(
            &merged,
            &fabro_store::platform_records::PlatformRecord::RunOrchestration(
                fabro_store::platform_records::RunOrchestrationRecord {
                    field:   fabro_types::OrchestrationField::Merge,
                    verdict: fabro_types::OrchestrationVerdict::Enabled,
                    by_run:  Some(adjudicator),
                },
            ),
            None,
        )
        .await
        .unwrap();

    let merge_as = |worker: &RunId, target: &RunId| {
        let app = app.clone();
        let token = state.test_issue_worker_token_with_run_tools(worker);
        let target = *target;
        async move {
            app.oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(api(&format!("/runs/{target}/merge")))
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .expect("merge request builds"),
            )
            .await
            .expect("merge request runs")
        }
    };

    // A plain develop worker is not on the allowlist at all.
    let refused = merge_as(&plain, &merged).await;
    assert_eq!(
        refused.status(),
        StatusCode::FORBIDDEN,
        "a plain worker refuses"
    );
    // The adjudicator judges content — it never integrates.
    let judge = merge_as(&adjudicator, &merged).await;
    assert_eq!(
        judge.status(),
        StatusCode::FORBIDDEN,
        "the adjudicator refuses"
    );
    let judge_body = response_json(judge, StatusCode::FORBIDDEN, "adjudicator refusal").await;
    assert!(
        judge_body["errors"][0]["detail"]
            .as_str()
            .expect("detail")
            .contains("merger"),
        "the refusal names the merger as the only integrator: {judge_body}"
    );
    // A run cannot merge itself.
    let self_merge = merge_as(&merger, &merger).await;
    assert_eq!(
        self_merge.status(),
        StatusCode::FORBIDDEN,
        "a self-merge refuses"
    );
    // A PR-less run has nothing to merge.
    let no_pr = merge_as(&merger, &pr_less).await;
    assert_eq!(
        no_pr.status(),
        StatusCode::CONFLICT,
        "a PR-less run refuses"
    );
    let no_pr_body = response_json(no_pr, StatusCode::CONFLICT, "PR-less refusal").await;
    assert!(
        no_pr_body["errors"][0]["detail"]
            .as_str()
            .expect("detail")
            .contains("no pull request"),
        "the refusal names the missing pull request: {no_pr_body}"
    );
}

/// The observations read door: a run's observation records in seq order
/// with the category that routes adjudication, the stage attribution
/// from the positioned fold, and the linked work-item column (the run
/// detail observations table, denkhaus/work#1098).
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
    // Position the FIRST record (seq 1) on a stage: patch the seeded
    // projection's stage fold the way the projector would — the stage's
    // observations list is the attribution the handler joins on.
    let pool = store.run_summary_store().pool();
    let projection_json: String =
        sqlx::query_scalar("SELECT projection_json FROM petri_projection WHERE run_id = ?")
            .bind(run.to_string())
            .fetch_one(&pool)
            .await
            .expect("seeded projection row");
    let mut projection: fabro_types::RunProjection =
        serde_json::from_str(&projection_json).expect("projection json");
    let stage = projection.stage_entry(
        "reviewer",
        3,
        std::num::NonZeroU32::new(1).expect("nonzero"),
    );
    stage.observations.push(fabro_types::StageObservation {
        seq:      1,
        category: fabro_types::ObservationCategory::Painpoint,
        text:     "positioned".to_string(),
    });
    let patched_json = serde_json::to_string(&projection).expect("patched projection json");
    serde_json::from_str::<fabro_types::RunProjection>(&patched_json)
        .expect("patched projection round-trips");
    sqlx::query("UPDATE petri_projection SET projection_json = ? WHERE run_id = ?")
        .bind(patched_json)
        .bind(run.to_string())
        .execute(&pool)
        .await
        .expect("projection patch");
    // Link one work item to the first record through the user-only link
    // surface — the column's data source.
    seed_work_project(&state).await;
    let link_app = fabro_server::test_support::build_test_router(Arc::clone(&state));
    let link_request = Request::builder()
        .method(Method::POST)
        .uri(api("/projects/fabro/work-items/link"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "itemKey": "denkhaus/work#9",
                "runId": run.to_string(),
                "seq": 1,
            }))
            .expect("link body serializes"),
        ))
        .expect("link request builds");
    let link_response = link_app.oneshot(link_request).await.expect("link runs");
    assert_eq!(
        link_response.status(),
        StatusCode::OK,
        "the observation link seeds through the user surface"
    );

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
    assert_eq!(
        observations[0]["stage"].as_str(),
        Some("reviewer@3"),
        "the positioned record carries its stage: {body}"
    );
    let linked = observations[0]["linked_work_items"]
        .as_array()
        .unwrap_or_else(|| panic!("linked items array: {body}"));
    assert_eq!(
        linked[0]["item_key"].as_str(),
        Some("denkhaus/work#9"),
        "the linked work item rides the record: {body}"
    );
    assert_eq!(
        linked[0]["item_state"].as_str(),
        Some("open"),
        "the linked item's live state rides the link: {body}"
    );
    assert_eq!(observations[1]["category"].as_str(), Some("note"));
    assert_eq!(
        observations[1]["stage"].as_str(),
        None,
        "a positionless record renders un-attributed: {body}"
    );
    assert!(
        observations[1]["linked_work_items"]
            .as_array()
            .expect("linked items always present")
            .is_empty(),
        "an unlinked record carries an empty column: {body}"
    );
}

/// A live claim outranks the tracker read: an open item with a live
/// claim resolves in_progress (a run is working it); a closed item
/// stays closed beside any claim state.
#[tokio::test]
async fn the_observations_read_resolves_the_claim_ledgers_in_progress() {
    let (app, state, store) = orchestration_app();
    let run = RunId::new();
    seed_run(&store, run, "develop", true).await;
    store
        .run_summary_store()
        .platform_records()
        .append(
            &run,
            &fabro_store::platform_records::PlatformRecord::ObservationRecorded(
                fabro_store::platform_records::ObservationRecordedRecord {
                    category:  fabro_types::ObservationCategory::Painpoint,
                    text:      "claimed painpoint".to_owned(),
                    execution: 1,
                    firing:    1,
                },
            ),
            None,
        )
        .await
        .expect("observation seed");
    seed_work_project(&state).await;
    let link_app = fabro_server::test_support::build_test_router(Arc::clone(&state));
    for key in ["denkhaus/work#9", "denkhaus/work#10"] {
        let link_request = Request::builder()
            .method(Method::POST)
            .uri(api("/projects/fabro/work-items/link"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::to_vec(&json!({
                    "itemKey": key,
                    "runId": run.to_string(),
                    "seq": 1,
                }))
                .expect("link body serializes"),
            ))
            .expect("link request builds");
        let link_response = link_app
            .clone()
            .oneshot(link_request)
            .await
            .expect("link runs");
        assert_eq!(link_response.status(), StatusCode::OK, "link {key} seeds");
    }
    // A live claim on the OPEN item: the ledger says a run works it.
    seed_claim(&state, &run, "denkhaus/work#9").await;

    let token = state.test_issue_worker_token_with_run_tools(&run);
    let request = Request::builder()
        .method(Method::GET)
        .uri(api(&format!("/runs/{run}/observations")))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .expect("observations request builds");
    let response = app.oneshot(request).await.expect("request runs");
    let body = response_json(response, StatusCode::OK, "observations read").await;
    let states: Vec<&str> = body["observations"][0]["linked_work_items"]
        .as_array()
        .expect("linked items")
        .iter()
        .filter_map(|link| link["item_state"].as_str())
        .collect();
    assert!(
        states.contains(&"in_progress"),
        "the claimed open item reads in_progress: {body}"
    );
    assert!(
        states.contains(&"closed"),
        "the closed item stays closed: {body}"
    );
}

/// The global observations feed: records across runs, newest first,
/// with the DERIVED triage state — the triage filter pages through the
/// derivation (over-fetch), and a live claim reads in_progress.
#[tokio::test]
async fn the_global_observations_feed_derives_and_filters_triage() {
    let (app, state, store) = orchestration_app();
    let older = RunId::new();
    let newer = RunId::new();
    seed_run(&store, older, "develop", true).await;
    seed_run(&store, newer, "develop", true).await;
    let records = store.run_summary_store().platform_records();
    for (run, text) in [(older, "unlinked painpoint"), (newer, "claimed painpoint")] {
        records
            .append(
                &run,
                &fabro_store::platform_records::PlatformRecord::ObservationRecorded(
                    fabro_store::platform_records::ObservationRecordedRecord {
                        category:  fabro_types::ObservationCategory::Painpoint,
                        text:      text.to_owned(),
                        execution: 1,
                        firing:    1,
                    },
                ),
                None,
            )
            .await
            .expect("observation seed");
    }
    records
        .append(
            &older,
            &fabro_store::platform_records::PlatformRecord::ObservationRecorded(
                fabro_store::platform_records::ObservationRecordedRecord {
                    category:  fabro_types::ObservationCategory::Note,
                    text:      "a plain note".to_owned(),
                    execution: 1,
                    firing:    1,
                },
            ),
            None,
        )
        .await
        .expect("observation seed");
    seed_work_project(&state).await;
    let link_app = fabro_server::test_support::build_test_router(Arc::clone(&state));
    let link_request = Request::builder()
        .method(Method::POST)
        .uri(api("/projects/fabro/work-items/link"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "itemKey": "denkhaus/work#9",
                "runId": newer.to_string(),
                "seq": 1,
            }))
            .expect("link body serializes"),
        ))
        .expect("link request builds");
    let link_response = link_app.oneshot(link_request).await.expect("link runs");
    assert_eq!(link_response.status(), StatusCode::OK, "link seeds");
    seed_claim(&state, &newer, "denkhaus/work#9").await;

    let token = state.test_issue_worker_token_with_run_tools(&newer);
    let request = Request::builder()
        .method(Method::GET)
        .uri(api("/observations?triage=in_progress"))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .expect("feed request builds");
    let response = app.clone().oneshot(request).await.expect("request runs");
    let body = response_json(response, StatusCode::OK, "observations feed").await;
    let feed = body["observations"].as_array().expect("feed array");
    assert!(
        feed.iter()
            .all(|item| item["triage_state"].as_str() == Some("in_progress")),
        "the triage filter holds: {body}"
    );
    assert!(
        feed.iter()
            .any(|item| item["text"].as_str() == Some("claimed painpoint")),
        "the claimed record is in the filtered feed: {body}"
    );
    assert!(
        !feed
            .iter()
            .any(|item| item["text"].as_str() == Some("unlinked painpoint")),
        "the unlinked record is filtered out: {body}"
    );
    // The pager's EXACT total (denkhaus/work#1296): the triage filter
    // counts its derived matches (the walk), the unfiltered feed counts
    // every record (the SQL count), and the category filter counts
    // through SQL — stable across a full walk, never a growing guess.
    assert_eq!(
        body["total"].as_i64(),
        Some(1),
        "the filtered feed counts its matches: {body}"
    );
    for (query, want) in [("", 3i64), ("category=painpoint", 2)] {
        let request = Request::builder()
            .method(Method::GET)
            .uri(api(&format!("/observations?{query}")))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .expect("feed request builds");
        let response = app.clone().oneshot(request).await.expect("request runs");
        let body = response_json(response, StatusCode::OK, "observations feed").await;
        assert_eq!(
            body["total"].as_i64(),
            Some(want),
            "the {query:?} feed names its exact total: {body}"
        );
    }
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
