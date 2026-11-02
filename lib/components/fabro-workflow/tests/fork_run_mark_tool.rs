//! Fork presence pin (ADR-0039 stage D, denkhaus/work#1107): the named
//! run-tools registry must resolve `fabro_run_mark` for stage sessions,
//! and the dispatch must reach the backend with the explicit foreign
//! target, the field, and the verdict. The adjudicator graph grants the
//! tool through `x.fabro_tools`; if the name stops resolving, the
//! registry skips it with only a warning and the adjudicator's agent
//! pass loses its verdict write on the next deploy (the fabro-43cf
//! silent-loss class). Fork-only file: an upstream merge cannot drop it
//! together with the feature.

use std::sync::Arc;

use async_trait::async_trait;
use fabro_api::types::{ApiQuestion, SubmitAnswerRequest};
use fabro_tool::{FABRO_RUN_MARK_TOOL_NAME, FabroToolBackend, tool_definitions};
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
fn named_registry_resolves_the_run_mark_tool() {
    let services = FabroRunToolServices {
        backend:        Arc::new(PinBackend),
        current_run_id: RunId::new(),
        stage:          None,
    };
    let tools = register_named_fabro_run_tools(&services, &[FABRO_RUN_MARK_TOOL_NAME]);
    assert_eq!(
        tools.len(),
        1,
        "fabro_run_mark must resolve through the named run-tools registry (ADR-0039 stage D)"
    );
    assert_eq!(tools[0].definition().name, FABRO_RUN_MARK_TOOL_NAME);
}

/// The wire schema carries both fields and all three writable verdicts:
/// `pending` is absent by design (it is the absence of a record), and
/// the enums render snake_case like serde puts them on the wire.
#[test]
fn the_wire_schema_knows_both_fields_and_no_pending() {
    let definition = tool_definitions()
        .iter()
        .find(|definition| definition.name == FABRO_RUN_MARK_TOOL_NAME)
        .expect("fabro_run_mark definition present");
    let rendered = definition.parameters.to_string();
    assert!(
        rendered.contains("merge"),
        "the merge field is offered: {rendered}"
    );
    assert!(
        rendered.contains("closable"),
        "the closable field is offered: {rendered}"
    );
    assert!(
        !rendered.contains("\"pending\"") && !rendered.contains("\"Pending\""),
        "pending is not a writable verdict: {rendered}"
    );
    for verdict in ["parked", "enabled", "refused"] {
        assert!(
            rendered.contains(verdict),
            "the {verdict} verdict is offered"
        );
    }
    assert!(
        definition.parameters.pointer("/properties/run").is_some(),
        "the explicit target is part of the contract: {rendered}"
    );
}

/// The validation refuses a blank target with the teaching message
/// before any request leaves (the tool twin's explicitness arm).
#[test]
fn validation_refuses_a_blank_target_with_the_teaching_message() {
    let error = fabro_tool::ValidatedRunMark::try_from(fabro_tool::FabroRunMarkParams {
        run:     "   ".to_string(),
        field:   fabro_tool::FabroOrchestrationField::Merge,
        verdict: fabro_tool::FabroOrchestrationVerdict::Enabled,
    })
    .expect_err("a blank target must fail");
    assert!(
        error
            .to_string()
            .contains("the verdict is about another run"),
        "the refusal teaches why the target is explicit: {error}"
    );
}
