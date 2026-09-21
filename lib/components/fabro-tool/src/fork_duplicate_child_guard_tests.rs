//! Presence tests for the fork duplicate-child guard (fabro-8ee1, petri
//! port W2-1). Fork-only file: a merge resolution that drops it dropped
//! the feature — restore it, never relax the tests.

use std::sync::Mutex;

use fabro_api::types;
use fabro_types::status::RunStatus;
use fabro_types::{Run, RunId, WorkflowVersionId};

use crate::common::{FabroToolBackend, ToolResult};
use crate::create::{
    CreateRunOptions, CreateRunSpec, CreateRunsResult, FabroRunCreateParams,
    create_runs_with_options,
};

fn version_id() -> WorkflowVersionId {
    WorkflowVersionId::from(fabro_types::BlobHash::new(b"guard-test-version"))
}

fn other_version_id() -> WorkflowVersionId {
    WorkflowVersionId::from(fabro_types::BlobHash::new(b"guard-other-version"))
}

fn child(status: RunStatus) -> Run {
    let mut run = run_body();
    run.lifecycle.status = status;
    run
}

fn run_body() -> Run {
    Run {
        id:               RunId::new(),
        parent_id:        None,
        children_count:   0,
        title:            "child".to_string(),
        goal:             "child".to_string(),
        workflow:         fabro_types::WorkflowRef {
            slug:       Some("develop".to_string()),
            name:       Some("Develop".to_string()),
            graph_name: None,
            node_count: 0,
            edge_count: 0,
        },
        automation:       None,
        repository:       None,
        created_by:       fabro_types::test_support::test_principal(),
        origin:           fabro_types::RunOrigin::default(),
        labels:           std::collections::HashMap::new(),
        lifecycle:        fabro_types::RunLifecycle {
            status:             RunStatus::Submitted,
            approval:           None,
            pending_control:    None,
            queue_position:     None,
            error:              None,
            conclusion_failure: None,
            archived:           false,
            archived_at:        None,
        },
        sandbox:          None,
        models:           Vec::new(),
        source_directory: None,
        timestamps:       fabro_types::RunTimestamps {
            created_at:    chrono::Utc::now(),
            started_at:    None,
            last_event_at: None,
            completed_at:  None,
        },
        timing:           None,
        usage:            Default::default(),
        size:             fabro_types::RunSize::default(),
        ask_fabro:        fabro_types::AskFabro::default(),
        diff:             None,
        pull_request:     None,
        current_question: None,
        superseded_by:    None,
        retried_from:     None,
        links:            fabro_types::RunLinks { web: None },
    }
}

/// Backend double: `list_store_runs_by_parent` serves the scripted children
/// with a scripted workflow version (petri: the version lives in the
/// projection's spec, so `get_run_state` serves it); `create_run_from_intent`
/// records the call; everything else bails.
#[derive(Default)]
struct GuardBackend {
    children: Mutex<Vec<(Run, Option<WorkflowVersionId>)>>,
    creates:  Mutex<Vec<RunId>>,
}

fn bails<T>(what: &str) -> anyhow::Result<T> {
    Err(anyhow::anyhow!(
        "bailing backend: {what} must not be called"
    ))
}

#[async_trait::async_trait]
impl FabroToolBackend for GuardBackend {
    async fn create_run_from_intent(
        &self,
        intent: fabro_types::RunIntent,
    ) -> anyhow::Result<RunId> {
        let _ = intent;
        let id = RunId::new();
        self.creates.lock().unwrap().push(id);
        Ok(id)
    }

    async fn resolve_run(&self, selector: &str) -> anyhow::Result<Run> {
        bails(selector)
    }

    async fn retrieve_run(&self, run_id: &RunId) -> anyhow::Result<Run> {
        self.children
            .lock()
            .unwrap()
            .iter()
            .find(|(run, _)| run.id == *run_id)
            .map(|(run, _)| run.clone())
            .ok_or_else(|| anyhow::anyhow!("unknown child {run_id}"))
    }

    async fn start_run(&self, _run_id: &RunId, _resume: bool) -> anyhow::Result<Run> {
        // create_runs_with_options starts by default; serve the summary.
        Ok(run_body())
    }

    async fn deny_run(&self, run_id: &RunId, _reason: Option<String>) -> anyhow::Result<Run> {
        bails(&run_id.to_string())
    }

    async fn cancel_run(&self, run_id: &RunId) -> anyhow::Result<Run> {
        bails(&run_id.to_string())
    }

    async fn interrupt_run(&self, run_id: &RunId) -> anyhow::Result<()> {
        bails(&run_id.to_string())
    }

    async fn steer_run(
        &self,
        run_id: &RunId,
        _text: String,
        _interrupt: bool,
    ) -> anyhow::Result<()> {
        bails(&run_id.to_string())
    }

    async fn archive_run(&self, run_id: &RunId) -> anyhow::Result<Run> {
        bails(&run_id.to_string())
    }

    async fn unarchive_run(&self, run_id: &RunId) -> anyhow::Result<Run> {
        bails(&run_id.to_string())
    }

    async fn approve_run(&self, run_id: &RunId) -> anyhow::Result<Run> {
        bails(&run_id.to_string())
    }

    async fn list_store_runs(&self) -> anyhow::Result<Vec<Run>> {
        bails("list_store_runs")
    }

    async fn list_store_runs_by_parent(&self, _parent_id: RunId) -> anyhow::Result<Vec<Run>> {
        Ok(self
            .children
            .lock()
            .unwrap()
            .iter()
            .map(|(run, _)| run.clone())
            .collect())
    }

    async fn link_run_parent(&self, child: &RunId, parent: &RunId) -> anyhow::Result<Run> {
        bails(&format!("{child}-{parent}"))
    }

    async fn unlink_run_parent(&self, child: &RunId) -> anyhow::Result<Run> {
        bails(&child.to_string())
    }

    async fn get_run_state(&self, run_id: &RunId) -> anyhow::Result<fabro_types::RunProjection> {
        let (run, version) = self
            .children
            .lock()
            .unwrap()
            .iter()
            .find(|(run, _)| run.id == *run_id)
            .map(|(run, version)| (run.clone(), *version))
            .ok_or_else(|| anyhow::anyhow!("unknown child {run_id}"))?;
        let _ = run;
        let mut projection = projection_body();
        projection.spec.workflow_version_id = version;
        Ok(projection)
    }

    async fn list_run_stream(
        &self,
        run_id: &RunId,
        _after: u64,
        _limit: Option<usize>,
    ) -> anyhow::Result<Vec<fabro_types::RunStreamItem>> {
        bails(&run_id.to_string())
    }

    async fn list_run_questions(&self, run_id: &RunId) -> anyhow::Result<Vec<types::ApiQuestion>> {
        bails(&run_id.to_string())
    }

    async fn submit_run_answer(
        &self,
        run_id: &RunId,
        _question_id: &str,
        _body: types::SubmitAnswerRequest,
    ) -> anyhow::Result<()> {
        bails(&run_id.to_string())
    }
}

fn projection_body() -> fabro_types::RunProjection {
    use std::collections::HashMap;
    let run_id = RunId::new();
    fabro_types::RunProjection::new(
        "guard".to_string(),
        fabro_types::RunSpec {
            run_id,
            settings: fabro_types::WorkflowSettings::default(),
            graph: fabro_types::RunGraph::new("test"),
            graph_source: None,
            workflow_slug: None,
            workflow_version_id: None,
            target: None,
            automation: None,
            source_directory: None,
            labels: HashMap::new(),
            provenance: fabro_types::test_support::test_run_provenance(),
            definition_blob: None,
            spec_blob: None,
            git: None,
            fork_source_ref: None,
            admission: fabro_types::PetriAdmission::default(),
        },
        chrono::Utc::now(),
    )
}

fn create_params(version: WorkflowVersionId) -> FabroRunCreateParams {
    FabroRunCreateParams {
        runs: vec![CreateRunSpec {
            workflow_version_id: version,
            target:              Some(fabro_types::RunTarget::None {}),
            args:                Default::default(),
            environment_id:      None,
            parent_id:           None,
            title:               None,
            goal:                None,
            start:               None,
        }],
    }
}

#[tokio::test]
async fn same_version_non_terminal_sibling_rejects_the_create() {
    let backend = std::sync::Arc::new(GuardBackend {
        children: Mutex::new(vec![(child(RunStatus::Running), Some(version_id()))]),
        creates:  Mutex::new(Vec::new()),
    });
    let result: ToolResult<CreateRunsResult> = create_runs_with_options(
        backend.clone(),
        create_params(version_id()),
        CreateRunOptions {
            forced_parent_id: Some(RunId::new()),
            ..Default::default()
        },
    )
    .await;
    let error = result.expect_err("duplicate must be rejected");
    assert!(
        error
            .to_string()
            .contains("duplicate child rejected (fabro-8ee1)"),
        "rejection names the guard: {error:?}"
    );
    assert!(
        backend.creates.lock().unwrap().is_empty(),
        "no run may be created on rejection"
    );
}

#[tokio::test]
async fn terminal_sibling_and_other_version_stay_allowed() {
    let backend = std::sync::Arc::new(GuardBackend {
        children: Mutex::new(vec![
            (
                child(RunStatus::Failed {
                    reason: fabro_types::FailureReason::WorkflowError,
                }),
                Some(version_id()),
            ),
            (child(RunStatus::Running), Some(other_version_id())),
        ]),
        creates:  Mutex::new(Vec::new()),
    });
    let result = create_runs_with_options(
        backend.clone(),
        create_params(version_id()),
        CreateRunOptions {
            forced_parent_id: Some(RunId::new()),
            ..Default::default()
        },
    )
    .await;
    let created = result.expect("terminal sibling and foreign version stay allowed");
    assert_eq!(
        created.runs.len(),
        1,
        "exactly the requested run is created"
    );
}

#[tokio::test]
async fn no_parent_create_bypasses_the_guard_entirely() {
    let backend = std::sync::Arc::new(GuardBackend::default());
    let result = create_runs_with_options(
        backend.clone(),
        create_params(version_id()),
        CreateRunOptions::default(),
    )
    .await;
    result.expect("unparented creates never consult the guard");
    assert_eq!(backend.creates.lock().unwrap().len(), 1);
}
