#![cfg_attr(
    test,
    expect(
        clippy::disallowed_methods,
        reason = "tests write a goal file synchronously before exercising creation"
    )
)]

//! Creating a run Petri admitted: the settings Fabro layered, the display
//! graph read off the admitted workflow, and the run's first records.
//!
//! Petri compiles the workflow (`fabro-petri`'s check) and its admission is
//! the graph the run executes. What Fabro adds at create is its own: the
//! run's goal and pull request settings materialized into the resolved
//! settings, the labels, the workflow slug, the bundle the run was created
//! from, and the durable `run.created` and `submitted` records.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use fabro_config::Storage;
use fabro_config::project::{resolve_working_directory_from_run, workflow_slug_from_path};
use fabro_config::run::resolve_run_goal_from_namespace;
use fabro_store::platform_records::{
    PlatformRecord, RunCreatedRecord, RunLifecycleKind, RunLifecycleRecord,
};
use fabro_store::{BlobStore, Database};
use fabro_types::settings::InterpString;
use fabro_types::settings::run::RunGoal;
use fabro_types::{
    AutomationRef, BlobHash, ForkSourceRef, GitContext, ManifestPath, PetriAdmission, RunGraph,
    RunId, RunProvenance, RunSpec, RunStatus, RunTarget, WorkflowSettings, WorkflowVersionId,
};
use tokio::task::spawn_blocking;

use crate::error::Error;
use crate::workflow_bundle::{RunDefinition, WorkflowBundle};

/// What Petri admitted for a run, as Fabro materializes it: the settings
/// Fabro layered and substituted, the display graph read off the admitted
/// workflow, the entrypoint's DOT, and the bundle it came from.
#[derive(Debug)]
pub struct AdmittedRunInput {
    pub settings:        WorkflowSettings,
    pub cwd:             PathBuf,
    pub graph:           RunGraph,
    /// The entrypoint's DOT as written, persisted as the run's
    /// `graph_source`.
    pub source:          String,
    pub workflow_path:   ManifestPath,
    pub workflow_bundle: WorkflowBundle,
}

/// Durable metadata joined to a materialized run before persistence.
/// `run_id` is already resolved, and `storage_root` is used to derive the
/// run's scratch directory during pure input assembly.
#[derive(Debug)]
pub struct CreateRunPersistenceMetadata {
    pub run_id:              RunId,
    pub storage_root:        PathBuf,
    pub workflow_slug:       Option<String>,
    pub workflow_version_id: Option<WorkflowVersionId>,
    pub target:              Option<RunTarget>,
    pub title:               Option<String>,
    pub automation:          Option<AutomationRef>,
    pub git:                 Option<GitContext>,
    pub fork_source_ref:     Option<ForkSourceRef>,
    pub parent_id:           Option<RunId>,
    pub provenance:          RunProvenance,
    pub web_url:             Option<String>,
    /// What Petri admitted for the run.
    pub admission:           PetriAdmission,
}

/// The run as persisted: its spec, the DOT it displays, and its scratch
/// directory.
#[derive(Debug)]
pub struct CreatedRun {
    pub spec:    RunSpec,
    /// The entrypoint's DOT, the same text as `spec.graph_source`.
    pub source:  String,
    pub run_id:  RunId,
    pub run_dir: PathBuf,
}

/// The admitted run with Fabro's run-level settings materialized: the goal
/// the run displays and runs under, and a pull request block the settings
/// disable dropped.
#[derive(Debug)]
pub struct MaterializedRun {
    settings:         WorkflowSettings,
    graph:            RunGraph,
    source:           String,
    workflow_slug:    Option<String>,
    definition:       RunDefinition,
    source_directory: String,
    labels:           HashMap<String, String>,
}

impl MaterializedRun {
    pub fn settings(&self) -> &WorkflowSettings {
        &self.settings
    }

    pub fn graph(&self) -> &RunGraph {
        &self.graph
    }
}

/// Complete input for creating a durable run. The run ID and run directory
/// are resolved during assembly, before persistence begins.
pub struct CreateRunPersistenceInput {
    materialized:        MaterializedRun,
    run_id:              RunId,
    run_dir:             PathBuf,
    workflow_slug:       Option<String>,
    workflow_version_id: Option<WorkflowVersionId>,
    target:              Option<RunTarget>,
    title:               Option<String>,
    automation:          Option<AutomationRef>,
    git:                 Option<GitContext>,
    fork_source_ref:     Option<ForkSourceRef>,
    parent_id:           Option<RunId>,
    provenance:          RunProvenance,
    web_url:             Option<String>,
    admission:           PetriAdmission,
}

impl CreateRunPersistenceInput {
    pub fn materialized(&self) -> &MaterializedRun {
        &self.materialized
    }

    pub fn run_id(&self) -> RunId {
        self.run_id
    }

    pub fn run_dir(&self) -> &Path {
        &self.run_dir
    }

    pub fn workflow_slug(&self) -> Option<&str> {
        self.workflow_slug.as_deref()
    }

    pub fn workflow_version_id(&self) -> Option<WorkflowVersionId> {
        self.workflow_version_id
    }

    pub fn automation(&self) -> Option<&AutomationRef> {
        self.automation.as_ref()
    }

    pub fn definition(&self) -> &RunDefinition {
        &self.materialized.definition
    }
}

/// Materialize Fabro's run-level settings around the admitted graph.
///
/// The goal is the settings' `run.goal` when the run has one (the API's
/// goal, or a `[run.goal]` layer, its `file` form read at the working
/// directory), else the goal Petri admitted; it becomes both the graph's
/// displayed goal and the inline `run.goal`, and stays absent when the
/// workflow has none. A pull request block the settings disable is dropped.
pub fn materialize_admitted_run(input: AdmittedRunInput) -> Result<MaterializedRun, Error> {
    let AdmittedRunInput {
        mut settings,
        cwd,
        mut graph,
        source,
        workflow_path,
        workflow_bundle,
    } = input;
    let working_directory = resolve_working_directory_from_run(&settings.run, &cwd);
    if let Some(resolved) = resolve_run_goal_from_namespace(&settings.run, &working_directory)
        .map_err(|err| Error::Parse(err.to_string()))?
    {
        graph.goal = resolved.text;
    }
    settings.run.goal = if graph.goal.is_empty() {
        None
    } else {
        Some(RunGoal::Inline(InterpString::parse(&graph.goal)))
    };
    if settings
        .run
        .pull_request
        .as_ref()
        .is_some_and(|pull_request| !pull_request.enabled)
    {
        settings.run.pull_request = None;
    }
    let labels = settings.combined_labels();
    Ok(MaterializedRun {
        settings,
        graph,
        source,
        workflow_slug: workflow_slug_from_path(workflow_path.as_path()),
        definition: RunDefinition::new(workflow_path, workflow_bundle),
        source_directory: working_directory.to_string_lossy().into_owned(),
        labels,
    })
}

/// Assemble all inputs needed for persistence without I/O.
pub fn assemble_create_run_persistence_input(
    materialized: MaterializedRun,
    metadata: CreateRunPersistenceMetadata,
) -> CreateRunPersistenceInput {
    let CreateRunPersistenceMetadata {
        run_id,
        storage_root,
        workflow_slug,
        workflow_version_id,
        target,
        title,
        automation,
        git,
        fork_source_ref,
        parent_id,
        provenance,
        web_url,
        admission,
    } = metadata;
    let run_dir = Storage::new(storage_root)
        .run_scratch(&run_id)
        .root()
        .to_path_buf();
    let workflow_slug = workflow_slug.or_else(|| materialized.workflow_slug.clone());

    CreateRunPersistenceInput {
        materialized,
        run_id,
        run_dir,
        workflow_slug,
        workflow_version_id,
        target,
        title,
        automation,
        git,
        fork_source_ref,
        parent_id,
        provenance,
        web_url,
        admission,
    }
}

/// Persist one materialized run: its scratch directory, then its first
/// records.
pub async fn persist_create_run(
    store: &Database,
    input: CreateRunPersistenceInput,
) -> Result<CreatedRun, Error> {
    let CreateRunPersistenceInput {
        materialized,
        run_id,
        run_dir,
        workflow_slug,
        workflow_version_id,
        target,
        title,
        automation,
        git,
        fork_source_ref,
        parent_id,
        provenance,
        web_url,
        admission,
    } = input;
    let MaterializedRun {
        settings,
        graph,
        source,
        workflow_slug: _,
        definition,
        source_directory,
        labels,
    } = materialized;
    let (source_directory, git) = match target.as_ref() {
        Some(RunTarget::None {}) => (None, None),
        Some(RunTarget::Folder { path }) => (Some(path.clone()), git),
        Some(RunTarget::Git(_)) | None => (Some(source_directory), git),
    };
    let spec = RunSpec {
        run_id,
        settings,
        graph,
        graph_source: Some(source.clone()),
        workflow_slug,
        workflow_version_id,
        target,
        automation,
        source_directory,
        labels,
        provenance,
        definition_blob: None,
        spec_blob: None,
        git,
        fork_source_ref,
        admission,
    };
    let scratch = run_dir.clone();
    spawn_blocking(move || {
        std::fs::create_dir_all(&scratch).map_err(|err| {
            Error::Io(format!(
                "creating run directory {}: {err}",
                scratch.display()
            ))
        })
    })
    .await
    .map_err(|err| Error::engine_with_source("workflow create task failed", err))??;

    let spec = Box::pin(persist_created_run(
        store,
        spec,
        &definition,
        title,
        parent_id,
        web_url,
    ))
    .await?;

    Ok(CreatedRun {
        spec,
        source,
        run_id,
        run_dir,
    })
}

/// The run's first records: `run.created` with the spec Fabro built, and
/// the `submitted` lifecycle transition. Both wake the run's projector.
/// Returns the spec as recorded, naming its definition and spec blobs.
async fn persist_created_run(
    store: &Database,
    mut spec: RunSpec,
    definition: &RunDefinition,
    explicit_title: Option<String>,
    parent_id: Option<RunId>,
    web_url: Option<String>,
) -> Result<RunSpec, Error> {
    let definition_bytes = serde_json::to_vec(definition)
        .map_err(|err| Error::engine_with_source("failed to serialize run definition", err))?;
    let spec_bytes = serde_json::to_vec(&spec)
        .map_err(|err| Error::engine_with_source("failed to serialize run spec", err))?;
    let blob_store = store.blobs();
    let (definition_blob, spec_blob) = tokio::try_join!(
        write_blob(&blob_store, &definition_bytes),
        write_blob(&blob_store, &spec_bytes),
    )?;

    let title = explicit_title.unwrap_or_else(|| fabro_types::infer_run_title(spec.graph.goal()));
    spec.definition_blob = Some(definition_blob);
    spec.spec_blob = Some(spec_blob);
    let run_id = spec.run_id;
    let created = PlatformRecord::RunCreated(RunCreatedRecord {
        spec: spec.clone(),
        title: Some(title),
        parent_id,
        retried_from: None,
        web_url,
    });
    let submitted = PlatformRecord::RunLifecycle(
        RunLifecycleRecord::new(RunLifecycleKind::Submitted).with_status(RunStatus::Submitted),
    );
    let summaries = store.run_summary_store();
    let platform_records = summaries.platform_records();
    for platform_record in [created, submitted] {
        platform_records
            .append(&run_id, &platform_record, None)
            .await
            .map_err(store_error)?;
    }
    summaries.notify_platform_record(run_id);
    Ok(spec)
}

async fn write_blob(blob_store: &BlobStore, bytes: &[u8]) -> Result<BlobHash, Error> {
    blob_store.write(bytes).await.map_err(store_error)
}

fn store_error(err: impl Into<anyhow::Error>) -> Error {
    Error::engine_with_source("run store operation failed", err)
}

pub fn make_run_dir(scratch_base: &Path, run_id: &RunId) -> PathBuf {
    fabro_config::RunScratch::for_run(scratch_base, run_id)
        .root()
        .to_path_buf()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fabro_config::{RunLayer, WorkflowSettingsBuilder};
    use fabro_store::platform_records::StoredPlatformRecord;
    use fabro_types::settings::interp::ResolveCtx;
    use fabro_types::settings::run::PullRequestSettings;
    use fabro_types::{RunGraphNode, StageHandler, fixtures, test_support};

    use super::*;
    use crate::workflow_bundle::BundledWorkflow;

    const DOT: &str = r#"digraph Test {
        graph [goal="Graph goal"]
        start [shape=Mdiamond]
        exit  [shape=Msquare]
        start -> exit
    }"#;

    fn workflow_path() -> ManifestPath {
        ManifestPath::from_wire("flows/ship.fabro").unwrap()
    }

    fn graph(goal: &str) -> RunGraph {
        let mut graph = RunGraph::new("Test");
        graph.goal = goal.to_string();
        graph.nodes.insert("start".to_string(), RunGraphNode {
            label: "start".to_string(),
            kind:  StageHandler::Start,
        });
        graph
    }

    fn settings(run: RunLayer) -> WorkflowSettings {
        WorkflowSettingsBuilder::new()
            .server_manifest_defaults(
                RunLayer::default(),
                fabro_environment::seeded_catalog_layer(),
            )
            .run_overrides(run)
            .build()
            .expect("settings should resolve")
    }

    fn admitted(settings: WorkflowSettings, goal: &str, cwd: &Path) -> AdmittedRunInput {
        let bundled = BundledWorkflow {
            path:   workflow_path(),
            source: DOT.to_string(),
            config: None,
            files:  HashMap::new(),
        };
        AdmittedRunInput {
            settings,
            cwd: cwd.to_path_buf(),
            graph: graph(goal),
            source: DOT.to_string(),
            workflow_path: workflow_path(),
            workflow_bundle: WorkflowBundle::new(HashMap::from([(workflow_path(), bundled)])),
        }
    }

    fn metadata(run_id: RunId, storage_root: &Path) -> CreateRunPersistenceMetadata {
        CreateRunPersistenceMetadata {
            run_id,
            storage_root: storage_root.to_path_buf(),
            workflow_slug: None,
            workflow_version_id: None,
            target: None,
            title: None,
            automation: None,
            git: None,
            fork_source_ref: None,
            parent_id: None,
            provenance: test_support::test_run_provenance(),
            web_url: None,
            admission: PetriAdmission::default(),
        }
    }

    fn inline_goal(settings: &WorkflowSettings) -> Option<String> {
        match settings.run.goal.as_ref()? {
            RunGoal::Inline(goal) => Some(goal.resolve_with(&mut ResolveCtx::default()).unwrap()),
            RunGoal::File(_) => panic!("the materialized goal should be inline"),
        }
    }

    async fn platform_records(store: &Database, run_id: RunId) -> Vec<StoredPlatformRecord> {
        store
            .run_summary_store()
            .platform_records()
            .read(&run_id)
            .await
            .unwrap()
    }

    #[test]
    fn the_admitted_goal_becomes_the_inline_run_goal() {
        let dir = tempfile::tempdir().unwrap();
        let materialized = materialize_admitted_run(admitted(
            settings(RunLayer::default()),
            "Graph goal",
            dir.path(),
        ))
        .unwrap();

        assert_eq!(materialized.graph().goal(), "Graph goal");
        assert_eq!(
            inline_goal(materialized.settings()).as_deref(),
            Some("Graph goal")
        );
        assert_eq!(materialized.workflow_slug.as_deref(), Some("ship"));
        assert_eq!(materialized.definition.workflow_path, workflow_path());
        assert_eq!(
            materialized.source_directory,
            dir.path().to_string_lossy().into_owned()
        );
    }

    #[test]
    fn the_settings_goal_wins_over_the_admitted_goal_and_a_file_goal_is_read() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("goal.md"), "Goal from file").unwrap();
        let inline = settings(RunLayer {
            goal: Some(fabro_config::RunGoalLayer::Inline(InterpString::parse(
                "Override goal",
            ))),
            ..RunLayer::default()
        });
        let from_file = settings(RunLayer {
            goal: Some(fabro_config::RunGoalLayer::File {
                file: InterpString::parse("goal.md"),
            }),
            ..RunLayer::default()
        });

        let inline = materialize_admitted_run(admitted(inline, "Graph goal", dir.path())).unwrap();
        let from_file =
            materialize_admitted_run(admitted(from_file, "Graph goal", dir.path())).unwrap();

        assert_eq!(inline.graph().goal(), "Override goal");
        assert_eq!(
            inline_goal(inline.settings()).as_deref(),
            Some("Override goal")
        );
        assert_eq!(from_file.graph().goal(), "Goal from file");
        assert_eq!(
            inline_goal(from_file.settings()).as_deref(),
            Some("Goal from file")
        );
    }

    #[test]
    fn a_workflow_without_a_goal_keeps_none_and_a_disabled_pull_request_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let mut disabled = settings(RunLayer::default());
        disabled.run.pull_request = Some(PullRequestSettings::default());
        assert!(!disabled.run.pull_request.as_ref().unwrap().enabled);

        let materialized = materialize_admitted_run(admitted(disabled, "", dir.path())).unwrap();

        assert_eq!(materialized.settings().run.goal, None);
        assert_eq!(materialized.settings().run.pull_request, None);
    }

    #[tokio::test]
    async fn persisting_records_the_spec_with_the_graph_and_its_source_then_submits() {
        let dir = tempfile::tempdir().unwrap();
        let storage_root = dir.path().join("storage");
        let materialized = materialize_admitted_run(admitted(
            settings(RunLayer::default()),
            "## Plan: Ship it\n\nDetails",
            dir.path(),
        ))
        .unwrap();
        let mut metadata = metadata(fixtures::RUN_2, &storage_root);
        metadata.workflow_version_id = Some(test_support::test_workflow_version_id());
        let store = Arc::new(fabro_store::test_support::test_database());

        let created = persist_create_run(
            store.as_ref(),
            assemble_create_run_persistence_input(materialized, metadata),
        )
        .await
        .unwrap();

        assert_eq!(created.run_id, fixtures::RUN_2);
        assert_eq!(created.source, DOT);
        assert!(created.run_dir.is_dir());
        assert_eq!(created.spec.workflow_slug.as_deref(), Some("ship"));
        assert!(created.spec.definition_blob.is_some());
        assert!(created.spec.spec_blob.is_some());

        let records = platform_records(&store, fixtures::RUN_2).await;
        assert_eq!(
            records
                .iter()
                .map(|stored| stored.record.kind().to_string())
                .collect::<Vec<_>>(),
            vec!["run.created", "run.lifecycle"]
        );
        let PlatformRecord::RunCreated(record) = &records[0].record else {
            panic!("first durable record should be run.created");
        };
        assert_eq!(record.title.as_deref(), Some("Ship it"));
        assert_eq!(record.spec.graph.name, "Test");
        assert_eq!(record.spec.graph.goal(), "## Plan: Ship it\n\nDetails");
        assert_eq!(record.spec.graph_source.as_deref(), Some(DOT));
        assert_eq!(
            record.spec.workflow_version_id,
            Some(test_support::test_workflow_version_id())
        );
        let PlatformRecord::RunLifecycle(submitted) = &records[1].record else {
            panic!("second durable record should be the submitted transition");
        };
        assert_eq!(submitted.transition, RunLifecycleKind::Submitted);
        assert_eq!(submitted.status, Some(RunStatus::Submitted));
    }

    #[tokio::test]
    async fn an_explicit_title_and_the_target_shape_the_recorded_spec() {
        let dir = tempfile::tempdir().unwrap();
        let materialized = materialize_admitted_run(admitted(
            settings(RunLayer::default()),
            "Graph goal",
            dir.path(),
        ))
        .unwrap();
        let mut metadata = metadata(fixtures::RUN_3, &dir.path().join("storage"));
        metadata.title = Some("Explicit".to_string());
        metadata.target = Some(RunTarget::None {});
        metadata.git = Some(GitContext {
            origin_url: "https://github.com/fabro-sh/fabro.git".to_string(),
            branch:     "main".to_string(),
            sha:        None,
            dirty:      fabro_types::DirtyStatus::Clean,
        });
        let store = Arc::new(fabro_store::test_support::test_database());

        let created = persist_create_run(
            store.as_ref(),
            assemble_create_run_persistence_input(materialized, metadata),
        )
        .await
        .unwrap();

        let records = platform_records(&store, fixtures::RUN_3).await;
        let PlatformRecord::RunCreated(record) = &records[0].record else {
            panic!("first durable record should be run.created");
        };
        assert_eq!(record.title.as_deref(), Some("Explicit"));
        assert_eq!(created.spec.source_directory, None);
        assert_eq!(created.spec.git, None);
    }
}
