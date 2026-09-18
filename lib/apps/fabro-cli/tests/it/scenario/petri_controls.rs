//! The run controls on a Petri run through a real server and its worker:
//! a pause holds the next stage until the unpause and the API says
//! `paused` in between; `SIGUSR1` and `SIGUSR2` on the worker do the same
//! without the API; a steer reaches the agent stage on the twin, which
//! sees it in its next request, and the stream carries the control record;
//! a run paused when its server and worker die resumes paused and goes on
//! once unpaused.
//!
//! The harness is `petri.rs`'s: a foreground server on disk storage, the
//! run started with `fabro run --detach`, and the host scope through the
//! sandbox-driver host plugin, so the tests skip, and say why, when the
//! plugin is not found.

#![expect(
    clippy::disallowed_methods,
    reason = "these scenarios stage workspaces with sync std::fs, start a real server subprocess and poll processes"
)]
#![expect(
    clippy::print_stderr,
    reason = "a scenario says where it is, and why it skipped"
)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fabro_petri::engine::{self, RunStatus};
use fabro_static::EnvVars;
use fabro_test::{TwinScenario, TwinScenarios, TwinToolCall, test_context, twin_openai};
use serde_json::{Value, json};

use super::petri::{
    RunningServer, count_of, host_plugin, run_detached, run_detached_with, run_json, run_status,
    run_stream, settled_stream, stream_names, wait_for_status, wait_for_worker,
    wait_until_gate_is_polled, write_petri_workflow,
};
use crate::support::TEST_DEV_TOKEN;

const POLL: Duration = Duration::from_millis(50);
const RUN_TIMEOUT: Duration = Duration::from_mins(1);
/// How long a stage that must not start is watched for.
const HOLD: Duration = Duration::from_secs(1);

const MODEL: &str = "gpt-5.4";
const PROMPT: &str = "Wait for the gate, then report.";
const STEER: &str = "Steer: mention the word lighthouse in your report.";

/// Two command stages: `a` waits on `gate`, `b` leaves `marker`.
fn two_stage_workspace(context: &fabro_test::TestContext, gate: &Path, marker: &Path) -> PathBuf {
    write_petri_workflow(
        context,
        &format!(
            "digraph Two {{\n  graph [goal=\"Run two commands\", default_max_retries=0]\n  start \
             [shape=Mdiamond]\n  exit [shape=Msquare]\n  a [shape=parallelogram, script=\"while [ \
             ! -f {gate} ]; do sleep 0.05; done\", max_retries=0]\n  b [shape=parallelogram, \
             script=\"touch {marker}\", max_retries=0]\n  start -> a -> b -> exit\n}}\n",
            gate = gate.display(),
            marker = marker.display()
        ),
    )
}

/// `POST /runs/{id}/<action>` as a user; the response status and body.
async fn control(
    server: &RunningServer,
    run_id: &str,
    action: &str,
    body: Option<Value>,
) -> (u16, Value) {
    let mut request = fabro_test::test_http_client()
        .post(format!(
            "{}/api/v1/runs/{run_id}/{action}",
            server.api_base_url
        ))
        .bearer_auth(TEST_DEV_TOKEN);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.expect("the control sends");
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    let body = serde_json::from_str(&text).unwrap_or(Value::String(text));
    (status, body)
}

async fn pause(server: &RunningServer, run_id: &str) {
    let (status, body) = control(server, run_id, "pause", None).await;
    assert_eq!(status, 200, "pause: {body}");
}

async fn unpause(server: &RunningServer, run_id: &str) {
    let (status, body) = control(server, run_id, "unpause", None).await;
    assert_eq!(status, 200, "unpause: {body}");
}

async fn steer(server: &RunningServer, run_id: &str, text: &str) {
    let (status, body) = control(
        server,
        run_id,
        "steer",
        Some(json!({ "text": text, "interrupt": false })),
    )
    .await;
    assert_eq!(status, 202, "steer: {body}");
}

/// The run's pending control, as the API shows it.
async fn pending_control(server: &RunningServer, run_id: &str) -> Value {
    run_json(server, &format!("runs/{run_id}")).await["lifecycle"]["pending_control"].clone()
}

/// Wait until the stream names `event` at least `times` times.
async fn wait_for_stream_count(
    server: &RunningServer,
    run_id: &str,
    event: &str,
    times: usize,
) -> Vec<String> {
    let deadline = Instant::now() + RUN_TIMEOUT;
    loop {
        let names = stream_names(&run_stream(server, run_id).await);
        if count_of(&names, event) >= times {
            return names;
        }
        assert!(
            Instant::now() < deadline,
            "the stream of {run_id} never carried {event} {times} times: {names:?}"
        );
        tokio::time::sleep(POLL).await;
    }
}

/// Wait until the run's pending control is cleared: the record the control
/// asked for has landed.
async fn wait_for_no_pending_control(server: &RunningServer, run_id: &str) {
    let deadline = Instant::now() + RUN_TIMEOUT;
    loop {
        let pending = pending_control(server, run_id).await;
        if pending.is_null() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the pending control of {run_id} never cleared: {pending}"
        );
        tokio::time::sleep(POLL).await;
    }
}

/// The run's Petri outcome: succeeded and whole, or the test says why not.
async fn assert_petri_succeeded(server: &RunningServer, run_id: &str) {
    let store = server.petri_store().await;
    let outcome = engine::outcome_of(&store, run_id)
        .await
        .expect("the run's Petri record inspects");
    assert_eq!(outcome.status, RunStatus::Success, "{outcome:?}");
    assert!(outcome.complete, "{:?}", outcome.incomplete);
}

/// A pause while `a` runs holds `b` at admission: the API says `paused`
/// with no pending control, `a` finishes on its own, `b` does not start,
/// and the unpause lets it through. Petri's records and Fabro's lifecycle
/// both carry the pause and the unpause.
#[tokio::test(flavor = "multi_thread")]
async fn a_pause_holds_the_next_stage_until_the_unpause() {
    if host_plugin().is_none() {
        return;
    }
    let context = test_context!();
    let server = RunningServer::start().await;
    let gate = context.temp_dir.join("a.gate");
    let marker = context.temp_dir.join("b.marker");
    let workspace = two_stage_workspace(&context, &gate, &marker);
    let run_id = run_detached(&context, &server, &workspace);

    wait_for_status(&server, &run_id, &["running"]).await;
    wait_until_gate_is_polled(&gate);
    eprintln!("run {run_id}: a is waiting on the gate");

    pause(&server, &run_id).await;
    wait_for_status(&server, &run_id, &["paused"]).await;
    wait_for_no_pending_control(&server, &run_id).await;
    eprintln!("run {run_id} is paused");

    std::fs::write(&gate, "go").expect("the gate opens");
    wait_for_stream_count(&server, &run_id, "step.finished", 2).await;
    tokio::time::sleep(HOLD).await;
    assert!(!marker.exists(), "b started while the run was paused");
    assert_eq!(run_status(&server, &run_id).await, "paused");

    unpause(&server, &run_id).await;
    let status = wait_for_status(&server, &run_id, &["succeeded", "failed"]).await;
    let items = settled_stream(&server, &run_id).await;
    let names = stream_names(&items);
    assert_eq!(
        status,
        "succeeded",
        "stream: {names:?}\nserver stderr:\n{}",
        server.stderr_text()
    );
    assert!(marker.exists(), "b ran after the unpause");
    assert_petri_succeeded(&server, &run_id).await;

    for event in [
        "run.paused",
        "run.unpaused",
        "lifecycle:pause_requested",
        "lifecycle:paused",
        "lifecycle:unpause_requested",
        "lifecycle:unpaused",
    ] {
        assert_eq!(count_of(&names, event), 1, "{event}: {names:?}");
    }
    let unpaused = names
        .iter()
        .position(|name| name == "run.unpaused")
        .expect("the unpause is recorded");
    // The stages start in order: `start`, `a`, then `b` after the unpause.
    let b_started = names
        .iter()
        .enumerate()
        .filter(|(_, name)| *name == "step.started")
        .nth(2)
        .map(|(index, _)| index)
        .expect("b started");
    assert!(
        unpaused < b_started,
        "b started before the unpause: {names:?}"
    );
    assert!(pending_control(&server, &run_id).await.is_null());
    server.shutdown();
}

/// `SIGUSR1` on the worker pauses the run the way the API's pause does,
/// and `SIGUSR2` unpauses it: `b` is held at admission in between, Petri's
/// records and Fabro's lifecycle both carry the pause and the unpause, and
/// no control request is recorded, since none went through the API.
#[tokio::test(flavor = "multi_thread")]
async fn the_user_signals_pause_and_unpause_the_worker() {
    if host_plugin().is_none() {
        return;
    }
    let context = test_context!();
    let server = RunningServer::start().await;
    let gate = context.temp_dir.join("a.gate");
    let marker = context.temp_dir.join("b.marker");
    let workspace = two_stage_workspace(&context, &gate, &marker);
    let run_id = run_detached(&context, &server, &workspace);

    wait_for_status(&server, &run_id, &["running"]).await;
    let worker = wait_for_worker(&run_id);
    wait_until_gate_is_polled(&gate);
    eprintln!("run {run_id}: a is waiting on the gate; sending SIGUSR1 to worker {worker}");

    fabro_proc::sigusr1(worker);
    wait_for_status(&server, &run_id, &["paused"]).await;
    eprintln!("run {run_id} is paused");

    std::fs::write(&gate, "go").expect("the gate opens");
    wait_for_stream_count(&server, &run_id, "step.finished", 2).await;
    tokio::time::sleep(HOLD).await;
    assert!(!marker.exists(), "b started while the run was paused");
    assert_eq!(run_status(&server, &run_id).await, "paused");
    assert!(pending_control(&server, &run_id).await.is_null());

    fabro_proc::sigusr2(worker);
    let status = wait_for_status(&server, &run_id, &["succeeded", "failed"]).await;
    let items = settled_stream(&server, &run_id).await;
    let names = stream_names(&items);
    assert_eq!(
        status,
        "succeeded",
        "stream: {names:?}\nserver stderr:\n{}",
        server.stderr_text()
    );
    assert!(marker.exists(), "b ran after the unpause");
    assert_petri_succeeded(&server, &run_id).await;

    for event in [
        "run.paused",
        "run.unpaused",
        "lifecycle:paused",
        "lifecycle:unpaused",
    ] {
        assert_eq!(count_of(&names, event), 1, "{event}: {names:?}");
    }
    for event in ["lifecycle:pause_requested", "lifecycle:unpause_requested"] {
        assert_eq!(
            count_of(&names, event),
            0,
            "a signal is not an API request: {event}: {names:?}"
        );
    }
    let unpaused = names
        .iter()
        .position(|name| name == "run.unpaused")
        .expect("the unpause is recorded");
    let b_started = names
        .iter()
        .enumerate()
        .filter(|(_, name)| *name == "step.started")
        .nth(2)
        .map(|(index, _)| index)
        .expect("b started");
    assert!(
        unpaused < b_started,
        "b started before the unpause: {names:?}"
    );
    server.shutdown();
}

/// A steer sent while the agent stage waits on a tool reaches its
/// session: the twin sees the steer text in the follow-up request, the
/// stream carries the `control.requested` record, and the run succeeds.
#[tokio::test(flavor = "multi_thread")]
async fn a_steer_reaches_the_agent_stage_on_the_twin() {
    if host_plugin().is_none() {
        return;
    }
    let context = test_context!();
    let twin = twin_openai().await;
    let namespace = format!("{}::{}", module_path!(), line!());
    let server = RunningServer::start_with(
        &format!(
            "\n[llm.providers.openai]\nbase_url = \"{}\"\n",
            twin.base_url
        ),
        &[(EnvVars::OPENAI_API_KEY, &namespace)],
    )
    .await;
    let gate = context.temp_dir.join("steer.gate");
    let scenario = || TwinScenario::responses(MODEL).input_contains(PROMPT);
    TwinScenarios::new(namespace.clone())
        .scenario(scenario().tool_call(TwinToolCall::new(
            "shell",
            json!({ "command": format!("while [ ! -f {} ]; do sleep 0.05; done", gate.display()) }),
        )))
        .scenario(scenario().text("The gate opened."))
        .scenario(
            TwinScenario::responses(MODEL)
                .input_contains(STEER)
                .text("Lighthouse noted."),
        )
        .load(twin)
        .await;
    let workspace = write_petri_workflow(
        &context,
        &format!(
            "digraph Steer {{\n  graph [goal=\"Wait then report\", default_max_retries=0]\n  \
             start [shape=Mdiamond]\n  exit [shape=Msquare]\n  work [shape=box, \
             prompt=\"{PROMPT}\", max_retries=0]\n  start -> work -> exit\n}}\n"
        ),
    );
    let run_id = run_detached_with(&context, &server, &workspace, &[
        "--auto-approve",
        "--provider",
        "openai",
        "--model",
        MODEL,
    ]);

    wait_for_status(&server, &run_id, &["running"]).await;
    wait_until_gate_is_polled(&gate);
    eprintln!("run {run_id}: the agent's tool is waiting on the gate");
    steer(&server, &run_id, STEER).await;
    wait_for_stream_count(&server, &run_id, "control.requested", 1).await;
    eprintln!("run {run_id}: the steer is recorded");
    std::fs::write(&gate, "go").expect("the gate opens");

    let status = wait_for_status(&server, &run_id, &["succeeded", "failed"]).await;
    let items = settled_stream(&server, &run_id).await;
    let names = stream_names(&items);
    assert_eq!(
        status,
        "succeeded",
        "stream: {names:?}\nserver stderr:\n{}",
        server.stderr_text()
    );
    assert_petri_succeeded(&server, &run_id).await;

    let delivery = items
        .iter()
        .find(|item| item["item"]["record"]["body"]["event"] == "control.requested")
        .expect("the steer is in the stream");
    let text = serde_json::to_string(delivery).expect("the item serializes");
    assert!(
        text.contains(STEER),
        "the control record carries the steer: {text}"
    );
    assert!(
        text.contains("\"deliverable\":true"),
        "the steer was delivered to a live firing: {text}"
    );

    let logs = twin.request_logs(&namespace).await;
    let inputs: Vec<String> = logs["requests"]
        .as_array()
        .expect("the twin request log is an array")
        .iter()
        .map(|request| {
            request["input_text"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .filter(|input| input.contains(PROMPT))
        .collect();
    assert_eq!(
        inputs.len(),
        3,
        "the tool call, its answer, the steer: {inputs:?}"
    );
    assert!(
        !inputs[1].contains(STEER),
        "the answer's request came before the steer's turn: {}",
        inputs[1]
    );
    assert!(
        inputs[2].contains(STEER),
        "the follow-up request carries the steer: {}",
        inputs[2]
    );
    server.shutdown();
}

/// A run paused with its next stage held at admission, whose server and
/// worker then die, resumes paused: the resumed worker reports the pause
/// again, admits nothing until the unpause, then finishes the run. (A
/// stage that was mid-flight at the crash is re-dispatched on resume
/// without a new admission: a pause holds admission, never running work.)
#[tokio::test(flavor = "multi_thread")]
async fn a_run_paused_before_a_crash_resumes_paused() {
    if host_plugin().is_none() {
        return;
    }
    let context = test_context!();
    let mut server = RunningServer::start().await;
    let gate = context.temp_dir.join("a.gate");
    let marker = context.temp_dir.join("b.marker");
    let workspace = two_stage_workspace(&context, &gate, &marker);
    let run_id = run_detached(&context, &server, &workspace);

    wait_for_status(&server, &run_id, &["running"]).await;
    let worker = wait_for_worker(&run_id);
    wait_until_gate_is_polled(&gate);
    pause(&server, &run_id).await;
    wait_for_status(&server, &run_id, &["paused"]).await;
    wait_for_no_pending_control(&server, &run_id).await;
    // `a` finishes under the pause; `b` reaches admission and is held.
    std::fs::write(&gate, "go").expect("the gate opens");
    wait_for_stream_count(&server, &run_id, "step.finished", 2).await;
    tokio::time::sleep(HOLD).await;
    assert!(!marker.exists(), "b started while the run was paused");
    eprintln!("run {run_id} is paused with b held at admission; crashing");

    server.kill();
    fabro_proc::sigkill_process_group(worker);
    let deadline = Instant::now() + Duration::from_secs(10);
    while fabro_proc::process_running(worker) {
        assert!(Instant::now() < deadline, "the worker did not die");
        std::thread::sleep(POLL);
    }

    server.launch().await;
    eprintln!("server restarted");
    let resumed = wait_for_worker(&run_id);
    assert_ne!(resumed, worker, "a new worker was launched");
    // The resumed worker reports the pause it came back under.
    let names = wait_for_stream_count(&server, &run_id, "lifecycle:paused", 2).await;
    assert_eq!(count_of(&names, "run.paused"), 1, "{names:?}");
    tokio::time::sleep(HOLD).await;
    assert!(
        !marker.exists(),
        "b was admitted while the resumed run was paused"
    );
    assert_eq!(run_status(&server, &run_id).await, "paused");
    assert!(pending_control(&server, &run_id).await.is_null());

    unpause(&server, &run_id).await;
    let status = wait_for_status(&server, &run_id, &["succeeded", "failed"]).await;
    let items = settled_stream(&server, &run_id).await;
    let names = stream_names(&items);
    assert_eq!(
        status,
        "succeeded",
        "stream: {names:?}\nserver stderr:\n{}",
        server.stderr_text()
    );
    assert!(marker.exists(), "b ran after the unpause");
    assert_petri_succeeded(&server, &run_id).await;
    assert_eq!(count_of(&names, "run.paused"), 1, "{names:?}");
    assert_eq!(count_of(&names, "run.unpaused"), 1, "{names:?}");
    assert_eq!(count_of(&names, "lifecycle:paused"), 2, "{names:?}");
    assert_eq!(count_of(&names, "lifecycle:unpaused"), 1, "{names:?}");
    assert_eq!(count_of(&names, "lifecycle:running"), 2, "{names:?}");
    let unpaused = names
        .iter()
        .position(|name| name == "run.unpaused")
        .expect("the unpause is recorded");
    let b_started = names
        .iter()
        .enumerate()
        .filter(|(_, name)| *name == "step.started")
        .nth(2)
        .map(|(index, _)| index)
        .expect("b started");
    assert!(
        unpaused < b_started,
        "b started before the unpause: {names:?}"
    );
    server.shutdown();
}
