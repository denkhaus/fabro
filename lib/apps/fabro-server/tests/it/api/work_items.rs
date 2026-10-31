use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use fabro_server::test_support::build_test_router;
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::helpers::{api, response_json, test_app_state};

const RUN_A: &str = "01JNQVR7M0EJ5GKAT2SC4ERS1Z";
const RUN_B: &str = "01JNQVR7M0EJ5GKAT2SC4ERS20";

fn app() -> axum::Router {
    build_test_router(test_app_state())
}

fn json_request(method: Method, path: &str, body: &Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(api(path))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(body).expect("work-item fixture should serialize"),
        ))
        .expect("work-item JSON request should build")
}

fn project_body() -> Value {
    json!({
        "displayName": "Fabro",
        "trackerProvider": "github",
        "trackerRepo": "denkhaus/work",
        // The run->project scope binding resolves through it
        // (denkhaus/work#991): a run on denkhaus/fabro maps here.
        "targetRepo": "denkhaus/fabro",
        "projectLabel": "project:fabro",
        "laneLabels": {"loop": "lane:loop"},
        "claimLabelPrefix": "fabro:run:",
        "claimCommentEnabled": false
    })
}

fn grant_body(item_key: &str, run_id: &str) -> Value {
    json!({"itemKey": item_key, "runId": run_id, "lane": "loop"})
}

async fn put_project(app: &axum::Router) {
    let response = app
        .clone()
        .oneshot(json_request(
            Method::PUT,
            "/projects/fabro",
            &project_body(),
        ))
        .await
        .expect("work-item test step should succeed");
    response_json(response, StatusCode::OK, "PUT /projects/fabro").await;
}

/// A close without evidence is refused at the boundary for EVERY
/// client (retro finding 2026-10-26: an empty close reason once landed
/// with nothing red): empty AND whitespace-only reasons both answer
/// 400 before any store or tracker call.
#[tokio::test]
async fn a_close_without_a_reason_is_refused_at_the_boundary() {
    let app = app();
    put_project(&app).await;

    for reason in ["", "   \n"] {
        let response = app
            .clone()
            .oneshot(json_request(
                Method::POST,
                "/projects/fabro/work-items/close",
                &json!({"itemKey": "denkhaus/work#1", "reason": reason}),
            ))
            .await
            .expect("close request should run");
        let value = response_json(
            response,
            StatusCode::BAD_REQUEST,
            "a reason-less close is refused",
        )
        .await;
        let detail = value["errors"][0]["detail"]
            .as_str()
            .expect("the error detail");
        assert!(
            detail.contains("close reason must not be empty"),
            "the refusal names the reason requirement: {detail}"
        );
    }
}

#[tokio::test]
async fn project_crud_and_the_claim_lifecycle() {
    let app = app();
    put_project(&app).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(api("/projects/fabro"))
                .body(Body::empty())
                .expect("work-item test step should succeed"),
        )
        .await
        .expect("work-item test step should succeed");
    let project = response_json(response, StatusCode::OK, "GET /projects/fabro").await;
    assert_eq!(project["projectId"], "fabro");
    assert_eq!(project["trackerRepo"], "denkhaus/work");

    let response = app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/projects/fabro/claims",
            &grant_body("denkhaus/work#1", RUN_A),
        ))
        .await
        .expect("work-item test step should succeed");
    let granted = response_json(response, StatusCode::OK, "POST /projects/fabro/claims").await;
    assert_eq!(granted["outcome"], "granted");
    assert_eq!(granted["claim"]["state"], "granted");
    let claim_id = granted["claim"]["claimId"]
        .as_str()
        .expect("claim id")
        .to_string();

    let response = app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/projects/fabro/claims",
            &grant_body("denkhaus/work#1", RUN_B),
        ))
        .await
        .expect("work-item test step should succeed");
    let conflict = response_json(response, StatusCode::OK, "second grant").await;
    assert_eq!(conflict["outcome"], "conflict");
    assert_eq!(conflict["claim"]["runId"], RUN_A);

    let response = app
        .clone()
        .oneshot(json_request(
            Method::POST,
            &format!("/claims/{claim_id}/transition"),
            &json!({"state": "active"}),
        ))
        .await
        .expect("work-item test step should succeed");
    let active = response_json(response, StatusCode::OK, "activate claim").await;
    assert_eq!(active["state"], "active");

    let response = app
        .clone()
        .oneshot(json_request(
            Method::POST,
            &format!("/claims/{claim_id}/transition"),
            &json!({"state": "awaiting_merge", "prUrl": "https://github.com/o/r/pull/1"}),
        ))
        .await
        .expect("work-item test step should succeed");
    let awaiting = response_json(response, StatusCode::OK, "await merge").await;
    assert_eq!(awaiting["state"], "awaiting_merge");
    assert_eq!(awaiting["prUrl"], "https://github.com/o/r/pull/1");

    let response = app
        .clone()
        .oneshot(json_request(
            Method::POST,
            &format!("/claims/{claim_id}/release"),
            &json!({"reason": "planner skipped"}),
        ))
        .await
        .expect("work-item test step should succeed");
    let released = response_json(response, StatusCode::OK, "release claim").await;
    assert_eq!(released["state"], "released");
    assert_eq!(released["releaseReason"], "planner skipped");

    let response = app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/projects/fabro/claims",
            &grant_body("denkhaus/work#1", RUN_B),
        ))
        .await
        .expect("work-item test step should succeed");
    let regranted = response_json(response, StatusCode::OK, "re-grant").await;
    assert_eq!(regranted["outcome"], "granted");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(api(&format!("/claims/{claim_id}/events")))
                .body(Body::empty())
                .expect("work-item test step should succeed"),
        )
        .await
        .expect("work-item test step should succeed");
    let events = response_json(response, StatusCode::OK, "claim events").await;
    let events = events["data"].as_array().expect("event list");
    assert_eq!(events.len(), 4);
    assert_eq!(events[0]["toState"], "granted");
    assert_eq!(events[1]["toState"], "active");
    assert_eq!(events[2]["toState"], "awaiting_merge");
    assert_eq!(events[3]["toState"], "released");
}

#[tokio::test]
async fn invalid_transition_returns_conflict() {
    let app = app();
    put_project(&app).await;
    let response = app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/projects/fabro/claims",
            &grant_body("denkhaus/work#2", RUN_A),
        ))
        .await
        .expect("work-item test step should succeed");
    let granted = response_json(response, StatusCode::OK, "grant").await;
    let claim_id = granted["claim"]["claimId"]
        .as_str()
        .expect("work-item test step should succeed");

    let response = app
        .clone()
        .oneshot(json_request(
            Method::POST,
            &format!("/claims/{claim_id}/transition"),
            &json!({"state": "awaiting_merge", "prUrl": "https://github.com/o/r/pull/2"}),
        ))
        .await
        .expect("work-item test step should succeed");
    let body = response_json(response, StatusCode::CONFLICT, "invalid transition").await;
    assert!(
        body["errors"][0]["detail"]
            .as_str()
            .expect("work-item test step should succeed")
            .contains("invalid work claim transition")
    );
}

#[tokio::test]
async fn missing_project_and_claim_return_not_found() {
    let app = app();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(api("/projects/missing"))
                .body(Body::empty())
                .expect("work-item test step should succeed"),
        )
        .await
        .expect("work-item test step should succeed");
    response_json(response, StatusCode::NOT_FOUND, "missing project").await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(api("/claims/00000000-0000-0000-0000-000000000000"))
                .body(Body::empty())
                .expect("work-item test step should succeed"),
        )
        .await
        .expect("work-item test step should succeed");
    response_json(response, StatusCode::NOT_FOUND, "missing claim").await;
}

#[tokio::test]
async fn invalid_project_id_is_rejected() {
    let app = app();
    let response = app
        .oneshot(json_request(
            Method::PUT,
            "/projects/Not_A_Project",
            &project_body(),
        ))
        .await
        .expect("work-item test step should succeed");
    response_json(response, StatusCode::BAD_REQUEST, "invalid project id").await;
}

#[tokio::test]
async fn the_reserved_self_project_id_is_refused() {
    let app = app();
    let response = app
        .oneshot(json_request(Method::PUT, "/projects/self", &project_body()))
        .await
        .expect("work-item test step should succeed");
    response_json(response, StatusCode::BAD_REQUEST, "reserved self").await;
}

mod next_item {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode, header};
    use chrono::Utc;
    use fabro_server::test_support::{TestAppStateBuilder, build_test_router};
    use fabro_work::{TrackerProvider, WorkItem, WorkItemDraft, WorkItemFilter, WorkProject};
    use serde_json::{Value, json};
    use tokio::sync::Mutex as AsyncMutex;
    use tower::ServiceExt;

    use crate::helpers::{api, response_json};

    const RUN_A: &str = "01JNQVR7M0EJ5GKAT2SC4ERS1Z";
    const RUN_B: &str = "01JNQVR7M0EJ5GKAT2SC4ERS20";

    /// A scripted tracker: fixed items, recorded claim projections and
    /// the filters every list call received.
    struct FakeTracker {
        items:   Mutex<Vec<WorkItem>>,
        claims:  AsyncMutex<Vec<(String, String)>>,
        cleared: AsyncMutex<Vec<(String, String)>>,
        filters: AsyncMutex<Vec<WorkItemFilter>>,
    }

    impl FakeTracker {
        fn with_items(items: Vec<WorkItem>) -> Arc<Self> {
            Arc::new(Self {
                items:   Mutex::new(items),
                claims:  AsyncMutex::new(Vec::new()),
                cleared: AsyncMutex::new(Vec::new()),
                filters: AsyncMutex::new(Vec::new()),
            })
        }
    }

    fn item(number: u64, priority: i64) -> WorkItem {
        WorkItem {
            item_key: format!("denkhaus/work#{number}"),
            title: format!("Item {number}"),
            body: "the spec".to_string(),
            state: "open".to_string(),
            labels: vec!["project:fabro".to_string()],
            priority,
            url: format!("https://github.com/denkhaus/work/issues/{number}"),
            updated_at: Some(Utc::now()),
        }
    }

    #[async_trait]
    impl TrackerProvider for FakeTracker {
        async fn list(
            &self,
            _project: &WorkProject,
            filter: WorkItemFilter,
        ) -> anyhow::Result<Vec<WorkItem>> {
            self.filters.lock().await.push(filter.clone());
            let mut items: Vec<WorkItem> = self
                .items
                .lock()
                .expect("work-item test step should succeed")
                .iter()
                .filter(|item| match filter.query.as_deref() {
                    // Search mode mirrors the GitHub adapter: both states,
                    // case-insensitive title+body text match, relevance
                    // approximated by recency (the fake has no ranking
                    // signal).
                    Some(query) => {
                        let needle = query.trim().to_lowercase();
                        item.title.to_lowercase().contains(&needle)
                            || item.body.to_lowercase().contains(&needle)
                    }
                    // List mode mirrors the open-only listing; the server
                    // grants in the order the provider returns and the
                    // GitHub adapter sorts by (priority, updated_at), so
                    // the fake sorts the same way.
                    None => item.state == "open",
                })
                .cloned()
                .collect();
            items.sort_by_key(|item| (item.priority, item.updated_at));
            items.truncate(filter.limit.max(1));
            Ok(items)
        }

        async fn get(
            &self,
            _project: &WorkProject,
            item_key: &str,
        ) -> anyhow::Result<Option<WorkItem>> {
            Ok(self
                .items
                .lock()
                .expect("work-item test step should succeed")
                .iter()
                .find(|item| item.item_key == item_key)
                .cloned())
        }

        async fn create(
            &self,
            _project: &WorkProject,
            draft: WorkItemDraft,
        ) -> anyhow::Result<WorkItem> {
            let mut items = self
                .items
                .lock()
                .expect("work-item test step should succeed");
            let number = items.len() as u64 + 1;
            let created = WorkItem {
                item_key:   format!("denkhaus/work#{number}"),
                title:      draft.title,
                body:       draft.body,
                state:      "open".to_string(),
                labels:     draft.labels,
                priority:   2,
                url:        format!("https://github.com/denkhaus/work/issues/{number}"),
                updated_at: Some(Utc::now()),
            };
            items.push(created.clone());
            Ok(created)
        }

        async fn update(
            &self,
            _project: &WorkProject,
            item_key: &str,
            title: Option<&str>,
            body: Option<&str>,
        ) -> anyhow::Result<WorkItem> {
            let mut items = self
                .items
                .lock()
                .expect("work-item test step should succeed");
            let entry = items
                .iter_mut()
                .find(|item| item.item_key == item_key)
                .ok_or_else(|| anyhow::anyhow!("work item {item_key} not found"))?;
            if let Some(title) = title {
                entry.title = title.to_string();
            }
            if let Some(body) = body {
                entry.body = body.to_string();
            }
            Ok(entry.clone())
        }

        async fn set_lane(
            &self,
            project: &WorkProject,
            item_key: &str,
            lane: Option<&str>,
        ) -> anyhow::Result<WorkItem> {
            let mut items = self
                .items
                .lock()
                .expect("work-item test step should succeed");
            let entry = items
                .iter_mut()
                .find(|item| item.item_key == item_key)
                .ok_or_else(|| anyhow::anyhow!("work item {item_key} not found"))?;
            // Mirrors the GitHub adapter: every declared lane label comes
            // off, then the new one goes on — a move never leaves two
            // lanes on one item.
            let declared: Vec<String> = project.lane_labels.values().cloned().collect();
            entry.labels.retain(|label| !declared.contains(label));
            if let Some(label) = lane.and_then(|lane| project.lane_labels.get(lane)) {
                entry.labels.push(label.clone());
            }
            Ok(entry.clone())
        }

        async fn close(
            &self,
            _project: &WorkProject,
            item_key: &str,
            reason: &str,
        ) -> anyhow::Result<()> {
            let mut items = self
                .items
                .lock()
                .expect("work-item test step should succeed");
            if let Some(entry) = items.iter_mut().find(|item| item.item_key == item_key) {
                entry.state = "closed".to_string();
                entry.body.push_str(reason);
            }
            Ok(())
        }

        async fn comment(
            &self,
            _project: &WorkProject,
            _item_key: &str,
            _body: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn project_claim(
            &self,
            _project: &WorkProject,
            item_key: &str,
            run_id: &str,
        ) -> anyhow::Result<()> {
            self.claims
                .lock()
                .await
                .push((item_key.to_string(), run_id.to_string()));
            Ok(())
        }

        async fn clear_claim(
            &self,
            _project: &WorkProject,
            item_key: &str,
            run_id: &str,
        ) -> anyhow::Result<()> {
            self.cleared
                .lock()
                .await
                .push((item_key.to_string(), run_id.to_string()));
            Ok(())
        }
    }

    async fn app_with_tracker(tracker: Arc<FakeTracker>) -> axum::Router {
        let state = TestAppStateBuilder::new()
            .work_tracker(tracker as Arc<dyn TrackerProvider>)
            .build();
        let app = build_test_router(state);
        put_project(&app).await;
        app
    }

    async fn put_project(app: &axum::Router) {
        let request = Request::builder()
            .method(Method::PUT)
            .uri(api("/projects/fabro"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::to_string(&json!({
                    "displayName": "Fabro",
                    "trackerProvider": "github",
                    "trackerRepo": "denkhaus/work",
                    // The run->project scope binding resolves through it
                    // (denkhaus/work#991): a run on denkhaus/fabro maps here.
                    "targetRepo": "denkhaus/fabro",
                    "projectLabel": "project:fabro",
                    "laneLabels": {"loop": "lane:loop"}
                }))
                .expect("work-item test step should succeed"),
            ))
            .expect("work-item test step should succeed");
        let response = app
            .clone()
            .oneshot(request)
            .await
            .expect("work-item test step should succeed");
        assert_eq!(response.status(), StatusCode::OK);
    }

    async fn json_request(app: axum::Router, method: Method, path: &str, body: &Value) -> Value {
        let request = Request::builder()
            .method(method)
            .uri(api(path))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::to_vec(body).expect("work-item test step should succeed"),
            ))
            .expect("work-item test step should succeed");
        let response = app
            .oneshot(request)
            .await
            .expect("work-item test step should succeed");
        serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("work-item test step should succeed"),
        )
        .expect("work-item test step should succeed")
    }

    #[tokio::test]
    async fn next_grants_the_highest_priority_free_item_and_projects_the_claim() {
        let tracker = FakeTracker::with_items(vec![item(2, 2), item(1, 1), item(3, 1)]);
        let app = app_with_tracker(Arc::clone(&tracker)).await;

        let granted = json_request(
            app.clone(),
            Method::POST,
            "/projects/fabro/next",
            &json!({"runId": RUN_A, "lane": "loop"}),
        )
        .await;
        // Both priority-1 items compete; the tracker's own order (priority,
        // then updated) picks item 1.
        assert_eq!(granted["outcome"], "granted");
        assert_eq!(granted["item"]["itemKey"], "denkhaus/work#1");
        assert_eq!(granted["claim"]["runId"], RUN_A);
        assert_eq!(granted["claim"]["state"], "granted");

        // The tracker projection happened (DB first, projection second).
        let claims = tracker.claims.lock().await;
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].0, "denkhaus/work#1");
        drop(claims);

        // A second run is routed to the NEXT free item, never the same one.
        let second = json_request(
            app.clone(),
            Method::POST,
            "/projects/fabro/next",
            &json!({"runId": RUN_B, "lane": "loop"}),
        )
        .await;
        assert_eq!(second["outcome"], "granted");
        assert_eq!(second["item"]["itemKey"], "denkhaus/work#3");

        // The same run asking again is idempotent.
        let again = json_request(
            app.clone(),
            Method::POST,
            "/projects/fabro/next",
            &json!({"runId": RUN_A, "lane": "loop"}),
        )
        .await;
        assert_eq!(again["outcome"], "already_held");
        assert_eq!(again["item"]["itemKey"], "denkhaus/work#1");
    }

    #[tokio::test]
    async fn next_reports_empty_when_everything_is_claimed() {
        let tracker = FakeTracker::with_items(vec![item(1, 1)]);
        let app = app_with_tracker(Arc::clone(&tracker)).await;

        let granted = json_request(
            app.clone(),
            Method::POST,
            "/projects/fabro/next",
            &json!({"runId": RUN_A, "lane": "loop"}),
        )
        .await;
        assert_eq!(granted["outcome"], "granted");

        let empty = json_request(
            app.clone(),
            Method::POST,
            "/projects/fabro/next",
            &json!({"runId": RUN_B, "lane": "loop"}),
        )
        .await;
        assert_eq!(empty["outcome"], "empty");
        assert!(empty["claim"].is_null());
    }

    #[tokio::test]
    async fn work_items_search_includes_closed_matches_and_plain_list_does_not() {
        let mut closed_match = item(7, 1);
        closed_match.state = "closed".to_string();
        closed_match.body = "the watchdog WARN threshold".to_string();
        let mut titled_match = item(9, 2);
        titled_match.title = "Watchdog tuning".to_string();
        let unrelated = item(10, 1);
        let tracker = FakeTracker::with_items(vec![closed_match, titled_match, unrelated]);
        let app = app_with_tracker(Arc::clone(&tracker)).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(api("/projects/fabro/work-items?query=watchdog"))
                    .body(Body::empty())
                    .expect("work-item test step should succeed"),
            )
            .await
            .expect("work-item test step should succeed");
        let searched = response_json(response, StatusCode::OK, "search mode").await;
        let keys = searched["data"]
            .as_array()
            .expect("search returns an array")
            .iter()
            .map(|item| item["itemKey"].as_str().expect("keys are strings"))
            .collect::<Vec<_>>();
        assert!(
            keys.contains(&"denkhaus/work#7"),
            "closed matches appear: {keys:?}"
        );
        assert!(
            keys.contains(&"denkhaus/work#9"),
            "open matches appear: {keys:?}"
        );
        assert!(
            !keys.contains(&"denkhaus/work#10"),
            "non-matching items stay out: {keys:?}"
        );
    }

    /// The label census seam (denkhaus/work#1001): `label` and `page`
    /// query params reach the tracker filter — `label` beside the
    /// project/lane labels, `page` as the 1-based results page.
    #[tokio::test]
    async fn label_and_page_query_params_reach_the_tracker_filter() {
        let tracker = FakeTracker::with_items(vec![item(1, 1), item(2, 2)]);
        let app = app_with_tracker(Arc::clone(&tracker)).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(api(
                        "/projects/fabro/work-items?label=arch&page=2&limit=100",
                    ))
                    .body(Body::empty())
                    .expect("work-item test step should succeed"),
            )
            .await
            .expect("work-item test step should succeed");
        response_json(response, StatusCode::OK, "labeled paged list").await;

        let filters = tracker.filters.lock().await;
        assert_eq!(filters.len(), 1, "one list call reached the tracker");
        assert_eq!(filters[0].label.as_deref(), Some("arch"));
        assert_eq!(filters[0].page, 2);
        assert_eq!(filters[0].limit, 100);

        // The default stays page 1 with no label filter.
        drop(filters);
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(api("/projects/fabro/work-items"))
                    .body(Body::empty())
                    .expect("work-item test step should succeed"),
            )
            .await
            .expect("work-item test step should succeed");
        response_json(response, StatusCode::OK, "plain list").await;
        let filters = tracker.filters.lock().await;
        assert_eq!(filters[1].label, None);
        assert_eq!(filters[1].page, 1);
    }

    #[tokio::test]
    async fn closing_an_item_releases_its_claim_and_clears_the_projection() {
        let tracker = FakeTracker::with_items(vec![item(1, 1)]);
        let app = app_with_tracker(Arc::clone(&tracker)).await;

        let granted = json_request(
            app.clone(),
            Method::POST,
            "/projects/fabro/next",
            &json!({"runId": RUN_A, "lane": "loop"}),
        )
        .await;
        let claim_id = granted["claim"]["claimId"]
            .as_str()
            .expect("work-item test step should succeed")
            .to_string();

        let closed = json_request(
            app.clone(),
            Method::POST,
            "/projects/fabro/work-items/close",
            &json!({"itemKey": "denkhaus/work#1", "reason": "landed"}),
        )
        .await;
        assert_eq!(closed["item"]["state"], "closed");
        assert_eq!(
            closed["overriddenRadius"]
                .as_array()
                .map(Vec::len)
                .unwrap_or(0),
            0,
            "an unforced close with an empty radius overrides nothing"
        );

        let claim =
            json_request(app, Method::GET, &format!("/claims/{claim_id}"), &json!({})).await;
        assert_eq!(claim["state"], "released");
        assert_eq!(claim["releaseReason"], "item closed");

        let cleared = tracker.cleared.lock().await;
        assert_eq!(cleared.len(), 1);
        assert_eq!(cleared[0].0, "denkhaus/work#1");
    }

    #[tokio::test]
    async fn creating_an_item_carries_the_project_label() {
        let tracker = FakeTracker::with_items(vec![]);
        let app = app_with_tracker(Arc::clone(&tracker)).await;

        let created = json_request(
            app,
            Method::POST,
            "/projects/fabro/work-items",
            &json!({"title": "Revisor finding", "body": "the spec", "labels": ["revision"]}),
        )
        .await;
        assert_eq!(created["itemKey"], "denkhaus/work#1");
        assert!(
            created["labels"]
                .as_array()
                .expect("work-item test step should succeed")
                .iter()
                .any(|label| label == "project:fabro")
        );
        assert!(
            created["labels"]
                .as_array()
                .expect("work-item test step should succeed")
                .iter()
                .any(|label| label == "revision")
        );
    }

    #[tokio::test]
    async fn work_routes_accept_run_tools_workers_and_refuse_plain_workers() {
        use fabro_server::test_support::{
            TestAppStateBuilder, build_test_router, create_test_run_with_target,
        };
        use tower::ServiceExt;

        let tracker = FakeTracker::with_items(vec![item(1, 1)]);
        let state = TestAppStateBuilder::new()
            .work_tracker(Arc::clone(&tracker) as Arc<dyn TrackerProvider>)
            .build();
        let app = build_test_router(state.clone());
        put_project(&app).await;

        // A real run on denkhaus/fabro: the work-route scope binds run ->
        // target repo -> project (denkhaus/work#991), and grant pins the
        // body runId to the authenticated run.
        let run_id = create_test_run_with_target(&state, "denkhaus/fabro", "denkhaus").await;
        let plain = state.test_issue_worker_token(&run_id);
        let refused = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(api("/projects/fabro/next"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .header("authorization", format!("Bearer {plain}"))
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "runId": run_id.to_string(),
                            "lane": "loop"
                        }))
                        .expect("fixture serializes"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(
            refused.status(),
            StatusCode::FORBIDDEN,
            "a plain run worker lacks the run-tools capability"
        );

        let tools = state.test_issue_worker_token_with_run_tools(&run_id);
        let granted = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(api("/projects/fabro/next"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .header("authorization", format!("Bearer {tools}"))
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "runId": run_id.to_string(),
                            "lane": "loop"
                        }))
                        .expect("fixture serializes"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        let status = granted.status();
        let granted_text = String::from_utf8(
            axum::body::to_bytes(granted.into_body(), usize::MAX)
                .await
                .expect("body")
                .to_vec(),
        )
        .expect("utf-8 body");
        assert_eq!(
            status,
            StatusCode::OK,
            "next grant through the bound worker: {granted_text}"
        );
        let body: Value = serde_json::from_str(&granted_text).expect("json");
        assert_eq!(body["outcome"], "granted");
    }

    #[tokio::test]
    async fn updating_an_item_edits_title_and_body_in_place() {
        let tracker = FakeTracker::with_items(vec![item(1, 1)]);
        let app = app_with_tracker(Arc::clone(&tracker)).await;

        let updated = json_request(
            app.clone(),
            Method::POST,
            "/projects/fabro/work-items/update",
            &json!({
                "itemKey": "denkhaus/work#1",
                "title": "Item 1 (corrected)",
                "body": "the corrected spec"
            }),
        )
        .await;
        assert_eq!(updated["itemKey"], "denkhaus/work#1");
        assert_eq!(updated["title"], "Item 1 (corrected)");
        assert_eq!(updated["body"], "the corrected spec");

        // A title-only edit leaves the body untouched.
        let retitled = json_request(
            app.clone(),
            Method::POST,
            "/projects/fabro/work-items/update",
            &json!({"itemKey": "denkhaus/work#1", "title": "Item 1 (retitled)"}),
        )
        .await;
        assert_eq!(retitled["title"], "Item 1 (retitled)");
        assert_eq!(retitled["body"], "the corrected spec");

        // An empty update is refused before it reaches the tracker.
        let request = Request::builder()
            .method(Method::POST)
            .uri(api("/projects/fabro/work-items/update"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::to_vec(&json!({"itemKey": "denkhaus/work#1"}))
                    .expect("work-item test step should succeed"),
            ))
            .expect("work-item test step should succeed");
        let response = app
            .oneshot(request)
            .await
            .expect("work-item test step should succeed");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn lane_moves_assign_and_clear_the_lane_label() {
        let tracker = FakeTracker::with_items(vec![item(1, 1)]);
        let app = app_with_tracker(Arc::clone(&tracker)).await;

        let moved = json_request(
            app.clone(),
            Method::POST,
            "/projects/fabro/work-items/lane",
            &json!({"itemKey": "denkhaus/work#1", "lane": "loop"}),
        )
        .await;
        assert_eq!(moved["itemKey"], "denkhaus/work#1");
        assert_eq!(
            moved["labels"],
            json!(["project:fabro", "lane:loop"]),
            "the lane label lands beside the project label"
        );

        // `null` clears the lane — the tranche model's unassign.
        let cleared = json_request(
            app.clone(),
            Method::POST,
            "/projects/fabro/work-items/lane",
            &json!({"itemKey": "denkhaus/work#1", "lane": null}),
        )
        .await;
        assert_eq!(
            cleared["labels"],
            json!(["project:fabro"]),
            "a null lane removes every lane label"
        );

        // An unknown lane is refused, naming the declared lanes, before
        // the tracker is touched.
        let request = Request::builder()
            .method(Method::POST)
            .uri(api("/projects/fabro/work-items/lane"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::to_vec(&json!({"itemKey": "denkhaus/work#1", "lane": "revisor"}))
                    .expect("work-item test step should succeed"),
            ))
            .expect("work-item test step should succeed");
        let response = app
            .oneshot(request)
            .await
            .expect("work-item test step should succeed");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn lane_moves_refuse_run_workers() {
        use fabro_server::test_support::{
            TestAppStateBuilder, build_test_router, create_test_run_with_target,
        };

        let tracker = FakeTracker::with_items(vec![item(1, 1)]);
        let state = TestAppStateBuilder::new()
            .work_tracker(Arc::clone(&tracker) as Arc<dyn TrackerProvider>)
            .build();
        let app = build_test_router(state.clone());
        put_project(&app).await;

        // Even the run-tools capability must not move lanes: lane
        // assignment is the user's tranche surface (work#1065), never an
        // agent-run capability.
        let run_id = create_test_run_with_target(&state, "denkhaus/fabro", "denkhaus").await;
        let tools = state.test_issue_worker_token_with_run_tools(&run_id);
        let refused = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(api("/projects/fabro/work-items/lane"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .header("authorization", format!("Bearer {tools}"))
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "itemKey": "denkhaus/work#1",
                            "lane": "loop"
                        }))
                        .expect("fixture serializes"),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn observation_links_refuse_run_workers_on_both_verbs() {
        use fabro_server::test_support::{
            TestAppStateBuilder, build_test_router, create_test_run_with_target,
        };

        let tracker = FakeTracker::with_items(vec![item(1, 1)]);
        let state = TestAppStateBuilder::new()
            .work_tracker(tracker as Arc<dyn TrackerProvider>)
            .build();
        let app = build_test_router(state.clone());
        put_project(&app).await;

        // The link surface is the user's backfill/bench surface (ADR-0038
        // stage 3): even the run-tools capability must not write links —
        // the run-end hook writes them internally server-side instead.
        let run_id = create_test_run_with_target(&state, "denkhaus/fabro", "denkhaus").await;
        let tools = state.test_issue_worker_token_with_run_tools(&run_id);
        for path in ["/work-items/link", "/work-items/unlink"] {
            let refused = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::POST)
                        .uri(api(&format!("/projects/fabro{path}")))
                        .header(header::CONTENT_TYPE, "application/json")
                        .header("authorization", format!("Bearer {tools}"))
                        .body(Body::from(
                            serde_json::to_vec(&json!({
                                "itemKey": "denkhaus/work#1",
                                "runId": run_id.to_string(),
                                "seq": 1
                            }))
                            .expect("fixture serializes"),
                        ))
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(
                refused.status(),
                StatusCode::FORBIDDEN,
                "{path} must refuse a worker principal before any store call"
            );
        }
    }

    #[tokio::test]
    async fn granting_a_named_item_answers_conflict_when_taken() {
        let tracker = FakeTracker::with_items(vec![item(1, 1), item(2, 2)]);
        let app = app_with_tracker(Arc::clone(&tracker)).await;

        // The goal-named fast path: exactly that item, no priority scan.
        let named = json_request(
            app.clone(),
            Method::POST,
            "/projects/fabro/claims",
            &json!({
                "itemKey": "denkhaus/work#2",
                "runId": RUN_A,
                "lane": "loop"
            }),
        )
        .await;
        assert_eq!(named["outcome"], "granted");
        assert_eq!(named["claim"]["itemKey"], "denkhaus/work#2");
        assert_eq!(named["item"]["itemKey"], "denkhaus/work#2");

        // A second run naming the SAME item gets a conflict, never a grant.
        let taken = json_request(
            app.clone(),
            Method::POST,
            "/projects/fabro/claims",
            &json!({
                "itemKey": "denkhaus/work#2",
                "runId": RUN_B,
                "lane": "loop"
            }),
        )
        .await;
        assert_eq!(taken["outcome"], "conflict");
    }

    #[tokio::test]
    async fn next_refuses_while_the_lane_awaits_a_merge() {
        let tracker = FakeTracker::with_items(vec![item(1, 1), item(2, 2)]);
        let app = app_with_tracker(Arc::clone(&tracker)).await;

        let granted = json_request(
            app.clone(),
            Method::POST,
            "/projects/fabro/next",
            &json!({"runId": RUN_A, "lane": "loop"}),
        )
        .await;
        let claim_id = granted["claim"]["claimId"].as_str().unwrap().to_string();

        // The run starts, then ends with an open pull request: the claim
        // awaits its merge.
        json_request(
            app.clone(),
            Method::POST,
            &format!("/claims/{claim_id}/transition"),
            &json!({"state": "active"}),
        )
        .await;
        json_request(
            app.clone(),
            Method::POST,
            &format!("/claims/{claim_id}/transition"),
            &json!({"state": "awaiting_merge", "prUrl": "https://github.com/o/r/pull/9"}),
        )
        .await;

        let blocked = json_request(
            app.clone(),
            Method::POST,
            "/projects/fabro/next",
            &json!({"runId": RUN_B, "lane": "loop"}),
        )
        .await;
        assert_eq!(blocked["outcome"], "blocked");
        assert_eq!(blocked["claim"]["claimId"], claim_id);
        assert_eq!(blocked["claim"]["state"], "awaiting_merge");

        // Once landed, the lane grants again.
        json_request(
            app.clone(),
            Method::POST,
            &format!("/claims/{claim_id}/transition"),
            &json!({"state": "landed"}),
        )
        .await;
        let granted_again = json_request(
            app,
            Method::POST,
            "/projects/fabro/next",
            &json!({"runId": RUN_B, "lane": "loop"}),
        )
        .await;
        assert_eq!(granted_again["outcome"], "granted");
    }
}

/// The closure-radius gate (ADR-0038): a close whose item has open work
/// items linked to the observations of its runs is refused with the
/// listing; `force` overrides and the response names what the override
/// rode over. The link surface is user-only and idempotent. Seeded
/// end-to-end: claim history (item -> runs), observation records
/// (runs -> observations), links (observations -> items), tracker state
/// (the open filter).
mod closure_radius {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode, header};
    use fabro_server::server::AppState;
    use fabro_server::test_support::{TestAppStateBuilder, build_test_router};
    use fabro_store::platform_records::{
        ObservationRecordedRecord, PlatformRecord, PlatformRecordStore,
    };
    use fabro_types::RunId;
    use fabro_work::{TrackerProvider, WorkItem, WorkItemDraft, WorkItemFilter, WorkProject};
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use crate::helpers::api;

    const CLOSING_RUN: &str = "01JNQVR7M0EJ5GKAT2SC4ERS1Z";

    /// An in-memory tracker: only what the gate reads — `get` (the open
    /// filter) and `close` (the mutation under test).
    struct RadiusTracker {
        items: Mutex<Vec<WorkItem>>,
    }

    impl RadiusTracker {
        fn with_items(items: Vec<WorkItem>) -> Arc<Self> {
            Arc::new(Self {
                items: Mutex::new(items),
            })
        }
    }

    fn item(number: u64, state: &str) -> WorkItem {
        WorkItem {
            item_key:   format!("denkhaus/work#{number}"),
            title:      format!("Item {number}"),
            body:       "the spec".to_string(),
            state:      state.to_string(),
            labels:     vec!["project:fabro".to_string()],
            priority:   2,
            url:        format!("https://github.com/denkhaus/work/issues/{number}"),
            updated_at: None,
        }
    }

    #[async_trait]
    impl TrackerProvider for RadiusTracker {
        async fn list(
            &self,
            _project: &WorkProject,
            filter: WorkItemFilter,
        ) -> anyhow::Result<Vec<WorkItem>> {
            Ok(self
                .items
                .lock()
                .expect("tracker lock")
                .iter()
                .filter(|item| filter.query.is_none() && item.state == "open")
                .cloned()
                .collect())
        }

        async fn get(
            &self,
            _project: &WorkProject,
            item_key: &str,
        ) -> anyhow::Result<Option<WorkItem>> {
            Ok(self
                .items
                .lock()
                .expect("tracker lock")
                .iter()
                .find(|item| item.item_key == item_key)
                .cloned())
        }

        async fn create(
            &self,
            _project: &WorkProject,
            _draft: WorkItemDraft,
        ) -> anyhow::Result<WorkItem> {
            anyhow::bail!("not needed by the radius fixtures")
        }

        async fn update(
            &self,
            _project: &WorkProject,
            _item_key: &str,
            _title: Option<&str>,
            _body: Option<&str>,
        ) -> anyhow::Result<WorkItem> {
            anyhow::bail!("not needed by the radius fixtures")
        }

        async fn close(
            &self,
            _project: &WorkProject,
            item_key: &str,
            _reason: &str,
        ) -> anyhow::Result<()> {
            let mut items = self.items.lock().expect("tracker lock");
            if let Some(entry) = items.iter_mut().find(|item| item.item_key == item_key) {
                entry.state = "closed".to_string();
            }
            Ok(())
        }

        async fn comment(
            &self,
            _project: &WorkProject,
            _item_key: &str,
            _body: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn project_claim(
            &self,
            _project: &WorkProject,
            _item_key: &str,
            _run_id: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn clear_claim(
            &self,
            _project: &WorkProject,
            _item_key: &str,
            _run_id: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }

    async fn app_with_state(
        tracker: Arc<RadiusTracker>,
    ) -> (axum::Router, Arc<AppState>, PlatformRecordStore) {
        let (store, artifacts) = fabro_server::test_support::test_store_bundle();
        let records = store.run_summary_store().platform_records();
        let state = TestAppStateBuilder::new()
            .work_tracker(tracker as Arc<dyn TrackerProvider>)
            .store_bundle(store, artifacts)
            .build();
        let app = build_test_router(Arc::clone(&state));
        let request = Request::builder()
            .method(Method::PUT)
            .uri(api("/projects/fabro"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::to_string(&json!({
                    "displayName": "Fabro",
                    "trackerProvider": "github",
                    "trackerRepo": "denkhaus/work",
                    "targetRepo": "denkhaus/fabro",
                    "projectLabel": "project:fabro",
                    "laneLabels": {"loop": "lane:loop"}
                }))
                .expect("project fixture should serialize"),
            ))
            .expect("project fixture should build");
        let response = app
            .clone()
            .oneshot(request)
            .await
            .expect("project put should run");
        assert_eq!(response.status(), StatusCode::OK);
        (app, state, records)
    }

    async fn json_call(
        app: &axum::Router,
        method: Method,
        path: &str,
        body: &Value,
    ) -> axum::http::Response<Body> {
        let request = Request::builder()
            .method(method)
            .uri(api(path))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::to_vec(body).expect("radius fixture should serialize"),
            ))
            .expect("radius request should build");
        app.clone()
            .oneshot(request)
            .await
            .expect("radius request should run")
    }

    async fn body_json(response: axum::http::Response<Body>) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("radius response body");
        serde_json::from_slice(&bytes).expect("radius response json")
    }

    /// One painpoint observation record for `run_id` at the next seq —
    /// the run side of the chain, seeded through the production store.
    async fn seed_observation(
        records: &PlatformRecordStore,
        run_id: &str,
        category_painpoint: bool,
    ) -> u64 {
        let run_id: RunId = run_id.parse().expect("fixture run id");
        let record = PlatformRecord::ObservationRecorded(ObservationRecordedRecord {
            category:  if category_painpoint {
                fabro_types::ObservationCategory::Painpoint
            } else {
                fabro_types::ObservationCategory::Note
            },
            text:      "the gate refused every push because a run PR stayed open".to_string(),
            execution: 1,
            firing:    1,
        });
        let stored = records
            .append(&run_id, &record, None)
            .await
            .expect("observation record seed");
        stored.seq
    }

    #[tokio::test]
    async fn a_close_over_an_open_radius_is_refused_with_the_listing_and_force_overrides() {
        // Item 1 closes; its run carries one observation; the
        // observation links to item 2 (open — blocks) and item 3
        // (closed — filtered out of the radius).
        let tracker =
            RadiusTracker::with_items(vec![item(1, "open"), item(2, "open"), item(3, "closed")]);
        let (app, _state, records) = app_with_state(Arc::clone(&tracker)).await;

        // The claim makes item 1 -> CLOSING_RUN a claims-history edge.
        let granted = body_json(
            json_call(
                &app,
                Method::POST,
                "/projects/fabro/claims",
                &json!({"itemKey": "denkhaus/work#1", "runId": CLOSING_RUN, "lane": "loop"}),
            )
            .await,
        )
        .await;
        assert_eq!(granted["outcome"], "granted", "the claim must land first");

        let seq = seed_observation(&records, CLOSING_RUN, true).await;

        // Link both radius items to the observation through the
        // user-only surface — link also proves idempotence here.
        let linked = body_json(
            json_call(
                &app,
                Method::POST,
                "/projects/fabro/work-items/link",
                &json!({"itemKey": "denkhaus/work#2", "runId": CLOSING_RUN, "seq": seq}),
            )
            .await,
        )
        .await;
        assert_eq!(linked["outcome"], "linked");
        let again = body_json(
            json_call(
                &app,
                Method::POST,
                "/projects/fabro/work-items/link",
                &json!({"itemKey": "denkhaus/work#2", "runId": CLOSING_RUN, "seq": seq}),
            )
            .await,
        )
        .await;
        assert_eq!(
            again["outcome"], "already_linked",
            "filing dedupes at the link — no second row, a named outcome"
        );
        body_json(
            json_call(
                &app,
                Method::POST,
                "/projects/fabro/work-items/link",
                &json!({"itemKey": "denkhaus/work#3", "runId": CLOSING_RUN, "seq": seq}),
            )
            .await,
        )
        .await;
        // A self-link (item 1's own observation linked to item 1) is a
        // self-loop, never a self-block.
        body_json(
            json_call(
                &app,
                Method::POST,
                "/projects/fabro/work-items/link",
                &json!({"itemKey": "denkhaus/work#1", "runId": CLOSING_RUN, "seq": seq}),
            )
            .await,
        )
        .await;

        // Without force: 409, the listing names the OPEN item (2), not
        // the closed one (3), not the item itself (1).
        let refused = json_call(
            &app,
            Method::POST,
            "/projects/fabro/work-items/close",
            &json!({"itemKey": "denkhaus/work#1", "reason": "landed as abc123"}),
        )
        .await;
        assert_eq!(
            refused.status(),
            StatusCode::CONFLICT,
            "an open closure radius refuses the close"
        );
        let refusal = body_json(refused).await;
        let detail = refusal["errors"][0]["detail"]
            .as_str()
            .expect("the refusal detail");
        assert!(
            detail.contains("denkhaus/work#2"),
            "the refusal lists the open radius item: {detail}"
        );
        assert!(
            !detail.contains("denkhaus/work#3"),
            "a CLOSED linked item is outside the radius: {detail}"
        );
        assert!(
            !detail.contains("denkhaus/work#1 ("),
            "the item itself never blocks itself: {detail}"
        );
        assert!(
            detail.contains("force: true"),
            "the refusal teaches the override: {detail}"
        );

        // With force: the close proceeds and the response names what the
        // override rode over — item 2, still only item 2.
        let forced = json_call(
            &app,
            Method::POST,
            "/projects/fabro/work-items/close",
            &json!({
                "itemKey": "denkhaus/work#1",
                "reason": "landed as abc123",
                "force": true
            }),
        )
        .await;
        assert_eq!(forced.status(), StatusCode::OK);
        let closed = body_json(forced).await;
        assert_eq!(closed["item"]["itemKey"], "denkhaus/work#1");
        assert_eq!(closed["item"]["state"], "closed");
        let overridden = closed["overriddenRadius"]
            .as_array()
            .expect("the override listing");
        assert_eq!(overridden.len(), 1, "exactly the open item: {closed}");
        assert_eq!(overridden[0]["itemKey"], "denkhaus/work#2");

        // The tracker really closed it.
        assert_eq!(
            tracker
                .get(&fake_project(), "denkhaus/work#1")
                .await
                .expect("tracker read")
                .expect("item 1")
                .state,
            "closed"
        );
    }

    #[tokio::test]
    async fn a_close_without_radius_links_proceeds_unforced() {
        let tracker = RadiusTracker::with_items(vec![item(5, "open")]);
        let (app, _state, records) = app_with_state(Arc::clone(&tracker)).await;
        let granted = body_json(
            json_call(
                &app,
                Method::POST,
                "/projects/fabro/claims",
                &json!({"itemKey": "denkhaus/work#5", "runId": CLOSING_RUN, "lane": "loop"}),
            )
            .await,
        )
        .await;
        assert_eq!(granted["outcome"], "granted");

        // An observation WITHOUT links (the stage-3 inert shape: no
        // writer exists yet) and even a link to an already-CLOSED item
        // leave the radius empty.
        let seq = seed_observation(&records, CLOSING_RUN, true).await;
        let mut closed_link_target = item(6, "closed");
        closed_link_target.item_key = "denkhaus/work#6".to_string();
        tracker
            .items
            .lock()
            .expect("tracker lock")
            .push(closed_link_target);
        body_json(
            json_call(
                &app,
                Method::POST,
                "/projects/fabro/work-items/link",
                &json!({"itemKey": "denkhaus/work#6", "runId": CLOSING_RUN, "seq": seq}),
            )
            .await,
        )
        .await;

        let response = json_call(
            &app,
            Method::POST,
            "/projects/fabro/work-items/close",
            &json!({"itemKey": "denkhaus/work#5", "reason": "landed as def456"}),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "an empty radius never blocks — the gate ships inert (ADR-0038 stage 3)"
        );
        let closed = body_json(response).await;
        assert_eq!(
            closed["overriddenRadius"]
                .as_array()
                .expect("the listing exists")
                .len(),
            0,
            "an unforced close lists nothing"
        );
    }

    #[tokio::test]
    async fn unlink_removes_the_link_and_reports_an_absent_pair() {
        let tracker = RadiusTracker::with_items(vec![item(1, "open"), item(2, "open")]);
        let (app, _state, records) = app_with_state(Arc::clone(&tracker)).await;
        let seq = seed_observation(&records, CLOSING_RUN, true).await;
        body_json(
            json_call(
                &app,
                Method::POST,
                "/projects/fabro/work-items/link",
                &json!({"itemKey": "denkhaus/work#2", "runId": CLOSING_RUN, "seq": seq}),
            )
            .await,
        )
        .await;

        let removed = body_json(
            json_call(
                &app,
                Method::POST,
                "/projects/fabro/work-items/unlink",
                &json!({"itemKey": "denkhaus/work#2", "runId": CLOSING_RUN, "seq": seq}),
            )
            .await,
        )
        .await;
        assert_eq!(removed["removed"], true);
        let absent = body_json(
            json_call(
                &app,
                Method::POST,
                "/projects/fabro/work-items/unlink",
                &json!({"itemKey": "denkhaus/work#2", "runId": CLOSING_RUN, "seq": seq}),
            )
            .await,
        )
        .await;
        assert_eq!(
            absent["removed"], false,
            "unlinking an absent pair reports false, never errors"
        );
    }

    #[tokio::test]
    async fn an_invalid_link_run_id_is_refused_as_a_bad_request() {
        let tracker = RadiusTracker::with_items(vec![item(1, "open")]);
        let (app, _state, _records) = app_with_state(tracker).await;
        let response = json_call(
            &app,
            Method::POST,
            "/projects/fabro/work-items/link",
            &json!({"itemKey": "denkhaus/work#1", "runId": "not-a-run-id", "seq": 1}),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    fn fake_project() -> WorkProject {
        WorkProject {
            project_id:               "fabro".to_string(),
            display_name:             "Fabro".to_string(),
            tracker_provider:         "github".to_string(),
            tracker_repo:             "denkhaus/work".to_string(),
            target_repo:              Some("denkhaus/fabro".to_string()),
            project_label:            "project:fabro".to_string(),
            lane_labels:              [("loop".to_string(), "lane:loop".to_string())]
                .into_iter()
                .collect(),
            claim_label_prefix:       "fabro:run:".to_string(),
            claim_comment_enabled:    false,
            backpressure_open_pr_cap: 0,
        }
    }
}
