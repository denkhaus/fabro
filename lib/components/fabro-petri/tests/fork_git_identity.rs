//! The run's Git identity in its sandboxes, end to end (fabro-19f9):
//! every process a run's sandbox spawns carries the run's
//! `GIT_AUTHOR_*`/`GIT_COMMITTER_*` identity, so a stage that commits
//! inside its workspace commits as the run — the revisor's
//! `Author identity unknown` (run 01M441TACSP4, pass 2) cannot recur —
//! without configuring a repository-local identity first.
//!
//! This file is a fork-only presence pin: upstream has no
//! `fork_git_identity`, so a merge that drops the feature reds here
//! instead of regressing quietly.

#![expect(
    clippy::disallowed_methods,
    reason = "the tests locate the host plugin through the process environment and inspect the workspace's Git history"
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
use tokio::fs;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

mod support;

const SETTINGS: &str = "_version = 1\n\n[workflow]\ngraph = \"workflow.fabro\"\n";

/// The run's author, as `[run.git.author]` would name it: distinct from
/// the default identity, so the pin cannot pass on a default or on the
/// host's own global Git configuration.
const AUTHOR: &str = "Fabro Pin";
const AUTHOR_EMAIL: &str = "pin@fabro.test";

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
        "digraph GitIdentity {{\n  graph [goal=\"Check the identity\", default_max_retries=0]\n  \
         start [shape=Mdiamond]\n  exit [shape=Msquare]\n{stages}\n  start -> work -> exit\n}}\n"
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

    /// The workspace's path on this host, as the hooks name it.
    fn workspace_path(&self) -> PathBuf {
        self.workspaces().workspace_path(WORKSPACE)
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

    /// The snapshot repository of the run's workspace: what the checkpoints
    /// published, the stage's own commit included.
    fn snapshot_repository(&self) -> PathBuf {
        self.workspaces().snapshot_repository(WORKSPACE)
    }
}

/// A stage that commits inside its workspace — `git init`, `git add`,
/// `git commit`, nothing configured — commits as the run, and its process
/// carries the run's identity. Without the injection the stage writes four
/// empty variables and no commit survives as the run.
#[tokio::test]
async fn a_stage_commits_as_the_runs_git_identity() {
    if !host_plugin_ready() {
        return;
    }
    let harness = Harness::new();
    let workflow = workflow(
        "  work [shape=parallelogram, script=\"echo \\
         \\\"$GIT_AUTHOR_NAME|$GIT_AUTHOR_EMAIL|$GIT_COMMITTER_NAME|$GIT_COMMITTER_EMAIL\\\" > \
         identity.txt && git init -q && git add -A && git commit -q -m stage-commit\"]\n",
    );
    let outcome = harness.run(&workflow).await;
    assert_eq!(outcome.status, RunStatus::Success, "{outcome:?}");
    assert!(outcome.complete, "{:?}", outcome.incomplete);

    let identity = fs::read_to_string(harness.workspace_path().join("identity.txt"))
        .await
        .expect("the stage wrote its identity");
    assert_eq!(
        identity.trim(),
        "Fabro Pin|pin@fabro.test|Fabro Pin|pin@fabro.test",
        "the run's identity reached the stage's process"
    );

    // The stage's own commit, published with the workspace's snapshots:
    // its author and committer are the run, not the host's Git identity.
    let log = git(&harness.snapshot_repository(), &[
        "log",
        "--all",
        "--format=%an|%ae|%cn|%ce|%s",
    ])
    .await;
    let committed = log
        .lines()
        .find(|line| line.ends_with("|stage-commit"))
        .unwrap_or_else(|| panic!("the stage's commit is published: {log}"));
    assert_eq!(
        committed, "Fabro Pin|pin@fabro.test|Fabro Pin|pin@fabro.test|stage-commit",
        "the stage's commit names the run as author and committer"
    );
}

/// The engine's own checkpoint commits keep the run's identity too: the
/// environment the facet applies outranks the `-c user.name=...` the
/// checkpoint passes, and both carry the run's author.
#[tokio::test]
async fn the_engines_checkpoints_keep_the_runs_identity() {
    if !host_plugin_ready() {
        return;
    }
    let harness = Harness::new();
    let workflow = workflow("  work [shape=parallelogram, script=\"echo ok > out.txt\"]\n");
    let outcome = harness.run(&workflow).await;
    assert_eq!(outcome.status, RunStatus::Success, "{outcome:?}");

    let log = git(&harness.snapshot_repository(), &[
        "log",
        "--all",
        "--format=%an|%ae",
    ])
    .await;
    assert!(
        !log.trim().is_empty(),
        "the checkpoint published at least one commit"
    );
    for line in log.lines() {
        assert_eq!(
            line, "Fabro Pin|pin@fabro.test",
            "every published commit names the run"
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

/// Run `git` in `repository` and return its stdout: the tests read the
/// published history, never a result they assert on.
async fn git(repository: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(repository)
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
