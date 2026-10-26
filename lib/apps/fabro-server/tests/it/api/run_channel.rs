//! The per-run capability channel (ADR-0035): one Unix socket serving
//! the application router with the run's worker principal stamped
//! internally. This IT pins the scope matrix and the stamping
//! discipline over the wire the sandbox CLI will speak — bearer-less
//! HTTP over the socket.

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use fabro_server::server::AppState;
use fabro_server::test_support::{TestAppStateBuilder, build_test_router, spawn_test_run_channel};
use fabro_types::RunId;
use fabro_work::{TrackerProvider, WorkItem, WorkItemDraft, WorkItemFilter, WorkProject};
use serde_json::json;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tower::ServiceExt;

/// One scripted item per list call; create mirrors the draft into an
/// item key. Enough surface for the channel's scope matrix.
struct ChannelFakeTracker;

#[async_trait]
impl TrackerProvider for ChannelFakeTracker {
    async fn list(
        &self,
        _project: &WorkProject,
        _filter: WorkItemFilter,
    ) -> anyhow::Result<Vec<WorkItem>> {
        Ok(vec![WorkItem {
            item_key:   "denkhaus/work#1".to_string(),
            title:      "channel fixture".to_string(),
            body:       "reachable through the run channel".to_string(),
            state:      "open".to_string(),
            labels:     vec!["project:fabro".to_string()],
            priority:   2,
            url:        "https://github.com/denkhaus/work/issues/1".to_string(),
            updated_at: None,
        }])
    }

    async fn get(
        &self,
        _project: &WorkProject,
        _item_key: &str,
    ) -> anyhow::Result<Option<WorkItem>> {
        Ok(None)
    }

    async fn create(
        &self,
        _project: &WorkProject,
        draft: WorkItemDraft,
    ) -> anyhow::Result<WorkItem> {
        Ok(WorkItem {
            item_key:   "denkhaus/work#2".to_string(),
            title:      draft.title,
            body:       draft.body,
            state:      "open".to_string(),
            labels:     draft.labels,
            priority:   2,
            url:        "https://github.com/denkhaus/work/issues/2".to_string(),
            updated_at: None,
        })
    }

    async fn update(
        &self,
        _project: &WorkProject,
        _item_key: &str,
        _title: Option<&str>,
        _body: Option<&str>,
    ) -> anyhow::Result<WorkItem> {
        anyhow::bail!("update is outside the channel IT's scope")
    }

    async fn close(
        &self,
        _project: &WorkProject,
        _item_key: &str,
        _reason: &str,
    ) -> anyhow::Result<()> {
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

/// Minimal HTTP/1.1 over the channel socket — exactly the wire a
/// sandbox-side client speaks: no Authorization header anywhere.
/// Async (tokio) so the channel's server task progresses while the
/// test reads — a blocking std read would park a single-threaded test
/// runtime and deadlock against the accept loop.
async fn uds_request(
    socket: &std::path::Path,
    request_line: &str,
    extra_headers: &[(&str, &str)],
) -> (u16, String) {
    let mut stream = tokio::net::UnixStream::connect(socket)
        .await
        .expect("connect to the channel socket");
    let headers: String = extra_headers
        .iter()
        .fold(String::new(), |acc, (name, value)| {
            acc + name + ": " + value + "\r\n"
        });
    let raw = format!(
        "{request_line} HTTP/1.1\r\nHost: run-channel\r\n{headers}Connection: close\r\n\r\n"
    );
    stream
        .write_all(raw.as_bytes())
        .await
        .expect("write the request");
    let mut body = String::new();
    stream
        .read_to_string(&mut body)
        .await
        .expect("read the response");
    let status = body
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .expect("a status line");
    (status, body)
}

async fn seed_project(state: &Arc<AppState>) {
    // Project upsert is user-only: seed it through the test router's
    // user bearer BEFORE the channel exists — the channel must not be
    // able to do this itself (pinned below).
    let app = build_test_router(Arc::clone(state));
    let request = Request::builder()
        .method(Method::PUT)
        .uri("/api/v1/projects/fabro")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "displayName": "Fabro",
                "trackerProvider": "github",
                "trackerRepo": "denkhaus/work",
                "projectLabel": "project:fabro",
                "laneLabels": {"loop": "lane:loop"},
                "claimLabelPrefix": "fabro:run:",
                "claimCommentEnabled": false
            }))
            .expect("project fixture should serialize"),
        ))
        .expect("project request should build");
    let response = app
        .oneshot(request)
        .await
        .expect("project upsert should succeed");
    assert_eq!(response.status(), StatusCode::OK, "project seed via user");
}

#[tokio::test]
async fn the_channel_serves_the_worker_scope_and_refuses_user_surfaces() {
    let state = TestAppStateBuilder::new()
        .work_tracker(Arc::new(ChannelFakeTracker) as Arc<dyn TrackerProvider>)
        .build();
    seed_project(&state).await;

    let run_id: RunId = "01JRANCHANNER000000000000A".parse().expect("a run id");
    let channel = spawn_test_run_channel(Arc::clone(&state), &run_id);
    let socket = channel.socket_path().to_path_buf();

    // health is unauthenticated everywhere — the channel is no exception
    let (health, _) = uds_request(&socket, "GET /health", &[]).await;
    assert_eq!(health, 200, "health through the channel");

    // work reads pass on the worker principal (the run-tools surface)
    let (list_status, list_body) =
        uds_request(&socket, "GET /api/v1/projects/fabro/work-items", &[]).await;
    assert_eq!(
        list_status, 200,
        "work list through the channel: {list_body}"
    );
    assert!(
        list_body.contains("denkhaus/work#1"),
        "the scripted item is served: {list_body}"
    );

    // work creates pass too (the filings path)
    let (create_status, create_body) = uds_request(
        &socket,
        "POST /api/v1/projects/fabro/work-items",
        &[("Content-Type", "application/json")],
    )
    .await;
    let _ = create_body; // body sent below in the raw variant is empty; status alone pins the principal
    assert_eq!(
        create_status, 400,
        "empty create fails validation, not authorization — proving the route was reached on the worker principal"
    );

    // user-only surfaces REFUSE the worker principal (401: the worker
    // is not an authenticated user — auth_rejection, not a permission
    // miss; either way: never served)
    let (projects_status, _) = uds_request(&socket, "GET /api/v1/projects", &[]).await;
    assert!(
        projects_status == 401 || projects_status == 403,
        "project registry listing is user-only, got {projects_status}"
    );
    let (upsert_status, _) = uds_request(&socket, "PUT /api/v1/projects/fabro", &[(
        "Content-Type",
        "application/json",
    )])
    .await;
    assert!(
        upsert_status == 401 || upsert_status == 403,
        "project registry writes are user-only, got {upsert_status}"
    );

    channel.stop().await;

    // the socket is gone: a racing connect fails loudly instead of hanging
    let refused = std::os::unix::net::UnixStream::connect(&socket).is_err();
    assert!(refused, "the channel socket is removed after stop");
}

#[tokio::test]
async fn a_client_supplied_authorization_header_is_replaced_not_honored() {
    let state = TestAppStateBuilder::new()
        .work_tracker(Arc::new(ChannelFakeTracker) as Arc<dyn TrackerProvider>)
        .build();
    seed_project(&state).await;

    let run_id: RunId = "01JRANCHANNER000000000000B".parse().expect("a run id");
    let channel = spawn_test_run_channel(Arc::clone(&state), &run_id);
    let socket = channel.socket_path().to_path_buf();

    // A forged bearer must neither escalate nor break the request: the
    // stamping layer replaces it with the run's worker token.
    let (status, body) = uds_request(&socket, "GET /api/v1/projects/fabro/work-items", &[(
        "Authorization",
        "Bearer forged-by-the-sandbox",
    )])
    .await;
    assert_eq!(status, 200, "the forged bearer is replaced: {body}");

    channel.stop().await;
}

#[tokio::test]
async fn create_reaches_the_tracker_with_the_channel_body() {
    let state = TestAppStateBuilder::new()
        .work_tracker(Arc::new(ChannelFakeTracker) as Arc<dyn TrackerProvider>)
        .build();
    seed_project(&state).await;

    let run_id: RunId = "01JRANCHANNER000000000000C".parse().expect("a run id");
    let channel = spawn_test_run_channel(Arc::clone(&state), &run_id);
    let socket = channel.socket_path().to_path_buf();

    let payload = json!({"title": "filed by a deterministic script", "body": "through the channel", "labels": []});
    let payload = serde_json::to_string(&payload).expect("payload serializes");
    let mut stream = tokio::net::UnixStream::connect(&socket)
        .await
        .expect("connect");
    let raw = format!(
        "POST /api/v1/projects/fabro/work-items HTTP/1.1\r\nHost: run-channel\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    stream
        .write_all(raw.as_bytes())
        .await
        .expect("write create");
    let mut body = String::new();
    stream.read_to_string(&mut body).await.expect("read create");
    assert!(
        body.starts_with("HTTP/1.1 201"),
        "create through the channel is created: {body}"
    );
    assert!(
        body.contains("denkhaus/work#2"),
        "the scripted tracker's item key returns: {body}"
    );

    channel.stop().await;
}
