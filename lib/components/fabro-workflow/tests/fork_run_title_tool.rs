//! Fork presence pin (fabro-9711/ADR-0030): the named run-tools registry
//! must resolve `fabro_run_set_title` for stage sessions. The develop,
//! loop and revisor planner graphs pin the tool in `x.fabro_tools`; if
//! the name stops resolving, the registry skips it with only a warning
//! and every lane loses agent-refined run titles on the next deploy
//! (the fabro-43cf silent-loss class). Fork-only file: an upstream merge
//! cannot drop it together with the feature.

use std::sync::Arc;

use async_trait::async_trait;
use fabro_api::types::{ApiQuestion, SubmitAnswerRequest};
use fabro_tool::FabroToolBackend;
use fabro_types::{Run, RunId, RunProjection, RunStreamItem};
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
fn named_registry_resolves_the_run_set_title_tool() {
    let services = FabroRunToolServices {
        backend:        Arc::new(PinBackend),
        current_run_id: RunId::new(),
    };
    let tools =
        register_named_fabro_run_tools(&services, &[fabro_tool::FABRO_RUN_SET_TITLE_TOOL_NAME]);
    assert_eq!(
        tools.len(),
        1,
        "fabro_run_set_title must resolve through the named run-tools registry (fabro-9711)"
    );
    assert_eq!(
        tools[0].definition().name,
        fabro_tool::FABRO_RUN_SET_TITLE_TOOL_NAME
    );
}
