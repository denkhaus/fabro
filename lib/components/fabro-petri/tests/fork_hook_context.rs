//! The stage-context bridge (fabro-931d), pinned fork-only: a
//! `stage_complete` hook's context must carry the stage's DECLARED
//! `context_updates` — the journal payload the stage reported, not only
//! the engine-provided entries.
//!
//! The bridge landed 2026-10-09 (petri 6cf8ba0: `after_visit` copies
//! `Outcome.context_updates` into the hook `Context`). Its first pin in
//! `tests/hooks.rs` — an upstream-owned file a merge can rewrite — only
//! checked that a `context_updates` object exists with engine entries;
//! before the bridge, the stage journal wrote `"data":{}` for every stage
//! and the loop lane lost its whole learning channel. These tests are the
//! fork's presence pin: they fail if an upstream merge drops the declared
//! payload out of the hook context, because the journal painpoints stop
//! arriving.

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
use fabro_petri::test_support::{MemoryBlobs, MemoryPlatformRecords};
use fabro_types::settings::run::RunNamespace;
use fabro_types::{RunId, SandboxProviderKind};
use petri_store::MemoryRunStore;
use tokio::fs;
use tokio_util::sync::CancellationToken;

mod support;

const SETTINGS: &str = "_version = 1\n\n[workflow]\ngraph = \"workflow.fabro\"\n";

/// A one-stage command bundle: the stage line goes between `start` and
/// `exit`.
fn workflow(stage: &str) -> String {
    format!(
        "digraph HookContext {{\n  graph [goal=\"Pin the stage-context bridge\", \
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

    /// The one workspace the run's root scope used.
    async fn workspace(&self) -> String {
        let mut entries = fs::read_dir(self.run_dir.join("scopes"))
            .await
            .expect("the scopes directory exists");
        let mut names = Vec::new();
        while let Some(entry) = entries.next_entry().await.expect("an entry reads") {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
        assert_eq!(names.len(), 1, "one workspace: {names:?}");
        names.remove(0)
    }

    /// The `ctx-updates.jsonl` lines the `stage_complete` hook captured,
    /// one parsed context per stage visit.
    async fn captured_contexts(&self) -> Vec<serde_json::Value> {
        let workspace = self.workspace().await;
        let path = self
            .run_dir
            .join("scopes")
            .join(workspace)
            .join("work")
            .join("ctx-updates.jsonl");
        let seen = fs::read_to_string(path)
            .await
            .expect("the stage hook saw the context file");
        seen.lines()
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_str(line).expect("each captured context is JSON"))
            .collect()
    }
}

/// The stage's DECLARED journal payload reaches the `stage_complete`
/// hook's context: the writer stage reports a `journal.painpoints` entry
/// through `context_updates`, and the hook reads that exact entry — not
/// just any `context_updates` object — out of `$FABRO_HOOK_CONTEXT`.
#[tokio::test]
async fn the_stage_hook_context_carries_the_declared_journal_painpoints() {
    let harness = Harness::new();
    let workflow = workflow(
        // The single-quoted printf argument is CLOSED (unlike the
        // upstream-owned pin's line, whose writer fails the shell parse):
        // the stage must genuinely succeed for its declared payload to
        // ride a `stage_complete` context.
        r#"  writer [shape=parallelogram, output_schema="routing", script="printf '%s' '{\"context_updates\":{\"journal\":{\"painpoints\":[{\"text\":\"probe\"}]}}}'" ]"#,
    );
    let settings = format!(
        "{SETTINGS}\n[[run.hooks]]\nevent = \"stage_complete\"\nscript = \"cat \
         \\\"$FABRO_HOOK_CONTEXT\\\" >> ctx-updates.jsonl; echo >> ctx-updates.jsonl\"\n"
    );
    let outcome = harness.run(&workflow, &settings).await;
    assert_eq!(outcome.status, RunStatus::Success, "{outcome:?}");

    let contexts = harness.captured_contexts().await;
    assert!(
        !contexts.is_empty(),
        "the stage hook ran and captured its context"
    );
    // The contexts that carry the declared payload, by payload (not by
    // node naming): exactly the writer stage's visits.
    let declared: Vec<&serde_json::Value> = contexts
        .iter()
        .filter(|context| !context["context_updates"]["journal"]["painpoints"].is_null())
        .collect();
    assert!(
        !declared.is_empty(),
        "some stage context carries the declared journal payload: {contexts:?}"
    );
    for context in &declared {
        assert_eq!(
            context["node_id"], "writer",
            "the declared payload belongs to the writer stage: {context}"
        );
        // The declared payload, structurally: the journal painpoints the
        // stage reported, verbatim — this is what the stage journal's
        // `data` is built from.
        assert_eq!(
            context["context_updates"]["journal"]["painpoints"][0]["text"], "probe",
            "the stage's declared journal payload must ride the hook context: {context}"
        );
        // The engine-provided entries ride beside it, under their own
        // flat dotted keys (`internal.run_id` belongs to the start
        // node's context; a command stage's own entry is
        // `command.output`).
        assert!(
            !context["context_updates"]["command.output"].is_null(),
            "the engine-provided command output rides the same object: {context}"
        );
    }
}

/// The bridge survives without a declared payload: a stage that reports
/// no `context_updates` of its own still hands the hook the
/// engine-provided entries under `context_updates` — the key is never
/// silently dropped.
#[tokio::test]
async fn a_stage_without_declared_updates_still_carries_the_engine_entries() {
    let harness = Harness::new();
    let workflow = workflow("  writer [shape=parallelogram, script=\"true\"]");
    let settings = format!(
        "{SETTINGS}\n[[run.hooks]]\nevent = \"stage_complete\"\nscript = \"cat \
         \\\"$FABRO_HOOK_CONTEXT\\\" >> ctx-updates.jsonl; echo >> ctx-updates.jsonl\"\n"
    );
    let outcome = harness.run(&workflow, &settings).await;
    assert_eq!(outcome.status, RunStatus::Success, "{outcome:?}");

    let contexts = harness.captured_contexts().await;
    assert!(
        !contexts.is_empty(),
        "the stage hook ran and captured its context"
    );
    assert!(
        contexts
            .iter()
            .any(|context| !context["context_updates"]["internal.run_id"].is_null()),
        "engine-provided updates ride the hook context: {contexts:?}"
    );
}
