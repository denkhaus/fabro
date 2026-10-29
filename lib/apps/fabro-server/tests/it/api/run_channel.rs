//! The per-run capability channel (ADR-0035): one Unix socket serving
//! the application router with the run's worker principal stamped
//! internally. This IT pins the scope matrix, the stamping
//! discipline, and the HANDSHAKE (work#1043: the owning run's token on
//! every request) over the wire the sandbox CLI will speak —
//! bearer-less HTTP over the socket.

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use fabro_server::server::AppState;
use fabro_server::test_support::{
    TestAppStateBuilder, build_test_router, spawn_for_run_for_test, spawn_test_run_channel,
};
use fabro_static::EnvVars;
use fabro_types::RunId;
use fabro_work::{TrackerProvider, WorkItem, WorkItemDraft, WorkItemFilter, WorkProject};
use serde_json::json;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tower::ServiceExt;

use crate::helpers::{checked_response, response_status};

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
/// sandbox-side client speaks: the handshake token header (work#1043)
/// and no Authorization anywhere. Async (tokio) so the channel's
/// server task progresses while the test reads — a blocking std read
/// would park a single-threaded test runtime and deadlock against the
/// accept loop.
async fn uds_request(
    socket: &std::path::Path,
    token: &str,
    request_line: &str,
    extra_headers: &[(&str, &str)],
    body: Option<&str>,
) -> (u16, String) {
    let mut stream = tokio::net::UnixStream::connect(socket)
        .await
        .expect("connect to the channel socket");
    let headers: String = extra_headers.iter().fold(
        format!("{}: {}\r\n", fabro_static::RUN_CHANNEL_TOKEN_HEADER, token),
        |acc, (name, value)| acc + name + ": " + value + "\r\n",
    );
    let raw = match body {
        Some(body) => format!(
            "{request_line} HTTP/1.1\r\nHost: run-channel\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ),
        None => format!(
            "{request_line} HTTP/1.1\r\nHost: run-channel\r\n{headers}Connection: close\r\n\r\n"
        ),
    };
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

/// POST a JSON body over the channel socket — the request shape a
/// deterministic step's nu script speaks (create/grant/close arms).
/// Delegates the transport to [`uds_request`]; only the framing is
/// JSON-specific.
async fn uds_json(
    socket: &std::path::Path,
    token: &str,
    path: &str,
    payload: &serde_json::Value,
) -> (u16, String) {
    let payload = serde_json::to_string(payload).expect("payload serializes");
    uds_request(
        socket,
        token,
        &format!("POST {path}"),
        &[("Content-Type", "application/json")],
        Some(&payload),
    )
    .await
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
                // The run->project scope binding resolves through it
                // (denkhaus/work#991): a run on denkhaus/fabro maps here.
                "targetRepo": "denkhaus/fabro",
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
    response_status(
        response,
        StatusCode::OK,
        "PUT /api/v1/projects/fabro (user seed)",
    )
    .await;
}

/// Seeds a claim held by a FOREIGN run through the user surface (the
/// worker must not be able to do this itself; the user path may grant
/// for any run) and returns its claim id.
async fn seed_foreign_claim(state: &Arc<AppState>, foreign_run: &RunId) -> String {
    let app = build_test_router(Arc::clone(state));
    let payload = json!({
        "itemKey": "denkhaus/work#7",
        "runId": foreign_run.to_string(),
        "lane": "loop"
    });
    let request = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/projects/fabro/claims")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&payload).expect("claim fixture serializes"),
        ))
        .expect("claim request should build");
    let response = app
        .oneshot(request)
        .await
        .expect("the foreign claim seed via user should succeed");
    let response = checked_response(
        response,
        StatusCode::OK,
        "POST /api/v1/projects/fabro/claims (user seed)",
    )
    .await;
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("claim response body");
    let claim: serde_json::Value = serde_json::from_slice(&bytes).expect("claim response json");
    claim["claim"]["claimId"]
        .as_str()
        .expect("the seeded claim id")
        .to_string()
}

/// Reads a claim's state back through the user surface — used to prove
/// a REFUSED worker operation mutated nothing.
async fn claim_state_via_user(state: &Arc<AppState>, claim_id: &str) -> String {
    let app = build_test_router(Arc::clone(state));
    let request = Request::builder()
        .method(Method::GET)
        .uri(format!("/api/v1/claims/{claim_id}"))
        .body(Body::empty())
        .expect("claim read request should build");
    let response = app
        .oneshot(request)
        .await
        .expect("the claim read via user should succeed");
    let response = checked_response(
        response,
        StatusCode::OK,
        "GET /api/v1/claims/{id} (user read)",
    )
    .await;
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("claim read body");
    let claim: serde_json::Value = serde_json::from_slice(&bytes).expect("claim read json");
    claim["state"]
        .as_str()
        .expect("the claim state")
        .to_string()
}

#[tokio::test]
async fn the_channel_serves_the_worker_scope_and_refuses_user_surfaces() {
    let state = TestAppStateBuilder::new()
        .work_tracker(Arc::new(ChannelFakeTracker) as Arc<dyn TrackerProvider>)
        .build();
    seed_project(&state).await;

    // A real run on denkhaus/fabro: the work-route scope resolves run ->
    // target repo -> project through the run's projection (denkhaus/work#991).
    let run_id = fabro_server::test_support::create_test_run_with_target(
        &state,
        "denkhaus/fabro",
        "denkhaus",
    )
    .await;
    let channel = spawn_test_run_channel(Arc::clone(&state), &run_id);
    let socket = channel.socket_path().to_path_buf();
    let token = channel.handshake_token().to_string();

    // health is unauthenticated everywhere — the channel is no exception
    let (health, _) = uds_request(&socket, &token, "GET /health", &[], None).await;
    assert_eq!(health, 200, "health through the channel");

    // work reads pass on the worker principal (the run-tools surface)
    let (list_status, list_body) = uds_request(
        &socket,
        &token,
        "GET /api/v1/projects/fabro/work-items",
        &[],
        None,
    )
    .await;
    assert_eq!(
        list_status, 200,
        "work list through the channel: {list_body}"
    );
    assert!(
        list_body.contains("denkhaus/work#1"),
        "the scripted item is served: {list_body}"
    );

    // work creates pass too (the filings path) — an empty body fails
    // VALIDATION (400), not authorization: reaching the validator
    // proves the route was reached on the worker principal; the
    // body-carrying variant lives in the create IT below.
    let (create_status, _) = uds_request(
        &socket,
        &token,
        "POST /api/v1/projects/fabro/work-items",
        &[("Content-Type", "application/json")],
        None,
    )
    .await;
    assert_eq!(
        create_status, 400,
        "empty create fails validation, not authorization — proving the route was reached on the worker principal"
    );

    // user-only surfaces REFUSE the worker principal with EXACTLY 401:
    // the worker token is not a user credential, so the auth layer
    // answers auth_rejection (unauthenticated) before any permission
    // check runs — never a 403, never served
    let (projects_status, _) =
        uds_request(&socket, &token, "GET /api/v1/projects", &[], None).await;
    assert_eq!(
        projects_status, 401,
        "project registry listing is user-only (auth_rejection)"
    );
    // The sandbox inventory is user-only too (work#1043 finding 3):
    // pre-fix a worker read every run's sandbox ids and states through
    // RequiredRunManagementActor.
    let (sandbox_status, _) =
        uds_request(&socket, &token, "GET /api/v1/sandboxes", &[], None).await;
    assert_eq!(
        sandbox_status, 401,
        "the sandbox inventory refuses the worker principal (work#1043 finding 3)"
    );
    let (upsert_status, _) = uds_request(
        &socket,
        &token,
        "PUT /api/v1/projects/fabro",
        &[("Content-Type", "application/json")],
        None,
    )
    .await;
    assert_eq!(
        upsert_status, 401,
        "project registry writes are user-only (auth_rejection)"
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

    let run_id = fabro_server::test_support::create_test_run_with_target(
        &state,
        "denkhaus/fabro",
        "denkhaus",
    )
    .await;
    let channel = spawn_test_run_channel(Arc::clone(&state), &run_id);
    let socket = channel.socket_path().to_path_buf();
    let token = channel.handshake_token().to_string();

    // A forged bearer must neither escalate nor break the request: the
    // stamping layer replaces it with the run's worker token.
    let (status, body) = uds_request(
        &socket,
        &token,
        "GET /api/v1/projects/fabro/work-items",
        &[("Authorization", "Bearer forged-by-the-sandbox")],
        None,
    )
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

    let run_id = fabro_server::test_support::create_test_run_with_target(
        &state,
        "denkhaus/fabro",
        "denkhaus",
    )
    .await;
    let channel = spawn_test_run_channel(Arc::clone(&state), &run_id);
    let socket = channel.socket_path().to_path_buf();
    let token = channel.handshake_token().to_string();

    let payload = json!({"title": "filed by a deterministic script", "body": "through the channel", "labels": []});
    let (status, body) = uds_json(
        &socket,
        &token,
        "/api/v1/projects/fabro/work-items",
        &payload,
    )
    .await;
    assert_eq!(status, 201, "create through the channel is created: {body}");
    assert!(
        body.contains("denkhaus/work#2"),
        "the scripted tracker's item key returns: {body}"
    );

    channel.stop().await;
}

/// The terminal drain (denkhaus/work#1003): taking the registration
/// stops the channel — socket gone — and the SECOND drain finds
/// nothing, so a racing terminal fold and stop() cannot double-stop.
#[tokio::test]
async fn the_terminal_drain_stops_the_socket_and_a_redrain_is_a_no_op() {
    use std::time::Duration;

    let state = TestAppStateBuilder::new()
        .work_tracker(Arc::new(ChannelFakeTracker) as Arc<dyn TrackerProvider>)
        .build();
    seed_project(&state).await;
    let run_id = fabro_server::test_support::create_test_run_with_target(
        &state,
        "denkhaus/fabro",
        "denkhaus",
    )
    .await;
    let channel = spawn_test_run_channel(Arc::clone(&state), &run_id);
    let socket = channel.socket_path().to_path_buf();
    let token = channel.handshake_token().to_string();

    // alive before the drain
    let (health, _) = uds_request(&socket, &token, "GET /health", &[], None).await;
    assert_eq!(health, 200, "the channel serves before the drain");

    // The drain runs the removal in a spawned task (the production
    // shape); poll for the socket FILE to disappear — a connect probe
    // would pass without the removal too (no listener -> refused even
    // on a stale file; mutation-proven blind, then fixed to exists()).
    let first = fabro_server::test_support::drain_test_run_channel(&state, &run_id);
    assert!(first, "the first drain takes the registration");
    let gone = tokio::time::timeout(Duration::from_secs(5), async {
        while socket.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(gone.is_ok(), "the drained channel's socket file is removed");
    let refused = std::os::unix::net::UnixStream::connect(&socket).is_err();
    assert!(refused, "a racing connect after the removal fails loudly");

    // A second drain (the racing terminal fold) wins nothing.
    let second = fabro_server::test_support::drain_test_run_channel(&state, &run_id);
    assert!(
        !second,
        "the second drain finds nothing — double stop is a no-op"
    );

    // And the handle's stop after the drain is the same no-op (it
    // proxies through the registry).
    channel.stop().await;
}

/// The re-grilled crash path (2026-10-26, review disproved the
/// original premise): a leftover file from a dead process makes bind
/// fail EADDRINUSE — spawn removes the STALE file once and retries.
#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "the IT plants the stale socket file — sync fs::write IS the point"
)]
async fn a_stale_socket_file_from_a_crash_is_self_healed() {
    let state = TestAppStateBuilder::new()
        .work_tracker(Arc::new(ChannelFakeTracker) as Arc<dyn TrackerProvider>)
        .build();
    seed_project(&state).await;
    let run_id = fabro_server::test_support::create_test_run_with_target(
        &state,
        "denkhaus/fabro",
        "denkhaus",
    )
    .await;

    let dir = std::env::temp_dir().join(format!("fabro-stale-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let socket = dir.join("channel.sock");
    std::fs::write(&socket, b"junk left by a crashed process").expect("stale file");

    let channel = fabro_server::test_support::spawn_run_channel_on(
        Arc::clone(&state),
        &run_id,
        socket.clone(),
    );
    let token = channel.handshake_token().to_string();
    let (health, _) = uds_request(&socket, &token, "GET /health", &[], None).await;
    assert_eq!(
        health, 200,
        "the stale file was replaced by a serving socket"
    );
    channel.stop().await;
    std::fs::remove_dir_all(&dir).ok();
}

/// A LIVE listener on the path keeps the refusal — spawn never unlinks
/// a serving socket (the retry probes with a connect first).
#[tokio::test]
async fn a_live_listener_keeps_the_refusal() {
    use tokio::net::UnixListener as TokioUnixListener;

    let state = TestAppStateBuilder::new()
        .work_tracker(Arc::new(ChannelFakeTracker) as Arc<dyn TrackerProvider>)
        .build();
    seed_project(&state).await;
    let run_id = fabro_server::test_support::create_test_run_with_target(
        &state,
        "denkhaus/fabro",
        "denkhaus",
    )
    .await;

    let dir = std::env::temp_dir().join(format!("fabro-live-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let socket = dir.join("channel.sock");
    let live_listener = TokioUnixListener::bind(&socket).expect("the live listener binds");

    let error =
        fabro_server::test_support::try_spawn_run_channel_on(&state, &run_id, socket.clone())
            .err()
            .expect("a serving socket is never taken over");
    assert!(
        error.to_string().contains("Address already in use"),
        "the original bind error surfaces: {error:#}"
    );
    // The chain-member contract (denkhaus/work#992): a spawn error
    // renders its cause through the chain (`{:#}`), never as a
    // flattened debug string. Pinned here on the one spawn error a
    // test can actually produce — the token-signing arm needs a broken
    // key that AppState construction already refuses (the item's
    // design names this deviation).
    let rendered = format!("{error:#}");
    assert!(
        rendered.contains("Address already in use"),
        "the chain rendering walks to the underlying cause: {rendered}"
    );
    assert!(socket.exists(), "the live listener's file stays untouched");
    drop(live_listener);
    std::fs::remove_dir_all(&dir).ok();
}

/// The claim-binding core over the wire (denkhaus/work#991): the three
/// refusals the cutover review demanded — a foreign project, a foreign
/// body runId on grant, another run's claim — each answered with 403
/// and the scope in the message, never served.
#[tokio::test]
async fn beyond_the_runs_scope_is_refused_with_the_scope_in_the_message() {
    let state = TestAppStateBuilder::new()
        .work_tracker(Arc::new(ChannelFakeTracker) as Arc<dyn TrackerProvider>)
        .build();
    seed_project(&state).await;
    let run_id = fabro_server::test_support::create_test_run_with_target(
        &state,
        "denkhaus/fabro",
        "denkhaus",
    )
    .await;

    // A claim held by a FOREIGN run — seeded through the user surface,
    // as another run's live claim would be.
    let foreign_run: RunId = "01JRANCHANNER000000000000F".parse().expect("a run id");
    let foreign_claim_id = seed_foreign_claim(&state, &foreign_run).await;

    let channel = spawn_test_run_channel(Arc::clone(&state), &run_id);
    let socket = channel.socket_path().to_path_buf();
    let token = channel.handshake_token().to_string();

    // (a) a foreign project — reads included, not just writes
    let (status, body) = uds_request(
        &socket,
        &token,
        "GET /api/v1/projects/other/work-items",
        &[],
        None,
    )
    .await;
    assert_eq!(
        status, 403,
        "a foreign project read is beyond the run's scope: {body}"
    );
    assert!(
        body.contains("beyond its scope"),
        "the refusal names the run's scope: {body}"
    );

    // (b) a foreign body runId on grant — refused loudly, never stamped
    let (status, body) = uds_json(
        &socket,
        &token,
        "/api/v1/projects/fabro/claims",
        &json!({
            "itemKey": "denkhaus/work#1",
            "runId": foreign_run.to_string(),
            "lane": "loop"
        }),
    )
    .await;
    assert_eq!(
        status, 403,
        "grant with a foreign body runId is refused: {body}"
    );
    assert!(
        body.contains("pinned to the authenticated run"),
        "the refusal names the pin: {body}"
    );

    // (c) another run's claim — release refused before any mutation
    let (status, body) = uds_json(
        &socket,
        &token,
        &format!("/api/v1/claims/{foreign_claim_id}/release"),
        &json!({"reason": "should never land"}),
    )
    .await;
    assert_eq!(
        status, 403,
        "releasing a foreign run's claim is refused: {body}"
    );
    assert!(
        body.contains("held by another run") && !body.contains("01JRANCH"),
        "the refusal names the condition only — no run ids (work#1043 finding 4): {body}"
    );
    let state_after = claim_state_via_user(&state, &foreign_claim_id).await;
    assert_ne!(
        state_after, "released",
        "the refused release mutated nothing, the claim is still {state_after}"
    );

    channel.stop().await;
}

/// The handshake end to end over the real socket (work#1043 finding
/// 1): a client without the OWNING run's token never reaches a route —
/// a sibling run on the host provider can open the socket but cannot
/// present the token, so the principal-confusion case dies at the
/// guard. The unit tests pin the layer's shapes; this pins the wire.
#[tokio::test]
async fn a_client_without_the_owning_handshake_token_is_refused() {
    let state = TestAppStateBuilder::new()
        .work_tracker(Arc::new(ChannelFakeTracker) as Arc<dyn TrackerProvider>)
        .build();
    seed_project(&state).await;
    let run_id = fabro_server::test_support::create_test_run_with_target(
        &state,
        "denkhaus/fabro",
        "denkhaus",
    )
    .await;
    let channel = spawn_test_run_channel(Arc::clone(&state), &run_id);
    let socket = channel.socket_path().to_path_buf();

    // No token at all: refused before any route — the work list would
    // answer 200 with the token (proven by the scope-matrix test).
    let mut stream = tokio::net::UnixStream::connect(&socket)
        .await
        .expect("connect to the channel socket");
    stream
        .write_all(
            b"GET /api/v1/projects/fabro/work-items HTTP/1.1\r\nHost: run-channel\r\nConnection: close\r\n\r\n",
        )
        .await
        .expect("write the tokenless request");
    let mut bare = String::new();
    stream.read_to_string(&mut bare).await.expect("read");
    assert!(
        bare.starts_with("HTTP/1.1 403"),
        "no token, no channel: {bare}"
    );
    assert!(
        bare.contains("handshake token"),
        "the refusal names the condition only: {bare}"
    );
    assert!(
        !bare.contains(&run_id.to_string()),
        "the refusal leaks no run id: {bare}"
    );

    // A SIBLING's wrong token refuses identically.
    let (wrong, _) = uds_request(&socket, "not-the-owning-token", "GET /health", &[], None).await;
    assert_eq!(wrong, 403, "a sibling's token opens nothing");

    channel.stop().await;
}

/// The production spawn path (denkhaus/work#989): the run's channel
/// lands at `<root>/<run>/channel.sock` and serves, the operator kill
/// switch refuses the spawn, `FABRO_RUN_CHANNEL_DIR` relocates the
/// root (the compose pin), and a respawn of the same run replaces the
/// registration cleanly — the stale stop is awaited before the re-bind,
/// so neither EADDRINUSE nor a post-bind removal race survives.
#[tokio::test]
async fn the_production_spawn_layouts_serves_switches_and_respawns() {
    let state = TestAppStateBuilder::new().build();
    let run_id = RunId::new();
    let (socket, token) = spawn_for_run_for_test(&state, run_id)
        .await
        .expect("the default settings spawn a channel");
    assert!(
        socket
            .to_string_lossy()
            .ends_with(&format!("run-channels/{run_id}/channel.sock")),
        "one subdir per run under the channel root: {}",
        socket.display()
    );
    assert!(socket.exists(), "the socket file is bound");
    let (health, _) = uds_request(&socket, &token, "GET /health", &[], None).await;
    assert_eq!(health, 200, "the production-spawned channel serves health");

    let (again, token_again) = spawn_for_run_for_test(&state, run_id)
        .await
        .expect("a respawn succeeds");
    assert_eq!(again, socket, "the layout is deterministic");
    assert_ne!(
        token_again, token,
        "every spawn mints a fresh handshake token"
    );
    let (health_again, _) = uds_request(&socket, &token_again, "GET /health", &[], None).await;
    assert_eq!(
        health_again, 200,
        "the respawned channel serves — the stale stop completed before the re-bind"
    );

    // Kill switch (work#989 grill R2.4): the spawn is refused entirely
    // — None means no channel path enters the run's spec, so no
    // channel serves and nothing is handed to the sandbox.
    let switched_off = fabro_config::ServerSettingsBuilder::from_toml(
        r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[server.run_channel]
enabled = false
"#,
    )
    .expect("the kill-switch settings resolve");
    let off = TestAppStateBuilder::new()
        .runtime_settings(switched_off, fabro_config::RunLayer::default())
        .build();
    let refused = spawn_for_run_for_test(&off, RunId::new()).await;
    assert!(
        refused.is_none(),
        "the kill switch refuses the spawn entirely"
    );

    // Env override (compose pin): the root moves wholesale, the
    // per-run subdir and file name stay.
    let root = std::env::temp_dir().join(format!(
        "fabro-run-channel-root-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let root_for_env = root.to_string_lossy().into_owned();
    let relocated = TestAppStateBuilder::new()
        .env_lookup(move |name| {
            (name == EnvVars::FABRO_RUN_CHANNEL_DIR).then(|| root_for_env.clone())
        })
        .build();
    let relocated_run = RunId::new();
    let (socket2, _token2) = spawn_for_run_for_test(&relocated, relocated_run)
        .await
        .expect("the env override still spawns");
    assert!(
        socket2.starts_with(&root),
        "FABRO_RUN_CHANNEL_DIR relocates the root: {}",
        socket2.display()
    );
    assert!(
        socket2
            .to_string_lossy()
            .ends_with(&format!("{relocated_run}/channel.sock")),
        "the per-run layout survives relocation: {}",
        socket2.display()
    );
}
