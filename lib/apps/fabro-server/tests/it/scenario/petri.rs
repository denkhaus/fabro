//! Runs on Petri through the server: every run executes on Petri, Petri's
//! record of the run agrees with Fabro's status, and Petri's diagnostics
//! refuse a run at create.
//!
//! The runs here execute in the server process under the handler-registry
//! test override; outside it the scheduler launches a worker for a Petri
//! run, which the CLI's scenario tests cover with the real binary
//! (`lib/apps/fabro-cli/tests/it/scenario/petri.rs`).
//!
//! The runs that execute take their host scope through the sandbox-driver
//! host plugin, so those tests skip, and say why, when the executable is not
//! found, unless `FABRO_REQUIRE_SANDBOX_PLUGINS` is set. The create-time
//! refusals need no plugin and always run.

#![expect(
    clippy::disallowed_methods,
    reason = "the tests locate the plugin executable through the process environment"
)]
#![expect(clippy::print_stderr, reason = "a skipped test says why on its stderr")]

use std::collections::BTreeMap;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use fabro_petri::engine::{self, RunStatus};
use fabro_petri::{SqliteRunStore, projector};
use fabro_server::server::AppState;
use fabro_server::test_support::{
    TestAppStateBuilder, llm_overlay_with_provider_base_url, test_app_db_pool,
    test_register_workflow_version,
};
use fabro_static::EnvVars;
use fabro_store::platform_records::{PlatformRecord, PlatformRecordKind, PlatformRecordStore};
use fabro_test::{TwinScenario, TwinScenarios, twin_openai};
use fabro_types::{RunId, WorkflowPath, WorkflowVersion};
use tower::ServiceExt;

use crate::helpers::{
    api, create_and_start_run_from_intent, read_repo_file, response_json, run_json,
    settings_from_toml, test_app_state_with_options, test_app_with_scheduler, test_settings,
    wait_for_run_status,
};

const HOST_PLUGIN: &str = "sandbox-driver-host";
const HOST_PLUGIN_OVERRIDE: &str = "PETRI_SANDBOX_HOST_PLUGIN";
const REQUIRE_ENV: &str = "FABRO_REQUIRE_SANDBOX_PLUGINS";

const OPENAI_MODEL: &str = "gpt-5.4";

/// A command-only workflow: one script stage between start and exit.
const COMMAND_DOT: &str = r#"digraph Command {
    graph [goal="Run one command"]
    start [shape=Mdiamond]
    exit [shape=Msquare]
    say [shape=parallelogram, script="echo hello from petri"]
    start -> say -> exit
}"#;

/// A workflow whose one stage carries an attribute the language does not
/// have.
const UNKNOWN_ATTRIBUTE_DOT: &str = r#"digraph Bad {
    graph [goal="Refuse me"]
    start [shape=Mdiamond]
    exit [shape=Msquare]
    work [shape=box, prompt="Do the work", bogus="yes"]
    start -> work -> exit
}"#;

/// A workflow with an edge to a node nobody declared.
const UNDECLARED_NODE_DOT: &str = r#"digraph Bad {
    graph [goal="Refuse me"]
    start [shape=Mdiamond]
    exit [shape=Msquare]
    work [shape=box, prompt="Do the work"]
    start -> work -> nowhere -> exit
}"#;

/// A workflow whose stage names a model no catalog has.
const UNKNOWN_MODEL_DOT: &str = r#"digraph Bad {
    graph [goal="Refuse me"]
    start [shape=Mdiamond]
    exit [shape=Msquare]
    work [shape=box, prompt="Do the work", model="no-such-model-9000"]
    start -> work -> exit
}"#;

/// Two command branches joined by a fan-in.
const PARALLEL_DOT: &str = r#"digraph Parallel {
    graph [goal="Run two branches"]
    start [shape=Mdiamond]
    exit [shape=Msquare]
    fork [shape=component]
    a [shape=parallelogram, script="echo a"]
    b [shape=parallelogram, script="echo b"]
    merge [shape=tripleoctagon]
    start -> fork
    fork -> a
    fork -> b
    a -> merge
    b -> merge
    merge -> exit
}"#;

pub(super) const PLAIN_SETTINGS: &str = "_version = 1\n\n[workflow]\ngraph = \"workflow.fabro\"\n";

/// The host plugin as Petri's lookup finds it: the override variable, else
/// the executable on `PATH`. `None`, after saying so, when the test should
/// skip; a panic when the environment forbids a skip.
pub(super) fn host_plugin() -> Option<PathBuf> {
    let found = env::var_os(HOST_PLUGIN_OVERRIDE)
        .map(PathBuf::from)
        .or_else(|| {
            env::split_paths(&env::var_os("PATH")?)
                .map(|dir| dir.join(HOST_PLUGIN))
                .find(|candidate| candidate.is_file())
        });
    if found.is_none() {
        assert!(
            env::var_os(REQUIRE_ENV).is_none(),
            "{REQUIRE_ENV} is set, but {HOST_PLUGIN} is not on PATH and {HOST_PLUGIN_OVERRIDE} is unset"
        );
        eprintln!("skipping: {HOST_PLUGIN} is not on PATH and {HOST_PLUGIN_OVERRIDE} is unset");
    }
    found
}

/// Register a version whose entrypoint is `workflow.fabro`, with the given
/// files beside it.
pub(super) async fn register_version(app: &axum::Router, files: &[(&str, &str)]) -> String {
    let entrypoint = WorkflowPath::new("workflow.fabro").expect("entrypoint path is valid");
    let files = files
        .iter()
        .map(|(path, text)| {
            (
                WorkflowPath::new(*path).expect("fixture path is valid"),
                (*text).to_string(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let version =
        WorkflowVersion::new(entrypoint, files, BTreeMap::new()).expect("fixture version is valid");
    test_register_workflow_version(app, &version, None)
        .await
        .to_string()
}

pub(super) fn intent(version_id: &str, workspace: &std::path::Path) -> serde_json::Value {
    serde_json::json!({
        "workflow_version_id": version_id,
        "target": {"kind": "folder", "path": workspace},
        "environment_id": "local",
        "args": {},
    })
}

/// The `hello` bundle checked into this repository.
fn hello_files() -> [(&'static str, String); 2] {
    let workflow = read_repo_file(".fabro/workflows/hello/workflow.fabro");
    let settings = read_repo_file(".fabro/workflows/hello/workflow.toml");
    [("workflow.fabro", workflow), ("workflow.toml", settings)]
}

/// The run's record in Petri's store, read through the same database the
/// server wrote it to.
async fn petri_outcome(state: &AppState, run_id: &str) -> engine::RunOutcome {
    let store = SqliteRunStore::new(test_app_db_pool(state));
    engine::outcome_of(&store, run_id)
        .await
        .expect("the run's Petri record inspects")
}

/// The run's projected state once its projector settled.
pub(super) async fn settled_state(
    state: &AppState,
    app: &axum::Router,
    run_id: &str,
) -> serde_json::Value {
    let id: RunId = run_id.parse().expect("the run id parses");
    state.test_petri_projector().settle(id).await;
    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{run_id}/state")))
        .body(Body::empty())
        .expect("state request should build");
    let response = app
        .clone()
        .oneshot(req)
        .await
        .expect("state request routes");
    response_json(
        response,
        StatusCode::OK,
        format!("GET /api/v1/runs/{run_id}/state"),
    )
    .await
}

/// How many items the run's projected stream holds.
async fn petri_stream_len(state: &AppState, run_id: &str) -> usize {
    let id: RunId = run_id.parse().expect("the run id parses");
    projector::stored_stream(&state.test_petri_view_pool(), id)
        .await
        .expect("the stream reads")
        .len()
}

async fn run_admission(app: &axum::Router, run_id: &str) -> serde_json::Value {
    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{run_id}/state")))
        .body(Body::empty())
        .expect("state request should build");
    let response = app
        .clone()
        .oneshot(req)
        .await
        .expect("state request routes");
    let body = response_json(
        response,
        StatusCode::OK,
        format!("GET /api/v1/runs/{run_id}/state"),
    )
    .await;
    body["spec"]["admission"].clone()
}

async fn create_run_response(app: &axum::Router, intent: serde_json::Value) -> serde_json::Value {
    let req = Request::builder()
        .method("POST")
        .uri(api("/runs"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&intent).expect("intent serializes"),
        ))
        .expect("create-run request should build");
    let response = app
        .clone()
        .oneshot(req)
        .await
        .expect("create request routes");
    response_json(
        response,
        StatusCode::UNPROCESSABLE_ENTITY,
        "POST /api/v1/runs",
    )
    .await
}

/// The `hello` bundle, whose one stage is a prompt, runs on Petri: the
/// prompt reaches the twin through Petri's model client, Fabro reports the
/// run succeeded, and Petri's record of the run says the same.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_hello_bundle_runs_on_petri() {
    if host_plugin().is_none() {
        return;
    }
    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let twin = twin_openai().await;
    let namespace = format!("{}::{}", module_path!(), line!());
    TwinScenarios::new(&namespace)
        .scenario(TwinScenario::responses(OPENAI_MODEL).text("A haiku, added."))
        .load(twin)
        .await;
    let settings = test_settings();
    // The handler-registry override is the test switch that keeps a Petri
    // run in this process; without it the scheduler launches a worker.
    let state = TestAppStateBuilder::new()
        .runtime_settings(settings.server_settings, settings.manifest_run_defaults)
        .max_concurrent_runs(5)
        .registry_factory(|interviewer| {
            fabro_workflow::handler::default_registry(interviewer, || None)
        })
        .llm_overlay(llm_overlay_with_provider_base_url(
            "openai",
            twin.base_url.clone(),
        ))
        .vault_entries([(EnvVars::OPENAI_API_KEY, namespace.clone())])
        .build();
    let app = test_app_with_scheduler(Arc::clone(&state));

    let [(workflow_path, workflow), (settings_path, settings)] = hello_files();
    let version_id = register_version(&app, &[
        (workflow_path, &workflow),
        (settings_path, &settings),
    ])
    .await;
    let mut intent = intent(&version_id, workspace.path());
    intent["args"]["model"] = serde_json::json!(OPENAI_MODEL);
    let run_id = create_and_start_run_from_intent(&app, intent).await;

    let status = wait_for_run_status(&app, &run_id, &["succeeded", "failed"]).await;
    let run = run_json(&app, &run_id).await;
    assert_eq!(status, "succeeded", "run: {run}");
    assert!(
        run_admission(&app, &run_id).await["graph"]["digest"].is_string(),
        "the run's spec names what Petri admitted"
    );
    let outcome = petri_outcome(&state, &run_id).await;
    assert_eq!(outcome.status, RunStatus::Success, "{outcome:?}");
    assert!(outcome.complete, "{:?}", outcome.incomplete);
    let projection = settled_state(&state, &app, &run_id).await;
    assert_eq!(projection["status"]["kind"], "succeeded", "{projection}");
    assert_eq!(
        projection["conclusion"]["status"], "succeeded",
        "{projection}"
    );
    let greet = &projection["stages"]["greet@1"];
    assert_eq!(greet["state"], "succeeded", "{projection}");
    assert_eq!(greet["handler"], "agent", "{greet}");
    assert!(
        greet["response"]
            .as_str()
            .is_some_and(|response| response.contains("A haiku, added.")),
        "the agent's answer is projected as the stage's response: {greet}"
    );
    assert!(run["usage"]["tokens"]["input"].as_u64().is_some(), "{run}");
    let logs = twin.request_logs(&namespace).await;
    let requests = logs["requests"]
        .as_array()
        .expect("twin request logs are an array");
    assert!(
        requests
            .iter()
            .any(|request| request["model"] == OPENAI_MODEL),
        "the prompt stage should have called the twin, got {logs}"
    );
    super::petri_stream::capture_settled(&state, &app, &run_id, "hello").await;
}

/// A command-only bundle runs on Petri, and Petri's record agrees.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_command_bundle_runs_on_petri() {
    if host_plugin().is_none() {
        return;
    }
    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let settings = settings_from_toml("_version = 1\n\n[run.environment]\nid = \"local\"\n");
    let state = test_app_state_with_options(settings, 5);
    let app = test_app_with_scheduler(Arc::clone(&state));

    let version_id = register_version(&app, &[
        ("workflow.fabro", COMMAND_DOT),
        ("workflow.toml", PLAIN_SETTINGS),
    ])
    .await;
    let run_id =
        create_and_start_run_from_intent(&app, intent(&version_id, workspace.path())).await;

    let status = wait_for_run_status(&app, &run_id, &["succeeded", "failed"]).await;
    let run = run_json(&app, &run_id).await;
    assert_eq!(status, "succeeded", "run: {run}");
    assert!(
        run_admission(&app, &run_id).await["graph"]["digest"].is_string(),
        "the run's spec names what Petri admitted"
    );
    let outcome = petri_outcome(&state, &run_id).await;
    assert_eq!(outcome.status, RunStatus::Success, "{outcome:?}");
    assert!(outcome.complete, "{:?}", outcome.incomplete);
    let projection = settled_state(&state, &app, &run_id).await;
    let say = &projection["stages"]["say@1"];
    assert_eq!(say["state"], "succeeded", "{projection}");
    assert_eq!(say["handler"], "command", "{say}");
    assert!(
        say["output"]
            .as_str()
            .is_some_and(|output| output.contains("hello from petri")),
        "{say}"
    );
    let stream = petri_stream_len(&state, &run_id).await;
    assert!(stream > 0, "the run's stream holds its events");
    super::petri_stream::capture_settled(&state, &app, &run_id, "command").await;
}

/// A parallel bundle with two command branches runs on Petri through the
/// server: each branch is a child execution, projected as a stage grouped
/// under the fork, and the fork carries the branch results.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_parallel_bundle_projects_its_branches_through_the_server() {
    if host_plugin().is_none() {
        return;
    }
    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let settings = settings_from_toml("_version = 1\n\n[run.environment]\nid = \"local\"\n");
    let state = test_app_state_with_options(settings, 5);
    let app = test_app_with_scheduler(Arc::clone(&state));

    let version_id = register_version(&app, &[
        ("workflow.fabro", PARALLEL_DOT),
        ("workflow.toml", PLAIN_SETTINGS),
    ])
    .await;
    let run_id =
        create_and_start_run_from_intent(&app, intent(&version_id, workspace.path())).await;

    let status = wait_for_run_status(&app, &run_id, &["succeeded", "failed"]).await;
    let run = run_json(&app, &run_id).await;
    assert_eq!(status, "succeeded", "run: {run}");
    let projection = settled_state(&state, &app, &run_id).await;
    for (branch, index) in [("a@1", 0), ("b@1", 1)] {
        let stage = &projection["stages"][branch];
        assert_eq!(stage["state"], "succeeded", "{branch}: {projection}");
        assert_eq!(
            stage["parallel_branch_id"],
            format!("fork@1:{index}"),
            "{stage}"
        );
    }
    let fork = &projection["stages"]["fork@1"];
    assert_eq!(
        fork["parallel_results"].as_array().map(Vec::len),
        Some(2),
        "{fork}"
    );
    assert_eq!(
        projection["conclusion"]["status"], "succeeded",
        "{projection}"
    );
}

/// A workflow with an attribute the language does not have is refused at
/// create with Petri's code in Fabro's diagnostic shape.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_attribute_is_refused_at_create_with_petris_code() {
    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let state = test_app_state_with_options(test_settings(), 5);
    let app = test_app_with_scheduler(state);

    let version_id = register_version(&app, &[
        ("workflow.fabro", UNKNOWN_ATTRIBUTE_DOT),
        ("workflow.toml", PLAIN_SETTINGS),
    ])
    .await;
    let body = create_run_response(&app, intent(&version_id, workspace.path())).await;

    let detail = body["errors"][0]["detail"].as_str().unwrap_or_default();
    assert_eq!(body["errors"][0]["code"], "run_compile_invalid", "{body}");
    assert!(
        detail.contains("attractor.unknown_attribute") && detail.contains("bogus"),
        "expected Petri's diagnostic in the detail, got {body}"
    );
}

/// An edge to a node nobody declared is refused at create with Petri's code.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_edge_to_an_undeclared_node_is_refused_at_create_with_petris_code() {
    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let state = test_app_state_with_options(test_settings(), 5);
    let app = test_app_with_scheduler(state);

    let version_id = register_version(&app, &[
        ("workflow.fabro", UNDECLARED_NODE_DOT),
        ("workflow.toml", PLAIN_SETTINGS),
    ])
    .await;
    let body = create_run_response(&app, intent(&version_id, workspace.path())).await;

    let detail = body["errors"][0]["detail"].as_str().unwrap_or_default();
    assert!(
        detail.contains("attractor.undeclared_node") && detail.contains("nowhere"),
        "expected Petri's diagnostic in the detail, got {body}"
    );
}

/// A model selector the catalog cannot resolve is refused at create with
/// `attractor.model.unknown`: Petri's admission pass pins every model
/// against the server's catalog, so nothing is left for a run to discover.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_model_is_refused_at_create_with_attractor_model_unknown() {
    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let state = test_app_state_with_options(test_settings(), 5);
    let app = test_app_with_scheduler(state);

    let version_id = register_version(&app, &[
        ("workflow.fabro", UNKNOWN_MODEL_DOT),
        ("workflow.toml", PLAIN_SETTINGS),
    ])
    .await;
    let body = create_run_response(&app, intent(&version_id, workspace.path())).await;

    let detail = body["errors"][0]["detail"].as_str().unwrap_or_default();
    assert!(
        detail.contains("attractor.model.unknown") && detail.contains("no-such-model-9000"),
        "expected the admission diagnostic in the detail, got {body}"
    );
}

/// A yes/no gate whose branches each leave a marker file.
fn gate_dot(markers: &std::path::Path) -> String {
    format!(
        r#"digraph Gate {{
    graph [goal="Ask before running"]
    start [shape=Mdiamond]
    exit [shape=Msquare]
    gate [shape=hexagon, label="Go?", question_type="yes_no"]
    yes [shape=parallelogram, script="touch {dir}/yes"]
    no [shape=parallelogram, script="touch {dir}/no"]
    start -> gate
    gate -> yes [label="[Y] Yes"]
    gate -> no [label="[N] No"]
    yes -> exit
    no -> exit
}}"#,
        dir = markers.display()
    )
}

/// The run's first pending question, once one is listed.
async fn wait_for_question(app: &axum::Router, run_id: &str) -> serde_json::Value {
    for _ in 0..600 {
        let req = Request::builder()
            .method("GET")
            .uri(api(&format!("/runs/{run_id}/questions")))
            .body(Body::empty())
            .expect("questions request should build");
        let response = app
            .clone()
            .oneshot(req)
            .await
            .expect("questions request routes");
        let body = response_json(
            response,
            StatusCode::OK,
            format!("GET /api/v1/runs/{run_id}/questions"),
        )
        .await;
        if let Some(question) = body["data"].as_array().and_then(|items| items.first()) {
            return question.clone();
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("run {run_id} never asked a question");
}

/// A human gate in a Petri run asks through the questions API and is
/// answered through it: the question is listed with the gate's stage and
/// options, the answer routes the gate, and the run's stream records the
/// interview as a legacy stage's would.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_human_gate_is_answered_through_the_questions_api() {
    if host_plugin().is_none() {
        return;
    }
    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let markers = tempfile::tempdir().expect("marker tempdir");
    let settings = settings_from_toml("_version = 1\n\n[run.environment]\nid = \"local\"\n");
    let state = test_app_state_with_options(settings, 5);
    let app = test_app_with_scheduler(Arc::clone(&state));

    let dot = gate_dot(markers.path());
    let version_id = register_version(&app, &[
        ("workflow.fabro", &dot),
        ("workflow.toml", PLAIN_SETTINGS),
    ])
    .await;
    let run_id =
        create_and_start_run_from_intent(&app, intent(&version_id, workspace.path())).await;

    let question = wait_for_question(&app, &run_id).await;
    assert_eq!(question["stage"], "gate@1", "{question}");
    assert_eq!(question["text"], "Go?", "{question}");
    assert_eq!(question["question_type"], "yes_no", "{question}");
    let keys: Vec<&str> = question["options"]
        .as_array()
        .expect("options")
        .iter()
        .filter_map(|option| option["key"].as_str())
        .collect();
    assert_eq!(keys, vec!["Y", "N"], "{question}");
    let question_id = question["id"].as_str().expect("an id").to_string();
    assert!(
        question_id.starts_with("gate#"),
        "Petri's id: {question_id}"
    );

    // Petri's id travels as one percent-encoded path segment, as the
    // generated clients send it.
    let encoded_id =
        percent_encoding::utf8_percent_encode(&question_id, percent_encoding::NON_ALPHANUMERIC)
            .to_string();
    let req = Request::builder()
        .method("POST")
        .uri(api(&format!(
            "/runs/{run_id}/questions/{encoded_id}/answer"
        )))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"kind":"no"}"#))
        .expect("answer request should build");
    let response = app
        .clone()
        .oneshot(req)
        .await
        .expect("answer request routes");
    crate::helpers::response_status(
        response,
        StatusCode::NO_CONTENT,
        format!("POST /api/v1/runs/{run_id}/questions/{question_id}/answer"),
    )
    .await;

    let status = wait_for_run_status(&app, &run_id, &["succeeded", "failed"]).await;
    let run = run_json(&app, &run_id).await;
    assert_eq!(status, "succeeded", "run: {run}");
    assert!(
        markers.path().join("no").exists() && !markers.path().join("yes").exists(),
        "the no branch ran"
    );
    let outcome = petri_outcome(&state, &run_id).await;
    assert_eq!(outcome.status, RunStatus::Success, "{outcome:?}");
    let state_body = {
        let req = Request::builder()
            .method("GET")
            .uri(api(&format!("/runs/{run_id}/state")))
            .body(Body::empty())
            .expect("state request should build");
        response_json(
            app.clone()
                .oneshot(req)
                .await
                .expect("state request routes"),
            StatusCode::OK,
            format!("GET /api/v1/runs/{run_id}/state"),
        )
        .await
    };
    assert!(
        state_body["pending_interviews"]
            .as_object()
            .is_some_and(serde_json::Map::is_empty),
        "the answered question is no longer pending: {}",
        state_body["pending_interviews"]
    );
    // Who answered is a platform record keyed on Petri's id, derived from
    // the adapter's `interview.completed` with the API caller as its actor.
    let answered = PlatformRecordStore::new(state.test_petri_view_pool())
        .read_kind(
            &run_id.parse().expect("the run id parses"),
            PlatformRecordKind::InterviewAnswered,
        )
        .await
        .expect("the platform records read");
    let [answered] = answered.as_slice() else {
        panic!("one question was answered: {answered:?}");
    };
    let PlatformRecord::InterviewAnswered(record) = &answered.record else {
        panic!("an answered record: {answered:?}");
    };
    assert_eq!(record.question, question_id);
    assert!(
        record.principal.is_some(),
        "the answering principal: {record:?}"
    );
    super::petri_stream::capture_settled(&state, &app, &run_id, "gate").await;
}
