//! Fork presence pin (fabro-c8e3): the run-wait tool and the bounded
//! PR-create retry are fork-added surfaces living in upstream-owned
//! crates (`fabro-tool/src/wait.rs`, `fabro-workflow/src/pull_request.rs`
//! retry loop) with their behavioral tests inline only — an upstream
//! merge can drop file and tests in one resolution without a red gate.
//! This fork-only file reds on removal.
//!
//! Pinned contracts (fabro-8795 + fabro-b5a9):
//! - `fabro_run_wait` resolves through the named run-tools registry (the
//!   conductor's child-run and merge waits have no substitute;
//!   `fabro_run_gather` cannot wait `until=merged`).
//! - The wait wire schema advertises run_id/until/timeout_ms.
//! - Param validation rejects blank run ids and zero timeouts.
//! - PR-create failures classify retryable (transport/5xx) vs deterministic
//!   (4xx/token/client/parse); the bounded retry inside
//!   `pull_request::open_pull_request` branches on exactly this.

use std::sync::Arc;

use async_trait::async_trait;
use fabro_api::types::{ApiQuestion, SubmitAnswerRequest};
use fabro_github::CreatePullRequestError;
use fabro_tool::{
    FabroRunWaitParams, FabroToolBackend, RunWaitUntil, ValidatedRunWait, tool_definitions,
};
use fabro_types::{Run, RunId, RunProjection, RunStreamItem};
use fabro_workflow::pull_request::open_pull_request;
use fabro_workflow::run_tools::register_named_fabro_run_tools;
use fabro_workflow::services::FabroRunToolServices;

struct PinBackend;

macro_rules! pin_unimplemented {
    () => {
        unimplemented!("the pin never executes tools")
    };
}

#[async_trait]
impl FabroToolBackend for PinBackend {
    async fn create_run_from_intent(
        &self,
        _intent: fabro_types::RunIntent,
    ) -> anyhow::Result<RunId> {
        pin_unimplemented!()
    }
    async fn resolve_run(&self, _selector: &str) -> anyhow::Result<Run> {
        pin_unimplemented!()
    }
    async fn retrieve_run(&self, _run_id: &RunId) -> anyhow::Result<Run> {
        pin_unimplemented!()
    }
    async fn start_run(&self, _run_id: &RunId, _resume: bool) -> anyhow::Result<Run> {
        pin_unimplemented!()
    }
    async fn approve_run(&self, _run_id: &RunId) -> anyhow::Result<Run> {
        pin_unimplemented!()
    }
    async fn deny_run(&self, _run_id: &RunId, _reason: Option<String>) -> anyhow::Result<Run> {
        pin_unimplemented!()
    }
    async fn cancel_run(&self, _run_id: &RunId) -> anyhow::Result<Run> {
        pin_unimplemented!()
    }
    async fn interrupt_run(&self, _run_id: &RunId) -> anyhow::Result<()> {
        pin_unimplemented!()
    }
    async fn steer_run(
        &self,
        _run_id: &RunId,
        _text: String,
        _interrupt: bool,
    ) -> anyhow::Result<()> {
        pin_unimplemented!()
    }
    async fn archive_run(&self, _run_id: &RunId) -> anyhow::Result<Run> {
        pin_unimplemented!()
    }
    async fn unarchive_run(&self, _run_id: &RunId) -> anyhow::Result<Run> {
        pin_unimplemented!()
    }
    async fn list_store_runs(&self) -> anyhow::Result<Vec<Run>> {
        pin_unimplemented!()
    }
    async fn list_store_runs_by_parent(&self, _parent_id: RunId) -> anyhow::Result<Vec<Run>> {
        pin_unimplemented!()
    }
    async fn link_run_parent(&self, _child_id: &RunId, _parent_id: &RunId) -> anyhow::Result<Run> {
        pin_unimplemented!()
    }
    async fn unlink_run_parent(&self, _child_id: &RunId) -> anyhow::Result<Run> {
        pin_unimplemented!()
    }
    async fn get_run_state(&self, _run_id: &RunId) -> anyhow::Result<RunProjection> {
        pin_unimplemented!()
    }
    async fn list_run_stream(
        &self,
        _run_id: &RunId,
        _after: u64,
        _limit: Option<usize>,
    ) -> anyhow::Result<Vec<RunStreamItem>> {
        pin_unimplemented!()
    }
    async fn list_run_questions(&self, _run_id: &RunId) -> anyhow::Result<Vec<ApiQuestion>> {
        pin_unimplemented!()
    }
    async fn submit_run_answer(
        &self,
        _run_id: &RunId,
        _question_id: &str,
        _body: SubmitAnswerRequest,
    ) -> anyhow::Result<()> {
        pin_unimplemented!()
    }
}

#[test]
fn named_registry_resolves_run_wait_for_stage_sessions() {
    let services = FabroRunToolServices {
        backend:        Arc::new(PinBackend),
        current_run_id: RunId::new(),
    };
    let tools = register_named_fabro_run_tools(&services, &[fabro_tool::FABRO_RUN_WAIT_TOOL_NAME]);
    assert_eq!(
        tools.len(),
        1,
        "fabro_run_wait must resolve through the named run-tools registry (fabro-8795)"
    );
    assert_eq!(
        tools[0].definition().name,
        fabro_tool::FABRO_RUN_WAIT_TOOL_NAME
    );
}

#[test]
fn run_wait_wire_schema_advertises_the_wait_contract() {
    let definition = tool_definitions()
        .iter()
        .find(|definition| definition.name == fabro_tool::FABRO_RUN_WAIT_TOOL_NAME)
        .expect("fabro_run_wait definition present");
    let properties = definition
        .parameters
        .get("properties")
        .expect("schema properties");
    for field in ["run_id", "until", "timeout_ms"] {
        assert!(
            properties.get(field).is_some(),
            "wait schema must advertise {field}"
        );
    }
}

#[test]
fn run_wait_validation_rejects_blank_run_id_and_zero_timeout() {
    let params = FabroRunWaitParams {
        run_id:     "  ".to_owned(),
        until:      RunWaitUntil::Terminal,
        timeout_ms: 60_000,
    };
    let error = ValidatedRunWait::try_from(params).expect_err("blank run ids must be rejected");
    assert!(
        error.to_string().contains("run_id is required"),
        "error names the field: {error}"
    );

    let params = FabroRunWaitParams {
        run_id:     "01M353ABHN6PMTWJQDW4W2GQYN".to_owned(),
        until:      RunWaitUntil::Merged,
        timeout_ms: 0,
    };
    let error = ValidatedRunWait::try_from(params).expect_err("zero timeouts must be rejected");
    assert!(
        error.to_string().contains("timeout_ms"),
        "error names the field: {error}"
    );
}

#[test]
fn pr_create_failures_classify_retryability_for_the_bounded_loop() {
    // Compile-presence of the retry loop's only public entry (the bounded
    // retry helper is private to pull_request.rs and called from here).
    let _ = open_pull_request;

    assert!(
        CreatePullRequestError::Transport(anyhow::anyhow!("timed out")).is_retryable(),
        "transport failures retry (fabro-b5a9/fabro-67e5)"
    );
    assert!(
        CreatePullRequestError::Status {
            status: 502,
            body:   "bad gateway".to_owned(),
        }
        .is_retryable(),
        "5xx answers retry"
    );
    assert!(
        !CreatePullRequestError::Status {
            status: 422,
            body:   "validation failed".to_owned(),
        }
        .is_retryable(),
        "deterministic 4xx answers must not retry"
    );
    assert!(
        !CreatePullRequestError::Token(anyhow::anyhow!("mint failed")).is_retryable(),
        "token minting failures must not retry"
    );
}
