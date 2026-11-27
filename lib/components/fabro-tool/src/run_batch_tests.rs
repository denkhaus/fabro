//! Pins for batch reads over run ids (denkhaus/work#1712): one missing
//! id must not drop the found runs.

use std::sync::Arc;

use async_trait::async_trait;
use fabro_api::types;
use fabro_client::error::{ApiFailure, tag_with_failure};
use fabro_http::StatusCode;
use fabro_types::test_support::test_principal;
use fabro_types::{
    AskFabro, Run, RunId, RunLifecycle, RunLinks, RunOrigin, RunProjection, RunSize, RunStatus,
    RunStreamItem, RunTimestamps, SuccessReason, WorkflowRef,
};
use serde_json::json;

use crate::common::FabroToolBackend;
use crate::gather::{FabroRunGatherParams, ValidatedGatherRuns, gather_runs};
use crate::search::{FabroRunSearchParams, ValidatedSearchRuns, search_runs, search_runs_text};

const KEPT: &str = "01KRBZW4DW0000000000000001";
const PURGED: &str = "01KRBZW4DW0000000000000002";

/// One stored run (`KEPT`); every other selector is answered the way the
/// server answers a purged or never-existing run (404, "Run not found.").
struct OneRunBackend;

fn not_found() -> anyhow::Error {
    tag_with_failure(
        anyhow::anyhow!("request failed with status 404 Not Found"),
        ApiFailure::new(StatusCode::NOT_FOUND, None),
    )
}

fn kept_run() -> Run {
    Run {
        id:                  KEPT.parse().expect("test run id should parse"),
        parent_id:           None,
        children_count:      0,
        claimed_seed_id:     None,
        title:               "kept".to_string(),
        goal:                "kept".to_string(),
        workflow:            WorkflowRef {
            slug:       Some("develop".to_string()),
            name:       Some("Develop".to_string()),
            graph_name: None,
            node_count: 0,
            edge_count: 0,
        },
        automation:          None,
        repository:          None,
        created_by:          test_principal(),
        origin:              RunOrigin::default(),
        labels:              std::collections::HashMap::new(),
        lifecycle:           RunLifecycle {
            status:             RunStatus::Succeeded {
                reason: SuccessReason::Completed,
            },
            approval:           None,
            pending_control:    None,
            queue_position:     None,
            error:              None,
            conclusion_failure: None,
            archived:           false,
            archived_at:        None,
        },
        sandbox:             None,
        workflow_version_id: None,
        models:              Vec::new(),
        source_directory:    None,
        timestamps:          RunTimestamps {
            created_at:    chrono::Utc::now(),
            started_at:    None,
            last_event_at: None,
            completed_at:  None,
        },
        timing:              None,
        usage:               types::Usage::default(),
        size:                RunSize::default(),
        ask_fabro:           AskFabro::default(),
        diff:                None,
        pull_request:        None,
        current_question:    None,
        superseded_by:       None,
        retried_from:        None,
        links:               RunLinks { web: None },
    }
}

fn bails<T>(what: &str) -> anyhow::Result<T> {
    Err(anyhow::anyhow!(
        "bailing backend: {what} must not be called"
    ))
}

#[async_trait]
impl FabroToolBackend for OneRunBackend {
    async fn create_run_from_intent(
        &self,
        _intent: fabro_types::RunIntent,
    ) -> anyhow::Result<RunId> {
        bails("create_run_from_intent")
    }

    async fn resolve_run(&self, selector: &str) -> anyhow::Result<Run> {
        if selector == KEPT {
            return Ok(kept_run());
        }
        if selector == PURGED {
            return Err(not_found());
        }
        bails(selector)
    }

    async fn retrieve_run(&self, run_id: &RunId) -> anyhow::Result<Run> {
        if run_id.to_string() == KEPT {
            return Ok(kept_run());
        }
        bails(&run_id.to_string())
    }

    async fn start_run(&self, _run_id: &RunId, _resume: bool) -> anyhow::Result<Run> {
        bails("start_run")
    }

    async fn approve_run(&self, _run_id: &RunId) -> anyhow::Result<Run> {
        bails("approve_run")
    }

    async fn deny_run(&self, _run_id: &RunId, _reason: Option<String>) -> anyhow::Result<Run> {
        bails("deny_run")
    }

    async fn cancel_run(&self, _run_id: &RunId) -> anyhow::Result<Run> {
        bails("cancel_run")
    }

    async fn interrupt_run(&self, _run_id: &RunId) -> anyhow::Result<()> {
        bails("interrupt_run")
    }

    async fn steer_run(
        &self,
        _run_id: &RunId,
        _text: String,
        _interrupt: bool,
    ) -> anyhow::Result<()> {
        bails("steer_run")
    }

    async fn archive_run(&self, _run_id: &RunId) -> anyhow::Result<Run> {
        bails("archive_run")
    }

    async fn unarchive_run(&self, _run_id: &RunId) -> anyhow::Result<Run> {
        bails("unarchive_run")
    }

    async fn list_store_runs(&self) -> anyhow::Result<Vec<Run>> {
        bails("list_store_runs")
    }

    async fn list_store_runs_by_parent(&self, _parent_id: RunId) -> anyhow::Result<Vec<Run>> {
        bails("list_store_runs_by_parent")
    }

    async fn link_run_parent(&self, _child: &RunId, _parent: &RunId) -> anyhow::Result<Run> {
        bails("link_run_parent")
    }

    async fn unlink_run_parent(&self, _child: &RunId) -> anyhow::Result<Run> {
        bails("unlink_run_parent")
    }

    async fn get_run_state(&self, _run_id: &RunId) -> anyhow::Result<RunProjection> {
        bails("get_run_state")
    }

    async fn list_run_stream(
        &self,
        _run_id: &RunId,
        _after: u64,
        _limit: Option<usize>,
    ) -> anyhow::Result<Vec<RunStreamItem>> {
        bails("list_run_stream")
    }

    async fn list_run_questions(&self, _run_id: &RunId) -> anyhow::Result<Vec<types::ApiQuestion>> {
        bails("list_run_questions")
    }

    async fn submit_run_answer(
        &self,
        _run_id: &RunId,
        _question_id: &str,
        _body: types::SubmitAnswerRequest,
    ) -> anyhow::Result<()> {
        bails("submit_run_answer")
    }
}

fn search_params(run_ids: Vec<String>) -> ValidatedSearchRuns {
    ValidatedSearchRuns::try_from(FabroRunSearchParams {
        run_ids:        Some(run_ids),
        workflow:       None,
        labels:         None,
        status:         None,
        archived:       None,
        created_after:  None,
        created_before: None,
        first:          None,
        after:          None,
        parent_id:      None,
    })
    .expect("search params should validate")
}

fn gather_params(run_ids: Vec<String>) -> ValidatedGatherRuns {
    ValidatedGatherRuns::try_from(FabroRunGatherParams {
        run_ids,
        timeout_seconds: None,
        poll_interval_seconds: None,
    })
    .expect("gather params should validate")
}

#[tokio::test]
async fn search_keeps_the_found_run_and_names_the_missing_id() {
    let result = search_runs(
        Arc::new(OneRunBackend),
        search_params(vec![KEPT.to_string(), PURGED.to_string()]),
    )
    .await
    .expect("one missing id must not fail the batch");

    let body = serde_json::to_value(&result).expect("result should serialize");
    assert_eq!(
        body["runs"]
            .as_array()
            .expect("runs should be an array")
            .iter()
            .map(|run| run["run_id"].as_str().expect("run id should be a string"))
            .collect::<Vec<_>>(),
        vec![KEPT]
    );
    assert_eq!(body["missing_run_ids"], json!([PURGED]));
}

#[tokio::test]
async fn gather_keeps_the_found_run_and_names_the_missing_id() {
    let result = gather_runs(
        Arc::new(OneRunBackend),
        gather_params(vec![KEPT.to_string(), PURGED.to_string()]),
    )
    .await
    .expect("one missing id must not fail the batch");

    let body = serde_json::to_value(&result).expect("result should serialize");
    assert_eq!(body["runs"][0]["run_id"], json!(KEPT));
    assert_eq!(body["missing_run_ids"], json!([PURGED]));
}

#[tokio::test]
async fn search_without_a_miss_carries_no_missing_entry() {
    let result = search_runs(
        Arc::new(OneRunBackend),
        search_params(vec![KEPT.to_string()]),
    )
    .await
    .expect("every requested run exists");

    let body = serde_json::to_value(&result).expect("result should serialize");
    assert_eq!(body["runs"][0]["run_id"], json!(KEPT));
    assert!(body.get("missing_run_ids").is_none());
}

#[tokio::test]
async fn gather_without_a_miss_carries_no_missing_entry() {
    let result = gather_runs(
        Arc::new(OneRunBackend),
        gather_params(vec![KEPT.to_string()]),
    )
    .await
    .expect("every requested run exists");

    let body = serde_json::to_value(&result).expect("result should serialize");
    assert_eq!(body["runs"][0]["run_id"], json!(KEPT));
    assert!(body.get("missing_run_ids").is_none());
}

#[tokio::test]
async fn search_of_only_missing_ids_answers_with_the_missing_list() {
    let result = search_runs(
        Arc::new(OneRunBackend),
        search_params(vec![PURGED.to_string()]),
    )
    .await
    .expect("a batch of only missing ids still answers");

    assert!(result.runs.is_empty());
    assert_eq!(result.missing_run_ids, Some(vec![PURGED.to_string()]));
}

#[tokio::test]
async fn search_names_the_missing_ids_in_its_summary() {
    let result = search_runs(
        Arc::new(OneRunBackend),
        search_params(vec![KEPT.to_string(), PURGED.to_string()]),
    )
    .await
    .expect("one missing id must not fail the batch");

    assert_eq!(
        search_runs_text(&result),
        format!("found 1 Fabro run(s); 1 requested run id(s) not found: {PURGED}")
    );
}

#[tokio::test]
async fn run_get_for_a_missing_run_keeps_its_not_found_error() {
    let error = crate::run_get(
        Arc::new(OneRunBackend),
        crate::ValidatedRunGet::try_from(crate::FabroRunGetParams {
            run_id: PURGED.to_string(),
        })
        .expect("run id should validate"),
    )
    .await
    .expect_err("a single-id read of a missing run must still fail");

    assert!(error.as_str().contains("404"));
}
