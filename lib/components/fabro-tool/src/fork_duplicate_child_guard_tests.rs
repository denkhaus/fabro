//! Fork-only presence tests for the duplicate-child guard (fabro-8ee1,
//! re-landed 2026-09-14 after the #832 salvage merge dropped it together
//! with its inline tests). PIN FILE: upstream does not have it; a merge
//! cannot conflict it away, and a dropped seam call in `create.rs` reds
//! the create test here.

use std::sync::{Arc, Mutex};

use chrono::TimeZone as _;
use fabro_api::types::{ApiQuestion, RunWaitResult, SubmitAnswerRequest};
use fabro_types::test_support::test_principal;
use fabro_types::{EventEnvelope, RunProjection, RunStatus, WorkflowVersionId};
use serde_json::json;

use crate::common::FabroToolBackend;
use crate::create::{CreateRunsResult, FabroRunCreateParams, create_runs};

fn version_id() -> WorkflowVersionId {
    fabro_types::BlobHash::new(b"workflow").into()
}

fn other_version_id() -> WorkflowVersionId {
    fabro_types::BlobHash::new(b"other").into()
}

/// A child run summary with the given version and status — the minimal
/// shape `reject_duplicate_active_child` reads.
fn child(version: Option<WorkflowVersionId>, status: RunStatus) -> fabro_types::Run {
    let mut run = run_body();
    run.workflow.workflow_version_id = version;
    run.lifecycle.status = status;
    run
}

fn run_body() -> fabro_types::Run {
    use fabro_types::outcome::FailureCategory;
    let _ = FailureCategory::Deterministic; // import surface stability
    fabro_types::Run {
        id:               fabro_types::RunId::new(),
        parent_id:        None,
        children_count:   0,
        title:            "child".to_string(),
        goal:             "child".to_string(),
        workflow:         fabro_types::WorkflowRef {
            workflow_version_id: None,
            slug:                Some("develop".to_string()),
            name:                Some("Develop".to_string()),
            graph_name:          None,
            node_count:          0,
            edge_count:          0,
        },
        automation:       None,
        repository:       None,
        created_by:       test_principal(),
        origin:           fabro_types::RunOrigin::default(),
        labels:           std::collections::HashMap::new(),
        lifecycle:        fabro_types::RunLifecycle {
            status:          RunStatus::Submitted,
            approval:        None,
            pending_control: None,
            queue_position:  None,
            error:           None,
            archived:        false,
            archived_at:     None,
        },
        sandbox:          None,
        models:           Vec::new(),
        source_directory: None,
        timestamps:       fabro_types::RunTimestamps {
            created_at:    chrono::Utc.with_ymd_and_hms(2026, 9, 14, 0, 0, 0).unwrap(),
            started_at:    None,
            last_event_at: None,
            completed_at:  None,
        },
        timing:           None,
        billing:          None,
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

/// Backend double: `list_store_runs_by_parent` serves the scripted
/// children; everything else bails like fork_seam_tests' BailingBackend.
#[derive(Default)]
struct GuardBackend {
    children: Mutex<Vec<fabro_types::Run>>,
    creates:  Mutex<Vec<fabro_types::RunId>>,
}

#[async_trait::async_trait]
impl FabroToolBackend for GuardBackend {
    async fn create_run_from_intent(
        &self,
        _intent: fabro_types::RunIntent,
    ) -> anyhow::Result<fabro_types::RunId> {
        let run_id = fabro_types::RunId::new();
        self.creates.lock().unwrap().push(run_id);
        Ok(run_id)
    }
    async fn list_store_runs_by_parent(
        &self,
        _parent_id: fabro_types::RunId,
    ) -> anyhow::Result<Vec<fabro_types::Run>> {
        Ok(self.children.lock().unwrap().clone())
    }
    async fn create_workflow_version(
        &self,
        _source: crate::ValidatedWorkflowVersionCreate,
    ) -> anyhow::Result<WorkflowVersionId> {
        anyhow::bail!("mock backend: create_workflow_version not implemented")
    }
    async fn resolve_run(&self, _selector: &str) -> anyhow::Result<fabro_types::Run> {
        anyhow::bail!("mock backend: resolve_run not implemented")
    }
    async fn retrieve_run(&self, _run_id: &fabro_types::RunId) -> anyhow::Result<fabro_types::Run> {
        anyhow::bail!("mock backend: retrieve_run not implemented")
    }
    async fn start_run(
        &self,
        _run_id: &fabro_types::RunId,
        _resume: bool,
    ) -> anyhow::Result<fabro_types::Run> {
        anyhow::bail!("mock backend: start_run not implemented")
    }
    async fn approve_run(&self, _run_id: &fabro_types::RunId) -> anyhow::Result<fabro_types::Run> {
        anyhow::bail!("mock backend: approve_run not implemented")
    }
    async fn deny_run(
        &self,
        _run_id: &fabro_types::RunId,
        _reason: Option<String>,
    ) -> anyhow::Result<fabro_types::Run> {
        anyhow::bail!("mock backend: deny_run not implemented")
    }
    async fn cancel_run(&self, _run_id: &fabro_types::RunId) -> anyhow::Result<fabro_types::Run> {
        anyhow::bail!("mock backend: cancel_run not implemented")
    }
    async fn interrupt_run(&self, _run_id: &fabro_types::RunId) -> anyhow::Result<()> {
        anyhow::bail!("mock backend: interrupt_run not implemented")
    }
    async fn steer_run(
        &self,
        _run_id: &fabro_types::RunId,
        _text: String,
        _interrupt: bool,
    ) -> anyhow::Result<()> {
        anyhow::bail!("mock backend: steer_run not implemented")
    }
    async fn archive_run(&self, _run_id: &fabro_types::RunId) -> anyhow::Result<fabro_types::Run> {
        anyhow::bail!("mock backend: archive_run not implemented")
    }
    async fn unarchive_run(
        &self,
        _run_id: &fabro_types::RunId,
    ) -> anyhow::Result<fabro_types::Run> {
        anyhow::bail!("mock backend: unarchive_run not implemented")
    }
    async fn list_store_runs(&self) -> anyhow::Result<Vec<fabro_types::Run>> {
        anyhow::bail!("mock backend: list_store_runs not implemented")
    }
    async fn link_run_parent(
        &self,
        _child_id: &fabro_types::RunId,
        _parent_id: &fabro_types::RunId,
    ) -> anyhow::Result<fabro_types::Run> {
        anyhow::bail!("mock backend: link_run_parent not implemented")
    }
    async fn unlink_run_parent(
        &self,
        _child_id: &fabro_types::RunId,
    ) -> anyhow::Result<fabro_types::Run> {
        anyhow::bail!("mock backend: unlink_run_parent not implemented")
    }
    async fn get_run_state(&self, _run_id: &fabro_types::RunId) -> anyhow::Result<RunProjection> {
        anyhow::bail!("mock backend: get_run_state not implemented")
    }
    async fn list_run_events(
        &self,
        _run_id: &fabro_types::RunId,
        _after: Option<u32>,
        _limit: Option<usize>,
    ) -> anyhow::Result<Vec<EventEnvelope>> {
        anyhow::bail!("mock backend: list_run_events not implemented")
    }
    async fn list_run_events_until(
        &self,
        _run_id: &fabro_types::RunId,
        _after: Option<u32>,
        _limit: usize,
    ) -> anyhow::Result<Vec<EventEnvelope>> {
        anyhow::bail!("mock backend: list_run_events_until not implemented")
    }
    async fn run_pull_request_state(
        &self,
        _run_id: &fabro_types::RunId,
    ) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
    async fn create_ask_session(
        &self,
        _run_id: &fabro_types::RunId,
        _title: &str,
    ) -> anyhow::Result<String> {
        anyhow::bail!("mock backend: create_ask_session not implemented")
    }
    async fn submit_ask_turn(
        &self,
        _run_id: &fabro_types::RunId,
        _session_id: &str,
        _question: &str,
    ) -> anyhow::Result<crate::AskTurnOutcome> {
        anyhow::bail!("mock backend: submit_ask_turn not implemented")
    }
    async fn wait_run(
        &self,
        _run_id: &fabro_types::RunId,
        _until: crate::RunWaitUntil,
        _timeout_ms: u64,
    ) -> anyhow::Result<RunWaitResult> {
        anyhow::bail!("mock backend: wait_run not implemented")
    }
    async fn list_runs_of_workflow(
        &self,
        _workflow: &str,
        _created_since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> anyhow::Result<Vec<fabro_types::Run>> {
        anyhow::bail!("mock backend: list_runs_of_workflow not implemented")
    }
    async fn list_run_questions(
        &self,
        _run_id: &fabro_types::RunId,
    ) -> anyhow::Result<Vec<ApiQuestion>> {
        anyhow::bail!("mock backend: list_run_questions not implemented")
    }
    async fn submit_run_answer(
        &self,
        _run_id: &fabro_types::RunId,
        _question_id: &str,
        _body: SubmitAnswerRequest,
    ) -> anyhow::Result<()> {
        anyhow::bail!("mock backend: submit_run_answer not implemented")
    }
}

fn create_params() -> FabroRunCreateParams {
    FabroRunCreateParams {
        runs: vec![
            serde_json::from_value(json!({
                "workflow_version_id": version_id(),
                "target": {"kind": "none"},
                "parent_id": fabro_types::RunId::new().to_string(),
                "start": false,
            }))
            .expect("params parse"),
        ],
    }
}

#[tokio::test]
async fn same_version_non_terminal_sibling_rejects_the_create() {
    let backend = Arc::new(GuardBackend::default());
    *backend.children.lock().unwrap() = vec![child(Some(version_id()), RunStatus::Running)];
    let error = create_runs(backend.clone(), create_params())
        .await
        .expect_err("duplicate active child must reject the create");
    assert!(
        error
            .as_str()
            .contains("duplicate child rejected (fabro-8ee1)"),
        "error names the guard: {}",
        error.as_str()
    );
    assert!(
        backend.creates.lock().unwrap().is_empty(),
        "no run may be created behind the guard"
    );
}

#[tokio::test]
async fn terminal_sibling_and_other_version_stay_allowed() {
    let backend = Arc::new(GuardBackend::default());
    *backend.children.lock().unwrap() = vec![
        child(Some(version_id()), RunStatus::Failed {
            reason: fabro_types::FailureReason::SoftStop,
        }),
        child(Some(other_version_id()), RunStatus::Running),
    ];
    let result: Result<CreateRunsResult, _> = create_runs(backend.clone(), create_params()).await;
    // The create passes the guard; the mock backend bails later at
    // start/retrieve — the guard must not be the error.
    let _ = result;
    assert_eq!(backend.creates.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn no_parent_create_bypasses_the_guard_entirely() {
    let backend = Arc::new(GuardBackend::default());
    let mut params = create_params();
    params.runs[0] = serde_json::from_value(json!({
        "workflow_version_id": version_id(),
        "target": {"kind": "none"},
        "start": false,
    }))
    .expect("params parse");
    let _ = create_runs(backend.clone(), params).await;
    assert_eq!(backend.creates.lock().unwrap().len(), 1);
}
