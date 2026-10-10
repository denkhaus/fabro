//! What the adapter tests share: a bundle admitted
//! through `check`, a run request over the engine assembly, and the run's
//! records read back from its store.

#![allow(
    dead_code,
    reason = "each test file uses the part of the support it needs"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use fabro_petri::admission::AdmittedGraphs;
use fabro_petri::blobs::Blobs;
use fabro_petri::check::{self, Bundle, CheckRequest, Launch};
use fabro_petri::checkpoint::RunGitSettings;
use fabro_petri::controls::RunControls;
use fabro_petri::engine::{self, Execution, RunRequest};
use fabro_petri::fork_stage_envelope::StageEnvelopes;
use fabro_petri::hooks::HooksSpec;
use fabro_petri::interview::{Approval, FabroInterviewer, QuestionNotice, QuestionSink};
use fabro_petri::platform_records::PlatformRecords;
use fabro_petri::runtime::RuntimeSpec;
use fabro_petri::test_support::{MemoryBlobs, MemoryPlatformRecords};
use fabro_types::{RunId, SandboxProviderKind};
use petri_execution::{ExecutionObserver, Interviewer, inspect};
use petri_store::{Access, LogId, MemoryRunStore, RunKey, RunStore};
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

pub(crate) const POLL: Duration = Duration::from_millis(10);
pub(crate) const PATIENCE: Duration = Duration::from_secs(30);

/// The `.fabro/workflows/hello` bundle checked into this repository.
pub(crate) fn hello_bundle() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../.fabro/workflows/hello")
}

pub(crate) const SETTINGS: &str = "_version = 1\n\n[workflow]\ngraph = \"workflow.fabro\"\n";

/// The host plugin check the scenario tests skip without: the same rule
/// the hooks suite applies.
#[expect(
    clippy::disallowed_methods,
    reason = "the tests locate the host plugin through the process environment"
)]
#[expect(clippy::print_stderr, reason = "a skipped test says why on its stderr")]
pub(crate) fn host_plugin_ready() -> bool {
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

pub(crate) fn bundle(files: &[(&str, &str)]) -> Bundle {
    Bundle {
        files:        files
            .iter()
            .map(|(path, text)| ((*path).to_string(), (*text).to_string()))
            .collect(),
        entrypoint:   "workflow.fabro".to_string(),
        project_toml: None,
    }
}

/// Admit a bundle as the create handler does, with the given launch.
pub(crate) fn admit(
    files: &[(&str, &str)],
    launch: Launch,
    runtime: &RuntimeSpec,
) -> AdmittedGraphs {
    let request = CheckRequest {
        bundle: bundle(files),
        inputs: BTreeMap::new(),
        vars: BTreeMap::new(),
        launch,
        runtime: runtime.clone(),
        unbound_is_warning: false,
    };
    let admitted = check::check(&request)
        .unwrap_or_else(|error| panic!("the workflow is admitted: {error:?}"));
    AdmittedGraphs {
        graph:    admitted.graph,
        children: admitted.children,
    }
}

/// A run request over the engine assembly, on the host sandbox, with a
/// fresh cancel token and nothing installed beyond the interviewer.
pub(crate) fn run_request(
    run_id: &str,
    run_dir: &Path,
    graphs: AdmittedGraphs,
    store: Arc<dyn RunStore>,
    runtime: RuntimeSpec,
    interviewer: FabroInterviewer,
) -> RunRequest {
    RunRequest {
        run_id: run_id.to_string(),
        run_dir: run_dir.to_path_buf(),
        execution: Execution::Start(graphs),
        store,
        runtime,
        provider: SandboxProviderKind::LOCAL,
        cancel: CancellationToken::new(),
        controls: RunControls::new(),
        observers: vec![interviewer.observer()],
        interviewer: Arc::new(interviewer),
        secrets: None,
        blobs: None,
        hooks: None,
    }
}

/// A check request over a bare two-file workflow bundle: the graph source
/// plus its settings, nothing pre-filled.
pub(crate) fn request(workflow: &str) -> CheckRequest {
    CheckRequest {
        bundle: bundle(&[
            ("workflow.fabro", workflow),
            ("workflow.toml", SETTINGS),
        ]),
        inputs: BTreeMap::new(),
        vars: BTreeMap::new(),
        launch: Launch::default(),
        runtime: RuntimeSpec::default(),
        unbound_is_warning: false,
    }
}

/// One run's pieces for an engine scenario: a fresh run identity, a host
/// run directory, and the in-memory records and blobs the hooks write into.
pub(crate) struct EngineHarness {
    pub run_id:  RunId,
    pub run_dir: PathBuf,
    pub records: Arc<MemoryPlatformRecords>,
    pub blobs:   Arc<MemoryBlobs>,
    _root:       tempfile::TempDir,
}

impl EngineHarness {
    pub(crate) fn new() -> Self {
        let root = tempfile::tempdir().expect("a temp dir");
        Self {
            run_id:  RunId::new(),
            run_dir: root.path().join("run"),
            records: Arc::new(MemoryPlatformRecords::new()),
            blobs:   Arc::new(MemoryBlobs::new()),
            _root:   root,
        }
    }

    /// Admit `workflow` and run it through the engine with `git` settings
    /// and stage `envelopes`, the way the worker would pass them.
    pub(crate) async fn run(
        &self,
        workflow: &str,
        git: RunGitSettings,
        envelopes: Option<Arc<StageEnvelopes>>,
    ) -> engine::RunOutcome {
        let admitted = check::check(&request(workflow)).expect("the bundle is admitted");
        let (interviewer, observers) = silent_run_interviewer();
        let hooks = HooksSpec {
            records: Arc::clone(&self.records) as Arc<dyn PlatformRecords>,
            git,
            artifacts: Vec::new(),
            envelopes,
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
}

/// An interviewer for a run that asks nothing, with its expiry observer.
pub(crate) fn silent_run_interviewer() -> (
    Arc<dyn Interviewer>,
    Vec<Arc<dyn ExecutionObserver>>,
) {
    let interviewer = no_questions(Arc::new(Silent));
    let observers = vec![interviewer.observer()];
    (Arc::new(interviewer), observers)
}

/// An interviewer whose answers nobody delivers, for runs that ask nothing.
pub(crate) fn no_questions(sink: Arc<dyn QuestionSink>) -> FabroInterviewer {
    FabroInterviewer::new(
        Arc::new(fabro_interview::ControlInterviewer::new()),
        Approval::Prompt,
    )
    .with_sink(sink)
}

/// A sink that drops every notice.
pub(crate) struct Silent;

#[async_trait::async_trait]
impl QuestionSink for Silent {
    async fn post(&self, _notice: QuestionNotice) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Every record of every log of a stored run, as JSON, in log order.
pub(crate) async fn all_records(store: &dyn RunStore, run_id: &str) -> Vec<serde_json::Value> {
    let logs = store
        .open(&RunKey::new(run_id), Access::Read)
        .await
        .expect("the run opens for reading");
    let inspection = inspect::inspect_run(&*logs)
        .await
        .expect("the stored run inspects");
    let mut ids = vec![LogId::Coordinator, LogId::Resources];
    ids.extend(
        inspection
            .executions
            .iter()
            .map(|execution| LogId::Execution(execution.execution)),
    );
    let mut records = Vec::new();
    for id in ids {
        records.extend(
            logs.read(&id)
                .await
                .expect("the log reads")
                .into_iter()
                .map(|record| record.record),
        );
    }
    records
}

/// Wait until `condition` holds, polling, or fail after [`PATIENCE`].
pub(crate) async fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        sleep(POLL).await;
    }
}
