//! Fork pins (fabro-43cf): the named run-tools registry must resolve
//! `fabro_ask` for stage sessions, and must NOT resolve the retired
//! `.seeds/` tracker tool `fabro_seeds_search` (work#1533).
//!
//! The revisor graphs declare `x.fabro_tools="fabro_ask"`; if the name
//! stops resolving, the registry skips it with only a warning and the
//! workflow loses the tool on deploy. These pins live in a fork-only test
//! file so an upstream merge cannot drop them together with the feature.

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

/// The retired `.seeds/` tracker tool (work#1533) must NOT resolve through
/// the named registry anymore: the work-item doors replaced it, and a
/// graph that still names the tool must lose it loudly (the registry's
/// unknown-name warning), not carry a dead door past the cutover.
#[test]
fn named_registry_does_not_resolve_the_retired_seeds_search_tool() {
    let services = FabroRunToolServices {
        backend:        Arc::new(PinBackend),
        current_run_id: RunId::new(),
        stage:          None,
    };
    let tools = register_named_fabro_run_tools(&services, &["fabro_seeds_search"]);
    assert!(
        tools.is_empty(),
        "the retired fabro_seeds_search tool must not resolve anymore (work#1533), got {:?}",
        tools
            .iter()
            .map(|tool| tool.definition().name.clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn named_registry_resolves_fabro_ask_for_stage_sessions() {
    let services = FabroRunToolServices {
        backend:        Arc::new(PinBackend),
        current_run_id: RunId::new(),
        stage:          None,
    };
    let tools = register_named_fabro_run_tools(&services, &[fabro_tool::FABRO_ASK_TOOL_NAME]);
    assert_eq!(
        tools.len(),
        1,
        "fabro_ask must resolve through the named run-tools registry (fabro-43cf)"
    );
    assert_eq!(tools[0].definition().name, fabro_tool::FABRO_ASK_TOOL_NAME);
}
