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
//!
//! The harness here (the server, the detached run, the status and event
//! reads) is shared with the run-tools scenarios in `petri_tools.rs`.

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
use fabro_vault::{SecretType, Vault};

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
pub(super) fn host_plugin() -> Option<PathBuf> {
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
pub(super) struct RunningServer {
    child:                   Option<Child>,
    home_root:               tempfile::TempDir,
    _storage_root:           tempfile::TempDir,
    pub(super) storage_dir:  PathBuf,
    config_path:             PathBuf,
    port:                    u16,
    pub(super) api_base_url: String,
}

impl RunningServer {
    pub(super) async fn start() -> Self {
        Self::start_with("", &[]).await
    }

    /// Start with `settings` appended to the server's settings file (the
    /// workers read the same file through `FABRO_CONFIG`) and `secrets`
    /// in the vault before the first launch, so the server and its workers
    /// see them from the start.
    pub(super) async fn start_with(settings: &str, secrets: &[(&str, &str)]) -> Self {
        let home_root = tempfile::tempdir_in("/tmp").expect("home tempdir");
        let storage_root = isolated_storage_dir();
        let storage_dir = storage_root.path().join("storage");
        let port = reserve_port();
        let config_path = home_root.path().join("settings.toml");
        std::fs::write(
            &config_path,
            format!("_version = 1\n\n[server.auth]\nmethods = [\"dev-token\"]\n{settings}"),
        )
        .expect("the server settings write");
        if !secrets.is_empty() {
            let mut vault = Vault::load(Storage::new(&storage_dir).secrets_path())
                .expect("the server vault loads");
            for (name, value) in secrets {
                vault
                    .set(name, value, SecretType::Token, None)
                    .expect("the secret stores in the server vault");
            }
        }
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

    pub(super) fn stderr_text(&self) -> String {
        std::fs::read_to_string(self.storage_dir.with_file_name("server.stderr.log"))
            .unwrap_or_default()
    }

    /// The `--server` target a CLI command reaches this server at.
    pub(super) fn target(&self) -> String {
        format!("{}/api/v1", self.api_base_url)
    }

    /// Kill the server outright, as a crash would; its workers live on in
    /// their own process groups.
    fn kill(&mut self) {
        let mut child = self.child.take().expect("the server is running");
        child.kill().expect("the server dies");
        let _ = child.wait();
    }

    pub(super) fn shutdown(mut self) {
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
    pub(super) async fn petri_store(&self) -> SqliteRunStore {
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
    let workspace = context.temp_dir.join("petri-workspace");
    std::fs::create_dir_all(&workspace).expect("the workspace creates");
    std::fs::write(
        workspace.join("workflow.fabro"),
        format!(
            "digraph Command {{\n  graph [goal=\"Run one command\", default_max_retries=0]\n  start \
             [shape=Mdiamond]\n  exit [shape=Msquare]\n  say [shape=parallelogram, \
             script=\"{script}\", max_retries=0]\n  start -> say -> exit\n}}\n"
        ),
    )
    .expect("the workflow writes");
    std::fs::write(
        workspace.join("workflow.toml"),
        "_version = 1\n\n[workflow]\ngraph = \"workflow.fabro\"\nengine = \"petri\"\n\n[run]\ngoal \
         = \"Run one command\"\n",
    )
    .expect("the settings write");
    workspace
}

/// `fabro run --detach` against the server: the run is created and started,
/// and its id comes back.
fn run_detached(
    context: &fabro_test::TestContext,
    server: &RunningServer,
    workspace: &Path,
) -> String {
    run_detached_with(context, server, workspace, &[])
}

/// [`run_detached`] with `extra` arguments on the command, such as the
/// model to run the workflow's agents on.
pub(super) fn run_detached_with(
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
        .args([
            "--server",
            &target,
            "--detach",
            "--auto-approve",
            "--environment",
            "local",
        ])
        .args(extra)
        .arg("workflow.toml")
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

pub(super) async fn run_json(server: &RunningServer, path: &str) -> serde_json::Value {
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

pub(super) async fn wait_for_status(
    server: &RunningServer,
    run_id: &str,
    expected: &[&str],
) -> String {
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
