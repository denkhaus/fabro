//! Runs on Petri through a real server and its worker subprocess: the run
//! is created and started with `fabro run --detach`, the server launches
//! `fabro run __run-worker` for it as it does for a legacy run, and the
//! worker executes it through Petri over the HTTP run store.
//!
//! Each test starts its own foreground server on disk storage, because the
//! session's shared daemon keeps its object store in memory and the resume
//! scenario restarts the server. The runs take their host scope through the
//! sandbox-driver host plugin, so the tests skip, and say why, when the
//! executable is not found, unless `FABRO_REQUIRE_SANDBOX_PLUGINS` is set.
//! The plugin's path override crosses into the server and its workers the
//! way `PATH` does.

#![expect(
    clippy::disallowed_methods,
    reason = "these scenarios start a real server subprocess, locate the plugin through the process environment, and poll processes"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the scenarios own the server Child so they can SIGKILL it mid-run"
)]
#![expect(clippy::print_stderr, reason = "a skipped test says why on its stderr")]

use std::env;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use fabro_client::ServerTarget;
use fabro_config::{Storage, envfile};
use fabro_petri::SqliteRunStore;
use fabro_petri::engine::{self, RunStatus};
use fabro_petri::petri::RunKey;
use fabro_static::EnvVars;
use fabro_store::EventEnvelope;
use fabro_test::{apply_test_isolation, expect_reqwest_json, isolated_storage_dir, test_context};
use fabro_types::EventBody;

use crate::cmd::support::created_run_id;
use crate::support::{
    TEST_DEV_TOKEN, TEST_SESSION_SECRET, parse_event_envelopes, seed_dev_token_auth,
};

const HOST_PLUGIN: &str = "sandbox-driver-host";
const REQUIRE_ENV: &str = "FABRO_REQUIRE_SANDBOX_PLUGINS";
const RUN_TIMEOUT: Duration = Duration::from_mins(1);
const POLL: Duration = Duration::from_millis(50);

/// The host plugin as Petri's lookup finds it: the override variable, else
/// the executable on `PATH`. `None`, after saying so, when the test should
/// skip; a panic when the environment forbids a skip.
fn host_plugin() -> Option<PathBuf> {
    let found = env::var_os(EnvVars::PETRI_SANDBOX_HOST_PLUGIN)
        .map(PathBuf::from)
        .or_else(|| {
            env::split_paths(&env::var_os(EnvVars::PATH)?)
                .map(|dir| dir.join(HOST_PLUGIN))
                .find(|candidate| candidate.is_file())
        });
    if found.is_none() {
        assert!(
            env::var_os(REQUIRE_ENV).is_none(),
            "{REQUIRE_ENV} is set, but {HOST_PLUGIN} is not on PATH and {} is unset",
            EnvVars::PETRI_SANDBOX_HOST_PLUGIN
        );
        eprintln!(
            "skipping: {HOST_PLUGIN} is not on PATH and {} is unset",
            EnvVars::PETRI_SANDBOX_HOST_PLUGIN
        );
    }
    found
}

/// A foreground server on its own disk storage, dev-token auth, started
/// from the compiled `fabro` binary. Dropping it kills the process.
struct RunningServer {
    child:         Option<Child>,
    home_root:     tempfile::TempDir,
    _storage_root: tempfile::TempDir,
    storage_dir:   PathBuf,
    config_path:   PathBuf,
    port:          u16,
    api_base_url:  String,
}

impl RunningServer {
    async fn start() -> Self {
        let home_root = tempfile::tempdir_in("/tmp").expect("home tempdir");
        let storage_root = isolated_storage_dir();
        let storage_dir = storage_root.path().join("storage");
        let port = reserve_port();
        let config_path = home_root.path().join("settings.toml");
        std::fs::write(
            &config_path,
            "_version = 1\n\n[server.auth]\nmethods = [\"dev-token\"]\n",
        )
        .expect("the server settings write");
        let runtime_directory = Storage::new(&storage_dir).runtime_directory();
        envfile::merge_env_file(&runtime_directory.env_path(), [
            ("SESSION_SECRET", TEST_SESSION_SECRET),
            ("FABRO_DEV_TOKEN", TEST_DEV_TOKEN),
        ])
        .expect("the server env writes");
        fabro_util::dev_token::write_dev_token(&runtime_directory.dev_token_path(), TEST_DEV_TOKEN)
            .expect("the dev token writes");
        let mut server = Self {
            child: None,
            home_root,
            _storage_root: storage_root,
            storage_dir,
            config_path,
            port,
            api_base_url: format!("http://127.0.0.1:{port}"),
        };
        server.launch().await;
        server
    }

    /// Start the server process over this storage; the same call brings
    /// it back after a kill.
    async fn launch(&mut self) {
        assert!(self.child.is_none(), "the server is already running");
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_fabro"));
        apply_test_isolation(&mut cmd, self.home_root.path());
        // The resume scenario restarts the server: its object store must
        // outlive the process.
        cmd.env(EnvVars::FABRO_TEST_IN_MEMORY_STORE, "0");
        cmd.env(
            EnvVars::FABRO_HOME,
            self.home_root.path().join("fabro-home"),
        );
        cmd.args(["server", "start", "--foreground"])
            .arg("--storage-dir")
            .arg(&self.storage_dir)
            .arg("--bind")
            .arg(format!("127.0.0.1:{}", self.port))
            .arg("--config")
            .arg(&self.config_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(self.stderr_log());
        let mut child = cmd.spawn().expect("the server spawns");
        wait_for_http_ready(&self.api_base_url, &mut child).await;
        self.child = Some(child);
    }

    /// Where the server's stderr goes: a file beside its storage, so a
    /// chatty server never blocks on a pipe nobody reads, and a failing
    /// test can show it.
    fn stderr_log(&self) -> Stdio {
        let path = self.storage_dir.with_file_name("server.stderr.log");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("the server stderr log opens");
        Stdio::from(file)
    }

    fn stderr_text(&self) -> String {
        std::fs::read_to_string(self.storage_dir.with_file_name("server.stderr.log"))
            .unwrap_or_default()
    }

    /// The `--server` target a CLI command reaches this server at.
    fn target(&self) -> String {
        format!("{}/api/v1", self.api_base_url)
    }

    /// Kill the server outright, as a crash would; its workers live on in
    /// their own process groups.
    fn kill(&mut self) {
        let mut child = self.child.take().expect("the server is running");
        child.kill().expect("the server dies");
        let _ = child.wait();
    }

    fn shutdown(mut self) {
        let mut stop = Command::new(env!("CARGO_BIN_EXE_fabro"));
        apply_test_isolation(&mut stop, self.home_root.path());
        stop.args(["server", "stop"])
            .arg("--storage-dir")
            .arg(&self.storage_dir);
        let output = stop.output().expect("server stop runs");
        assert!(
            output.status.success(),
            "server stop failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let status = self
            .child
            .take()
            .expect("the server is running")
            .wait()
            .expect("the server exit status reads");
        assert!(
            status.success(),
            "the server exited unsuccessfully\nstderr:\n{}",
            self.stderr_text()
        );
    }

    /// Petri's store over the server's database, read beside the server:
    /// what `petri inspect` would see.
    async fn petri_store(&self) -> SqliteRunStore {
        let database = fabro_db::Database::connect(Storage::new(&self.storage_dir).sqlite_path())
            .await
            .expect("the server database opens");
        SqliteRunStore::new(database.clone_pool())
    }
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

fn reserve_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("a port binds")
        .local_addr()
        .expect("the listener has an address")
        .port()
}

async fn wait_for_http_ready(base_url: &str, child: &mut Child) {
    let client = fabro_test::test_http_client();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match client.get(format!("{base_url}/health")).send().await {
            Ok(response) if response.status().is_success() => return,
            Ok(_) | Err(_) if Instant::now() < deadline => {
                if let Some(status) = child.try_wait().expect("the server polls") {
                    panic!("the server exited before it was ready with status {status}");
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Ok(response) => panic!("server at {base_url} was not ready: {}", response.status()),
            Err(err) => panic!("server at {base_url} was not ready: {err}"),
        }
    }
}

/// A workspace holding a command-only bundle whose `workflow.toml` names
/// Petri, with the given stage script.
fn write_petri_workspace(context: &fabro_test::TestContext, script: &str) -> PathBuf {
    write_petri_workflow(
        context,
        &format!(
            "digraph Command {{\n  graph [goal=\"Run one command\", default_max_retries=0]\n  start \
             [shape=Mdiamond]\n  exit [shape=Msquare]\n  say [shape=parallelogram, \
             script=\"{script}\", max_retries=0]\n  start -> say -> exit\n}}\n"
        ),
    )
}

/// A workspace holding the given workflow with a `workflow.toml` that names
/// Petri.
fn write_petri_workflow(context: &fabro_test::TestContext, dot: &str) -> PathBuf {
    let workspace = context.temp_dir.join("petri-workspace");
    std::fs::create_dir_all(&workspace).expect("the workspace creates");
    std::fs::write(workspace.join("workflow.fabro"), dot).expect("the workflow writes");
    std::fs::write(
        workspace.join("workflow.toml"),
        "_version = 1\n\n[workflow]\ngraph = \"workflow.fabro\"\nengine = \"petri\"\n\n[run]\ngoal \
         = \"Run one command\"\n",
    )
    .expect("the settings write");
    workspace
}

/// `fabro run --detach --auto-approve` against the server: the run is
/// created and started, and its id comes back.
fn run_detached(
    context: &fabro_test::TestContext,
    server: &RunningServer,
    workspace: &Path,
) -> String {
    run_detached_with(context, server, workspace, &["--auto-approve"])
}

/// `fabro run --detach` against the server with extra arguments.
fn run_detached_with(
    context: &fabro_test::TestContext,
    server: &RunningServer,
    workspace: &Path,
    extra: &[&str],
) -> String {
    let target = server.target();
    seed_dev_token_auth(
        &context.home_dir,
        &ServerTarget::http_url(&target).expect("the target parses"),
        TEST_DEV_TOKEN,
    );
    let output = context
        .run_cmd()
        .current_dir(workspace)
        .args(["--server", &target, "--detach"])
        .args(extra)
        .args(["--environment", "local", "workflow.toml"])
        .output()
        .expect("the detached run executes");
    assert!(
        output.status.success(),
        "detached run failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    created_run_id(&output)
}

async fn run_json(server: &RunningServer, path: &str) -> serde_json::Value {
    let response = fabro_test::test_http_client()
        .get(format!("{}/api/v1/{path}", server.api_base_url))
        .bearer_auth(TEST_DEV_TOKEN)
        .send()
        .await
        .expect("the request sends");
    expect_reqwest_json(
        response,
        fabro_http::StatusCode::OK,
        format!("GET /api/v1/{path}"),
    )
    .await
}

async fn run_status(server: &RunningServer, run_id: &str) -> String {
    run_json(server, &format!("runs/{run_id}")).await["lifecycle"]["status"]["kind"]
        .as_str()
        .expect("the run has a status kind")
        .to_string()
}

async fn wait_for_status(server: &RunningServer, run_id: &str, expected: &[&str]) -> String {
    let deadline = Instant::now() + RUN_TIMEOUT;
    loop {
        let status = run_status(server, run_id).await;
        if expected.contains(&status.as_str()) {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "run {run_id} did not reach {expected:?}; last status {status}"
        );
        tokio::time::sleep(POLL).await;
    }
}

async fn run_events(server: &RunningServer, run_id: &str) -> Vec<EventEnvelope> {
    parse_event_envelopes(&run_json(server, &format!("runs/{run_id}/events")).await)
}

fn event_names(events: &[EventEnvelope]) -> Vec<&str> {
    events
        .iter()
        .map(|envelope| envelope.event.event_name())
        .collect()
}

/// The pid of the worker subprocess the server launched for the run: the
/// worker retitles itself `fabro <first 12 of the run id> <phase>`, so that
/// is what the process table shows.
fn worker_pid(run_id: &str) -> Option<u32> {
    let short_id: String = run_id.chars().take(12).collect();
    let output = Command::new("pgrep")
        .args(["-f", &format!("^fabro {short_id} ")])
        .output()
        .expect("pgrep runs");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.trim().parse().ok())
}

fn wait_for_worker(run_id: &str) -> u32 {
    let deadline = Instant::now() + RUN_TIMEOUT;
    loop {
        if let Some(pid) = worker_pid(run_id) {
            return pid;
        }
        assert!(
            Instant::now() < deadline,
            "no worker process appeared for run {run_id}"
        );
        std::thread::sleep(POLL);
    }
}

/// Whether a process is waiting on the gate file: the stage is mid-flight.
fn gate_is_polled(gate: &Path) -> bool {
    let output = Command::new("pgrep")
        .args(["-f", &gate.display().to_string()])
        .output()
        .expect("pgrep runs");
    output.status.success()
}

fn wait_until_gate_is_polled(gate: &Path) {
    let deadline = Instant::now() + RUN_TIMEOUT;
    while !gate_is_polled(gate) {
        assert!(
            Instant::now() < deadline,
            "the stage never started waiting on {}",
            gate.display()
        );
        std::thread::sleep(POLL);
    }
}

/// A command-only Petri bundle runs to completion in the worker the server
/// launched: Fabro reports the run succeeded, the worker wrote Petri's
/// records through the HTTP store, and its lease ended with it.
#[tokio::test(flavor = "multi_thread")]
async fn a_petri_run_executes_in_the_server_launched_worker() {
    if host_plugin().is_none() {
        return;
    }
    let context = test_context!();
    let server = RunningServer::start().await;
    let workspace = write_petri_workspace(&context, "echo hello from petri");
    let run_id = run_detached(&context, &server, &workspace);

    let status = wait_for_status(&server, &run_id, &["succeeded", "failed"]).await;
    let run = run_json(&server, &format!("runs/{run_id}")).await;
    assert_eq!(status, "succeeded", "run: {run}");
    let state = run_json(&server, &format!("runs/{run_id}/state")).await;
    assert_eq!(state["spec"]["engine"]["kind"], "petri", "state: {state}");

    let events = run_events(&server, &run_id).await;
    let names = event_names(&events);
    assert_eq!(
        names
            .iter()
            .filter(|name| **name == "run.completed")
            .count(),
        1,
        "{names:?}"
    );
    assert!(
        names.contains(&"run.starting") && names.contains(&"run.running"),
        "{names:?}"
    );

    let store = server.petri_store().await;
    let outcome = engine::outcome_of(&store, &run_id)
        .await
        .expect("the run's Petri record inspects");
    assert_eq!(outcome.status, RunStatus::Success, "{outcome:?}");
    assert!(outcome.complete, "{:?}", outcome.incomplete);
    let key = RunKey::new(run_id.as_str());
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let holder = store.owner(&key).await.expect("the lease reads");
        if holder.is_none() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the worker's lease {holder:?} outlived the worker"
        );
        tokio::time::sleep(POLL).await;
    }
    server.shutdown();
}

/// A Petri run whose worker and server both die mid-stage continues after
/// the server restarts: the new server releases the dead worker's lease,
/// asks the run to start again as a resume, and launches a worker in
/// resume mode, which finishes the run with one `run.completed`.
#[tokio::test(flavor = "multi_thread")]
async fn a_petri_run_resumes_in_a_new_worker_after_the_server_restarts() {
    if host_plugin().is_none() {
        return;
    }
    let context = test_context!();
    let mut server = RunningServer::start().await;
    let gate = context.temp_dir.join("resume.gate");
    let script = format!("while [ ! -f {} ]; do sleep 0.05; done", gate.display());
    let workspace = write_petri_workspace(&context, &script);
    let run_id = run_detached(&context, &server, &workspace);

    wait_for_status(&server, &run_id, &["running"]).await;
    eprintln!("run {run_id} is running");
    let worker = wait_for_worker(&run_id);
    eprintln!("worker {worker} launched");
    wait_until_gate_is_polled(&gate);
    eprintln!("stage is waiting on the gate");

    // The crash: the server first, so it never observes the worker exit,
    // then the worker's whole process group, plugin and stage included.
    server.kill();
    fabro_proc::sigkill_process_group(worker);
    let deadline = Instant::now() + Duration::from_secs(10);
    while fabro_proc::process_running(worker) {
        assert!(Instant::now() < deadline, "the worker did not die");
        std::thread::sleep(POLL);
    }
    assert_eq!(
        run_status_offline(&server).await,
        None,
        "the server is down"
    );

    server.launch().await;
    eprintln!("server restarted");
    let status = wait_for_status(&server, &run_id, &["running", "succeeded", "failed"]).await;
    eprintln!("run {run_id} is {status} after the restart");
    let resumed = wait_for_worker(&run_id);
    assert_ne!(resumed, worker, "a new worker was launched");
    eprintln!("worker {resumed} launched for the resume");
    wait_until_gate_is_polled(&gate);
    eprintln!("stage is waiting on the gate again");
    std::fs::write(&gate, "go").expect("the gate opens");

    let status = wait_for_status(&server, &run_id, &["succeeded", "failed"]).await;
    let events = run_events(&server, &run_id).await;
    let names = event_names(&events);
    assert_eq!(
        status,
        "succeeded",
        "events: {names:?}\nserver stderr:\n{}",
        server.stderr_text()
    );
    assert_eq!(
        names
            .iter()
            .filter(|name| **name == "run.completed")
            .count(),
        1,
        "{names:?}"
    );
    // `fabro run` asked for the first start; the restart asked for a
    // resume, after the run had been running.
    let first_running = names
        .iter()
        .position(|name| *name == "run.running")
        .expect("the run ran before the crash");
    let resume_request = events
        .iter()
        .position(|envelope| {
            matches!(
                &envelope.event.body,
                EventBody::RunStartRequested(props) if props.resume
            )
        })
        .expect("the restart asked for a resume");
    assert!(resume_request > first_running, "{names:?}");
    assert_eq!(
        names.iter().filter(|name| **name == "run.running").count(),
        2,
        "the run ran once before and once after the restart: {names:?}"
    );

    let store = server.petri_store().await;
    let outcome = engine::outcome_of(&store, &run_id)
        .await
        .expect("the run's Petri record inspects");
    assert_eq!(outcome.status, RunStatus::Success, "{outcome:?}");
    assert!(outcome.complete, "{:?}", outcome.incomplete);
    server.shutdown();
}

/// The run's status while the server may be down: `None` when it is.
async fn run_status_offline(server: &RunningServer) -> Option<String> {
    fabro_test::test_http_client()
        .get(format!("{}/health", server.api_base_url))
        .send()
        .await
        .ok()
        .map(|response| response.status().to_string())
}

/// The run's pending questions, as the API lists them.
async fn questions(server: &RunningServer, run_id: &str) -> Vec<serde_json::Value> {
    run_json(server, &format!("runs/{run_id}/questions")).await["data"]
        .as_array()
        .cloned()
        .expect("the questions list is an array")
}

/// Wait until `count` questions are pending at once.
async fn wait_for_questions(
    server: &RunningServer,
    run_id: &str,
    count: usize,
) -> Vec<serde_json::Value> {
    let deadline = Instant::now() + RUN_TIMEOUT;
    loop {
        let pending = questions(server, run_id).await;
        if pending.len() >= count {
            return pending;
        }
        assert!(
            Instant::now() < deadline,
            "run {run_id} did not ask {count} question(s); pending: {pending:?}"
        );
        tokio::time::sleep(POLL).await;
    }
}

/// Answer a question through the API, as the web app and the CLI do. The
/// question id is Petri's (`gate#2`), so it travels as one percent-encoded
/// path segment, as the generated clients send it.
async fn answer(server: &RunningServer, run_id: &str, question_id: &str, body: serde_json::Value) {
    let mut url = fabro_http::Url::parse(&format!(
        "{}/api/v1/runs/{run_id}/questions",
        server.api_base_url
    ))
    .expect("the API base URL parses");
    url.path_segments_mut()
        .expect("the API URL has a path")
        .push(question_id)
        .push("answer");
    let response = fabro_test::test_http_client()
        .post(url)
        .bearer_auth(TEST_DEV_TOKEN)
        .json(&body)
        .send()
        .await
        .expect("the answer sends");
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    assert_eq!(
        status,
        fabro_http::StatusCode::NO_CONTENT,
        "POST /api/v1/runs/{run_id}/questions/{question_id}/answer: {body}"
    );
}

/// A yes/no gate whose branches each leave a marker file.
fn gate_dot(markers: &Path, gate_attrs: &str) -> String {
    format!(
        "digraph Gate {{\n  graph [goal=\"Ask before running\"]\n  start [shape=Mdiamond]\n  \
         exit [shape=Msquare]\n  gate [shape=hexagon, label=\"Go?\", \
         question_type=\"yes_no\"{gate_attrs}]\n  yes [shape=parallelogram, script=\"touch \
         {dir}/yes\"]\n  no [shape=parallelogram, script=\"touch {dir}/no\"]\n  start -> gate\n  \
         gate -> yes [label=\"[Y] Yes\"]\n  gate -> no [label=\"[N] No\"]\n  yes -> exit\n  no \
         -> exit\n}}\n",
        dir = markers.display()
    )
}

/// Two gates as the branches of one parallel node; the join's results are
/// written out, so each gate's answer is read from its branch result.
fn two_gates_dot(markers: &Path) -> String {
    format!(
        "digraph Gates {{\n  graph [goal=\"Ask twice at once\"]\n  start [shape=Mdiamond]\n  \
         exit [shape=Msquare]\n  fan [shape=component]\n  a [shape=hexagon, label=\"A?\", \
         question_type=\"yes_no\"]\n  b [shape=hexagon, label=\"B?\", \
         question_type=\"yes_no\"]\n  join [shape=tripleoctagon]\n  report \
         [shape=parallelogram, script=\"cat > {dir}/results.json\", \
         stdin_source=\"context.parallel.results\"]\n  start -> fan\n  fan -> a\n  fan -> b\n  \
         a -> join [label=\"[Y] Yes\"]\n  a -> join [label=\"[N] No\"]\n  b -> join [label=\"[Y] \
         Yes\"]\n  b -> join [label=\"[N] No\"]\n  join -> report -> exit\n}}\n",
        dir = markers.display()
    )
}

/// A human gate in the worker asks through the server: the question is
/// listed by the questions API with the gate's stage and options, the
/// answer reaches the worker over its control channel and routes the gate,
/// and the run's stream records the interview.
#[tokio::test(flavor = "multi_thread")]
async fn a_human_gate_in_the_worker_is_answered_through_the_api() {
    if host_plugin().is_none() {
        return;
    }
    let context = test_context!();
    let server = RunningServer::start().await;
    let markers = context.temp_dir.join("markers");
    std::fs::create_dir_all(&markers).expect("the marker dir creates");
    let workspace = write_petri_workflow(&context, &gate_dot(&markers, ""));
    let run_id = run_detached_with(&context, &server, &workspace, &[]);

    let pending = wait_for_questions(&server, &run_id, 1).await;
    let question = &pending[0];
    assert_eq!(question["stage"], "gate@1", "{question}");
    assert_eq!(question["question_type"], "yes_no", "{question}");
    let question_id = question["id"].as_str().expect("an id").to_string();
    assert!(
        question_id.starts_with("gate#"),
        "Petri's id: {question_id}"
    );
    answer(
        &server,
        &run_id,
        &question_id,
        serde_json::json!({ "kind": "no" }),
    )
    .await;

    let status = wait_for_status(&server, &run_id, &["succeeded", "failed"]).await;
    assert_eq!(
        status,
        "succeeded",
        "server stderr:\n{}",
        server.stderr_text()
    );
    assert!(
        markers.join("no").exists() && !markers.join("yes").exists(),
        "the no branch ran"
    );
    let names = run_events(&server, &run_id).await;
    let names = event_names(&names);
    assert!(
        names.contains(&"interview.started") && names.contains(&"interview.completed"),
        "{names:?}"
    );
    assert!(questions(&server, &run_id).await.is_empty());
    let store = server.petri_store().await;
    let outcome = engine::outcome_of(&store, &run_id)
        .await
        .expect("the run's Petri record inspects");
    assert_eq!(outcome.status, RunStatus::Success, "{outcome:?}");
    server.shutdown();
}

/// Two branches of a parallel node ask at once; each answer, given through
/// the API in the other order, binds to its own branch.
#[tokio::test(flavor = "multi_thread")]
async fn two_parallel_gates_in_the_worker_each_bind_their_own_answer() {
    if host_plugin().is_none() {
        return;
    }
    let context = test_context!();
    let server = RunningServer::start().await;
    let markers = context.temp_dir.join("markers");
    std::fs::create_dir_all(&markers).expect("the marker dir creates");
    let workspace = write_petri_workflow(&context, &two_gates_dot(&markers));
    let run_id = run_detached_with(&context, &server, &workspace, &[]);

    let pending = wait_for_questions(&server, &run_id, 2).await;
    let id_of = |stage: &str| {
        pending
            .iter()
            .find(|question| question["stage"] == stage)
            .and_then(|question| question["id"].as_str())
            .unwrap_or_else(|| panic!("`{stage}` is pending: {pending:?}"))
            .to_string()
    };
    let (a, b) = (id_of("a@1"), id_of("b@1"));
    assert_ne!(a, b);
    for question in &pending {
        assert_eq!(question["question_type"], "yes_no", "{question}");
    }
    // A yes/no question takes `yes` or `no`, as the API validates it.
    answer(&server, &run_id, &b, serde_json::json!({ "kind": "yes" })).await;
    answer(&server, &run_id, &a, serde_json::json!({ "kind": "no" })).await;

    let status = wait_for_status(&server, &run_id, &["succeeded", "failed"]).await;
    assert_eq!(
        status,
        "succeeded",
        "server stderr:\n{}",
        server.stderr_text()
    );
    let results: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(markers.join("results.json")).expect("the join wrote its results"),
    )
    .expect("the results parse");
    let results = results.as_array().expect("a list of branch results");
    assert_eq!(results.len(), 2, "{results:?}");
    assert_eq!(results[0]["id"], "a");
    assert_eq!(results[0]["context_updates"]["human.gate.selected"], "N");
    assert_eq!(results[1]["id"], "b");
    assert_eq!(results[1]["context_updates"]["human.gate.selected"], "Y");
    server.shutdown();
}

/// A gate nobody answers expires on its own deadline: the run takes the
/// gate's default, the stream records the timeout, and nothing stays
/// pending.
#[tokio::test(flavor = "multi_thread")]
async fn an_unanswered_gate_in_the_worker_expires_with_its_default() {
    if host_plugin().is_none() {
        return;
    }
    let context = test_context!();
    let server = RunningServer::start().await;
    let markers = context.temp_dir.join("markers");
    std::fs::create_dir_all(&markers).expect("the marker dir creates");
    let workspace = write_petri_workflow(
        &context,
        &gate_dot(&markers, ", timeout=\"2s\", human.default_choice=\"no\""),
    );
    let run_id = run_detached_with(&context, &server, &workspace, &[]);

    let pending = wait_for_questions(&server, &run_id, 1).await;
    assert_eq!(pending[0]["timeout_seconds"], 2.0, "{}", pending[0]);

    let status = wait_for_status(&server, &run_id, &["succeeded", "failed"]).await;
    assert_eq!(
        status,
        "succeeded",
        "server stderr:\n{}",
        server.stderr_text()
    );
    assert!(
        markers.join("no").exists() && !markers.join("yes").exists(),
        "the default ran"
    );
    let events = run_events(&server, &run_id).await;
    let names = event_names(&events);
    assert!(names.contains(&"interview.timeout"), "{names:?}");
    assert!(questions(&server, &run_id).await.is_empty());
    server.shutdown();
}
