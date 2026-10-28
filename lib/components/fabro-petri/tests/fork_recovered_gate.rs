//! Fork presence pin (fabro-29e3): a gate failure the graph's bounce
//! cycle RECOVERED — the failing node's later visit succeeds — must not
//! survive into the conclusion. The loop lane's real shape (runs
//! 01M4XWJ9C8XE, 01M50BH4): tester@1 red, implementer repairs, tester@2
//! green, every later stage green, the engine finishes success — and the
//! b00c guard downgraded the run to `failed/workflow_error` on the
//! tester@1 row, stranding work the pass had already closed. The guard
//! must judge legs by their FINAL visit.
//!
//! The second arm pins the invariant the fix must not weaken: a catch-all
//! failure no later visit supersedes still refuses the green conclusion
//! (fabro-b00c, user decision 2026-09-25).
//!
//! The third arm pins the terminal-status split (work#1029, run
//! 01M5ZEWKAMMZ): a gate red the graph routes onward over an EXPLICIT
//! conditional bounce edge is control flow — the tester's FINAL visit
//! stays red, the run still reaches exit, the engine concludes success,
//! and the projection must keep that green. The lane graphs' "Gate red"
//! bounce carries condition="outcome=failed" for exactly this reason; an
//! unconditional bounce edge re-splits engine success from the platform
//! record (the run landed failed/workflow_error with the publish path
//! skipped while its event stream said succeeded).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use fabro_db::DbPool;
use fabro_petri::blobs::{Blobs, RunBlobs};
use fabro_petri::projector::Projector;
use fabro_petri::providers::{self, SandboxProviderConfig};
use fabro_petri::{SqliteRunStore, test_support as petri_support};
use fabro_store::platform_records::{
    PlatformRecord, PlatformRecordStore, RunCreatedRecord, RunLifecycleKind, RunLifecycleRecord,
};
use fabro_store::{BlobStore, test_support};
use fabro_types::{
    BlobHash, FailureReason, PetriAdmission, PetriGraphRef, RunId, RunStatus, StageId, StageState,
    test_support as types_support,
};
use petri_execution::host::{self, HostRun};
use petri_frontend_fabro::Fabro;
use petri_runtime::RunOptions;
use petri_runtime::executor::Retention;
use petri_runtime::frontend::CompileInputs;
use petri_runtime::ir::RunStatus as PetriRunStatus;
use petri_store::RunKey;
use tokio::fs;

const SETTINGS: &str = "_version = 1\n\n[workflow]\ngraph = \"workflow.fabro\"\n";

/// The loop lane's gate-bounce cycle over a real gate: `tester` fails
/// until `implementer` leaves the marker, exactly as the lane's
/// `tester -> implementer [label="Gate red"]` catch-all and
/// `tester -> exit [condition="outcome=succeeded"]` routes do.
fn recovered_gate_workflow(marker: &Path) -> String {
    format!(
        r#"digraph RecoveredGate {{
    graph [goal="Recover one gate red"]
    start [shape=Mdiamond]
    exit [shape=Msquare]
    tester [shape=parallelogram, script="test -f {marker}"]
    implementer [shape=parallelogram, script="touch {marker}"]
    start -> tester
    tester -> implementer [label="Gate red"]
    tester -> exit [label="Gate green", condition="outcome=succeeded"]
    implementer -> tester [label="Repaired"]
}}"#,
        marker = marker.display()
    )
}

/// The unrecovered control: the tester fails, the bounce runs, the
/// implementer gives up, and the run still reaches `exit` green — a
/// catch-all failure no later visit supersedes.
const UNRECOVERED_GATE_WORKFLOW: &str = r#"digraph UnrecoveredGate {
    graph [goal="End green over an unrecovered gate red"]
    start [shape=Mdiamond]
    exit [shape=Msquare]
    tester [shape=parallelogram, script="exit 1"]
    implementer [shape=parallelogram, script="true"]
    start -> tester
    tester -> implementer [label="Gate red"]
    tester -> exit [label="Gate green", condition="outcome=succeeded"]
    implementer -> exit [label="Give up"]
}"#;

/// The work#1029 shape: the tester's red rides an EXPLICIT conditional
/// bounce edge (the lane graphs' fixed `Gate red` form) and the tester's
/// FINAL visit stays failed — no repair round revisits it; the
/// implementer delivers and the run reaches exit. The engine concludes
/// success; the projection must agree (routed control flow, not a
/// catch-all fake-green). The unconditional deadlock edge mirrors the
/// lane graphs: the engine requires one fallback edge per node
/// (attractor.all_conditional_edges), and the deadlock exit doubles as
/// the unroutable-outcome guard.
fn routed_red_bounce_workflow(marker: &Path) -> String {
    format!(
        r#"digraph RoutedRedBounce {{
    graph [goal="Route one gate red explicitly and deliver"]
    start [shape=Mdiamond]
    exit [shape=Msquare]
    tester [shape=parallelogram, script="test -f {marker}"]
    implementer [shape=parallelogram, script="touch {marker}"]
    start -> tester
    tester -> implementer [label="Gate red", condition="outcome=failed"]
    tester -> exit [label="Gate green", condition="outcome=succeeded"]
    tester -> exit [x.kind="deadlock", label="Unroutable outcome, needs a human"]
    implementer -> exit [label="Delivered"]
}}"#,
        marker = marker.display()
    )
}

/// A fresh in-memory database with every table the projection touches.
fn pool() -> DbPool {
    test_support::in_memory_pool_with(&[
        fabro_db::BLOBS_MIGRATION_SQL,
        fabro_db::RUNS_MIGRATION_SQL,
        fabro_db::DROP_RUN_EVENTS_MIGRATION_SQL,
        fabro_db::PETRI_RECORDS_MIGRATION_SQL,
        fabro_db::PETRI_PROJECTION_MIGRATION_SQL,
        fabro_db::RUN_TOMBSTONES_MIGRATION_SQL,
    ])
}

async fn install_bundle(root: &Path, files: &[(&str, &str)]) -> PathBuf {
    let bundle = root.join(".fabro").join("workflows").join("recovered");
    fs::create_dir_all(&bundle)
        .await
        .expect("the bundle directory is creatable");
    for (file, text) in files {
        fs::write(bundle.join(file), text)
            .await
            .expect("the bundle file is writable");
    }
    bundle.join("workflow.fabro")
}

/// The run's `run.created` record with the graph source, as the create
/// handler writes it: the fork's conclusion seams read the graph back.
async fn create_run(pool: &DbPool, run_id: RunId, graph_source: &str) {
    let store = PlatformRecordStore::new(pool.clone());
    let mut spec = types_support::test_run_spec();
    spec.run_id = run_id;
    spec.graph_source = Some(graph_source.to_string());
    spec.admission = PetriAdmission {
        graph:    PetriGraphRef {
            blob:   BlobHash::new(b"graph"),
            digest: "digest".to_string(),
        },
        children: Vec::new(),
    };
    store
        .append(
            &run_id,
            &PlatformRecord::RunCreated(RunCreatedRecord {
                spec,
                title: Some("recovered gate".to_string()),
                parent_id: None,
                retried_from: None,
                web_url: None,
            }),
            None,
        )
        .await
        .expect("the created record stores");
    for (transition, status) in [
        (RunLifecycleKind::Runnable, RunStatus::Runnable),
        (RunLifecycleKind::Starting, RunStatus::Starting),
        (RunLifecycleKind::Running, RunStatus::Running),
    ] {
        store
            .append(
                &run_id,
                &PlatformRecord::RunLifecycle(
                    RunLifecycleRecord::new(transition).with_status(status),
                ),
                None,
            )
            .await
            .expect("the lifecycle record stores");
    }
}

/// One scenario: its bundle installed, its run created, live-run through
/// the projector, the stored projection read back.
async fn run_and_project(graph: &str) -> fabro_types::RunProjection {
    let root = tempfile::tempdir().expect("a temp dir");
    let workflow = install_bundle(root.path(), &[
        ("workflow.fabro", graph),
        ("workflow.toml", SETTINGS),
    ])
    .await;
    let pool = pool();
    let run_id = RunId::new();
    create_run(&pool, run_id, graph).await;

    let projector = Projector::new(pool.clone(), pool.clone());
    projector.signal(run_id);
    let store = projector.observe_store(Arc::new(SqliteRunStore::new(pool.clone())));
    let runtime =
        providers::standard_runtime(&SandboxProviderConfig::default()).frontend(Fabro::new());
    let runtime = petri_attractor_steps::register(runtime);
    let blobs: Arc<dyn Blobs> = Arc::new(BlobStore::new(pool.clone()));
    let runtime = runtime.capability(RunBlobs::output_store(blobs));
    let mut options = RunOptions::new(root.path().join("run"));
    options.grace = Duration::from_secs(2);
    options.retention = Retention::Never;
    options.echo = false;
    options.run_key = Some(RunKey::new(run_id.to_string()));
    let rt = runtime.store(store).options(options);
    let lowered = match rt.check(&workflow, None, None, &CompileInputs::new()) {
        Ok(lowered) => lowered,
        Err(error) => panic!("the workflow file loads: {error:#}"),
    };
    let host_run =
        HostRun::new(lowered.graph.expect("the workflow lowers")).with_children(lowered.children);
    let report = host::run_configured(&rt, host_run, |_, _| {})
        .await
        .expect("the run completes");
    assert_eq!(
        report.status,
        PetriRunStatus::Success,
        "the engine finishes success in all three arms: {:?}",
        report.state.errors()
    );
    projector.settle(run_id).await;
    petri_support::stored_projection(&pool, run_id)
        .await
        .expect("reads")
        .expect("stored")
}

/// fabro-29e3's recovered arm: tester@1 red, implementer repairs,
/// tester@2 green — a fully recovered bounce concludes succeeded with no
/// failure, the conclusion publishing feeds on.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_recovered_gate_bounce_concludes_succeeded() {
    let marker = tempfile::tempdir().expect("a temp dir");
    let stored = run_and_project(&recovered_gate_workflow(&marker.path().join("green"))).await;

    // The bounce really happened: the tester failed once and recovered.
    assert_eq!(
        stored
            .stage(&StageId::new("tester", 1))
            .expect("tester@1")
            .state,
        StageState::Failed,
        "the first tester visit is the red the graph recovered"
    );
    assert_eq!(
        stored
            .stage(&StageId::new("tester", 2))
            .expect("tester@2")
            .state,
        StageState::Succeeded,
        "the second tester visit is green"
    );
    assert!(
        matches!(stored.status, RunStatus::Succeeded { reason } if reason == fabro_types::SuccessReason::Completed),
        "a recovered bounce ends green: {:?}",
        stored.status
    );
    let conclusion = stored.conclusion.as_ref().expect("the run concluded");
    assert!(
        conclusion.failure.is_none(),
        "no failure survives the recovery: {:?}",
        conclusion.failure
    );
}

/// The invariant the fix must not weaken (fabro-b00c): a catch-all
/// failure no later visit supersedes still refuses the green conclusion.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unrecovered_gate_red_still_refuses_the_green_conclusion() {
    let stored = run_and_project(UNRECOVERED_GATE_WORKFLOW).await;

    assert_eq!(
        stored
            .stage(&StageId::new("tester", 1))
            .expect("tester@1")
            .state,
        StageState::Failed,
        "the tester stayed red"
    );
    assert!(
        matches!(stored.status, RunStatus::Failed {
            reason: FailureReason::WorkflowError,
        }),
        "an unrecovered red the catch-all consumed never reads green: {:?}",
        stored.status
    );
    let conclusion = stored.conclusion.as_ref().expect("the run concluded");
    assert!(
        conclusion.failure.is_some(),
        "the downgrade names the failure"
    );
}

/// work#1029's routed arm: a gate red bounced over an EXPLICIT
/// conditional edge — the tester's final visit stays failed, no repair
/// revisits it, the run reaches exit and the engine concludes success —
/// concludes succeeded with no failure, exactly as the engine did for
/// run 01M5ZEWKAMMZ before the projection split it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_explicitly_routed_gate_red_concludes_succeeded() {
    let marker = tempfile::tempdir().expect("a temp dir");
    let stored = run_and_project(&routed_red_bounce_workflow(
        &marker.path().join("delivered"),
    ))
    .await;

    // No recovery: the tester's FINAL (and only) visit stays red — this
    // is what separates the routed arm from the recovered one.
    assert_eq!(
        stored
            .stage(&StageId::new("tester", 1))
            .expect("tester@1")
            .state,
        StageState::Failed,
        "the tester's final visit stays red; the explicit edge routed it onward"
    );
    assert!(
        matches!(stored.status, RunStatus::Succeeded { reason } if reason == fabro_types::SuccessReason::Completed),
        "a red routed over an explicit conditional edge ends green: {:?}",
        stored.status
    );
    let conclusion = stored.conclusion.as_ref().expect("the run concluded");
    assert!(
        conclusion.failure.is_none(),
        "no failure survives an explicitly routed red: {:?}",
        conclusion.failure
    );
}
