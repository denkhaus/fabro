use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use fabro_test::test_context;
use httpmock::MockServer;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

const REVISION_1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const REVISION_2: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const REVISION_3: &str = "3333333333333333333333333333333333333333333333333333333333333333";

fn environment_json(docker: &str, revision: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "toolchain",
        "revision": revision,
        "provider": "docker",
        "image": { "docker": docker, "dockerfile": null },
        "resources": { "cpu": null, "memory": null, "disk": null },
        "network": { "mode": "allow_all", "allow": [] },
        "lifecycle": {
            "preserve": false,
            "stop_on_terminal": true,
            "auto_stop": null
        },
        "labels": {},
        "env": {}
    })
}

fn conflict_body() -> serde_json::Value {
    serde_json::json!({
        "errors": [{ "status": "409", "title": "Conflict", "detail": "environment revision is stale: toolchain" }]
    })
}

#[test]
fn update_puts_with_if_match_and_prints_old_to_new_revision() {
    let context = test_context!();
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method("GET").path("/api/v1/environments/toolchain");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(environment_json("old-image:1", REVISION_1).to_string());
    });
    let put = server.mock(|when, then| {
        when.method("PUT")
            .path("/api/v1/environments/toolchain")
            .header("if-match", REVISION_1)
            .json_body(serde_json::json!({
                "provider": "docker",
                "image": { "docker": "new-image:2", "dockerfile": null },
                "resources": { "cpu": null, "memory": null, "disk": null },
                "network": { "mode": "allow_all", "allow": [] },
                "lifecycle": {
                    "preserve": false,
                    "stop_on_terminal": true,
                    "auto_stop": null
                },
                "labels": {},
                "env": {}
            }));
        then.status(200)
            .header("Content-Type", "application/json")
            .body(environment_json("new-image:2", REVISION_2).to_string());
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["env", "update", "toolchain", "--image", "new-image:2"])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("111111111111 -> 222222222222"),
        "update should print old -> new revision: {stderr}"
    );
    put.assert();
}

/// Stateful environment server that rejects the first `If-Match` with a 409
/// (simulating a concurrent writer) and accepts the retried replace.
struct RacingEnvironmentServer {
    base_url:    String,
    shutdown_tx: Option<oneshot::Sender<()>>,
    join_handle: Option<std::thread::JoinHandle<()>>,
}

impl Drop for RacingEnvironmentServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
    }
}

#[derive(Default)]
struct RacingState {
    raced: AtomicBool,
}

async fn racing_get(State(state): State<Arc<RacingState>>) -> Json<serde_json::Value> {
    let revision = if state.raced.load(Ordering::SeqCst) {
        REVISION_2
    } else {
        REVISION_1
    };
    Json(environment_json("old-image:1", revision))
}

async fn racing_put(
    State(state): State<Arc<RacingState>>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> axum::response::Response {
    let if_match = headers
        .get("if-match")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    match if_match {
        REVISION_1 => {
            state.raced.store(true, Ordering::SeqCst);
            (StatusCode::CONFLICT, Json(conflict_body())).into_response()
        }
        REVISION_2 => {
            let docker = body["image"]["docker"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            (StatusCode::OK, Json(environment_json(&docker, REVISION_3))).into_response()
        }
        other => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "errors": [{ "status": "409", "title": "Conflict", "detail": format!("environment revision is stale (got {other})") }]
            })),
        )
            .into_response(),
    }
}

#[expect(
    clippy::disallowed_types,
    reason = "binds a std TcpListener before converting it to tokio's, mirroring the               model_test concurrency harness"
)]
fn start_racing_environment_server() -> RacingEnvironmentServer {
    let std_listener = std::net::TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    std_listener
        .set_nonblocking(true)
        .expect("test listener should be nonblocking");
    let addr: SocketAddr = std_listener
        .local_addr()
        .expect("test server should have addr");
    let state = Arc::new(RacingState::default());
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    #[expect(
        clippy::disallowed_methods,
        reason = "Owns a dedicated OS thread that hosts a fresh Tokio runtime for the stateful \
                  test server, mirroring the model_test concurrency harness."
    )]
    let join_handle = std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().expect("test runtime should start");
        runtime.block_on(async move {
            let listener =
                TcpListener::from_std(std_listener).expect("test listener should convert");
            let app = Router::new()
                .route(
                    "/api/v1/environments/toolchain",
                    get(racing_get).put(racing_put),
                )
                .with_state(state);
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await;
        });
    });

    RacingEnvironmentServer {
        base_url:    format!("http://{addr}"),
        shutdown_tx: Some(shutdown_tx),
        join_handle: Some(join_handle),
    }
}

#[test]
fn update_retries_once_on_revision_race() {
    let context = test_context!();
    let server = start_racing_environment_server();
    context.set_http_target(&server.base_url);

    let output = context
        .command()
        .args(["env", "update", "toolchain", "--image", "new-image:2"])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("222222222222 -> 333333333333"),
        "retried update should print the raced revision transition: {stderr}"
    );
}

#[test]
fn update_fails_closed_when_revision_race_persists() {
    let context = test_context!();
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method("GET").path("/api/v1/environments/toolchain");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(environment_json("old-image:1", REVISION_1).to_string());
    });
    server.mock(|when, then| {
        when.method("PUT").path("/api/v1/environments/toolchain");
        then.status(409)
            .header("Content-Type", "application/json")
            .body(conflict_body().to_string());
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["env", "update", "toolchain", "--image", "new-image:2"])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("revision mismatch"),
        "persistent 409 should explain the revision mismatch: {stderr}"
    );
    assert!(
        stderr.contains("1111111111111111"),
        "persistent 409 should name the expected revision: {stderr}"
    );
}

#[test]
fn update_with_no_effective_change_skips_the_write() {
    let context = test_context!();
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method("GET").path("/api/v1/environments/toolchain");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(environment_json("same-image:1", REVISION_1).to_string());
    });
    let put = server.mock(|when, then| {
        when.method("PUT").path("/api/v1/environments/toolchain");
        then.status(200);
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["env", "update", "toolchain", "--image", "same-image:1"])
        .output()
        .expect("command should run");

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unchanged"),
        "no-op update should report unchanged: {stderr}"
    );
    put.assert_calls(0);
}

#[test]
fn update_applies_resource_and_lifecycle_flags() {
    let context = test_context!();
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method("GET").path("/api/v1/environments/toolchain");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(environment_json("image:1", REVISION_1).to_string());
    });
    let put = server.mock(|when, then| {
        when.method("PUT")
            .path("/api/v1/environments/toolchain")
            .header("if-match", REVISION_1)
            .json_body(serde_json::json!({
                "provider": "docker",
                "image": { "docker": "image:1", "dockerfile": null },
                "resources": { "cpu": 4, "memory": "4GiB", "disk": null },
                "network": { "mode": "allow_all", "allow": [] },
                "lifecycle": {
                    "preserve": true,
                    "stop_on_terminal": false,
                    "auto_stop": "30m"
                },
                "labels": {},
                "env": {}
            }));
        then.status(200)
            .header("Content-Type", "application/json")
            .body(environment_json("image:1", REVISION_2).to_string());
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args([
            "env",
            "update",
            "toolchain",
            "--cpu",
            "4",
            "--memory",
            "4GiB",
            "--preserve",
            "--no-stop-on-terminal",
            "--auto-stop",
            "30m",
        ])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    put.assert();
}
