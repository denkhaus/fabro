//! Fork presence pin (fabro-9711/3a5e, ADR-0030): a claiming planner's
//! `current_seed_id` context update lifts to the run projection AND the
//! run summary, so the board tile and the runs list can name the seed a
//! run works on. The fold lives in `projection::engine`'s StepFinished
//! arm; this pin survives an upstream merge, which rewrites the upstream
//! test files freely.

mod support;

use std::sync::Arc;

use fabro_db::DbPool;
use fabro_petri::blobs::RunBlobs;
use fabro_petri::projector::Projector;
use fabro_petri::providers::{self, SandboxProviderConfig};
use fabro_petri::{SqliteRunStore, test_support as petri_support};
use fabro_store::platform_records::{
    PlatformRecord, PlatformRecordStore, RunCreatedRecord, RunLifecycleKind, RunLifecycleRecord,
};
use fabro_store::{BlobStore, build_summary, test_support};
use fabro_types::{
    BlobHash, PetriAdmission, PetriGraphRef, RunId, RunStatus, test_support as types_support,
};
use petri_execution::host::{self, HostRun};
use petri_frontend_fabro::Fabro;
use petri_runtime::RunOptions;
use petri_runtime::executor::Retention;
use petri_runtime::frontend::CompileInputs;
use petri_runtime::ir::RunStatus as PetriRunStatus;
use petri_store::RunKey;
use tokio::fs;

/// One command node that declares the claim context the develop planner
/// declares (workflow.fabro:200): `current_seed_id` (and its siblings).
const CLAIM_WORKFLOW: &str = r#"digraph Claim {
    graph [goal="Claim one seed"]
    start [shape=Mdiamond]
    exit [shape=Msquare]
    plan [shape=parallelogram, output_schema="routing", script="echo '{\"outcome\": \"succeeded\", \"context_updates\": {\"current_seed_id\": \"fabro-9711\", \"current_seed_title\": \"Meaningful run titles\"}}'"]
    start -> plan -> exit
}"#;

const SETTINGS: &str = "_version = 1\n\n[workflow]\ngraph = \"workflow.fabro\"\n";

fn pool() -> DbPool {
    test_support::in_memory_pool_with(&[
        fabro_db::BLOBS_MIGRATION_SQL,
        fabro_db::RUNS_MIGRATION_SQL,
        fabro_db::DROP_RUN_EVENTS_MIGRATION_SQL,
        fabro_db::PETRI_RECORDS_MIGRATION_SQL,
        fabro_db::PETRI_PROJECTION_MIGRATION_SQL,
    ])
}

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
                title: Some("Claim one seed".to_string()),
                parent_id: None,
                retried_from: None,
                web_url: None,
            }),
            None,
        )
        .await
        .expect("the run.created record appends");
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

/// Run the claim workflow to completion under a live projector and return
/// the settled projection read back from the store.
async fn projected_claim_run() -> fabro_types::RunProjection {
    let root = tempfile::tempdir().expect("a temp dir");
    let bundle = root.path().join(".fabro").join("workflows").join("claim");
    fs::create_dir_all(&bundle)
        .await
        .expect("the bundle directory is creatable");
    fs::write(bundle.join("workflow.fabro"), CLAIM_WORKFLOW)
        .await
        .expect("the workflow file is writable");
    fs::write(bundle.join("workflow.toml"), SETTINGS)
        .await
        .expect("the settings file is writable");
    let workflow = bundle.join("workflow.fabro");

    let db = pool();
    let run_id = RunId::new();
    create_run(&db, run_id, CLAIM_WORKFLOW).await;

    let projector = Projector::new(db.clone(), db.clone());
    projector.signal(run_id);
    let store = projector.observe_store(Arc::new(SqliteRunStore::new(db.clone())));

    let runtime =
        providers::standard_runtime(&SandboxProviderConfig::default()).frontend(Fabro::new());
    let runtime = petri_attractor_steps::register(runtime);
    let runtime = runtime.capability(RunBlobs::output_store(Arc::new(BlobStore::new(db.clone()))));
    let mut options = RunOptions::new(root.path().join("run"));
    options.grace = std::time::Duration::from_secs(2);
    options.retention = Retention::Never;
    options.echo = false;
    options.run_key = Some(RunKey::new(run_id.to_string()));
    let rt = runtime.store(store).options(options);
    let lowered = rt
        .check(&workflow, None, None, &CompileInputs::new())
        .expect("the workflow file loads");
    let graph = lowered
        .graph
        .unwrap_or_else(|| panic!("the workflow lowers: {:?}", lowered.diagnostics));
    let report = host::run_configured(&rt, HostRun::new(graph), |_, _| {})
        .await
        .expect("the run completes");
    assert_eq!(
        report.status,
        PetriRunStatus::Success,
        "errors: {:?}",
        report.state.errors()
    );
    projector.settle(run_id).await;

    petri_support::stored_projection(&db, run_id)
        .await
        .expect("the stored projection reads")
        .expect("the run has a stored projection")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_current_seed_id_context_update_lifts_to_projection_and_summary() {
    let projection = projected_claim_run().await;
    assert_eq!(
        projection.claimed_seed_id.as_deref(),
        Some("fabro-9711"),
        "the planner's current_seed_id context update must lift to the run projection \
         (fabro-9711/3a5e)"
    );
    let summary = build_summary(&projection, &RunId::new());
    assert_eq!(
        summary.claimed_seed_id.as_deref(),
        Some("fabro-9711"),
        "the run summary carries the claimed seed id for the board tile"
    );
}
