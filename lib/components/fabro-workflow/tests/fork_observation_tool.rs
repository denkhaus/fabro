//! Fork presence pin (ADR-0038): the observation recorder must resolve
//! through the named run-tools registry for stage sessions, advertise
//! its two categories, and validate its text like every teaching
//! refusal. Graphs enable the tool per node with
//! `x.fabro_tools="fabro_observation_record"`; if the name stops
//! resolving, the registry skips it with only a warning and every
//! `x.observations="required"` node fails its presence contract on the
//! next deploy (the fabro-43cf silent-loss class). Fork-only file: an
//! upstream merge cannot drop it together with the feature.

use std::sync::Arc;

use async_trait::async_trait;
use fabro_api::types::{ApiQuestion, SubmitAnswerRequest};
use fabro_tool::{
    FABRO_OBSERVATION_RECORD_TOOL_NAME, FabroObservationCategory, FabroObservationRecordParams,
    FabroToolBackend, ValidatedObservation, tool_definitions,
};
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
fn named_registry_resolves_the_observation_record_tool() {
    let services = FabroRunToolServices {
        backend:        Arc::new(PinBackend),
        current_run_id: RunId::new(),
        stage:          None,
    };
    let tools = register_named_fabro_run_tools(&services, &[FABRO_OBSERVATION_RECORD_TOOL_NAME]);
    assert_eq!(
        tools.len(),
        1,
        "fabro_observation_record must resolve through the named run-tools registry (ADR-0038)"
    );
    assert_eq!(
        tools[0].definition().name,
        FABRO_OBSERVATION_RECORD_TOOL_NAME
    );
}

#[test]
fn observation_wire_schema_advertises_the_two_categories() {
    let definition = tool_definitions()
        .iter()
        .find(|definition| definition.name == FABRO_OBSERVATION_RECORD_TOOL_NAME)
        .expect("fabro_observation_record definition present");
    let rendered = definition.parameters.to_string();
    assert!(
        rendered.contains("painpoint") && rendered.contains("note"),
        "the schema names both categories however schemars renders them: {rendered}"
    );
    assert!(
        !rendered.contains("\"Painpoint\"") && !rendered.contains("\"Note\""),
        "the categories are snake_case on the wire (serde and schema must agree): {rendered}"
    );
    assert!(
        definition.parameters.pointer("/properties/text").is_some(),
        "the schema advertises the text field"
    );
}

#[test]
fn observation_validation_teaches_the_empty_entry_and_the_cap() {
    let blank = ValidatedObservation::try_from(FabroObservationRecordParams {
        category: FabroObservationCategory::Note,
        text:     "   ".to_owned(),
    })
    .expect_err("a blank entry must fail");
    assert!(
        blank.to_string().contains("nothing durable to record"),
        "the refusal teaches the explicit empty entry: {blank}"
    );

    let long = ValidatedObservation::try_from(FabroObservationRecordParams {
        category: FabroObservationCategory::Painpoint,
        text:     "x".repeat(2400),
    })
    .expect_err("an over-long entry must fail");
    let message = long.to_string();
    assert!(
        message.contains("2000-character limit") && message.contains("2400 characters"),
        "the refusal names limit and length so one trim is enough: {message}"
    );

    let note = ValidatedObservation::try_from(FabroObservationRecordParams {
        category: FabroObservationCategory::Note,
        text:     "nothing durable to record".to_owned(),
    })
    .expect("the explicit empty entry validates");
    assert_eq!(
        note.category,
        fabro_types::ObservationCategory::Note,
        "the empty entry is a note"
    );
}
