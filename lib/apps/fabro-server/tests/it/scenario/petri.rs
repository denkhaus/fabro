//! Runs on Petri through the server: a run goes to Petri when its workflow
//! version names `engine = "petri"` or when the server's
//! `[server.execution] engine` says so, Petri's record of the run agrees
//! with Fabro's status, and Petri's diagnostics refuse a run at create.
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

const PLAIN_SETTINGS: &str = "_version = 1\n\n[workflow]\ngraph = \"workflow.fabro\"\n";
const PETRI_SETTINGS: &str =
    "_version = 1\n\n[workflow]\ngraph = \"workflow.fabro\"\nengine = \"petri\"\n";

/// The host plugin as Petri's lookup finds it: the override variable, else
/// the executable on `PATH`. `None`, after saying so, when the test should
/// skip; a panic when the environment forbids a skip.
fn host_plugin() -> Option<PathBuf> {
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
async fn register_version(app: &axum::Router, files: &[(&str, &str)]) -> String {
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

fn intent(version_id: &str, workspace: &std::path::Path) -> serde_json::Value {
    serde_json::json!({
        "workflow_version_id": version_id,
        "target": {"kind": "folder", "path": workspace},
        "environment_id": "local",
        "args": {},
    })
}

/// The `hello` bundle checked into this repository, with `engine = "petri"`
/// added to its `[workflow]` table.
fn hello_files() -> [(&'static str, String); 2] {
    let workflow = read_repo_file(".fabro/workflows/hello/workflow.fabro");
    let settings = read_repo_file(".fabro/workflows/hello/workflow.toml");
    assert!(
        settings.trim_end().ends_with("graph = \"workflow.fabro\""),
        "the hello settings end with the [workflow] table, so an engine key appends to it"
    );
    [
        ("workflow.fabro", workflow),
        (
            "workflow.toml",
            format!("{}\nengine = \"petri\"\n", settings.trim_end()),
        ),
    ]
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
async fn settled_state(state: &AppState, app: &axum::Router, run_id: &str) -> serde_json::Value {
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
    projector::stored_stream(&test_app_db_pool(state), id)
        .await
        .expect("the stream reads")
        .len()
}

async fn run_engine(app: &axum::Router, run_id: &str) -> serde_json::Value {
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
    body["spec"]["engine"].clone()
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

/// The `hello` bundle, whose one stage is a prompt, runs on Petri when its
/// version names the engine: the prompt reaches the twin through Petri's
/// model client, Fabro reports the run succeeded, and Petri's record of the
/// run says the same.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_hello_bundle_runs_on_petri_when_the_version_names_the_engine() {
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
    assert_eq!(run_engine(&app, &run_id).await["kind"], "petri");
    let outcome = petri_outcome(&state, &run_id).await;
    assert_eq!(outcome.status, RunStatus::Success, "{outcome:?}");
    assert!(outcome.complete, "{:?}", outcome.incomplete);
    let projection = settled_state(&state, &app, &run_id).await;
    assert_eq!(projection["status"]["kind"], "succeeded", "{projection}");
    assert_eq!(
        projection["conclusion"]["status"], "succeeded",
        "{projection}"
    );
    let stages = projection["stages"]
        .as_object()
        .expect("the state carries its stages");
    let prompt = stages
        .values()
        .find(|stage| stage["handler"] == "prompt")
        .unwrap_or_else(|| panic!("the hello prompt stage is projected: {projection}"));
    assert_eq!(prompt["state"], "succeeded", "{prompt}");
    assert!(
        prompt["response"]
            .as_str()
            .is_some_and(|response| response.contains("A haiku, added.")),
        "the prompt's response is projected: {prompt}"
    );
    assert_eq!(
        run["usage"]["tokens"]["input"].as_u64().is_some(),
        true,
        "{run}"
    );
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
}

/// A command-only bundle runs on Petri when the server's setting names the
/// engine and the version names none, and Petri's record agrees.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_command_bundle_runs_on_petri_under_the_server_setting() {
    if host_plugin().is_none() {
        return;
    }
    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let settings = settings_from_toml(
        "_version = 1\n\n[run.environment]\nid = \"local\"\n\n[server.execution]\nengine = \
         \"petri\"\n",
    );
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
    assert_eq!(run_engine(&app, &run_id).await["kind"], "petri");
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
    let settings = settings_from_toml(
        "_version = 1\n\n[run.environment]\nid = \"local\"\n\n[server.execution]\nengine = \
         \"petri\"\n",
    );
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
    for branch in ["a@1", "b@1"] {
        let stage = &projection["stages"][branch];
        assert_eq!(stage["state"], "succeeded", "{branch}: {projection}");
        assert_eq!(stage["parallel_branch_id"]["group"], "fork@1", "{stage}");
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

/// A version that names no engine on a server whose setting is the default
/// keeps the legacy executor: the run's spec records no Petri admission.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_version_that_names_no_engine_stays_on_the_legacy_executor() {
    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let state = test_app_state_with_options(test_settings(), 5);
    let app = test_app_with_scheduler(state);

    let version_id = register_version(&app, &[
        ("workflow.fabro", COMMAND_DOT),
        ("workflow.toml", PLAIN_SETTINGS),
    ])
    .await;
    let mut intent = intent(&version_id, workspace.path());
    intent["args"]["dry_run"] = serde_json::json!(true);
    let run_id = create_and_start_run_from_intent(&app, intent).await;

    assert_eq!(run_engine(&app, &run_id).await, serde_json::Value::Null);
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
        ("workflow.toml", PETRI_SETTINGS),
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
        ("workflow.toml", PETRI_SETTINGS),
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
        ("workflow.toml", PETRI_SETTINGS),
    ])
    .await;
    let body = create_run_response(&app, intent(&version_id, workspace.path())).await;

    let detail = body["errors"][0]["detail"].as_str().unwrap_or_default();
    assert!(
        detail.contains("attractor.model.unknown") && detail.contains("no-such-model-9000"),
        "expected the admission diagnostic in the detail, got {body}"
    );
}
