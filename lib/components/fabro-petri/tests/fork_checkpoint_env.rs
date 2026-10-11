//! The checkpoint's author under an ambient identity (fabro-114c),
//! pinned fork-only.
//!
//! Git resolves the commit author from `GIT_AUTHOR_*` /
//! `GIT_COMMITTER_*` before any `-c user.name/email` config, so an
//! ambient identity in the engine process would override the
//! checkpoint's own author. Every run sandbox carries the stage-env
//! identity injection (fabro-19f9), and the cost of missing this guard
//! was two red develop cycles: checkpoint commits landed as the ambient
//! identity, and the runs failed their own publish path
//! (fabro-114c). The fix strips the four identity variables from the
//! checkpoint's Git command in `checkpoint.rs`.
//!
//! The first pin lives in `tests/hooks.rs` — an upstream-owned file a
//! merge can rewrite. This file is the fork's presence pin: it fails if
//! an upstream merge drops the env-strip out of the checkpoint, because
//! the engine's checkpoint commits name the ambient impostor again.

#![expect(
    clippy::disallowed_methods,
    reason = "the pin plants an ambient identity the way a polluted engine environment would, and inspects the workspace's Git history"
)]
#![expect(clippy::print_stderr, reason = "a skipped test says why on its stderr")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use fabro_checkpoint::author::GitAuthor;
use fabro_petri::admission::AdmittedGraphs;
use fabro_petri::blobs::Blobs;
use fabro_petri::check::{self, Bundle, CheckRequest, Launch};
use fabro_petri::checkpoint::{RunGitSettings, RunWorkspaces};
use fabro_petri::controls::RunControls;
use fabro_petri::engine::{self, Execution, RunRequest, RunStatus};
use fabro_petri::hooks::HooksSpec;
use fabro_petri::platform_records::PlatformRecords;
use fabro_petri::runtime::RuntimeSpec;
use fabro_petri::test_support::{MemoryBlobs, MemoryPlatformRecords};
use fabro_types::settings::run::{GitAuthorSettings, RunCheckpointSettings, RunNamespace};
use fabro_types::{RunId, SandboxProviderKind};
use petri_store::MemoryRunStore;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

mod support;

const SETTINGS: &str = "_version = 1\n\n[workflow]\ngraph = \"workflow.fabro\"\n";

/// The run's author, as `[run.git.author]` would name it: distinct from
/// both the impostor below and any host-level Git configuration, so the
/// pin cannot pass on a default.
const AUTHOR: &str = "Fabro Pin";
const AUTHOR_EMAIL: &str = "pin@fabro.test";

/// The ambient impostor identity, the shape a polluted engine
/// environment carries: distinct from the run's author, so a checkpoint
/// that resolves from the environment names it and the pin fails.
const IMPOSTOR: &str = "Ambient Impostor";
const IMPOSTOR_EMAIL: &str = "impostor@ambient.test";

/// The one scope a single-scope workflow's workspace belongs to.
const WORKSPACE: &str = "invocation-0-scope-0";

/// The host plugin check the scenario tests skip without: the same rule
/// the hooks suite applies.
fn host_plugin_ready() -> bool {
    let found = std::env::var_os("PETRI_SANDBOX_HOST_PLUGIN")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("PATH").and_then(|paths| {
                std::env::split_paths(&paths)
                    .map(|dir| dir.join("sandbox-driver-host"))
                    .find(|candidate| candidate.is_file())
            })
        });
    if found.is_none() {
        eprintln!("skipping: sandbox-driver-host is not on PATH");
    }
    found.is_some()
}

fn workflow(stages: &str) -> String {
    format!(
        "digraph CheckpointAuthor {{\n  graph [goal=\"Pin the checkpoint author under an \
         ambient identity\", default_max_retries=0]\n  start [shape=Mdiamond] exit \
         [shape=Msquare]\n{stages}\n  start -> work -> exit\n}}\n"
    )
}

fn request(workflow: &str) -> CheckRequest {
    CheckRequest {
        bundle:             Bundle {
            files:        BTreeMap::from([
                ("workflow.fabro".to_string(), workflow.to_string()),
                ("workflow.toml".to_string(), SETTINGS.to_string()),
            ]),
            entrypoint:   "workflow.fabro".to_string(),
            project_toml: None,
        },
        inputs:             BTreeMap::new(),
        vars:               BTreeMap::new(),
        launch:             Launch::default(),
        runtime:            RuntimeSpec::default(),
        unbound_is_warning: false,
    }
}

/// One run's pieces, with the run's author the way the worker's settings
/// name it.
struct Harness {
    run_id:  RunId,
    run_dir: PathBuf,
    records: Arc<MemoryPlatformRecords>,
    blobs:   Arc<MemoryBlobs>,
    _root:   tempfile::TempDir,
}

impl Harness {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("a temp dir");
        Self {
            run_id:  RunId::new(),
            run_dir: root.path().join("run"),
            records: Arc::new(MemoryPlatformRecords::new()),
            blobs:   Arc::new(MemoryBlobs::new()),
            _root:   root,
        }
    }

    async fn run(&self, workflow: &str) -> engine::RunOutcome {
        let admitted = check::check(&request(workflow)).expect("the bundle is admitted");
        let (interviewer, observers) = no_questions();
        // The run's settings, as `[run.git.author]` would give them:
        // `RunGitSettings::from` derives the identity source, so the
        // fixture never restates the production rule.
        let git = named_git_settings(AUTHOR, AUTHOR_EMAIL);
        let hooks = HooksSpec {
            records: Arc::clone(&self.records) as Arc<dyn PlatformRecords>,
            git,
            artifacts: Vec::new(),
            envelopes: None,
            hook_write_roots: Vec::new(),
            test_gates: None,
        };
        let request = RunRequest {
            run_id: self.run_id.to_string(),
            run_dir: self.run_dir.clone(),
            execution: Execution::Start(AdmittedGraphs {
                graph:    admitted.graph,
                children: admitted.children,
            }),
            store: Arc::new(MemoryRunStore::new()),
            runtime: RuntimeSpec::default(),
            provider: SandboxProviderKind::LOCAL,
            cancel: CancellationToken::new(),
            controls: RunControls::new(),
            interviewer,
            observers,
            secrets: None,
            blobs: Some(Arc::clone(&self.blobs) as Arc<dyn Blobs>),
            hooks: Some(hooks),
        };
        engine::run(request).await.expect("the run executes")
    }

    fn workspaces(&self) -> RunWorkspaces {
        RunWorkspaces::new(
            self.run_dir.clone(),
            self.run_id.to_string(),
            GitAuthor {
                name:  AUTHOR.to_string(),
                email: AUTHOR_EMAIL.to_string(),
            },
            &RunCheckpointSettings::default(),
        )
    }

    /// The snapshot repository of the run's workspace: what the
    /// checkpoints published.
    fn snapshot_repository(&self) -> PathBuf {
        self.workspaces().snapshot_repository(WORKSPACE)
    }
}

/// The engine's checkpoint commits keep the run's configured author even
/// when the engine process carries an ambient `GIT_AUTHOR_*` /
/// `GIT_COMMITTER_*` identity: the fix strips the four variables from
/// the checkpoint's Git command, so the command-line identity wins.
/// The stage never commits, so every published commit is the engine's
/// own — the pin cannot pass on the stage-env injection covering for a
/// missing strip.
#[tokio::test]
async fn checkpoint_commits_keep_the_runs_author_under_an_ambient_identity() {
    if !host_plugin_ready() {
        return;
    }
    std::env::set_var("GIT_AUTHOR_NAME", IMPOSTOR);
    std::env::set_var("GIT_AUTHOR_EMAIL", IMPOSTOR_EMAIL);
    std::env::set_var("GIT_COMMITTER_NAME", IMPOSTOR);
    std::env::set_var("GIT_COMMITTER_EMAIL", IMPOSTOR_EMAIL);

    let harness = Harness::new();
    let workflow = workflow("  work [shape=parallelogram, script=\"echo ok > out.txt\"]\n");
    let outcome = harness.run(&workflow).await;
    assert_eq!(outcome.status, RunStatus::Success, "{outcome:?}");
    assert!(outcome.complete, "{:?}", outcome.incomplete);

    let log = git(&harness.snapshot_repository(), &[
        "log",
        "--all",
        "--format=%an|%ae|%cn|%ce",
    ])
    .await;
    assert!(
        !log.trim().is_empty(),
        "the checkpoint published at least one commit"
    );
    for line in log.lines() {
        assert_eq!(
            line, "Fabro Pin|pin@fabro.test|Fabro Pin|pin@fabro.test",
            "every checkpoint commit names the run's author, not the ambient identity: {log}"
        );
    }
}

/// The run's Git settings for an authored run, derived the way the worker
/// derives them (`RunGitSettings::from`): the fixture never restates the
/// identity-source rule.
fn named_git_settings(name: &str, email: &str) -> RunGitSettings {
    let mut namespace = RunNamespace::default();
    namespace.git.author = Some(GitAuthorSettings {
        name:  Some(name.to_string()),
        email: Some(email.to_string()),
    });
    let mut git = RunGitSettings::from(&namespace);
    git.host_workspaces = true;
    git
}

/// An interviewer for a run that asks nothing, with its expiry observer.
fn no_questions() -> (
    Arc<dyn petri_execution::Interviewer>,
    Vec<Arc<dyn petri_execution::ExecutionObserver>>,
) {
    let interviewer = support::no_questions(Arc::new(support::Silent));
    let observers = vec![interviewer.observer()];
    (Arc::new(interviewer), observers)
}

/// Run `git` in `repository` and return its stdout: the test reads the
/// published history, never a result it asserts on. The variables are
/// removed first so the reader cannot inherit the impostor either.
async fn git(repository: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(repository)
        .env_remove("GIT_AUTHOR_NAME")
        .env_remove("GIT_AUTHOR_EMAIL")
        .env_remove("GIT_COMMITTER_NAME")
        .env_remove("GIT_COMMITTER_EMAIL")
        .args(args)
        .output()
        .await
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}
