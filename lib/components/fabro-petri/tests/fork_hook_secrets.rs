//! Per-hook declared secrets (fabro-225f), pinned fork-only: a
//! `sandbox = false` command hook's `secrets = [...]` declaration resolves
//! against the secret provider the RuntimeSpec installs and reaches
//! exactly that hook's process environment — the provider-agnostic
//! credential channel that replaced env allowlisting (no hardcoded env
//! names in the worker spawn, no credential file on disk).
//!
//! The engine half lives in the petri fork (`petri 78c524b`: lowering
//! validation, `LocalHooks::set_secrets`, host-command injection); this
//! file pins FABRO's half: the RuntimeSpec wiring that hands the run's
//! provider to the local hook service, and the admission rejection that
//! keeps a sandboxed hook from ever declaring one. An upstream merge that
//! drops either half fails here.

#![expect(
    clippy::disallowed_methods,
    reason = "the harness reads provider environment variables the way the worker's runtime spec does"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use fabro_petri::admission::AdmittedGraphs;
use fabro_petri::blobs::Blobs;
use fabro_petri::check::{self, Bundle, CheckRequest, Launch};
use fabro_petri::controls::RunControls;
use fabro_petri::engine::{self, Execution, RunRequest, RunStatus};
use fabro_petri::hooks::HooksSpec;
use fabro_petri::platform_records::PlatformRecords;
use fabro_petri::providers::SandboxProviderConfig;
use fabro_petri::runtime::RuntimeSpec;
use fabro_petri::secrets::HookSecretProvider;
use fabro_petri::test_support::{MemoryBlobs, MemoryPlatformRecords};
use fabro_types::settings::run::RunNamespace;
use fabro_types::{RunId, SandboxProviderKind};
use petri_runtime::executor::MapSecrets;
use petri_store::MemoryRunStore;
use tokio::fs;
use tokio_util::sync::CancellationToken;

mod support;

/// A one-stage command bundle: the stage line goes between `start` and
/// `exit`.
fn workflow(stage: &str) -> String {
    format!(
        "digraph HookSecrets {{\n  graph [goal=\"Pin per-hook declared secrets\", \
         default_max_retries=0]\n  start [shape=Mdiamond]\n  exit [shape=Msquare]\n{stage}\n  \
         start -> writer -> exit\n}}\n"
    )
}

/// The bundle admitted the way the create handler admits it.
fn admit(workflow: &str, settings: &str) -> AdmittedGraphs {
    let request = CheckRequest {
        bundle:             Bundle {
            files:        BTreeMap::from([
                ("workflow.fabro".to_string(), workflow.to_string()),
                ("workflow.toml".to_string(), settings.to_string()),
            ]),
            entrypoint:   "workflow.fabro".to_string(),
            project_toml: None,
        },
        inputs:             BTreeMap::new(),
        vars:               BTreeMap::new(),
        launch:             Launch::default(),
        runtime:            RuntimeSpec::default(),
        unbound_is_warning: false,
    };
    let admitted = check::check(&request).expect("the bundle is admitted");
    AdmittedGraphs {
        graph:    admitted.graph,
        children: admitted.children,
    }
}

/// One run's pieces: the store, its in-memory platform records, its dir.
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

    /// Run the bundle on the local provider with the given hook-secrets
    /// provider installed through the RuntimeSpec, as the worker does.
    async fn run(
        &self,
        workflow: &str,
        settings: &str,
        hook_secrets: Option<Arc<dyn HookSecretProvider>>,
    ) -> engine::RunOutcome {
        let namespace = RunNamespace::default();
        let hooks = HooksSpec::for_run(
            Arc::clone(&self.records) as Arc<dyn PlatformRecords>,
            &namespace,
        );
        let interviewer = support::no_questions(Arc::new(support::Silent));
        let observers: Vec<Arc<dyn petri_execution::ExecutionObserver>> =
            vec![interviewer.observer()];
        let request = RunRequest {
            run_id: self.run_id.to_string(),
            run_dir: self.run_dir.clone(),
            execution: Execution::Start(admit(workflow, settings)),
            store: Arc::clone(&self.store) as Arc<dyn petri_store::RunStore>,
            runtime: RuntimeSpec {
                sandbox: SandboxProviderConfig::from_lookup(None, |name| std::env::var(name).ok()),
                hook_secrets,
                ..RuntimeSpec::default()
            },
            provider: SandboxProviderKind::LOCAL,
            cancel: CancellationToken::new(),
            controls: RunControls::new(),
            interviewer: Arc::new(interviewer) as Arc<dyn petri_execution::Interviewer>,
            observers,
            secrets: None,
            blobs: Some(Arc::clone(&self.blobs) as Arc<dyn Blobs>),
            hooks: Some(hooks),
        };
        engine::run(request).await.expect("the run executes")
    }
}

/// The declared secret reaches exactly the host hook's own process: the
/// hook writes the value it saw, a run with no provider installed leaves
/// the probe file absent and the hook recorded as unable to run.
#[tokio::test]
async fn a_declared_secret_reaches_the_host_hook_through_the_runtime_spec() {
    let writer = "writer [shape=parallelogram, script=\"echo wrote > wrote.txt\"]";
    let graph = workflow(writer);

    let harness = Harness::new();
    let probe = harness.run_dir.join("probe-token.txt");
    let settings = format!(
        "_version = 1\n\n[workflow]\ngraph = \"workflow.fabro\"\n\n[[run.hooks]]\nname = \
         \"token-probe\"\nevent = \"stage_complete\"\nsandbox = false\nblocking = false\n\
         secrets = [\"FORK_PIN_TOKEN\"]\nscript = \"printf %s \\\"$FORK_PIN_TOKEN\\\" > {}\"\n",
        probe.display()
    );
    let provider: Arc<dyn HookSecretProvider> = Arc::new(MapSecrets::from_pairs(&[(
        "FORK_PIN_TOKEN",
        "fork-pin-value",
    )]));
    let outcome = harness.run(&graph, &settings, Some(provider)).await;
    assert_eq!(outcome.status, RunStatus::Success, "{outcome:?}");
    assert_eq!(
        fs::read_to_string(&probe)
            .await
            .expect("the probe hook ran"),
        "fork-pin-value",
        "the hook's process saw the resolved secret"
    );
}

/// Without a provider, the hook cannot run and says so; the non-blocking
/// run stays green (the loud-failure contract: never a silent
/// credential-less execution).
#[tokio::test]
async fn a_declared_secret_without_a_provider_fails_the_hook_loudly() {
    let writer = "writer [shape=parallelogram, script=\"echo wrote > wrote.txt\"]";
    let graph = workflow(writer);

    let harness = Harness::new();
    let probe = harness.run_dir.join("probe-token.txt");
    let settings = format!(
        "_version = 1\n\n[workflow]\ngraph = \"workflow.fabro\"\n\n[[run.hooks]]\nname = \
         \"token-probe\"\nevent = \"stage_complete\"\nsandbox = false\nblocking = false\n\
         secrets = [\"FORK_PIN_TOKEN\"]\nscript = \"printf %s \\\"$FORK_PIN_TOKEN\\\" > {}\"\n",
        probe.display()
    );
    let outcome = harness.run(&graph, &settings, None).await;
    assert_eq!(outcome.status, RunStatus::Success, "{outcome:?}");
    assert!(!probe.exists(), "the hook never ran without its credential");
}

/// A sandboxed hook cannot declare secrets: admission refuses the bundle,
/// so the sandbox can never be handed a host credential through a
/// declaration the engine would silently ignore.
#[test]
fn a_sandboxed_hook_cannot_declare_secrets() {
    let settings = "_version = 1\n\n[workflow]\ngraph = \"workflow.fabro\"\n\n[[run.\
                    hooks]]\nname = \"sandboxed\"\nevent = \"stage_complete\"\nscript = \
                    \"true\"\nsecrets = [\"FORK_PIN_TOKEN\"]\n";
    let request = CheckRequest {
        bundle:             Bundle {
            files:        BTreeMap::from([
                (
                    "workflow.fabro".to_string(),
                    workflow("writer [shape=parallelogram, script=\"true\"]"),
                ),
                ("workflow.toml".to_string(), settings.to_string()),
            ]),
            entrypoint:   "workflow.fabro".to_string(),
            project_toml: None,
        },
        inputs:             BTreeMap::new(),
        vars:               BTreeMap::new(),
        launch:             Launch::default(),
        runtime:            RuntimeSpec::default(),
        unbound_is_warning: false,
    };
    let Err(refused) = check::check(&request) else {
        panic!("secrets on a sandboxed hook must not lower")
    };
    let text = format!("{refused:?}");
    assert!(
        text.contains("secrets"),
        "the refusal names the field: {text}"
    );
}
