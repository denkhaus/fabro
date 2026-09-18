// Shared fixtures and helpers for the per-domain server test modules.
// Domain files use `use super::*;` to reach these helpers and imports.

use std::collections::HashMap;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc as StdArc, Mutex as StdMutex};

use async_zip::base::read::mem::ZipFileReader;
use axum::body::Body;
use axum::http::{Method, Request, header};
use chrono::{Duration as ChronoDuration, Utc};
use fabro_automation::AutomationId;
use fabro_config::bind::Bind;
use fabro_config::{
    EnvironmentLayer, LlmLayer, MergeMap, RunLayer, ServerSettingsBuilder, WorkflowSettingsBuilder,
};
use fabro_interview::{
    AnswerValue, ControlInterviewer, Interviewer, Question, WorkerControlDeliveryFrame,
    WorkerControlEnvelope, WorkerControlMessage,
};
use fabro_llm::lithos_catalog::Catalog;
use fabro_types::settings::ServerAuthMethod;
use fabro_types::settings::run::ApprovalMode;
use fabro_types::{
    AgentBackend, AttrValue, AuthMethod, BlobHash, CommandTermination, ContextWindowBreakdownItem,
    ContextWindowCategory, ContextWindowCountMethod, ContextWindowSnapshot, ContextWindowStaleness,
    ContextWindowWarning, FailureCategory, FailureDetail, GitRunTarget, Graph,
    InterviewQuestionRecord, ModelRef, Node, Outcome, ParallelBranchId, QuestionType, RunId,
    RunSpec, RunTarget, SandboxProviderKind, StageModelUsage, StageTiming, SuccessReason,
    SystemActorKind, WorkflowSettings, fixtures, test_support,
};
use fabro_util::check_report::CheckStatus;
use fabro_workflow::records::CheckpointExt;
use httpmock::Method::{GET, POST};
use httpmock::MockServer;
use lithos_llm::catalog::ModelId;
use lithos_llm::types::{
    Cost, CostSource, ReasoningEffort, ReasoningOutput, Request as LlmRequest, Speed, TokenCounts,
};
use pebble_coding_agent::events::{CodingAgentEvent, CodingEvent, Usage};
use serde_json::json;
use tokio::sync::Notify;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::Message as WebSocketMessage;
use tower::ServiceExt;
use tracing::field::{Field, Visit};
use tracing::{Event as TracingEvent, Subscriber, subscriber};
use tracing_subscriber::layer::Context as SubscriberContext;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{Layer, Registry};
use ulid::Ulid;

use super::*;
use crate::automation_materializer::AutomationRunMaterializeInput;
use crate::github_webhooks::compute_signature;
use crate::jwt_auth::{AuthMode, ConfiguredAuth};
use crate::test_support::*;
use crate::worker_control::{
    LocalWorkerControlBus, WorkerControlBus, WorkerControlCursor, WorkerControlReceiver,
};
use crate::worker_runtime::{
    LocalWorkerRuntime, StartedWorker, WorkerExit, WorkerLaunchSpec, WorkerRef, WorkerRuntime,
};

const MINIMAL_DOT: &str = r#"digraph Test {
    graph [goal="Test"]
    start [shape=Mdiamond]
    exit  [shape=Msquare]
    start -> exit
}"#;

const TEST_WEBHOOK_SECRET: &str = "webhook-secret";

const TEST_DEV_TOKEN: &str =
    "fabro_dev_abababababababababababababababababababababababababababababababab";

const TEST_SESSION_SECRET: &str = "server-test-session-key-0123456789";

const TEST_JWT_ISSUER: &str = "https://fabro.example";

const WRONG_DEV_TOKEN: &str =
    "fabro_dev_cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";

fn manifest_run_defaults_from_toml(source: &str) -> fabro_config::RunLayer {
    let mut document: toml::Table = source.parse().expect("run defaults should parse");
    document
        .remove("run")
        .map(toml::Value::try_into::<fabro_config::RunLayer>)
        .transpose()
        .expect("run defaults should parse")
        .unwrap_or_default()
}

fn test_environment_store(
    default_provider: Option<SandboxProviderKind>,
    local_enabled: bool,
) -> (tempfile::TempDir, EnvironmentStore) {
    let temp = tempfile::tempdir().expect("environment store tempdir should be created");
    let db_path = temp.path().join("fabro.sqlite3");
    let pool = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("environment store setup runtime should build");
        runtime.block_on(async move {
            let database = fabro_db::Database::connect(db_path)
                .await
                .expect("test environment database should connect");
            database
                .migrate()
                .await
                .expect("test environment database should migrate");
            if let Some(provider) = default_provider {
                fabro_environment::seed_default_environment(database.pool(), provider)
                    .await
                    .expect("test default environment should seed");
            }
            database.clone_pool()
        })
    })
    .join()
    .expect("environment store setup thread should not panic");
    let store = load_store_blocking("environment store", move || async move {
        EnvironmentStore::load(pool, local_enabled)
            .await
            .map_err(anyhow::Error::new)
    })
    .expect("test environment store should load");
    (temp, store)
}

fn test_mcp_server_store() -> (tempfile::TempDir, McpServerStore) {
    let temp = tempfile::tempdir().expect("MCP server store tempdir should be created");
    let db_path = temp.path().join("fabro.sqlite3");
    let pool = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("MCP server store setup runtime should build");
        runtime.block_on(async move {
            let database = fabro_db::Database::connect(db_path)
                .await
                .expect("test MCP server database should connect");
            database
                .migrate()
                .await
                .expect("test MCP server database should migrate");
            database.clone_pool()
        })
    })
    .join()
    .expect("MCP server store setup thread should not panic");
    let mcps_dir = temp.path().join("mcps");
    let store = load_store_blocking("MCP server store", move || async move {
        McpServerStore::open(pool, mcps_dir)
            .await
            .map_err(anyhow::Error::new)
    })
    .expect("test MCP server store should load");
    (temp, store)
}

fn server_settings_from_toml(source: &str) -> ServerSettings {
    ServerSettingsBuilder::from_toml(source).expect("server settings should resolve")
}

fn resolved_runtime_settings_from_toml(source: &str) -> ResolvedAppStateSettings {
    resolved_runtime_settings_for_tests(
        server_settings_from_toml(source),
        manifest_run_defaults_from_toml(source),
        LlmLayer::default(),
    )
}

fn test_app_with() -> Router {
    let state = test_app_state();
    crate::test_support::build_test_router_with_options(state, RouterOptions {
        static_asset_root: Some(spa_fixture_root()),
        ..RouterOptions::default()
    })
}

fn spa_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/spa")
}

fn state_test_catalog() -> Arc<Catalog> {
    Arc::new(fabro_llm::test_support::test_catalog())
}

fn test_app_with_scheduler(state: Arc<AppState>) -> Router {
    spawn_scheduler(Arc::clone(&state));
    crate::test_support::build_test_router(state)
}

fn test_app_state_with_isolated_storage() -> Arc<AppState> {
    let storage_dir = std::env::temp_dir().join(format!("fabro-server-test-{}", Ulid::new()));
    std::fs::create_dir_all(&storage_dir).expect("test storage dir should be creatable");
    let source = format!(
        r#"
_version = 1

[server.storage]
root = "{}"

[server.auth]
methods = ["dev-token"]
"#,
        storage_dir.display()
    );

    test_app_state_with_options(
        server_settings_from_toml(&source),
        manifest_run_defaults_from_toml(&source),
        5,
    )
}

async fn body_json(body: Body) -> serde_json::Value {
    let bytes = to_bytes(body, usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn run_json_id(run: &serde_json::Value) -> Option<&str> {
    run["id"].as_str().or_else(|| run["run_id"].as_str())
}

fn run_json_status(run: &serde_json::Value) -> &serde_json::Value {
    &run["lifecycle"]["status"]
}

fn run_json_pending_control(run: &serde_json::Value) -> &serde_json::Value {
    &run["lifecycle"]["pending_control"]
}

fn run_json_archived(run: &serde_json::Value) -> bool {
    run["lifecycle"]["archived"].as_bool().unwrap_or(false)
}

async fn mock_daytona_auth_probe(server: &MockServer) -> httpmock::Mock<'_> {
    server
        .mock_async(|when, then| {
            when.method(GET)
                .path("/sandbox/paginated")
                .query_param("page", "1")
                .query_param("limit", "1");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(json!({
                    "items": [],
                    "total": 0,
                    "page": 1,
                    "totalPages": 0
                }));
        })
        .await
}

async fn mock_daytona_current_key<'a>(
    server: &'a MockServer,
    permissions: Vec<&'static str>,
) -> httpmock::Mock<'a> {
    server
        .mock_async(move |when, then| {
            when.method(GET)
                .path("/api-keys/current")
                .header("authorization", "Bearer dtn_test");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(json!({
                    "name": "delete-only",
                    "value": "dtn_****",
                    "createdAt": "2026-05-01T00:00:00Z",
                    "permissions": permissions,
                    "lastUsedAt": null,
                    "expiresAt": null,
                    "userId": "user_123"
                }));
        })
        .await
}

fn openai_oauth_credential() -> fabro_auth::OAuthCredential {
    fabro_auth::OAuthCredential {
        tokens:     fabro_auth::OAuthTokens {
            access_token:  "access".to_string(),
            refresh_token: Some("refresh".to_string()),
            expires_at:    Utc::now() + ChronoDuration::hours(1),
        },
        config:     fabro_auth::OAuthConfig {
            auth_url:     "https://auth.openai.com".to_string(),
            token_url:    "https://auth.openai.com/oauth/token".to_string(),
            client_id:    "client".to_string(),
            scopes:       vec!["openid".to_string()],
            redirect_uri: Some("https://auth.openai.com/deviceauth/callback".to_string()),
            use_pkce:     true,
        },
        account_id: Some("acct_123".to_string()),
    }
}

fn openai_oauth_credential_json() -> String {
    serde_json::to_string(&openai_oauth_credential()).unwrap()
}

fn openai_responses_payload(text: &str) -> serde_json::Value {
    json!({
        "id": "resp_1",
        "model": "gpt-5.4",
        "output": [
            {
                "type": "message",
                "role": "assistant",
                "content": [
                    {
                        "type": "output_text",
                        "text": text
                    }
                ]
            }
        ],
        "status": "completed",
        "usage": {
            "input_tokens": 10,
            "output_tokens": 20
        }
    })
}

/// An operator-defined OpenAI-compatible provider `acme` offering one model,
/// `acme-large`, with `credential` (`env:NAME` or `vault:NAME`).
/// An operator-defined provider. Its API key is `ACME_API_KEY`, the name
/// lithos derives from the provider id, whether it lives in the vault or the
/// environment.
fn acme_overlay(base_url: &str) -> String {
    format!(
        r#"
[providers.acme]
display_name = "Acme"
adapter = "openai-compatible"
codec = "openai-chat"
base_url = {base_url}
auth = {{ type = "bearer" }}
priority = 120
default_model = "acme-large"

[providers.acme.metadata.agent]
profile = "openai"

[providers.acme.models."acme-large"]
display_name = "Acme Large"
api_model = "acme-large"
limits = {{ context_tokens = 128000, max_output_tokens = 8192 }}
capabilities = {{ text = true, tools = true }}
probe = true

"#,
        base_url = toml::Value::String(base_url.to_string()),
    )
}

macro_rules! assert_status {
    ($response:expr, $expected:expr) => {
        fabro_test::assert_axum_status($response, $expected, concat!(file!(), ":", line!()))
    };
}

macro_rules! checked_response {
    ($response:expr, $expected:expr) => {
        fabro_test::expect_axum_status($response, $expected, concat!(file!(), ":", line!()))
    };
}

#[derive(Clone, Debug)]
struct CapturedTracingEvent {
    fields: Vec<(String, String)>,
}

#[derive(Default)]
struct CaptureVisitor {
    fields: Vec<(String, String)>,
}

impl Visit for CaptureVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.fields
            .push((field.name().to_string(), format!("{value:?}")));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields
            .push((field.name().to_string(), value.to_string()));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields
            .push((field.name().to_string(), value.to_string()));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields
            .push((field.name().to_string(), value.to_string()));
    }
}

struct ServerLogCaptureLayer {
    events: StdArc<StdMutex<Vec<CapturedTracingEvent>>>,
}

impl<S: Subscriber> Layer<S> for ServerLogCaptureLayer {
    fn on_event(&self, event: &TracingEvent<'_>, _ctx: SubscriberContext<'_, S>) {
        if !event
            .metadata()
            .target()
            .starts_with("fabro_server::server")
        {
            return;
        }
        let mut visitor = CaptureVisitor::default();
        event.record(&mut visitor);
        if visitor
            .fields
            .iter()
            .any(|(name, value)| name == "message" && value == "HTTP response")
        {
            self.events
                .lock()
                .expect("captured log events lock poisoned")
                .push(CapturedTracingEvent {
                    fields: visitor.fields,
                });
        }
    }
}

fn capture_server_logs() -> (
    tracing::dispatcher::DefaultGuard,
    StdArc<StdMutex<Vec<CapturedTracingEvent>>>,
) {
    let events = StdArc::new(StdMutex::new(Vec::new()));
    let subscriber = Registry::default().with(ServerLogCaptureLayer {
        events: StdArc::clone(&events),
    });
    let guard = subscriber::set_default(subscriber);
    (guard, events)
}

fn captured_field<'a>(event: &'a CapturedTracingEvent, name: &str) -> Option<&'a str> {
    event
        .fields
        .iter()
        .find_map(|(field_name, value)| (field_name == name).then_some(value.as_str()))
}

fn assert_log_field(event: &CapturedTracingEvent, name: &str, expected: &str) {
    let actual = captured_field(event, name)
        .unwrap_or_else(|| panic!("expected log field {name}; fields were {:?}", event.fields));
    let debug_expected = format!("{expected:?}");
    assert!(
        actual == expected || actual == debug_expected,
        "expected field {name} to be {expected:?}, got {actual:?}; fields were {:?}",
        event.fields
    );
}

fn assert_log_field_absent(event: &CapturedTracingEvent, name: &str) {
    assert!(
        captured_field(event, name).is_none(),
        "expected log field {name} to be absent; fields were {:?}",
        event.fields
    );
}

macro_rules! response_json {
    ($response:expr, $expected:expr) => {
        fabro_test::expect_axum_json($response, $expected, concat!(file!(), ":", line!()))
    };
}

macro_rules! response_bytes {
    ($response:expr, $expected:expr) => {
        fabro_test::expect_axum_bytes($response, $expected, concat!(file!(), ":", line!()))
    };
}

fn api(path: &str) -> String {
    format!("/api/v1{path}")
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "Test helper mirrors the public build_router convenience API."
)]
fn webhook_test_app(auth_mode: AuthMode) -> Router {
    let state = TestAppStateBuilder::new()
        .env_lookup(|_| None)
        .vault_entries([(WEBHOOK_SECRET_ENV, TEST_WEBHOOK_SECRET)])
        .build();
    build_router_with_options(state, &auth_mode, RouterOptions {
        web_enabled: false,
        ..RouterOptions::default()
    })
}

fn webhook_request(
    signature: Option<&str>,
    authorization: Option<&str>,
    body: &[u8],
) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(api("/webhooks/github"))
        .header("x-github-delivery", "delivery-1")
        .header("x-github-event", "pull_request");
    if let Some(sig) = signature {
        builder = builder.header("x-hub-signature-256", sig);
    }
    if let Some(value) = authorization {
        builder = builder.header(header::AUTHORIZATION, value);
    }
    builder.body(Body::from(body.to_vec())).unwrap()
}

fn dev_token_auth_mode() -> AuthMode {
    AuthMode::Enabled(ConfiguredAuth {
        methods:    vec![ServerAuthMethod::DevToken],
        dev_token:  Some(TEST_DEV_TOKEN.to_string()),
        jwt_key:    None,
        jwt_issuer: None,
    })
}

fn jwt_auth_mode() -> AuthMode {
    AuthMode::Enabled(ConfiguredAuth {
        methods:    vec![ServerAuthMethod::Github],
        dev_token:  None,
        jwt_key:    Some(
            auth::derive_jwt_key(TEST_SESSION_SECRET.as_bytes())
                .expect("test JWT key should derive"),
        ),
        jwt_issuer: Some(TEST_JWT_ISSUER.to_string()),
    })
}

fn jwt_auth_state() -> Arc<AppState> {
    test_app_state_with_session_key(
        default_test_server_settings(),
        RunLayer::default(),
        Some(TEST_SESSION_SECRET),
    )
}

fn jwt_auth_app() -> (Arc<AppState>, Router) {
    let state = jwt_auth_state();
    let app = build_router(Arc::clone(&state), jwt_auth_mode());
    (state, app)
}

fn test_user_subject() -> auth::JwtSubject {
    auth::JwtSubject {
        identity:    fabro_types::IdpIdentity::new("https://github.com", "12345").unwrap(),
        login:       "octocat".to_string(),
        name:        "The Octocat".to_string(),
        email:       "octocat@example.com".to_string(),
        avatar_url:  "https://example.com/octocat.png".to_string(),
        user_url:    "https://github.com/octocat".to_string(),
        auth_method: AuthMethod::Github,
    }
}

fn issue_test_user_jwt() -> String {
    let key =
        auth::derive_jwt_key(TEST_SESSION_SECRET.as_bytes()).expect("test JWT key should derive");
    auth::issue(
        &key,
        TEST_JWT_ISSUER,
        &test_user_subject(),
        ChronoDuration::minutes(10),
    )
}

fn issue_test_worker_token(run_id: &RunId) -> String {
    let keys = WorkerTokenKeys::from_master_secret(TEST_SESSION_SECRET.as_bytes())
        .expect("worker keys should derive");
    crate::worker_token::issue_worker_token(&keys, run_id).expect("worker token should issue")
}

fn issue_test_run_tools_worker_token(run_id: &RunId) -> String {
    let keys = WorkerTokenKeys::from_master_secret(TEST_SESSION_SECRET.as_bytes())
        .expect("worker keys should derive");
    crate::worker_token::issue_worker_token_with_scopes(
        &keys,
        run_id,
        &crate::worker_token::WorkerScopeSet::run_worker_with_agent_run_tools(),
    )
    .expect("worker token should issue")
}

async fn create_run_with_bearer(app: &Router, bearer: &str) -> RunId {
    create_run_with_bearer_for_graph(app, bearer, MINIMAL_DOT).await
}

async fn create_run_with_bearer_for_graph(app: &Router, bearer: &str, _dot_source: &str) -> RunId {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api("/runs"))
                .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    test_intent_with_bearer(app, "workflow.fabro", MINIMAL_DOT, None, Some(bearer))
                        .await
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::CREATED).await;
    body["id"].as_str().unwrap().parse().unwrap()
}

fn pair_test_target() -> PairTarget {
    PairTarget {
        stage_id:   StageId::new("agent", 1),
        node_label: "Agent".to_string(),
    }
}

async fn append_pair_transcript_fixture(state: &Arc<AppState>, run_id: RunId) -> PairId {
    let pair_id = "01HZX6M29F1CD5YYMHT1F5D7WQ".parse().unwrap();
    let run_store = state
        .stores
        .runs
        .open_run(&run_id)
        .await
        .expect("test run should be openable");
    workflow_event::append_event(
        &run_store,
        &run_id,
        &workflow_event::Event::RunPairStarted {
            pair_id,
            target: pair_test_target(),
            actor: None,
        },
    )
    .await
    .unwrap();
    workflow_event::append_event(
        &run_store,
        &run_id,
        &workflow_event::Event::AgentPairUserMessage {
            node_id: "agent".to_string(),
            visit: 1,
            session_id: "session-1".to_string(),
            pair_id,
            message_id: PairMessageId::new(),
            client_message_id: None,
            text: "hello pair".to_string(),
            actor: None,
        },
    )
    .await
    .unwrap();
    pair_id
}

fn bearer_request(method: Method, path: &str, bearer: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(api(path))
        .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
        .body(body)
        .unwrap()
}

struct WorkerControlWsTestServer {
    base_url: String,
    task:     tokio::task::JoinHandle<()>,
}

impl WorkerControlWsTestServer {
    async fn spawn(app: Router) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test WebSocket listener should bind");
        let addr = listener
            .local_addr()
            .expect("test WebSocket listener should have a local address");
        let task = tokio::spawn(async move {
            let result = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await;
            if let Err(err) = result {
                tracing::debug!(error = %err, "test WebSocket server stopped");
            }
        });
        Self {
            base_url: format!("ws://{addr}"),
            task,
        }
    }

    fn worker_control_url(&self, run_id: RunId, after: Option<&str>) -> String {
        let mut url = format!(
            "{}/api/v1/runs/{run_id}/worker/control-stream",
            self.base_url
        );
        if let Some(after) = after {
            url.push_str("?after=");
            url.push_str(after);
        }
        url
    }
}

impl Drop for WorkerControlWsTestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn worker_control_ws_request(
    server: &WorkerControlWsTestServer,
    run_id: RunId,
    bearer: Option<&str>,
    after: Option<&str>,
) -> Request<()> {
    let mut request = server
        .worker_control_url(run_id, after)
        .into_client_request()
        .expect("test worker-control WebSocket request should build");
    if let Some(bearer) = bearer {
        request.headers_mut().insert(
            header::AUTHORIZATION,
            format!("Bearer {bearer}")
                .parse()
                .expect("test bearer header should parse"),
        );
    }
    request
}

async fn connect_worker_control_ws(
    server: &WorkerControlWsTestServer,
    run_id: RunId,
    bearer: &str,
    after: Option<&str>,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let request = worker_control_ws_request(server, run_id, Some(bearer), after);
    let (socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .expect("worker-control WebSocket should connect");
    socket
}

async fn assert_worker_control_ws_rejected(
    server: &WorkerControlWsTestServer,
    run_id: RunId,
    bearer: Option<&str>,
    after: Option<&str>,
    expected: StatusCode,
) {
    let request = worker_control_ws_request(server, run_id, bearer, after);
    let error = tokio_tungstenite::connect_async(request)
        .await
        .expect_err("worker-control WebSocket should be rejected");
    match error {
        tokio_tungstenite::tungstenite::Error::Http(response) => {
            assert_eq!(response.status(), expected);
        }
        other => panic!("expected HTTP rejection {expected}, got {other:#}"),
    }
}

async fn next_worker_control_frame(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> WorkerControlDeliveryFrame {
    let message = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let message = futures_util::StreamExt::next(socket)
                .await
                .expect("worker-control WebSocket should remain open")
                .expect("worker-control WebSocket frame should be ok");
            match message {
                WebSocketMessage::Text(text) => return text,
                WebSocketMessage::Ping(payload) => {
                    futures_util::SinkExt::send(socket, WebSocketMessage::Pong(payload))
                        .await
                        .expect("test worker-control pong should send");
                }
                WebSocketMessage::Pong(_)
                | WebSocketMessage::Binary(_)
                | WebSocketMessage::Frame(_) => {}
                WebSocketMessage::Close(frame) => {
                    panic!("worker-control WebSocket closed before text frame: {frame:?}");
                }
            }
        }
    })
    .await
    .expect("worker-control frame should arrive");
    serde_json::from_str(message.as_str()).expect("worker-control delivery frame should parse")
}

fn json_bearer_request(
    method: Method,
    path: &str,
    bearer: &str,
    body: &serde_json::Value,
) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(api(path))
        .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

fn json_request(method: Method, path: &str, body: &serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(api(path))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

fn canonical_origin_settings(url: &str) -> ServerSettings {
    server_settings_from_toml(&format!(
        r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[server.web]
url = "{url}"
"#
    ))
}

fn canonical_host_test_app() -> Router {
    let state = test_app_state_with_options(
        canonical_origin_settings("http://127.0.0.1:32276"),
        RunLayer::default(),
        5,
    );
    crate::test_support::build_test_router_with_options(state, RouterOptions::default())
}

fn create_token_secret_request(name: &str, value: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(api("/secrets"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "name": name,
                "value": value,
                "type": "token"
            }))
            .unwrap(),
        ))
        .unwrap()
}

struct FailingCredentialSource;

#[async_trait::async_trait]
impl CredentialProvider for FailingCredentialSource {
    async fn credentials(
        &self,
        provider: &fabro_llm::lithos_catalog::CatalogProvider,
    ) -> Result<fabro_llm::credentials::Credentials, fabro_llm::credentials::CredentialError> {
        Err(fabro_llm::credentials::CredentialError::NotConfigured {
            provider: provider.id().clone(),
        })
    }

    async fn is_configured(&self, _provider: &fabro_llm::lithos_catalog::CatalogProvider) -> bool {
        false
    }
}

fn slack_app_state_with_secret_sources(
    vault_entries: &[(&str, &str, SecretType)],
    server_secret_env: HashMap<String, String>,
) -> Arc<AppState> {
    slack_app_state_with_settings_and_secret_sources(
        default_test_server_settings(),
        vault_entries,
        server_secret_env,
    )
}

fn slack_app_state_with_settings_and_secret_sources(
    settings: ServerSettings,
    vault_entries: &[(&str, &str, SecretType)],
    server_secret_env: HashMap<String, String>,
) -> Arc<AppState> {
    let (store, artifact_store) = test_store_bundle();
    let vault_path = test_secret_store_path();
    let server_env_path = vault_path.with_file_name("server.env");
    let mut vault = Vault::load(vault_path.clone()).unwrap();
    for (name, value, secret_type) in vault_entries {
        vault.set(name, value, *secret_type, None).unwrap();
    }
    build_app_state(AppStateConfig {
        resolved_settings: resolved_runtime_settings_for_tests(
            settings,
            RunLayer::default(),
            LlmLayer::default(),
        ),
        registry_factory_override: None,
        max_concurrent_runs: 5,
        store,
        artifact_store,
        db_pool: test_db_pool_for_vault_path(&vault_path).expect("test db pool should build"),
        preloaded_vault: vault,
        server_secrets: load_test_server_secrets(server_env_path, server_secret_env),
        env_lookup: default_env_lookup(),
        github_api_base_url: None,
        active_config_path: tempfile::tempdir().unwrap().path().join("settings.toml"),
        http_client: Some(fabro_http::test_http_client().expect("test HTTP client should build")),
        sandbox_inventory: None,
        shutdown: tokio_util::sync::CancellationToken::new(),
        worker_control_bus: None,
        worker_runtime: None,
        automation_materializer_override: None,
        automation_breaker_notifier_override: None,
    })
    .expect("slack test app state should build")
}

fn slack_test_vault_tokens() -> [(&'static str, &'static str, SecretType); 2] {
    [
        (
            EnvVars::FABRO_SLACK_BOT_TOKEN,
            "xoxb-test",
            SecretType::Token,
        ),
        (
            EnvVars::FABRO_SLACK_APP_TOKEN,
            "xapp-test",
            SecretType::Token,
        ),
    ]
}

fn test_worker_ref(pid: u32) -> WorkerRef {
    WorkerRef::Local { pid }
}

#[cfg(unix)]
fn worker_command(
    state: &AppState,
    run_id: RunId,
    mode: RunExecutionMode,
    run_dir: &Path,
    agent_fabro_tools_enabled: bool,
) -> anyhow::Result<Command> {
    let spec = worker_launch_spec(
        state,
        run_id,
        mode,
        run_dir,
        agent_fabro_tools_enabled,
        &[],
        None,
    )?;
    Ok(LocalWorkerRuntime::command_for_spec(&spec))
}

fn worker_command_test_state(
    storage_dir: &Path,
    methods: &[&str],
    dev_token: Option<&str>,
) -> Arc<AppState> {
    worker_command_test_state_with_extra_config(storage_dir, methods, dev_token, "")
}

fn worker_command_test_state_with_extra_config(
    storage_dir: &Path,
    methods: &[&str],
    dev_token: Option<&str>,
    extra_config: &str,
) -> Arc<AppState> {
    worker_command_test_state_with_extra_config_and_env_lookup(
        storage_dir,
        methods,
        dev_token,
        extra_config,
        &[],
        |_| None,
    )
}

fn worker_command_test_state_with_extra_config_and_env_lookup(
    storage_dir: &Path,
    methods: &[&str],
    dev_token: Option<&str>,
    extra_config: &str,
    extra_server_secrets: &[(&str, &str)],
    env_lookup: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
) -> Arc<AppState> {
    worker_command_test_state_inner(
        storage_dir,
        methods,
        dev_token,
        extra_config,
        extra_server_secrets,
        env_lookup,
        None,
    )
}

fn worker_command_test_state_with_active_config_path(
    storage_dir: &Path,
    methods: &[&str],
    dev_token: Option<&str>,
    active_config_path: PathBuf,
) -> Arc<AppState> {
    worker_command_test_state_inner(
        storage_dir,
        methods,
        dev_token,
        "",
        &[],
        |_| None,
        Some(active_config_path),
    )
}

fn worker_command_test_state_inner(
    storage_dir: &Path,
    methods: &[&str],
    dev_token: Option<&str>,
    extra_config: &str,
    extra_server_secrets: &[(&str, &str)],
    env_lookup: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
    active_config_path: Option<PathBuf>,
) -> Arc<AppState> {
    let dev_token = dev_token.map(str::to_owned);
    std::fs::create_dir_all(storage_dir).unwrap();
    let source = format!(
        r#"
_version = 1

[server.storage]
root = "{}"

[server.auth]
methods = [{}]

[server.auth.github]
allowed_usernames = ["octocat"]
{extra_config}
"#,
        storage_dir.display(),
        methods
            .iter()
            .map(|method| format!("\"{method}\""))
            .collect::<Vec<_>>()
            .join(", ")
    );
    write_test_server_record(storage_dir);

    let mut server_secret_env: HashMap<String, String> = dev_token
        .map(|token| HashMap::from([("FABRO_DEV_TOKEN".to_string(), token)]))
        .unwrap_or_default();
    for (key, value) in extra_server_secrets {
        server_secret_env.insert((*key).to_string(), (*value).to_string());
    }
    let mut builder = TestAppStateBuilder::new()
        .runtime_settings(
            server_settings_from_toml(&source),
            manifest_run_defaults_from_toml(&source),
        )
        .max_concurrent_runs(5)
        .env_lookup(env_lookup)
        .server_secret_env(server_secret_env);
    if let Some(active_config_path) = active_config_path {
        builder = builder.active_config_path(active_config_path);
    }
    builder.build()
}

#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
enum EnvOverride {
    Unchanged,
    Removed,
    Set(String),
}

#[cfg(unix)]
fn command_env_value(cmd: &Command, key: &str) -> EnvOverride {
    cmd.as_std()
        .get_envs()
        .find_map(|(name, value)| {
            (name.to_str() == Some(key)).then(|| match value {
                Some(value) => EnvOverride::Set(value.to_string_lossy().into_owned()),
                None => EnvOverride::Removed,
            })
        })
        .unwrap_or(EnvOverride::Unchanged)
}

#[cfg(unix)]
fn assert_worker_command_passes_token_only_by_env(cmd: &Command) {
    assert!(matches!(
        command_env_value(cmd, EnvVars::FABRO_WORKER_TOKEN),
        EnvOverride::Set(_)
    ));
    assert_eq!(
        command_env_value(cmd, EnvVars::FABRO_DEV_TOKEN),
        EnvOverride::Unchanged
    );
    let args = cmd
        .as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>();
    assert!(!args.iter().any(|arg| arg == "--artifact-upload-token"));
    assert!(!args.iter().any(|arg| arg == "--worker-token"));
}

#[cfg(unix)]
fn worker_token_claims(cmd: &Command, state: &AppState) -> crate::worker_token::WorkerTokenClaims {
    let EnvOverride::Set(token) = command_env_value(cmd, EnvVars::FABRO_WORKER_TOKEN) else {
        panic!("worker token env should be set");
    };

    jsonwebtoken::decode::<crate::worker_token::WorkerTokenClaims>(
        &token,
        state.worker_token_keys().decoding_key(),
        state.worker_token_keys().validation(),
    )
    .expect("worker token should decode")
    .claims
}

fn write_test_server_record(storage_dir: &Path) {
    let runtime_directory = Storage::new(storage_dir).runtime_directory();
    ServerDaemon::new(
        std::process::id(),
        Bind::Tcp(
            "127.0.0.1:32276"
                .parse::<std::net::SocketAddr>()
                .expect("test bind should parse"),
        ),
        runtime_directory.log_path(),
    )
    .write(&runtime_directory)
    .expect("test server record should be written");
}

/// Waits up to one second for `condition` to hold, re-checking whenever
/// `notify` fires.
async fn wait_until(notify: &Notify, condition: impl Fn() -> bool, expectation: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let notified = notify.notified();
            if condition() {
                return;
            }
            notified.await;
        }
    })
    .await
    .expect(expectation);
}

#[derive(Default)]
struct RecordingWorkerRuntime {
    requested:     StdMutex<Vec<WorkerRef>>,
    forced:        StdMutex<Vec<WorkerRef>>,
    alive:         AtomicBool,
    forced_notify: Notify,
}

impl RecordingWorkerRuntime {
    fn requested_refs(&self) -> Vec<WorkerRef> {
        self.requested
            .lock()
            .expect("requested lock poisoned")
            .clone()
    }

    fn forced_refs(&self) -> Vec<WorkerRef> {
        self.forced.lock().expect("forced lock poisoned").clone()
    }

    fn set_alive(&self, alive: bool) {
        self.alive.store(alive, Ordering::Relaxed);
    }

    async fn wait_for_forced_ref(&self, worker_ref: &WorkerRef) {
        wait_until(
            &self.forced_notify,
            || self.forced_refs().contains(worker_ref),
            "worker should be force-stopped after the cancellation grace period",
        )
        .await;
    }
}

#[derive(Clone, Copy)]
enum PreStartWorkerOutcome {
    LaunchFailure,
    EarlyExit,
}

/// Test worker runtime whose `start` fails before the worker reaches
/// `Starting`, either by refusing to launch or by exiting immediately. When
/// built with `held`, `start` blocks until `release_held_start` so a test can
/// act while the launch is in flight.
struct PreStartWorkerRuntime {
    outcome:       PreStartWorkerOutcome,
    starts:        AtomicUsize,
    start_entered: Notify,
    release_start: Option<Notify>,
}

impl PreStartWorkerRuntime {
    fn new(outcome: PreStartWorkerOutcome) -> Self {
        Self {
            outcome,
            starts: AtomicUsize::new(0),
            start_entered: Notify::new(),
            release_start: None,
        }
    }

    fn held(outcome: PreStartWorkerOutcome) -> Self {
        Self {
            release_start: Some(Notify::new()),
            ..Self::new(outcome)
        }
    }

    fn start_count(&self) -> usize {
        self.starts.load(Ordering::Relaxed)
    }

    async fn wait_for_start(&self) {
        wait_until(
            &self.start_entered,
            || self.start_count() > 0,
            "test worker runtime should receive one start request",
        )
        .await;
    }

    fn release_held_start(&self) {
        self.release_start
            .as_ref()
            .expect("runtime should have been built with a held start")
            .notify_one();
    }
}

#[async_trait::async_trait]
impl WorkerRuntime for PreStartWorkerRuntime {
    async fn start(&self, _spec: WorkerLaunchSpec) -> anyhow::Result<StartedWorker> {
        self.starts.fetch_add(1, Ordering::Relaxed);
        self.start_entered.notify_waiters();
        if let Some(release_start) = &self.release_start {
            release_start.notified().await;
        }

        match self.outcome {
            PreStartWorkerOutcome::LaunchFailure => {
                anyhow::bail!("test worker launch failed")
            }
            PreStartWorkerOutcome::EarlyExit => Ok(StartedWorker {
                worker_ref: test_worker_ref(u32::MAX),
                stderr:     Box::pin(tokio::io::empty()),
                wait:       Box::pin(async {
                    Ok(WorkerExit {
                        success: false,
                        detail:  "test worker exited before starting".to_string(),
                    })
                }),
            }),
        }
    }

    async fn request_stop(&self, _worker_ref: &WorkerRef) {}

    async fn force_stop(&self, _worker_ref: &WorkerRef) {}

    async fn is_alive(&self, _worker_ref: &WorkerRef) -> bool {
        false
    }
}

#[async_trait::async_trait]
impl WorkerRuntime for RecordingWorkerRuntime {
    async fn start(&self, _spec: WorkerLaunchSpec) -> anyhow::Result<StartedWorker> {
        anyhow::bail!("recording runtime does not start workers")
    }

    async fn request_stop(&self, worker_ref: &WorkerRef) {
        self.requested
            .lock()
            .expect("requested lock poisoned")
            .push(worker_ref.clone());
    }

    async fn force_stop(&self, worker_ref: &WorkerRef) {
        self.forced
            .lock()
            .expect("forced lock poisoned")
            .push(worker_ref.clone());
        self.alive.store(false, Ordering::Relaxed);
        self.forced_notify.notify_one();
    }

    async fn is_alive(&self, _worker_ref: &WorkerRef) -> bool {
        self.alive.load(Ordering::Relaxed)
    }
}

async fn worker_transport_with_receiver(
    run_id: RunId,
) -> (RunAnswerTransport, WorkerControlReceiver) {
    let bus = StdArc::new(LocalWorkerControlBus::new());
    let receiver = bus
        .subscribe(run_id, WorkerControlCursor::Start)
        .await
        .expect("test worker bus should subscribe");
    // Ensure the subscription task is waiting before the test publishes.
    tokio::task::yield_now().await;
    let bus: StdArc<dyn WorkerControlBus> = bus;
    let transport = RunAnswerTransport::Worker { run_id, bus };
    (transport, receiver)
}

async fn recv_worker_control_envelope(
    receiver: &mut WorkerControlReceiver,
) -> WorkerControlEnvelope {
    receiver
        .recv()
        .await
        .expect("test worker control receiver should stay open")
        .expect("test worker control delivery should succeed")
        .envelope
}

fn manifest_json(target_path: &str, dot_source: &str) -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "cwd": "/tmp",
        "target": {
            "path": target_path,
        },
        "workflows": {
            target_path: {
                "source": dot_source,
                "files": {},
            },
        },
    })
}

fn minimal_manifest_json(dot_source: &str) -> serde_json::Value {
    manifest_json("workflow.fabro", dot_source)
}

fn manifest_body(dot_source: &str) -> Body {
    Body::from(serde_json::to_string(&minimal_manifest_json(dot_source)).unwrap())
}

async fn test_intent_with_bearer(
    app: &Router,
    entrypoint: &str,
    source: &str,
    config: Option<&str>,
    bearer: Option<&str>,
) -> serde_json::Value {
    let entrypoint = fabro_types::WorkflowPath::new(entrypoint).unwrap();
    let mut files = std::collections::BTreeMap::from([(entrypoint.clone(), source.to_string())]);
    if let Some(config) = config {
        files.insert(
            entrypoint.resolve_reference("workflow.toml").unwrap(),
            config.to_string(),
        );
    }
    let version =
        fabro_types::WorkflowVersion::new(entrypoint, files, std::collections::BTreeMap::new())
            .unwrap();
    let id = crate::test_support::test_register_workflow_version(app, &version, bearer).await;
    json!({"workflow_version_id": id, "target": {"kind": "none"}, "args": {}})
}

async fn test_intent(app: &Router, source: &str) -> serde_json::Value {
    test_intent_with_bearer(app, "workflow.fabro", source, None, None).await
}

async fn intent_body(app: &Router, source: &str) -> Body {
    Body::from(test_intent(app, source).await.to_string())
}

async fn create_run(app: &Router, dot_source: &str) -> String {
    let req = Request::builder()
        .method("POST")
        .uri(api("/runs"))
        .header("content-type", "application/json")
        .body(intent_body(app, dot_source).await)
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let body = body_json(response.into_body()).await;
    body["id"].as_str().unwrap().to_string()
}

async fn post_run_intent(app: &Router, intent: serde_json::Value) -> serde_json::Value {
    let response = post_run_intent_response(app, intent).await;
    response_json!(response, StatusCode::CREATED).await
}

async fn post_run_intent_response(app: &Router, intent: serde_json::Value) -> Response {
    app.clone()
        .oneshot(json_request(Method::POST, "/runs", &intent))
        .await
        .unwrap()
}

/// App state whose default environment runs in place on the server, which is
/// the only placement folder targets admit.
fn local_test_app_state() -> Arc<AppState> {
    TestAppStateBuilder::new()
        .default_environment_provider(Some(SandboxProviderKind::LOCAL))
        .vault_entries([(fabro_static::EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
        .build()
}

fn folder_intent(
    workflow_version_id: fabro_types::WorkflowVersionId,
    path: impl serde::Serialize,
) -> serde_json::Value {
    json!({
        "workflow_version_id": workflow_version_id,
        "target": { "kind": "folder", "path": path },
        "args": {}
    })
}

async fn store_workflow_version(
    state: &AppState,
    graph: &str,
    workflow_toml: Option<&str>,
) -> fabro_types::WorkflowVersionId {
    store_workflow_version_with_entrypoint(state, "workflow.fabro", graph, workflow_toml).await
}

async fn store_workflow_version_with_entrypoint(
    state: &AppState,
    entrypoint: &str,
    graph: &str,
    workflow_toml: Option<&str>,
) -> fabro_types::WorkflowVersionId {
    let entrypoint = fabro_types::WorkflowPath::new(entrypoint).unwrap();
    let mut files = std::collections::BTreeMap::from([(entrypoint.clone(), graph.to_string())]);
    if let Some(workflow_toml) = workflow_toml {
        files.insert(
            fabro_types::WorkflowPath::new("workflow.toml").unwrap(),
            workflow_toml.to_string(),
        );
        files.insert(
            fabro_types::WorkflowPath::new("goal.md").unwrap(),
            "Goal loaded from immutable version bytes".to_string(),
        );
        files.insert(
            fabro_types::WorkflowPath::new("Dockerfile").unwrap(),
            "FROM alpine:3".to_string(),
        );
    }
    let version =
        fabro_types::WorkflowVersion::new(entrypoint, files, std::collections::BTreeMap::new())
            .unwrap();
    let version = fabro_workflow_version::ValidatedWorkflowVersion::new(version).unwrap();
    let blobs = state.store_ref().blobs();
    fabro_workflow_version::WorkflowVersionStore::new(blobs)
        .put(&version)
        .await
        .unwrap()
}

/// Posts a Git and a `none` run intent against `state` and asserts both are
/// rejected as `integration_unavailable` without persisting anything.
async fn assert_run_intent_targets_unavailable(state: &Arc<AppState>) {
    let version_id = store_workflow_version(state, MINIMAL_DOT, None).await;
    let app = crate::test_support::build_test_router(Arc::clone(state));
    for target in [
        json!({
            "kind": "git",
            "repo": "fabro-sh/fabro",
            "branch": "feature/run-intent"
        }),
        json!({ "kind": "none" }),
    ] {
        let intent = json!({
            "workflow_version_id": version_id,
            "target": target,
            "args": {}
        });
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(api("/runs"))
                    .header("content-type", "application/json")
                    .body(Body::from(intent.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = response_json!(response, StatusCode::SERVICE_UNAVAILABLE).await;
        assert_eq!(body["errors"][0]["code"], "integration_unavailable");
    }
    assert!(state.runs.lock().expect("runs lock poisoned").is_empty());
    assert!(
        state
            .stores
            .run_summaries
            .list_identities()
            .await
            .unwrap()
            .is_empty()
    );
}

async fn mock_openai_title_response<'a>(
    server: &'a MockServer,
    title: &str,
    delay: Option<std::time::Duration>,
) -> httpmock::Mock<'a> {
    let title = title.to_string();
    server
        .mock_async(move |when, then| {
            when.method(POST).path("/v1/responses");
            let then = then
                .status(200)
                .header("content-type", "application/json")
                .json_body(openai_responses_payload(
                    &json!({ "title": title }).to_string(),
                ));
            if let Some(delay) = delay {
                then.delay(delay);
            }
        })
        .await
}

async fn wait_for_run_title(state: &AppState, run_id: RunId, expected: &str) {
    for _ in 0..50 {
        let title = state
            .stores
            .run_summaries
            .get(&run_id, Utc::now())
            .await
            .unwrap()
            .unwrap()
            .title;
        if title == expected {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("run {run_id} title did not become {expected:?}");
}

async fn wait_for_mock_hits(mock: &httpmock::Mock<'_>, expected: usize) {
    for _ in 0..50 {
        if mock.calls_async().await >= expected {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("mock did not receive {expected} request(s)");
}

/// Poll `GET /runs/{id}/pull_request/creation` until the creation leaves
/// `pending`, returning the terminal creation body.
async fn wait_for_pull_request_creation(app: &Router, run_id: RunId) -> serde_json::Value {
    for _ in 0..150 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(api(&format!("/runs/{run_id}/pull_request/creation")))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = response_json!(response, StatusCode::OK).await;
        if body["status"] != "pending" {
            return body;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("pull request creation for run {run_id} did not finish");
}

async fn title_update_event_count(state: &AppState, run_id: RunId) -> usize {
    let run_store = state.stores.runs.open_run(&run_id).await.unwrap();
    run_store
        .list_events()
        .await
        .unwrap()
        .into_iter()
        .filter(|event| event.event.event_name() == "run.title.updated")
        .count()
}

async fn create_run_for_target(app: &Router, target_path: &str, dot_source: &str) -> String {
    let intent = test_intent_with_bearer(app, target_path, dot_source, None, None).await;
    post_run_intent(app, intent).await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn create_run_for_target_with_workflow_name(
    app: &Router,
    target_path: &str,
    dot_source: &str,
    workflow_name: &str,
) -> String {
    let config = format!("_version = 1\n[workflow]\nname = {workflow_name:?}\n");
    let intent = test_intent_with_bearer(app, target_path, dot_source, Some(&config), None).await;
    post_run_intent(app, intent).await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn named_workflow_dot(name: &str, goal: &str) -> String {
    format!(
        r#"digraph {name} {{
    graph [goal="{goal}"]
    start [shape=Mdiamond]
    exit  [shape=Msquare]
    start -> exit
}}"#
    )
}

fn multipart_body(
    boundary: &str,
    manifest: &serde_json::Value,
    files: &[(&str, &str, &[u8])],
) -> Body {
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Disposition: form-data; name=\"manifest\"\r\n");
    body.extend_from_slice(b"Content-Type: application/json\r\n\r\n");
    body.extend_from_slice(serde_json::to_string(manifest).unwrap().as_bytes());
    body.extend_from_slice(b"\r\n");

    for (part, filename, bytes) in files {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{part}\"; filename=\"{filename}\"\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }

    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    Body::from(body)
}

/// Create a run via POST /runs, then start it via POST /runs/{id}/start.
/// Returns the run_id string.
async fn create_and_start_run(app: &Router, dot_source: &str) -> String {
    let run_id = create_run(app, dot_source).await;

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/start")))
        .body(Body::empty())
        .unwrap();
    app.clone().oneshot(req).await.unwrap();

    run_id
}

fn subprocess_pre_start_failure_state(runtime: StdArc<PreStartWorkerRuntime>) -> Arc<AppState> {
    let state = TestAppStateBuilder::new()
        .vault_entries([(EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
        .worker_runtime(runtime)
        .build();
    write_test_server_record(&state.server_storage_dir());
    state
}

fn run_failed_reasons(events: &[EventEnvelope]) -> Vec<FailureReason> {
    events
        .iter()
        .filter_map(|envelope| match &envelope.event.body {
            EventBody::RunFailed(props) => Some(props.failure.reason),
            _ => None,
        })
        .collect()
}

/// Asserts that a run which failed before its worker reached `Starting`
/// recorded exactly one `run.failed` event with `expected_reason` and that the
/// durable and in-memory statuses agree. Returns the run's events for further
/// inspection.
async fn assert_run_failed_before_start(
    state: &Arc<AppState>,
    run_id: RunId,
    expected_reason: FailureReason,
) -> Vec<EventEnvelope> {
    let run_store = state
        .stores
        .runs
        .open_run_reader(&run_id)
        .await
        .expect("failed run should remain readable");
    let events = run_store
        .list_events()
        .await
        .expect("failed run events should remain readable");
    assert_eq!(run_failed_reasons(&events), vec![expected_reason]);

    let expected_status = RunStatus::Failed {
        reason: expected_reason,
    };
    assert_eq!(
        run_store
            .state()
            .await
            .expect("failed run state should load")
            .status,
        expected_status
    );
    assert_eq!(
        state
            .runs
            .lock()
            .expect("runs lock poisoned")
            .get(&run_id)
            .expect("managed run should remain present")
            .status,
        expected_status
    );
    events
}

async fn assert_subprocess_pre_start_failure(
    outcome: PreStartWorkerOutcome,
    expected_reason: FailureReason,
) {
    let runtime = StdArc::new(PreStartWorkerRuntime::new(outcome));
    let state = subprocess_pre_start_failure_state(StdArc::clone(&runtime));
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = create_and_start_run(&app, MINIMAL_DOT)
        .await
        .parse::<RunId>()
        .expect("created run id should parse");
    let run_store = state
        .stores
        .runs
        .open_run_reader(&run_id)
        .await
        .expect("created run should remain readable");

    assert_eq!(
        run_store
            .state()
            .await
            .expect("runnable run state should load")
            .status,
        RunStatus::Runnable
    );

    execute_run(Arc::clone(&state), run_id).await;

    assert_eq!(runtime.start_count(), 1);
    let events = assert_run_failed_before_start(&state, run_id, expected_reason).await;
    let lifecycle_events = events
        .iter()
        .map(|envelope| envelope.event.event_name())
        .filter(|name| matches!(*name, "run.runnable" | "run.starting" | "run.failed"))
        .collect::<Vec<_>>();
    assert_eq!(lifecycle_events, vec!["run.runnable", "run.failed"]);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{run_id}")))
                .body(Body::empty())
                .expect("run request should build"),
        )
        .await
        .expect("run request should complete");
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(run_json_status(&body)["kind"], "failed");
    assert_eq!(
        run_json_status(&body)["reason"],
        expected_reason.to_string()
    );

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api("/runs"))
                .body(Body::empty())
                .expect("run list request should build"),
        )
        .await
        .expect("run list request should complete");
    let body = response_json!(response, StatusCode::OK).await;
    let run_id_string = run_id.to_string();
    let listed = body["data"]
        .as_array()
        .expect("run list data should be an array")
        .iter()
        .find(|run| run_json_id(run) == Some(run_id_string.as_str()))
        .expect("failed run should remain listed");
    assert_eq!(run_json_status(listed)["kind"], "failed");
    assert_eq!(
        run_json_status(listed)["reason"],
        expected_reason.to_string()
    );
}

async fn create_durable_run_with_events(
    state: &Arc<AppState>,
    run_id: RunId,
    events: &[workflow_event::Event],
) {
    let run_store = state.stores.runs.create_run(&run_id).await.unwrap();
    if !matches!(
        events.first(),
        Some(workflow_event::Event::RunCreated { .. })
    ) {
        append_default_run_created(&run_store, run_id).await;
    }
    let needs_running = events.iter().any(|event| {
        matches!(
            event,
            workflow_event::Event::WorkflowRunCompleted { .. }
                | workflow_event::Event::WorkflowRunFailed { .. }
        )
    });
    let has_starting = events
        .iter()
        .any(|event| matches!(event, workflow_event::Event::RunStarting));
    let has_runnable = events
        .iter()
        .any(|event| matches!(event, workflow_event::Event::RunRunnable { .. }));
    let has_running = events
        .iter()
        .any(|event| matches!(event, workflow_event::Event::RunRunning));
    let mut inserted_runnable = has_runnable;
    let mut inserted_starting = has_starting;
    for event in events {
        if !inserted_runnable
            && matches!(
                event,
                workflow_event::Event::RunStarting
                    | workflow_event::Event::RunRunning
                    | workflow_event::Event::RunBlocked { .. }
                    | workflow_event::Event::RunPaused
                    | workflow_event::Event::WorkflowRunCompleted { .. }
                    | workflow_event::Event::WorkflowRunFailed { .. }
            )
        {
            workflow_event::append_event(
                &run_store,
                &run_id,
                &workflow_event::Event::RunRunnable {
                    source: fabro_types::RunRunnableSource::StartRequested,
                    actor:  None,
                },
            )
            .await
            .unwrap();
            inserted_runnable = true;
        }
        if !inserted_starting
            && matches!(
                event,
                workflow_event::Event::RunRunning
                    | workflow_event::Event::RunBlocked { .. }
                    | workflow_event::Event::RunPaused
                    | workflow_event::Event::WorkflowRunCompleted { .. }
                    | workflow_event::Event::WorkflowRunFailed { .. }
            )
        {
            workflow_event::append_event(&run_store, &run_id, &workflow_event::Event::RunStarting)
                .await
                .unwrap();
            inserted_starting = true;
        }
        if needs_running
            && !has_running
            && matches!(
                event,
                workflow_event::Event::WorkflowRunCompleted { .. }
                    | workflow_event::Event::WorkflowRunFailed { .. }
            )
        {
            workflow_event::append_event(&run_store, &run_id, &workflow_event::Event::RunRunning)
                .await
                .unwrap();
        }
        workflow_event::append_event(&run_store, &run_id, event)
            .await
            .unwrap();
    }
}

fn stage_started_event(node_id: &str, handler_type: &str) -> workflow_event::Event {
    workflow_event::Event::StageStarted {
        graph_visit:           None,
        resumed_from_stage_id: None,
        node_id:               node_id.to_string(),
        name:                  node_id.to_string(),
        index:                 1,
        handler_type:          handler_type.to_string(),
        attempt:               1,
        max_attempts:          1,
    }
}

fn command_started_event(node_id: &str) -> workflow_event::Event {
    workflow_event::Event::CommandStarted {
        node_id:    node_id.to_string(),
        script:     "echo ok".to_string(),
        command:    "echo ok".to_string(),
        language:   "shell".to_string(),
        timeout_ms: None,
    }
}

fn agent_session_activated_event(node_id: &str, visit: u32) -> workflow_event::Event {
    workflow_event::Event::AgentSessionActivated {
        node_id: node_id.to_string(),
        visit,
        session_id: "session-1".to_string(),
        thread_id: None,
        provider: Some("openai".to_string()),
        model: Some("gpt-5.4".to_string()),
        reasoning_effort: None,
        speed: None,
        permission_level: None,
        capabilities: Vec::new(),
    }
}

fn stage_completed_event(node_id: &str) -> workflow_event::Event {
    workflow_event::Event::StageCompleted {
        node_id: node_id.to_string(),
        name: node_id.to_string(),
        index: 1,
        timing: StageTiming::wall_only(42),
        status: "succeeded".to_string(),
        preferred_label: None,
        suggested_next_ids: Vec::new(),
        usage_by_model: Vec::new(),
        usage: None,
        failure: None,
        notes: None,
        files_touched: Vec::new(),
        context_updates: None,
        jump_to_node: None,
        context_values: None,
        node_visits: None,
        loop_failure_signatures: None,
        restart_failure_signatures: None,
        response: None,
        attempt: 1,
        max_attempts: 1,
    }
}

fn agent_message_event(
    stage: &str,
    visit: u32,
    session_id: &str,
    text: &str,
    context_window: Option<ContextWindowSnapshot>,
    reasoning: Option<ReasoningOutput>,
) -> workflow_event::Event {
    workflow_event::Event::Agent {
        stage: stage.to_string(),
        visit,
        event: CodingAgentEvent::new(
            session_id,
            CodingEvent::AssistantMessage {
                text: text.to_string(),
                model: "gpt-5.4".to_string(),
                usage: Usage::default(),
                tool_call_count: 0,
                context_window,
                reasoning,
            },
            std::time::SystemTime::now(),
        ),
    }
}

fn context_window_event(
    stage: &str,
    visit: u32,
    context_window: ContextWindowSnapshot,
) -> workflow_event::Event {
    agent_message_event(
        stage,
        visit,
        "session-1",
        "assistant response",
        Some(context_window),
        None,
    )
}

fn context_window_snapshot(
    input_tokens: u64,
    warnings: Vec<ContextWindowWarning>,
) -> ContextWindowSnapshot {
    ContextWindowSnapshot {
        provider: "openai".to_string(),
        model: "gpt-5.4".to_string(),
        context_window_tokens: 400_000,
        input_tokens,
        usage_percent: input_tokens as f64 * 100.0 / 400_000.0,
        count_method: ContextWindowCountMethod::ResponseUsageScaledBreakdown,
        staleness: ContextWindowStaleness::Live,
        generated_at: std::time::SystemTime::now(),
        event_seq: None,
        breakdown: vec![ContextWindowBreakdownItem {
            category:      ContextWindowCategory::Conversation,
            tokens:        input_tokens,
            usage_percent: input_tokens as f64 * 100.0 / 400_000.0,
        }],
        warnings,
    }
}

async fn append_default_run_created(run_store: &fabro_store::RunDatabase, run_id: RunId) {
    workflow_event::append_event(run_store, &run_id, &workflow_event::Event::RunCreated {
        run_id,
        title: None,
        settings: serde_json::to_value(WorkflowSettings::default()).unwrap(),
        graph: serde_json::to_value(Graph::new("test")).unwrap(),
        workflow_source: None,
        labels: std::collections::BTreeMap::default(),
        source_directory: None,
        workflow_slug: None,
        workflow_version_id: None,
        target: None,
        automation: None,
        provenance: test_support::test_run_provenance(),
        spec_blob: None,
        git: None,
        fork_source_ref: None,
        retried_from: None,
        parent_id: None,
        web_url: None,
    })
    .await
    .unwrap();
}

pub(super) fn workflow_settings_with_run_notifications(
    run_toml: &str,
    workflow_name: Option<&str>,
) -> WorkflowSettings {
    let mut settings = WorkflowSettingsBuilder::new()
        .server_manifest_defaults(RunLayer::default(), test_environment_defaults())
        .workflow_toml(run_toml)
        .expect("run notification settings should parse")
        .build()
        .expect("run notification settings should resolve");
    settings.workflow.name = workflow_name.map(str::to_string);
    settings
}

fn test_environment_defaults() -> MergeMap<EnvironmentLayer> {
    MergeMap::from(HashMap::from([("default".to_string(), EnvironmentLayer {
        provider: Some("local".to_string()),
        ..EnvironmentLayer::default()
    })]))
}

pub(super) async fn create_slack_notification_run(
    state: &Arc<AppState>,
    run_id: RunId,
    settings: WorkflowSettings,
    graph_name: &str,
    workflow_slug: Option<&str>,
) -> fabro_store::RunDatabase {
    let run_store = state.stores.runs.create_run(&run_id).await.unwrap();
    workflow_event::append_event(&run_store, &run_id, &workflow_event::Event::RunCreated {
        run_id,
        title: None,
        settings: serde_json::to_value(settings).unwrap(),
        graph: serde_json::to_value(Graph::new(graph_name)).unwrap(),
        workflow_source: None,
        labels: std::collections::BTreeMap::default(),
        source_directory: None,
        workflow_slug: workflow_slug.map(str::to_string),
        workflow_version_id: None,
        target: None,
        automation: None,
        provenance: test_support::test_run_provenance(),
        spec_blob: None,
        git: None,
        fork_source_ref: None,
        retried_from: None,
        parent_id: None,
        web_url: None,
    })
    .await
    .unwrap();
    run_store
}

pub(super) async fn append_slack_notification_event(
    run_store: &fabro_store::RunDatabase,
    run_id: RunId,
    event: &workflow_event::Event,
) -> EventEnvelope {
    workflow_event::append_event(run_store, &run_id, event)
        .await
        .unwrap();
    run_store
        .list_events()
        .await
        .unwrap()
        .last()
        .expect("appended event should be present")
        .clone()
}

pub(super) async fn mock_slack_post<'a>(
    server: &'a MockServer,
    body_includes: Vec<String>,
    ts: &'static str,
) -> httpmock::Mock<'a> {
    server
        .mock_async(move |when, then| {
            let mut when = when
                .method(POST)
                .path("/chat.postMessage")
                .header("authorization", "Bearer xoxb-test");
            for part in body_includes {
                when = when.body_includes(part);
            }
            then.status(200)
                .header("content-type", "application/json")
                .json_body(json!({
                    "ok": true,
                    "channel": "C123",
                    "ts": ts,
                }));
        })
        .await
}

pub(super) fn slack_lifecycle_service(
    base_url: String,
    default_channel: Option<&str>,
) -> SlackService {
    SlackService {
        client:          fabro_slack::client::SlackClient::with_api_base_and_http(
            "xoxb-test".to_string(),
            base_url,
            fabro_http::test_http_client().expect("test HTTP client should build"),
        ),
        app_token:       "xapp-test".to_string(),
        default_channel: default_channel.map(str::to_string),
        posted_messages: StdArc::new(StdMutex::new(HashMap::new())),
        thread_registry: StdArc::new(ThreadRegistry::new()),
        connection:      StdArc::new(StdMutex::new(SlackConnectionRuntimeState::default())),
    }
}

fn workflow_run_started_event(run_id: RunId) -> workflow_event::Event {
    workflow_event::Event::WorkflowRunStarted {
        name: "run.started event name".to_string(),
        run_id,
        base_branch: None,
        base_sha: None,
        run_branch: None,
        worktree_dir: None,
        goal: None,
    }
}

/// Append a stage lifecycle event with an explicit `StageScope`, so the
/// stored envelope carries the full `stage_id` (`node_id@visit`). The bare
/// [`workflow_event::append_event`] helper only writes `node_id` because
/// stage lifecycle variants don't carry visit in their payload — production
/// always emits via `Emitter::emit_scoped`.
async fn append_scoped_stage_event(
    state: &Arc<AppState>,
    run_id: RunId,
    node_id: &str,
    visit: u32,
    event: &workflow_event::Event,
) {
    let scope = fabro_workflow::event::StageScope {
        node_id: node_id.to_string(),
        visit,
        parallel_group_id: None,
        parallel_branch_id: None,
    };
    append_event_with_scope(state, run_id, event, &scope).await;
}

async fn append_event_with_scope(
    state: &Arc<AppState>,
    run_id: RunId,
    event: &workflow_event::Event,
    scope: &fabro_workflow::event::StageScope,
) {
    let stored = fabro_workflow::event::to_run_event_at(&run_id, event, Utc::now(), Some(scope));
    let payload = fabro_workflow::event::build_redacted_event_payload(&stored, &run_id).unwrap();
    let run_store = state.stores.runs.open_run(&run_id).await.unwrap();
    run_store.append_event(&payload).await.unwrap();
}

fn stage_status<'a>(body: &'a serde_json::Value, id: &str) -> &'a str {
    body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|stage| stage["id"] == id)
        .and_then(|stage| stage["status"].as_str())
        .unwrap()
}

fn stage_entry<'a>(body: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|stage| stage["id"] == id)
        .unwrap_or_else(|| panic!("stage {id} not found in {body:#?}"))
}

fn test_priced_usage(
    model_id: &str,
    input_tokens: u64,
    output_tokens: u64,
) -> fabro_types::ModelUsage {
    fabro_types::ModelUsage::new(
        ModelRef::new(
            lithos_llm::catalog::builtin::openai(),
            ModelId::new(model_id),
        ),
        Usage {
            tokens: TokenCounts {
                input: input_tokens,
                output: output_tokens,
                ..TokenCounts::default()
            },
            cost:   Some(Cost {
                usd_micros: input_tokens + output_tokens,
                source:     CostSource::Catalog,
            }),
        },
    )
}

async fn create_priced_retry_run(state: &Arc<AppState>, run_id: RunId) {
    create_durable_run_with_events(state, run_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
    ])
    .await;

    append_scoped_stage_event(
        state,
        run_id,
        "verify",
        1,
        &workflow_event::Event::StageFailed {
            node_id:        "verify".to_string(),
            name:           "Verify".to_string(),
            index:          1,
            failure:        FailureDetail::new("try again", FailureCategory::TransientInfra),
            will_retry:     true,
            timing:         fabro_types::StageTiming::wall_only(1200),
            usage_by_model: Vec::new(),
            usage:          Some(test_priced_usage("gpt-old", 100, 10)),
            actor:          None,
        },
    )
    .await;
    append_scoped_stage_event(
        state,
        run_id,
        "verify",
        2,
        &workflow_event::Event::StageCompleted {
            node_id: "verify".to_string(),
            name: "Verify".to_string(),
            index: 1,
            timing: fabro_types::StageTiming::wall_only(800),
            status: "succeeded".to_string(),
            preferred_label: None,
            suggested_next_ids: Vec::new(),
            usage_by_model: Vec::new(),
            usage: Some(test_priced_usage("gpt-new", 200, 20)),
            failure: None,
            notes: None,
            files_touched: Vec::new(),
            context_updates: None,
            jump_to_node: None,
            context_values: None,
            node_visits: None,
            loop_failure_signatures: None,
            restart_failure_signatures: None,
            response: None,
            attempt: 2,
            max_attempts: 2,
        },
    )
    .await;
}

fn revisit_test_started(node_id: &str) -> workflow_event::Event {
    workflow_event::Event::StageStarted {
        graph_visit:           None,
        resumed_from_stage_id: None,
        node_id:               node_id.to_string(),
        name:                  node_id.to_string(),
        index:                 0,
        handler_type:          "command".to_string(),
        attempt:               1,
        max_attempts:          1,
    }
}

fn revisit_test_completed_with_visit(
    node_id: &str,
    duration_ms: u64,
    visit: usize,
) -> workflow_event::Event {
    let mut node_visits = std::collections::BTreeMap::new();
    node_visits.insert(node_id.to_string(), visit);
    workflow_event::Event::StageCompleted {
        node_id: node_id.to_string(),
        name: node_id.to_string(),
        index: 0,
        timing: fabro_types::StageTiming::wall_only(duration_ms),
        status: "succeeded".to_string(),
        preferred_label: None,
        suggested_next_ids: Vec::new(),
        usage_by_model: Vec::new(),
        usage: None,
        failure: None,
        notes: None,
        files_touched: Vec::new(),
        context_updates: None,
        jump_to_node: None,
        context_values: None,
        node_visits: Some(node_visits),
        loop_failure_signatures: None,
        restart_failure_signatures: None,
        response: None,
        attempt: 1,
        max_attempts: 1,
    }
}

async fn append_raw_run_event(
    state: &Arc<AppState>,
    run_id: RunId,
    seq_hint: &str,
    ts: &str,
    event: &str,
    properties: serde_json::Value,
    node_id: Option<&str>,
) {
    let run_store = state.stores.runs.open_run(&run_id).await.unwrap();
    let payload = fabro_store::EventPayload::new(
        json!({
            "id": format!("evt-{seq_hint}"),
            "ts": ts,
            "run_id": run_id,
            "event": event,
            "node_id": node_id,
            "properties": properties,
        }),
        &run_id,
    )
    .unwrap();
    run_store.append_event(&payload).await.unwrap();
}

async fn create_unreadable_durable_run(state: &Arc<AppState>, run_id: RunId) {
    let run_store = state.stores.runs.create_run(&run_id).await.unwrap();
    append_default_run_created(&run_store, run_id).await;
    workflow_event::append_event(&run_store, &run_id, &workflow_event::Event::RunRunnable {
        source: fabro_types::RunRunnableSource::StartRequested,
        actor:  None,
    })
    .await
    .unwrap();
    workflow_event::append_event(&run_store, &run_id, &workflow_event::Event::RunStarting)
        .await
        .unwrap();
    workflow_event::append_event(&run_store, &run_id, &workflow_event::Event::RunRunning)
        .await
        .unwrap();
    let seq = run_store.last_event_seq().await.unwrap().unwrap() + 1;
    let completed = workflow_event::to_run_event_at(
        &run_id,
        &workflow_event::Event::WorkflowRunCompleted {
            timing:               fabro_types::RunTiming::wall_only(1),
            artifact_count:       0,
            status:               "legacy-status".to_string(),
            reason:               SuccessReason::Completed,
            failure:              None,
            final_git_commit_sha: None,
            final_patch:          None,
            diff_summary:         None,
            usage:                None,
        },
        "2026-05-05T20:46:33Z".parse().unwrap(),
        None,
    );
    let payload = workflow_event::build_redacted_event_payload(&completed, &run_id).unwrap();
    fabro_store::test_support::put_unvalidated_run_event(
        &state.stores.runs,
        &run_id,
        seq,
        payload.as_value(),
    )
    .await
    .unwrap();
    let unreadable = state.stores.runs.open_run_reader(&run_id).await;
    let err = unreadable.expect_err("poison event should make the run projection unreadable");
    assert!(
        err.to_string().contains("invalid completed stage status"),
        "unexpected projection error: {err}"
    );
}

fn github_token_settings() -> ServerSettings {
    ServerSettingsBuilder::from_toml(
        r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[server.integrations.github]
strategy = "token"
"#,
    )
    .expect("github token settings fixture should resolve")
}

fn create_github_token_app_state(
    token: Option<&str>,
    github_api_base_url: Option<String>,
) -> Arc<AppState> {
    create_github_token_app_state_with_env_lookup(token, github_api_base_url, |_| None)
}

fn create_github_token_app_state_with_env_lookup(
    token: Option<&str>,
    github_api_base_url: Option<String>,
    env_lookup: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
) -> Arc<AppState> {
    create_github_token_app_state_with_env_lookup_and_llm_catalog_settings(
        token,
        github_api_base_url,
        env_lookup,
        LlmLayer::default(),
    )
}

fn create_github_token_app_state_with_env_lookup_and_llm_catalog_settings(
    token: Option<&str>,
    github_api_base_url: Option<String>,
    env_lookup: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
    llm_overlay: LlmLayer,
) -> Arc<AppState> {
    let (store, artifact_store) = test_store_bundle();
    let vault_path = test_secret_store_path();
    let server_env_path = vault_path.with_file_name("server.env");
    let active_config_path = vault_path.with_file_name("settings.toml");
    if let Some(token) = token {
        Vault::load(vault_path.clone())
            .expect("test vault should load")
            .set("GITHUB_TOKEN", token, SecretType::Token, None)
            .expect("test github token should be writable");
    }
    Vault::load(vault_path.clone())
        .expect("test vault should load")
        .set(
            EnvVars::OPENAI_API_KEY,
            "test-openai-api-key",
            SecretType::Token,
            None,
        )
        .expect("test OpenAI credential should be writable");
    let db_pool = test_db_pool_for_vault_path(&vault_path).expect("test db pool should build");
    let preloaded_vault = crate::test_support::test_secret_snapshot(db_pool.clone())
        .expect("test secret snapshot should build");
    let config = AppStateConfig {
        resolved_settings: resolved_runtime_settings_for_tests(
            github_token_settings(),
            RunLayer::default(),
            llm_overlay,
        ),
        registry_factory_override: None,
        max_concurrent_runs: 5,
        store,
        artifact_store,
        db_pool,
        preloaded_vault,
        server_secrets: load_test_server_secrets(server_env_path, HashMap::new()),
        env_lookup: Arc::new(env_lookup),
        github_api_base_url,
        active_config_path,
        http_client: Some(fabro_http::test_http_client().expect("test HTTP client should build")),
        sandbox_inventory: None,
        shutdown: tokio_util::sync::CancellationToken::new(),
        worker_control_bus: None,
        worker_runtime: None,
        automation_materializer_override: None,
        automation_breaker_notifier_override: None,
    };
    build_app_state(config).expect("test app state should build")
}

/// What a node handler observed about the GitHub credential bridge while the
/// workflow executed.
#[derive(Clone, Debug, PartialEq)]
struct GithubBridgeObservation {
    token_source_present: bool,
    env_has_github_token: bool,
}

/// Wait-node handler that records the credential-bridge state the engine
/// handed to node execution, then delegates to the real wait handler. The
/// capture cannot live on the exit node: terminal nodes complete without a
/// handler dispatch.
struct BridgeCapturingWaitHandler {
    observations: StdArc<StdMutex<Vec<GithubBridgeObservation>>>,
}

#[async_trait::async_trait]
impl fabro_workflow::handler::Handler for BridgeCapturingWaitHandler {
    async fn execute(
        &self,
        node: &fabro_graphviz::graph::Node,
        context: &fabro_workflow::context::Context,
        graph: &fabro_graphviz::graph::Graph,
        run_dir: &Path,
        services: &fabro_workflow::handler::EngineServices,
        _attempt: &fabro_workflow::handler::AttemptInfo,
    ) -> Result<fabro_workflow::outcome::Outcome, fabro_workflow::error::Error> {
        let stage_env = services
            .env_for_stage()
            .await
            .expect("stage env should resolve for the bridge observation");
        self.observations
            .lock()
            .expect("bridge observation lock poisoned")
            .push(GithubBridgeObservation {
                token_source_present: services.github_token.is_some(),
                env_has_github_token: stage_env.contains_key(EnvVars::GITHUB_TOKEN),
            });
        fabro_workflow::handler::wait::WaitHandler
            .execute(node, context, graph, run_dir, services, _attempt)
            .await
    }
}

/// start -> work (wait 1ms) -> exit: the work node is the bridge
/// observation point; the terminal exit node never dispatches a handler.
async fn bridge_intent_run(app: &Router, bearer: Option<&str>, config: &str) -> String {
    let mut intent = test_intent_with_bearer(
        app,
        "workflow.fabro",
        BRIDGE_CAPTURE_DOT,
        Some(config),
        bearer,
    )
    .await;
    // A folder target admits on the LOCAL provider (a `none` target requires
    // a clone-based Docker/Daytona environment, which needs a Docker daemon).
    // The tempdir is deliberately leaked for the run's lifetime.
    let folder = tempfile::tempdir().unwrap().keep();
    intent["target"] = json!({"kind": "folder", "path": folder});
    let mut builder = Request::builder()
        .method("POST")
        .uri(api("/runs"))
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(bearer) = bearer {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {bearer}"));
    }
    let req = builder.body(Body::from(intent.to_string())).unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let body = body_json(response.into_body()).await;
    body["id"].as_str().unwrap().to_string()
}

const BRIDGE_CAPTURE_DOT: &str = r#"digraph Test {
    graph [goal="Test"]
    start [shape=Mdiamond]
    work  [shape=insulator, duration="1ms"]
    exit  [shape=Msquare]
    start -> work
    work -> exit
}"#;

/// Build the (state, router, run_id) triple every PR-endpoint test
/// needs. Use this instead of repeating the
/// state/build_router/fixtures::RUN_1 incantation per test.
fn pr_test_app(
    token: Option<&str>,
    github_api_base_url: Option<String>,
) -> (Arc<AppState>, Router, RunId) {
    let state = create_github_token_app_state(token, github_api_base_url);
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    (state, app, fixtures::RUN_1)
}

/// Same as [`pr_test_app`] but creates a fresh minimal run via the
/// HTTP create-run endpoint instead of using fixtures::RUN_1. For
/// tests that exercise endpoints expecting a real on-disk run rather
/// than a synthetic fixture id.
async fn pr_test_app_with_minimal_run(
    token: Option<&str>,
    github_api_base_url: Option<String>,
) -> (Arc<AppState>, Router, String) {
    let state = create_github_token_app_state(token, github_api_base_url);
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = create_run(&app, MINIMAL_DOT).await;
    (state, app, run_id)
}

/// Same as [`pr_test_app`] but the run is set up as a completed
/// workflow ready for `POST /runs/{id}/pull_request`. The branches
/// and diff are fixed defaults; only the origin URL varies per
/// test (None to test missing-origin rejection, gitlab.com to test
/// non-github rejection, etc.).
async fn pr_test_app_with_completed_run(
    token: Option<&str>,
    github_api_base_url: Option<String>,
    repo_origin_url: Option<&str>,
) -> (Arc<AppState>, Router, RunId) {
    let (state, app, run_id) = pr_test_app(token, github_api_base_url);
    Box::pin(create_completed_run_ready_for_pull_request(
        &state,
        run_id,
        repo_origin_url,
        Some("main"),
        Some("fabro/run/42"),
        "diff --git a/src/lib.rs b/src/lib.rs\n+fn shipped() {}\n",
    ))
    .await;
    (state, app, run_id)
}

async fn create_run_with_pull_request_record(
    state: &Arc<AppState>,
    run_id: RunId,
    pr_url: &str,
    pr_number: u64,
    title: &str,
) {
    create_durable_run_with_events(state, run_id, &[
        workflow_event::Event::PullRequestCreated {
            pr_url: pr_url.to_string(),
            pr_number,
            owner: "acme".to_string(),
            repo: "widgets".to_string(),
            base_branch: "main".to_string(),
            head_branch: "feature".to_string(),
            head_sha: Some("final-sha".to_string()),
            title: title.to_string(),
            draft: false,
            auto_merge: None,
        },
    ])
    .await;
}

async fn create_run_with_linked_pull_request_record(
    state: &Arc<AppState>,
    run_id: RunId,
    pull_request: PullRequestLink,
) {
    create_durable_run_with_events(state, run_id, &[workflow_event::Event::PullRequestLinked {
        pull_request,
    }])
    .await;
}

async fn create_completed_run_ready_for_pull_request(
    state: &Arc<AppState>,
    run_id: RunId,
    repo_origin_url: Option<&str>,
    base_branch: Option<&str>,
    run_branch: Option<&str>,
    final_patch: &str,
) {
    let mut graph = Graph::new("test");
    graph.attrs.insert(
        "goal".to_string(),
        AttrValue::String("Ship the server-side PR".to_string()),
    );
    let git = match (repo_origin_url, base_branch) {
        (Some(origin), Some(branch)) => Some(fabro_types::GitContext {
            origin_url: origin.to_string(),
            branch:     branch.to_string(),
            sha:        None,
            dirty:      fabro_types::DirtyStatus::Clean,
        }),
        _ => None,
    };
    let run_spec = RunSpec {
        run_id,
        settings: fabro_types::WorkflowSettings::default(),
        graph,
        graph_source: None,
        workflow_slug: Some("test".to_string()),
        workflow_version_id: None,
        target: None,
        automation: None,
        source_directory: Some("/tmp/project".to_string()),
        git: git.clone(),
        labels: HashMap::new(),
        provenance: test_support::test_run_provenance(),
        definition_blob: None,
        spec_blob: None,
        fork_source_ref: None,
    };

    create_durable_run_with_events(state, run_id, &[
        workflow_event::Event::RunCreated {
            run_id,
            title: None,
            settings: serde_json::to_value(&run_spec.settings).unwrap(),
            graph: serde_json::to_value(&run_spec.graph).unwrap(),
            workflow_source: None,
            labels: run_spec.labels.clone().into_iter().collect(),
            source_directory: run_spec.source_directory.clone(),
            workflow_slug: run_spec.workflow_slug.clone(),
            workflow_version_id: run_spec.workflow_version_id,
            target: run_spec.target.clone(),
            automation: None,
            provenance: run_spec.provenance.clone(),
            spec_blob: None,
            git,
            fork_source_ref: None,
            retried_from: None,
            parent_id: None,
            web_url: None,
        },
        workflow_event::Event::WorkflowRunStarted {
            name: "test".to_string(),
            run_id,
            base_branch: base_branch.map(str::to_string),
            base_sha: None,
            run_branch: run_branch.map(str::to_string),
            worktree_dir: None,
            goal: Some("Ship the server-side PR".to_string()),
        },
        workflow_event::Event::WorkflowRunCompleted {
            timing:               fabro_types::RunTiming::wall_only(1),
            artifact_count:       0,
            status:               "succeeded".to_string(),
            reason:               SuccessReason::Completed,
            failure:              None,
            final_git_commit_sha: Some("final-sha".to_string()),
            final_patch:          Some(final_patch.to_string()),
            diff_summary:         None,
            usage:                None,
        },
    ])
    .await;
}

fn test_event_envelope(seq: u32, run_id: RunId, body: EventBody) -> EventEnvelope {
    EventEnvelope {
        seq,
        event: RunEvent {
            id: format!("evt-{seq}"),
            ts: Utc::now(),
            run_id,
            node_id: None,
            node_label: None,
            stage_id: None,
            parallel_group_id: None,
            parallel_branch_id: None,
            session_id: None,
            parent_session_id: None,
            tool_call_id: None,
            actor: None,
            body,
        },
    }
}

// ---------------------------------------------------------------------------
// Staleness supervisor (fabro-94e8): update-branch on dirty run PRs, cap/age
// close with a durable pull_request.closed event.
// ---------------------------------------------------------------------------

/// Wire fixtures for one staleness pass over a run-linked PR #42.
fn dirty_run_pull_request_mock<'a>(
    github: &'a MockServer,
    pr_state: &str,
    mergeable: bool,
    mergeable_state: &str,
    created_at: &str,
) -> httpmock::Mock<'a> {
    github.mock(|when, then| {
        when.method("GET")
            .path("/repos/acme/widgets/pulls/42")
            .header("authorization", "Bearer ghu_test");
        then.status(200).json_body(json!({
            "number": 42,
            "title": "Ship it",
            "body": "",
            "state": pr_state,
            "draft": false,
            "merged": false,
            "mergeable": mergeable,
            "mergeable_state": mergeable_state,
            "additions": 1,
            "deletions": 0,
            "changed_files": 1,
            "html_url": "https://github.com/acme/widgets/pull/42",
            "user": { "login": "octocat" },
            "head": { "ref": "fabro/run/42" },
            "base": { "ref": "main" },
            "created_at": created_at,
            "updated_at": created_at
        }));
    })
}

fn update_branch_mock(github: &MockServer, status: u16) -> httpmock::Mock<'_> {
    github.mock(move |when, then| {
        when.method("PUT")
            .path("/repos/acme/widgets/pulls/42/update-branch")
            .header("authorization", "Bearer ghu_test");
        then.status(status)
            .json_body(json!({ "message": "Updating pull request branch." }));
    })
}

fn close_branch_mock(github: &MockServer) -> httpmock::Mock<'_> {
    github.mock(|when, then| {
        when.method("PATCH")
            .path("/repos/acme/widgets/pulls/42")
            .header("authorization", "Bearer ghu_test")
            .json_body(json!({ "state": "closed" }));
        then.status(200).json_body(json!({
            "number": 42,
            "state": "closed",
        }));
    })
}

/// Files changed on one side of a `compare/{base}...{head}` (fabro-895d).
fn compare_files_mock<'a>(
    github: &'a MockServer,
    basehead: &str,
    filenames: &[&str],
) -> httpmock::Mock<'a> {
    let files: Vec<_> = filenames
        .iter()
        .map(|filename| json!({ "filename": filename }))
        .collect();
    github.mock(move |when, then| {
        when.method("GET")
            .path(format!("/repos/acme/widgets/compare/{basehead}"))
            .header("authorization", "Bearer ghu_test");
        then.status(200).json_body(json!({
            "status": "diverged",
            "files": files,
        }));
    })
}

/// Files changed by the pull request (age-cap bookkeeping check, fabro-895d).
fn pr_files_mock<'a>(
    github: &'a MockServer,
    number: u64,
    filenames: &[&str],
) -> httpmock::Mock<'a> {
    let files: Vec<_> = filenames
        .iter()
        .map(|filename| json!({ "filename": filename }))
        .collect();
    github.mock(move |when, then| {
        when.method("GET")
            .path(format!("/repos/acme/widgets/pulls/{number}/files"))
            .header("authorization", "Bearer ghu_test");
        then.status(200).json_body(files);
    })
}

/// Raw file content served by the contents API (fabro-895d).
fn raw_contents_mock<'a>(
    github: &'a MockServer,
    path: &str,
    git_ref: &str,
    body: &str,
) -> httpmock::Mock<'a> {
    github.mock(move |when, then| {
        when.method("GET")
            .path(format!("/repos/acme/widgets/contents/{path}"))
            .query_param("ref", git_ref)
            .header("authorization", "Bearer ghu_test");
        then.status(200).body(body);
    })
}

/// `GET /repos/.../commits/{git_ref}` answering commit sha and tree sha.
fn commit_info_mock<'a>(
    github: &'a MockServer,
    git_ref: &str,
    sha: &str,
    tree: &str,
) -> httpmock::Mock<'a> {
    github.mock(move |when, then| {
        when.method("GET")
            .path(format!("/repos/acme/widgets/commits/{git_ref}"))
            .header("authorization", "Bearer ghu_test");
        then.status(200).json_body(json!({
            "sha": sha,
            "commit": { "tree": { "sha": tree } },
        }));
    })
}

/// `POST /repos/.../git/{suffix}` (blobs, trees, commits) answering a sha.
fn git_object_create_mock<'a>(
    github: &'a MockServer,
    suffix: &str,
    response_sha: &str,
) -> httpmock::Mock<'a> {
    github.mock(move |when, then| {
        when.method("POST")
            .path(format!("/repos/acme/widgets/{suffix}"))
            .header("authorization", "Bearer ghu_test");
        then.status(201).json_body(json!({ "sha": response_sha }));
    })
}

/// Fast-forward push of the run branch head (git refs API, fabro-895d).
fn push_ref_mock<'a>(github: &'a MockServer, branch: &str, status: u16) -> httpmock::Mock<'a> {
    github.mock(move |when, then| {
        when.method("PATCH")
            .path(format!("/repos/acme/widgets/git/refs/heads/{branch}"))
            .header("authorization", "Bearer ghu_test")
            .json_body(json!({ "sha": "m1", "force": false }));
        then.status(status)
            .json_body(json!({ "object": { "sha": "m1" } }));
    })
}

async fn staleness_test_run(state: &Arc<AppState>, run_id: RunId) {
    create_run_with_linked_pull_request_record(state, run_id, PullRequestLink {
        owner:  "acme".to_string(),
        repo:   "widgets".to_string(),
        number: 42,
    })
    .await;
}

async fn run_pull_request_closed_events(state: &AppState, run_id: &RunId) -> Vec<(String, String)> {
    let run_store = state.stores.runs.open_run(run_id).await.unwrap();
    let events = run_store.list_events().await.unwrap();
    events
        .iter()
        .filter_map(|envelope| match &envelope.event.body {
            fabro_types::run_event::EventBody::PullRequestClosed(props) => {
                Some((props.close_reason.clone(), props.pull_request.html_url()))
            }
            _ => None,
        })
        .collect()
}

/// Builds an app state over shared object, blob, and summary stores so a test
/// can drop it and open a second state that sees the same durable data.
fn test_app_state_over_shared_stores(
    object_store: &Arc<dyn object_store::ObjectStore>,
    blobs: &Arc<fabro_store::BlobStore>,
    summaries: &Arc<fabro_store::RunSummaryStore>,
) -> Arc<AppState> {
    let store = Arc::new(fabro_store::test_support::test_database_with_stores(
        Arc::clone(object_store),
        "runs",
        std::time::Duration::from_millis(1),
        None,
        Arc::clone(blobs),
        Arc::clone(summaries),
    ));
    test_app_state_with_store(
        default_test_server_settings(),
        RunLayer::default(),
        5,
        store,
        ArtifactStore::new(Arc::clone(object_store), "artifacts"),
    )
}

fn issue_test_inspects_worker_token(run_id: &RunId, inspects: &[String]) -> String {
    let keys = WorkerTokenKeys::from_master_secret(TEST_SESSION_SECRET.as_bytes())
        .expect("worker keys should derive");
    crate::worker_token::issue_worker_token_with_scopes(
        &keys,
        run_id,
        &crate::worker_token::WorkerScopeSet::run_worker_with_agent_run_tools_and_inspects(
            inspects,
        ),
    )
    .expect("worker token should issue")
}

fn unique_run_id() -> RunId {
    RunId::new()
}

async fn create_run_with_workflow_slug(state: &Arc<AppState>, run_id: RunId, workflow_slug: &str) {
    create_slack_notification_run(
        state,
        run_id,
        fabro_types::WorkflowSettings::default(),
        "revisor-scope",
        Some(workflow_slug),
    )
    .await;
}

/// Store-level parent-link corruption for authorization tests: the link
/// API rejects cycles, so a cyclic chain can only be written directly as
/// events.
async fn append_parent_linked_event(state: &Arc<AppState>, child: RunId, parent: RunId) {
    let run_store = state.stores.runs.open_run(&child).await.unwrap();
    workflow_event::append_event(
        &run_store,
        &child,
        &workflow_event::Event::RunParentLinked {
            previous_parent_id: None,
            parent_id:          parent,
            actor:              None,
        },
    )
    .await
    .unwrap();
}

fn insert_running_control_run(
    state: &Arc<AppState>,
    run_id: RunId,
    answer_transport: Option<RunAnswerTransport>,
) -> tempfile::TempDir {
    let temp_dir = tempfile::tempdir().unwrap();
    let mut run = managed_run(
        String::new(),
        RunStatus::Running,
        chrono::Utc::now(),
        temp_dir.path().join(run_id.to_string()),
        RunExecutionMode::Start,
    );
    run.answer_transport = answer_transport;
    state
        .runs
        .lock()
        .expect("runs lock poisoned")
        .insert(run_id, run);
    temp_dir
}

fn acp_event_for_stage(run_id: &RunId, event: &workflow_event::Event) -> fabro_types::RunEvent {
    workflow_event::to_run_event_at(
        run_id,
        event,
        Utc::now(),
        Some(&workflow_event::StageScope {
            node_id:            "agent".to_string(),
            visit:              1,
            parallel_group_id:  None,
            parallel_branch_id: None,
        }),
    )
}

#[cfg(unix)]
fn write_test_executable(script: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir should exist");
    let path = dir.path().join("fake-fabro");
    std::fs::write(&path, script).expect("script should be written");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("script should be executable");
    (dir, path)
}

#[cfg(unix)]
async fn render_graph_with_override(dot_source: &str, exe_path: &Path) -> Response {
    render_graph_bytes_with_exe_override(dot_source, Some(exe_path)).await
}

fn run_submitted_event() -> workflow_event::Event {
    workflow_event::Event::RunSubmitted {
        definition_blob: None,
    }
}

fn workflow_completed_event() -> workflow_event::Event {
    workflow_event::Event::WorkflowRunCompleted {
        timing:               fabro_types::RunTiming::wall_only(1000),
        artifact_count:       0,
        status:               "succeeded".to_string(),
        reason:               SuccessReason::Completed,
        failure:              None,
        final_git_commit_sha: None,
        final_patch:          None,
        diff_summary:         None,
        usage:                None,
    }
}

async fn create_succeeded_run(state: &Arc<AppState>, run_id: RunId) {
    create_durable_run_with_events(state, run_id, &[
        run_submitted_event(),
        workflow_completed_event(),
    ])
    .await;
}

async fn create_running_run(state: &Arc<AppState>, run_id: RunId) {
    create_durable_run_with_events(state, run_id, &[
        run_submitted_event(),
        workflow_event::Event::RunRunning,
    ])
    .await;
}

async fn create_preserved_local_sandbox_run(state: &Arc<AppState>, run_id: RunId) {
    let mut settings = fabro_types::WorkflowSettings::default();
    settings.run.environment.lifecycle.preserve = true;
    let graph = Graph::new("test");

    create_durable_run_with_events(state, run_id, &[
        workflow_event::Event::RunCreated {
            run_id,
            title: None,
            settings: serde_json::to_value(settings).unwrap(),
            graph: serde_json::to_value(graph).unwrap(),
            workflow_source: None,
            labels: std::collections::BTreeMap::default(),
            source_directory: Some("/tmp/fabro-run".to_string()),
            workflow_slug: Some("test".to_string()),
            workflow_version_id: None,
            target: None,
            automation: None,
            provenance: test_support::test_run_provenance(),
            spec_blob: None,
            git: None,
            fork_source_ref: None,
            retried_from: None,
            parent_id: None,
            web_url: None,
        },
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::SandboxInitialized {
            provider:          SandboxProviderKind::LOCAL,
            id:                "sandbox-preserve-1".to_string(),
            working_directory: "/tmp/fabro-preserved-sandbox".to_string(),
            image:             None,
            snapshot:          None,
            repo_cloned:       None,
            clone_origin_url:  None,
            clone_branch:      None,
            workspace_root:    None,
            repos_root:        None,
            primary_repo_path: None,
            primary_repo_link: None,
        },
    ])
    .await;
}

fn batch_lifecycle_body(run_ids: &[RunId]) -> serde_json::Value {
    json!({
        "run_ids": run_ids.iter().map(ToString::to_string).collect::<Vec<_>>(),
    })
}

fn batch_delete_body(run_ids: &[RunId], force: bool) -> serde_json::Value {
    json!({
        "run_ids": run_ids.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "force": force,
    })
}

fn assert_batch_result(result: &serde_json::Value, run_id: RunId, ok: bool, outcome: &str) {
    assert_eq!(result["run_id"], run_id.to_string());
    assert_eq!(result["ok"], ok);
    assert_eq!(result["outcome"], outcome);
    if ok {
        assert!(
            result["run"].is_object(),
            "successful result should include run: {result}"
        );
        assert!(
            result["error"].is_null(),
            "successful result should omit error: {result}"
        );
    } else {
        assert!(
            result["error"].is_object(),
            "failed result should include error: {result}"
        );
        assert!(
            result["run"].is_null(),
            "failed result should omit run: {result}"
        );
    }
}

fn assert_batch_delete_result(result: &serde_json::Value, run_id: RunId, ok: bool, outcome: &str) {
    assert_eq!(result["run_id"], run_id.to_string());
    assert_eq!(result["ok"], ok);
    assert_eq!(result["outcome"], outcome);
    if ok {
        assert!(
            result["error"].is_null(),
            "successful delete result should omit error: {result}"
        );
    } else {
        assert!(
            result["error"].is_object(),
            "failed delete result should include error: {result}"
        );
    }
}

async fn advance_past_worker_cancel_grace() {
    tokio::task::yield_now().await;
    tokio::time::advance(WORKER_CANCEL_GRACE).await;
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
}

mod authentication;
mod completions;
mod events;
mod github_webhooks;
mod graph;
mod lifecycle;
mod models;
mod pair;
mod pull_requests;
mod runs;
mod sandbox;
mod secrets;
mod sessions;
mod slack;
mod system;
mod usage;
mod worker_control;
mod workflow_versions;
