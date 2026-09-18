//! Fabro's run tools on a Petri run from this crate (integration plan item
//! F3.4): `RuntimeSpec::run_tools` installs the adapter as Petri's host
//! tool capability, a workflow with one agent stage runs on the real step
//! registry against a scripted model, and the stage's session gets the
//! tools the legacy worker registers, bound to the run: the model is
//! advertised every run tool, its `fabro_run_create` call reaches Fabro's
//! API with the Petri run as the child's parent, the API's answer comes
//! back to the model, and the call is in the run's record under the stage.
//!
//! Every run takes its scope through the sandbox-driver host plugin, so
//! the tests skip when that executable is not found, unless
//! `FABRO_REQUIRE_SANDBOX_PLUGINS` is set.

#![expect(
    clippy::disallowed_methods,
    reason = "the tests locate the plugin executable through the process environment"
)]
#![expect(clippy::print_stderr, reason = "a skipped test says why on its stderr")]

use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use fabro_petri::host_tools::recorded::{self, ExecutionId, InvocationId};
use fabro_petri::runtime::RuntimeSpec;
use fabro_tool::fabro_client::ClientBackend;
use fabro_types::{BlobHash, RunId, WorkflowVersionId};
use fabro_workflow::handler::llm::register_fabro_run_tools;
use fabro_workflow::services::FabroRunToolServices;
use httpmock::{Method, MockServer};
use lithos_llm::types::Request;
use pebble_coding_agent::test_support::{
    ScriptedCall, ScriptedProvider, scripted_client, text_response, tool_call_response,
};
use petri_execution::host::{self, HostRun};
use petri_runtime::executor::Retention;
use petri_runtime::frontend::CompileInputs;
use petri_runtime::ir::RunStatus;
use petri_runtime::{RunOptions, Runtime};
use petri_store::{MemoryRunStore, RunKey, RunStore};
use serde_json::json;
use tokio::fs;

const HOST_PLUGIN: &str = "sandbox-driver-host";
const HOST_PLUGIN_OVERRIDE: &str = "PETRI_SANDBOX_HOST_PLUGIN";
const REQUIRE_ENV: &str = "FABRO_REQUIRE_SANDBOX_PLUGINS";

/// One agent stage on the native backend, pinned to the scripted model.
const AGENT_WORKFLOW: &str = r#"digraph Agent {
    graph [goal="Start a child run", backend="api", default_max_retries=0]
    start [shape=Mdiamond]
    exit [shape=Msquare]
    work [shape=box, prompt="Start the child run", model="test/model", max_retries=0]
    start -> work -> exit
}"#;

const AGENT_SETTINGS: &str = "_version = 1\n\n[workflow]\ngraph = \"workflow.fabro\"\n";

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

/// Write the agent bundle into `<root>/.fabro/workflows/agent`; the
/// workflow file.
async fn install_bundle(root: &Path) -> PathBuf {
    let bundle = root.join(".fabro").join("workflows").join("agent");
    fs::create_dir_all(&bundle)
        .await
        .expect("the bundle directory is creatable");
    fs::write(bundle.join("workflow.fabro"), AGENT_WORKFLOW)
        .await
        .expect("the workflow is writable");
    fs::write(bundle.join("workflow.toml"), AGENT_SETTINGS)
        .await
        .expect("the settings are writable");
    bundle.join("workflow.fabro")
}

/// The run tools' services over `server`, as the worker binds them: the
/// client backend and the run the tools serve.
fn services(server: &MockServer, run_id: RunId) -> FabroRunToolServices {
    let client = fabro_client::Client::new_no_proxy(&server.url("")).expect("the client builds");
    FabroRunToolServices {
        backend:        Arc::new(ClientBackend::new(Arc::new(client))),
        current_run_id: run_id,
    }
}

/// The scripted model: one `fabro_run_create` call, then a closing line.
fn scripted_model(version_id: &str) -> (lithos_llm::Client, Arc<ScriptedProvider>) {
    scripted_client(vec![
        ScriptedCall::response(tool_call_response(
            "fabro_run_create",
            "create",
            json!({
                "runs": [{
                    "workflow_version_id": version_id,
                    "target": {"kind": "none"},
                    "args": {"auto_approve": false},
                }],
            }),
        )),
        ScriptedCall::response(text_response("Asked for the child run.")),
    ])
}

/// The runtime the worker would build for the run: the scripted model as
/// the model client and `run_tools` as the run tools.
fn runtime(
    run_dir: &Path,
    run_id: &str,
    model_client: lithos_llm::Client,
    run_tools: Option<FabroRunToolServices>,
    store: &Arc<MemoryRunStore>,
) -> Runtime {
    let mut options = RunOptions::new(run_dir);
    options.grace = Duration::from_secs(2);
    options.retention = Retention::Never;
    options.echo = false;
    options.run_key = Some(RunKey::new(run_id));
    RuntimeSpec {
        model_client: Some(model_client),
        run_tools,
        ..RuntimeSpec::default()
    }
    .runtime(true)
    .store(Arc::clone(store) as Arc<dyn RunStore>)
    .options(options)
}

/// Lower the bundle and run it to its end.
async fn run(rt: &Runtime, workflow: &Path) {
    let lowered = rt
        .check(workflow, None, None, &CompileInputs::new())
        .expect("the workflow file loads");
    let graph = lowered
        .graph
        .unwrap_or_else(|| panic!("the workflow lowers: {:?}", lowered.diagnostics));
    let report = host::run_configured(rt, HostRun::new(graph), |_, _| {})
        .await
        .expect("the run completes");
    assert_eq!(
        report.status,
        RunStatus::Success,
        "errors: {:?}; history: {:#?}",
        report.state.errors(),
        report.state.history()
    );
}

/// The tools a request advertised, as `(name, description)`.
fn advertised(request: &Request) -> Vec<(String, String)> {
    request
        .tools()
        .iter()
        .map(|tool| (tool.name.clone(), tool.description.clone()))
        .collect()
}

/// The stage's session is given every run tool the legacy worker
/// registers, by the same names and descriptions; its `fabro_run_create`
/// call reaches Fabro's API with the Petri run as the parent; the API's
/// answer reaches the model; the call is in the record under the stage.
#[tokio::test]
async fn a_petri_stage_calls_a_run_tool_bound_to_the_run() {
    if host_plugin().is_none() {
        return;
    }
    let root = tempfile::tempdir().expect("a temp dir");
    let workflow = install_bundle(root.path()).await;
    let run_id = RunId::new();
    let version_id: WorkflowVersionId = BlobHash::new(b"child workflow").into();
    let server = MockServer::start_async().await;
    // Admission rejection proves the tool reached the canonical API with
    // the Petri run as the child's parent, and nothing was created.
    let create = server
        .mock_async(|when, then| {
            when.method(Method::POST)
                .path("/api/v1/runs")
                .json_body(json!({
                    "workflow_version_id": version_id,
                    "target": {"kind": "none"},
                    "parent_id": run_id,
                    "args": {"auto_approve": false},
                }));
            then.status(422).body("native admission rejection");
        })
        .await;
    let services = services(&server, run_id);
    let legacy: Vec<(String, String)> = register_fabro_run_tools(&services)
        .iter()
        .map(|tool| {
            (
                tool.definition().name.clone(),
                tool.definition().description.clone(),
            )
        })
        .collect();
    let (client, provider) = scripted_model(&version_id.to_string());
    let store = Arc::new(MemoryRunStore::new());
    let rt = runtime(
        &root.path().join("run"),
        &run_id.to_string(),
        client,
        Some(services),
        &store,
    );

    run(&rt, &workflow).await;

    let requests = provider.requests();
    assert_eq!(requests.len(), 2, "one tool call, one closing turn");
    let tools = advertised(&requests[0]);
    assert!(!legacy.is_empty());
    for tool in &legacy {
        assert!(
            tools.contains(tool),
            "the model was advertised {tool:?} as the legacy worker registers it: {tools:?}"
        );
    }
    assert!(
        tools.iter().any(|(name, _)| name == "shell"),
        "Pebble's own tools stay: {tools:?}"
    );
    let answer = serde_json::to_string(&requests[1]).expect("the request serializes");
    assert!(
        answer.contains("native admission rejection"),
        "the model read the API's answer: {answer}"
    );
    create.assert_calls_async(1).await;

    let calls = recorded::tool_calls(store.as_ref(), &run_id.to_string(), "fabro_run_create")
        .await
        .expect("the record replays");
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(calls[0].node, "work", "recorded under the stage");
    assert_eq!(calls[0].invocation, Some(InvocationId::ROOT));
    assert_eq!(calls[0].execution, Some(ExecutionId::new(0)));
    assert!(calls[0].parent_session.is_none());
    assert_eq!(calls[0].payload["is_error"], true, "{:?}", calls[0].payload);
}

/// Services bound to another run give the stage no run tools: the model
/// is not advertised them, and its call is refused as an unknown tool
/// rather than parenting a child run to the wrong run.
#[tokio::test]
async fn services_for_another_run_give_the_stage_no_run_tools() {
    if host_plugin().is_none() {
        return;
    }
    let root = tempfile::tempdir().expect("a temp dir");
    let workflow = install_bundle(root.path()).await;
    let run_id = RunId::new();
    let server = MockServer::start_async().await;
    let create = server
        .mock_async(|when, then| {
            when.method(Method::POST).path("/api/v1/runs");
            then.status(500);
        })
        .await;
    let services = services(&server, RunId::new());
    let (client, provider) = scripted_model("0000");
    let store = Arc::new(MemoryRunStore::new());
    let rt = runtime(
        &root.path().join("run"),
        &run_id.to_string(),
        client,
        Some(services),
        &store,
    );

    run(&rt, &workflow).await;

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    let tools = advertised(&requests[0]);
    assert!(
        !tools.iter().any(|(name, _)| name.starts_with("fabro_")),
        "no run tool was advertised: {tools:?}"
    );
    create.assert_calls_async(0).await;
}
