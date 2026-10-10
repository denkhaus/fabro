//! The non-blocking hook watchdog (fabro-6922), pinned: a persistent
//! failure of a non-blocking `[[run.hooks]]` entry must reach the run's
//! stream as a `run.notice` warning — never as a run failure.
//!
//! The judgment-shadow host hook exited 1 on every stage of every run
//! since the cutover and nothing surfaced it; it was found only by
//! reading raw hook reports. These tests are the fork's presence pin:
//! they fail if an upstream merge drops `fork_hook_warning`'s watchdog
//! out of the runtime's hook service, because the notices stop arriving.

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
use fabro_petri::fork_hook_warning::{HookWarningSink, WARNING_INTERVAL};
use fabro_petri::hooks::HooksSpec;
use fabro_petri::platform_records::PlatformRecords;
use fabro_petri::providers::SandboxProviderConfig;
use fabro_petri::runtime::RuntimeSpec;
use fabro_petri::test_support::{MemoryBlobs, MemoryPlatformRecords};
use fabro_store::PlatformRecord;
use fabro_types::settings::run::RunNamespace;
use fabro_types::{RunId, RunNoticeLevel, SandboxProviderKind};
use petri_store::MemoryRunStore;
use tokio_util::sync::CancellationToken;

mod support;

const SETTINGS: &str = "_version = 1\n\n[workflow]\ngraph = \"workflow.fabro\"\n";

/// A two-stage command bundle: the stages go between `start` and `exit`.
fn workflow(stages: &str) -> String {
    format!(
        "digraph Watchdog {{\n  graph [goal=\"Pin the watchdog\", default_max_retries=0]\n  \
         start [shape=Mdiamond]\n  exit [shape=Msquare]\n{stages}\n  start -> first -> second -> \
         exit\n}}\n"
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

    /// Run the bundle on the local provider, as the worker does, and
    /// report what the record says.
    async fn run(&self, workflow: &str, settings: &str) -> engine::RunOutcome {
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

    /// The run's non-blocking-hook-failure notices, as (level, message).
    fn notices(&self) -> Vec<(RunNoticeLevel, String)> {
        self.records
            .records(&self.run_id)
            .into_iter()
            .filter_map(|stored| match stored.record {
                PlatformRecord::RunNotice(notice) => Some(notice),
                _ => None,
            })
            .filter(|notice| notice.code == "non_blocking_hook_failed")
            .map(|notice| (notice.level, notice.message))
            .collect()
    }
}

/// A non-blocking hook that fails on every stage does not fail the run,
/// and its failure reaches the run's records as a warning notice naming
/// the hook and its own failure.
#[tokio::test]
async fn a_persistently_failing_non_blocking_hook_warns_on_the_stream() {
    let harness = Harness::new();
    let workflow = workflow(
        "  first [shape=parallelogram, script=\"true\"]\n  second [shape=parallelogram, \
         script=\"true\"]",
    );
    let settings = format!(
        "{SETTINGS}\n[[run.hooks]]\nname = \"judgment-shadow\"\nevent = \"stage_complete\"\n\
         blocking = false\nscript = \"exit 1\"\n"
    );
    let outcome = harness.run(&workflow, &settings).await;
    assert_eq!(
        outcome.status,
        RunStatus::Success,
        "{outcome:?}: a non-blocking hook's failure never fails the run"
    );

    let notices = harness.notices();
    assert!(
        !notices.is_empty(),
        "the failing hook must surface: no run.notice warning names it"
    );
    for (level, message) in &notices {
        assert_eq!(*level, RunNoticeLevel::Warn, "the notice warns: {message}");
        assert!(
            message.contains("judgment-shadow"),
            "the notice names the hook: {message}"
        );
        assert!(
            message.contains("consecutive failures"),
            "the notice carries the streak: {message}"
        );
        assert!(
            message.contains("hook exited with code 1"),
            "the notice carries the hook's own failure: {message}"
        );
    }
}

/// A hook that fails and then recovers writes one warning, not one per
/// stage: the streak resets when the hook runs clean again.
#[tokio::test]
async fn a_recovered_hook_warns_once_and_resets_its_streak() {
    let harness = Harness::new();
    // The hook fails until the first stage creates its marker: one
    // failure at `first`'s stage_complete, then clean at `second`'s.
    let workflow = workflow(
        "  first [shape=parallelogram, script=\"touch flapping-marker\"]\n  second \
         [shape=parallelogram, script=\"true\"]",
    );
    let settings = format!(
        "{SETTINGS}\n[[run.hooks]]\nname = \"flapping\"\nevent = \"stage_complete\"\nblocking = \
         false\nscript = \"test -f flapping-marker\"\n"
    );
    let outcome = harness.run(&workflow, &settings).await;
    assert_eq!(outcome.status, RunStatus::Success, "{outcome:?}");

    let notices = harness.notices();
    assert_eq!(
        notices.len(),
        1,
        "one failure earns one warning, the recovery resets the streak: {notices:?}"
    );
    assert!(
        notices[0].1.contains("flapping"),
        "the notice names the hook: {}",
        notices[0].1
    );
}

/// The watchdog module itself stays present and wired: the sink is
/// constructible over the platform-records seam, and the notice cadence
/// the unit tests pin is exported for it. Presence pin for
/// `fork_hook_warning` (an upstream merge that drops the module breaks
/// the build here, not silently at runtime).
#[test]
fn the_watchdog_seam_is_present() {
    let _sink = HookWarningSink::new(
        Arc::new(MemoryPlatformRecords::new()) as Arc<dyn PlatformRecords>,
        RunId::new(),
    );
    assert_eq!(WARNING_INTERVAL, 20);
    assert!(HookWarningSink::earns_notice(1));
    assert!(!HookWarningSink::earns_notice(2));
}
