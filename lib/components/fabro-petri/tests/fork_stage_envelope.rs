//! Stage envelopes on Petri, end to end (fabro-aa5f, ADR-0009 rev):
//! the `x.fs_write`/`x.fs_hide`/`x.preamble_*` values ride the run's
//! `graph_source`, the create-time fork lints refuse invalid globs and
//! warn about inconsistent ones, and the checkpoint guard QUARANTINES
//! staged files that fall outside the node's `fs_write` scope (ADR-0051,
//! work#1624 — superseding the run-fatal enforcement, fabro-aa5f): they
//! are excluded from the commit, named in a `run.notice` on the stage's
//! position, and the stage succeeds — the reviewer judges the named
//! deferral. Nothing out of scope ever reaches the committed tree, and
//! the files stay in the workspace.
//!
//! This file is a fork-only presence pin: upstream has no
//! `fork_stage_envelope`, so a merge that drops the feature reds here
//! instead of regressing quietly.

#![expect(
    clippy::disallowed_methods,
    reason = "the tests locate the host plugin through the process environment"
)]
#![expect(clippy::print_stderr, reason = "a skipped test says why on its stderr")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use fabro_checkpoint::author::GitAuthor;
use fabro_petri::admission::AdmittedGraphs;
use fabro_petri::blobs::Blobs;
use fabro_petri::check::{self, Bundle, CheckError, CheckRequest, Launch};
use fabro_petri::checkpoint::{RunGitSettings, RunWorkspaces};
use fabro_petri::controls::RunControls;
use fabro_petri::engine::{self, Execution, RunRequest, RunStatus};
use fabro_petri::fork_stage_envelope::StageEnvelopes;
use fabro_petri::hooks::HooksSpec;
use fabro_petri::platform_records::PlatformRecords;
use fabro_petri::runtime::RuntimeSpec;
use fabro_petri::test_support::{MemoryBlobs, MemoryPlatformRecords};
use fabro_store::PlatformRecord;
use fabro_types::settings::run::RunCheckpointSettings;
use fabro_types::{RunId, SandboxProviderKind};
use petri_execution::inspect;
use petri_store::{Access, MemoryRunStore, RunKey, RunStore as _};
use tokio::fs;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

mod support;

const SETTINGS: &str = "_version = 1\n\n[workflow]\ngraph = \"workflow.fabro\"\n";

/// The notice code the checkpoint quarantine names its deferrals under
/// (ADR-0051): the `RunNoticeCode::StageEnvelopeQuarantined` wire form.
const QUARANTINE_CODE: &str = "stage_envelope_quarantined";

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
        "digraph Envelope {{\n  graph [goal=\"Check the envelope\", default_max_retries=0]\n  \
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

/// An invalid envelope glob refuses the workflow at check, under the fork
/// lint's stable code — Petri itself accepts `x.*` blindly.
/// The per-node run-tools allowlist parses off the graph source and
/// reaches the host-tools filter (fabro-96c6).
#[test]
fn fabro_tools_allowlist_parses_per_node() {
    let workflow = workflow(
        "  work [shape=parallelogram, script=\"echo hi\", \
         x.fabro_tools=\"fabro_run_search,fabro_run_wait\"]\n  bare \
         [shape=parallelogram, script=\"echo no\"]\n",
    );
    let envelopes = StageEnvelopes::parse(&workflow);
    let work = envelopes.envelope("work").expect("work declares tools");
    assert_eq!(
        work.fabro_tools,
        Some(vec![
            "fabro_run_search".to_string(),
            "fabro_run_wait".to_string()
        ])
    );
    assert!(envelopes.envelope("bare").is_none());
}

#[test]
fn invalid_fs_globs_are_refused_at_check() {
    let workflow = workflow(
        "  work [shape=parallelogram, script=\"echo hi\", \
         x.fs_write=\"../escape\"]\n",
    );
    let Err(CheckError::Rejected(diagnostics)) = check::check(&request(&workflow)) else {
        panic!("the invalid glob refuses the workflow");
    };
    let finding = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "fork.fs_globs_valid")
        .expect("the fork lint fired");
    assert!(finding.is_error(), "{finding:?}");
    assert!(finding.message.contains("work"), "{finding:?}");
}

/// `x.fs_write=""` is a read-only stage (fabro-ba96): the envelope holds
/// an empty write list — which is not the same as no envelope at all, the
/// `None` that means the node declares none — and Petri's check admits
/// the workflow. The Attractor frontend's attribute surface takes the
/// empty value; fabro's envelope reader is what gives it meaning.
#[test]
fn an_empty_fs_write_is_a_read_only_stage() {
    let workflow = workflow("  work [shape=parallelogram, script=\"echo hi\", x.fs_write=\"\"]\n");
    let envelopes = StageEnvelopes::parse(&workflow);
    let work = envelopes
        .envelope("work")
        .expect("an empty value still declares the envelope");
    assert_eq!(
        work.fs_write,
        Some(Vec::new()),
        "an empty list admits no write at all"
    );
    let scope = envelopes
        .fs_scope("work")
        .expect("the node declares the envelope")
        .expect("the empty envelope compiles");
    assert!(
        scope.check_write("/workspace", "workflow.fabro").is_err(),
        "nothing is writable under an empty fs_write"
    );

    let admitted = check::check(&request(&workflow)).expect("the bundle is admitted");
    let refusals: Vec<&str> = admitted
        .warnings
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .filter(|code| code.starts_with("fork.fs_"))
        .collect();
    assert!(refusals.is_empty(), "no fs finding fires for {refusals:?}");
}

/// Consistency findings warn without refusing: an inline ceiling above the
/// budget, and a write entry under a hide root.
#[test]
fn consistency_findings_warn_without_refusing() {
    let workflow = workflow(
        "  work [shape=parallelogram, script=\"echo hi\", \
         x.preamble_inline_max_kb=64, x.fs_hide=\"lib/**\", \
         x.fs_write=\"lib/**\"]\n",
    );
    // The graph-level budget: the default (48) is below the node's 64.
    let admitted = check::check(&request(&workflow)).expect("the bundle is admitted");
    let codes: Vec<&str> = admitted
        .warnings
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    assert!(
        codes.contains(&"fork.preamble_budget_consistency"),
        "{codes:?}"
    );
    assert!(codes.contains(&"fork.fs_scope_consistency"), "{codes:?}");
}

/// One run's pieces for the guard scenarios, with the envelopes the worker
/// would parse off the run's `graph_source`.
struct Harness {
    run_id:  RunId,
    run_dir: PathBuf,
    store:   Arc<MemoryRunStore>,
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
            store:   Arc::new(MemoryRunStore::new()),
            records: Arc::new(MemoryPlatformRecords::new()),
            blobs:   Arc::new(MemoryBlobs::new()),
            _root:   root,
        }
    }

    async fn run(&self, workflow: &str) -> engine::RunOutcome {
        let admitted = check::check(&request(workflow)).expect("the bundle is admitted");
        let (interviewer, observers) = no_questions();
        let hooks = HooksSpec {
            records:          Arc::clone(&self.records) as Arc<dyn PlatformRecords>,
            git:              RunGitSettings {
                host_workspaces: true,
                ..RunGitSettings::default()
            },
            artifacts:        Vec::new(),
            envelopes:        Some(Arc::new(StageEnvelopes::parse(workflow))),
            hook_write_roots: Vec::new(),
            test_gates:       None,
        };
        let request = RunRequest {
            run_id: self.run_id.to_string(),
            run_dir: self.run_dir.clone(),
            execution: Execution::Start(AdmittedGraphs {
                graph:    admitted.graph,
                children: admitted.children,
            }),
            store: Arc::clone(&self.store) as Arc<dyn petri_store::RunStore>,
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

    /// The one workspace the run's root scope used.
    async fn workspace(&self) -> String {
        let mut names = Vec::new();
        let mut entries = fs::read_dir(self.run_dir.join("scopes"))
            .await
            .expect("the scopes directory exists");
        while let Some(entry) = entries.next_entry().await.expect("an entry reads") {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
        assert_eq!(names.len(), 1, "one workspace: {names:?}");
        names.remove(0)
    }

    /// The run's workspaces, as the hooks reach them.
    fn workspaces(&self) -> RunWorkspaces {
        RunWorkspaces::new(
            self.run_dir.clone(),
            self.run_id.to_string(),
            GitAuthor::default(),
            &RunCheckpointSettings::default(),
        )
    }

    /// Where `workspace` lives on this host.
    fn workspace_path(&self, workspace: &str) -> PathBuf {
        self.workspaces().workspace_path(workspace)
    }

    /// The sha of `node`'s own checkpoint commit: the tree the stage's
    /// snapshot carries, as the node's envelope shaped it. A later
    /// unrestricted stage may carry a quarantined file — the gate is the
    /// committing node's scope — so the pin reads the enforcing node's
    /// commit, not the branch head.
    async fn checkpoint_sha_of(&self, node: &str) -> String {
        let logs = self
            .store
            .open(&RunKey::new(self.run_id.to_string()), Access::Read)
            .await
            .expect("the run opens for reading");
        let inspection = inspect::inspect_run(&*logs)
            .await
            .expect("the stored run inspects");
        let history: Vec<(u64, String)> = inspection
            .executions
            .iter()
            .filter_map(|execution| execution.engine.as_ref())
            .flat_map(|engine| engine.history.iter())
            .map(|record| (record.firing, record.node.to_string()))
            .collect();
        self.records
            .records(&self.run_id)
            .into_iter()
            .filter_map(|stored| match stored.record {
                PlatformRecord::Checkpoint(record) => Some(record),
                _ => None,
            })
            .find(|record| {
                history
                    .iter()
                    .any(|(firing, name)| *firing == record.firing && name == node)
            })
            .and_then(|record| record.git_commit_sha)
            .unwrap_or_else(|| panic!("the {node} stage recorded a checkpoint naming its commit"))
    }
}

/// A stage that writes outside its `x.fs_write` scope is QUARANTINED, not
/// fatal (ADR-0051, work#1624 — supersedes the run-fatal envelope pin,
/// fabro-aa5f): the run succeeds, the out-of-scope path is excluded from
/// the committed tree while the in-scope work lands, the deferral is
/// named in a `run.notice` on the stage's position, and the file stays
/// in the workspace.
#[tokio::test]
async fn a_write_outside_fs_write_is_quarantined_and_named_not_fatal() {
    if !host_plugin_ready() {
        return;
    }
    let harness = Harness::new();
    let workflow = workflow(
        "  work [shape=parallelogram, script=\"mkdir -p allowed && echo ok > allowed/a.txt \
         && echo escape > escape.txt\", x.fs_write=\"allowed/**\"]\n",
    );
    let outcome = harness.run(&workflow).await;
    assert_eq!(
        outcome.status,
        RunStatus::Success,
        "a quarantined write does not fail the run: {outcome:?}"
    );
    assert!(
        outcome.failure.is_none(),
        "the quarantine is not a run failure: {:?}",
        outcome.failure
    );

    // The committed tree holds the in-scope work and nothing out of
    // scope: the quarantining stage's own checkpoint commit. (A later
    // unrestricted stage may carry a quarantined file — the gate is the
    // committing node's scope, the file stays in the shared workspace —
    // so the pin reads the enforcing node's commit, not the branch head.)
    let workspace = harness.workspace().await;
    let repository = harness.workspaces().snapshot_repository(&workspace);
    let sha = harness.checkpoint_sha_of("work").await;
    let tree = git(&repository, &["ls-tree", "-r", "--name-only", &sha]).await;
    assert!(
        tree.lines().any(|path| path == "allowed/a.txt"),
        "the in-scope work is committed: {tree}"
    );
    assert!(
        !tree.lines().any(|path| path == "escape.txt"),
        "the out-of-scope path is not committed: {tree}"
    );

    // The deferral is named on the stage's outcome: the quarantine
    // notice carries the node and the path.
    let notice = harness
        .records
        .records(&harness.run_id)
        .into_iter()
        .find_map(|stored| match stored.record {
            PlatformRecord::RunNotice(record) => (record.code == QUARANTINE_CODE).then_some(record),
            _ => None,
        })
        .expect("the quarantine is named in a run notice");
    assert!(
        notice.message.contains("work"),
        "the node is named: {notice:?}"
    );
    assert!(
        notice.message.contains("escape.txt"),
        "the path is named: {notice:?}"
    );

    // The file stays in the workspace: the sandbox is disposable, the
    // quarantine never deletes.
    assert!(
        harness
            .workspace_path(&workspace)
            .join("escape.txt")
            .is_file(),
        "the quarantined file stays in the workspace"
    );
}

/// A stage that stays inside its scope commits normally: the envelope
/// restricts, it does not block.
#[tokio::test]
async fn a_write_inside_fs_write_commits() {
    if !host_plugin_ready() {
        return;
    }
    let harness = Harness::new();
    let workflow = workflow(
        "  work [shape=parallelogram, script=\"mkdir -p allowed && echo ok > \
         allowed/a.txt\", x.fs_write=\"allowed/**\"]\n",
    );
    let outcome = harness.run(&workflow).await;
    assert_eq!(outcome.status, RunStatus::Success, "{outcome:?}");
    assert!(!harness.records.records(&harness.run_id).is_empty());
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

/// `git` in a repository, its stdout.
async fn git(path: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .await
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}
