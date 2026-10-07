//! Context tokens through Fabro's own engine (fabro-e71b): `{{ context.NAME }}`
//! survives Fabro's lowering verbatim (scripts keep non-`inputs`/`vars`
//! tokens; the frontend masks prompt tokens behind PUA sentinels), and the
//! stage-dispatch pass resolves them against the run context — strictly, so
//! an unresolved token fails the run naming the token and the visible keys.
//!
//! This file is a fork-only presence pin (the petri fork's
//! `fork_context_tokens` carries the feature; upstream has none of it): a merge
//! that drops the feature reds here instead of regressing quietly.

#![expect(
    clippy::disallowed_methods,
    reason = "the tests locate the host plugin through the process environment"
)]
#![expect(clippy::print_stderr, reason = "a skipped test says why on its stderr")]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use fabro_petri::admission::AdmittedGraphs;
use fabro_petri::blobs::Blobs;
use fabro_petri::check::{self, Bundle, CheckRequest, Launch};
use fabro_petri::checkpoint::RunGitSettings;
use fabro_petri::controls::RunControls;
use fabro_petri::engine::{self, Execution, RunRequest, RunStatus};
use fabro_petri::hooks::HooksSpec;
use fabro_petri::platform_records::PlatformRecords;
use fabro_petri::runtime::RuntimeSpec;
use fabro_petri::test_support::{MemoryBlobs, MemoryPlatformRecords};
use fabro_types::{RunId, SandboxProviderKind};
use petri_store::MemoryRunStore;
use tokio_util::sync::CancellationToken;

mod support;

const SETTINGS: &str = "_version = 1\n\n[workflow]\ngraph = \"workflow.fabro\"\n";

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
        "digraph Tokens {{\n  graph [goal=\"Resolve context tokens\", default_max_retries=0]\n  \
         start [shape=Mdiamond]\n  exit [shape=Msquare]\n{stages}\n  start -> seed -> check -> exit\n}}\n"
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

/// A `{{ context.NAME }}` token in a script survives Fabro's lowering
/// verbatim: the admitted graph carries the token, not a rendering.
#[test]
fn a_script_token_survives_fabro_lowering_verbatim() {
    let workflow = workflow(
        "  seed [shape=parallelogram, output_schema=\"routing\", script=\"echo \
         '{\\\"context_updates\\\": {\\\"seed_id\\\": \\\"fabro-e71b\\\"}}'\"]\n  \
         check [shape=parallelogram, script=\"echo {{ context.seed_id }}\"]\n",
    );
    let admitted = check::check(&request(&workflow)).expect("the bundle is admitted");
    let config = admitted
        .graph
        .nodes
        .iter()
        .find(|node| node.name == "check")
        .expect("the check node")
        .step
        .config
        .clone();
    assert_eq!(
        config.get("script").and_then(|v| v.as_str()),
        Some("echo {{ context.seed_id }}"),
        "the token rides the admitted graph verbatim: {config}"
    );
}

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
        let hooks = HooksSpec {
            records:          Arc::clone(&self.records) as Arc<dyn PlatformRecords>,
            git:              RunGitSettings::default(),
            artifacts:        Vec::new(),
            envelopes:        None,
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
}

/// A seeding stage plus a script that proves the resolved value: the run
/// stays green only when `{{ context.seed_id }}` became `fabro-e71b`.
#[tokio::test]
async fn a_script_token_resolves_at_dispatch_through_the_fabro_engine() {
    if !host_plugin_ready() {
        return;
    }
    let harness = Harness::new();
    let workflow = workflow(
        "  seed [shape=parallelogram, output_schema=\"routing\", script=\"echo \
         '{\\\"context_updates\\\": {\\\"seed_id\\\": \\\"fabro-e71b\\\"}}'\"]\n  \
         check [shape=parallelogram, script=\"echo seed={{ context.seed_id }} | grep -qx seed=fabro-e71b\"]\n",
    );
    let outcome = harness.run(&workflow).await;
    assert_eq!(outcome.status, RunStatus::Success, "{outcome:?}");
}

/// Strict resolution through Fabro's engine: an unresolved token fails the
/// run naming the token and the visible keys.
#[tokio::test]
async fn an_unresolved_token_fails_the_run_naming_the_token() {
    if !host_plugin_ready() {
        return;
    }
    let harness = Harness::new();
    let workflow = workflow(
        "  seed [shape=parallelogram, output_schema=\"routing\", script=\"echo \
         '{\\\"context_updates\\\": {\\\"seed_id\\\": \\\"fabro-e71b\\\"}}'\"]\n  \
         check [shape=parallelogram, script=\"echo {{ context.absent_key }}\", on_failure=\"exit\"]\n",
    );
    let outcome = harness.run(&workflow).await;
    assert_eq!(outcome.status, RunStatus::Failed, "{outcome:?}");
    let failure = outcome.failure.expect("the failure message");
    assert!(
        failure.contains("`{{ context.absent_key }}`"),
        "names the token: {failure}"
    );
    assert!(
        failure.contains("visible context keys") && failure.contains("seed_id"),
        "names the visible keys: {failure}"
    );
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
