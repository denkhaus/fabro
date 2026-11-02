//! Fork presence pin (ADR-0039 stage D part 2, denkhaus/work#1107): the
//! named run-tools registry must resolve the adjudicator's four READ
//! twins — `fabro_run_queue`, `fabro_observation_list`,
//! `fabro_work_radius`, `fabro_work_comments` — and their wire schemas
//! must carry the parameters the CLI facades take. Door symmetry
//! (decision 12): every orchestration surface exists as BOTH agent tool
//! and CLI facade over the same endpoint, so a graph granting a read
//! door through `x.fabro_tools` must find the name in the catalog. If a
//! name stops resolving, the registry skips it with only a warning and
//! the adjudicator's agent pass silently loses the read on the next
//! deploy (the fabro-43cf silent-loss class). Fork-only file: an
//! upstream merge cannot drop it together with the feature.

use std::sync::Arc;

use fabro_tool::fabro_client::ClientBackend;
use fabro_tool::{
    FABRO_OBSERVATION_LIST_TOOL_NAME, FABRO_RUN_QUEUE_TOOL_NAME, FABRO_WORK_COMMENTS_TOOL_NAME,
    FABRO_WORK_RADIUS_TOOL_NAME, tool_definitions,
};
use fabro_types::RunId;
use fabro_workflow::run_tools::register_named_fabro_run_tools;
use fabro_workflow::services::FabroRunToolServices;

/// The four read twins, in catalog order. Every one is granted by the
/// adjudicator graph's `x.fabro_tools`.
const ADJUDICATOR_READ_TOOLS: [&str; 4] = [
    FABRO_RUN_QUEUE_TOOL_NAME,
    FABRO_OBSERVATION_LIST_TOOL_NAME,
    FABRO_WORK_RADIUS_TOOL_NAME,
    FABRO_WORK_COMMENTS_TOOL_NAME,
];

fn services() -> FabroRunToolServices {
    // A registry-resolution pin never dispatches: a client that cannot
    // connect is enough to build the real backend.
    let client = fabro_client::Client::new_no_proxy("http://127.0.0.1:1").expect("test client");
    FabroRunToolServices {
        backend:        Arc::new(ClientBackend::new(Arc::new(client))),
        current_run_id: RunId::new(),
        stage:          None,
    }
}

#[test]
fn the_named_registry_resolves_every_adjudicator_read_twin() {
    let tools = register_named_fabro_run_tools(&services(), &ADJUDICATOR_READ_TOOLS);
    assert_eq!(
        tools.len(),
        ADJUDICATOR_READ_TOOLS.len(),
        "every adjudicator read twin must resolve through the named run-tools registry \
         (ADR-0039 stage D part 2, door symmetry)"
    );
    for name in ADJUDICATOR_READ_TOOLS {
        assert!(
            tools.iter().any(|tool| tool.definition().name == name),
            "{name} must resolve through the named run-tools registry"
        );
    }
}

/// The wire schemas are the contract the model calls: the queue takes
/// the field plus the optional workflow filter, the observation read
/// takes the run, and both work-item reads take the item key spelled
/// `itemKey` like every other work tool.
#[test]
fn the_wire_schema_carries_each_read_doors_parameters() {
    let definitions = tool_definitions();
    let find = |name: &str| {
        definitions
            .iter()
            .find(|definition| definition.name == name)
            .unwrap_or_else(|| panic!("{name} definition present"))
    };

    let queue = &find(FABRO_RUN_QUEUE_TOOL_NAME).parameters;
    assert!(
        queue.pointer("/properties/field").is_some(),
        "the queue read is keyed by orchestration field: {queue}"
    );
    assert!(
        queue.pointer("/properties/workflow").is_some(),
        "the queue read carries the workflow filter: {queue}"
    );
    for field in ["merge", "closable"] {
        assert!(
            queue.to_string().contains(field),
            "the {field} field is offered: {queue}"
        );
    }

    let observations = &find(FABRO_OBSERVATION_LIST_TOOL_NAME).parameters;
    assert!(
        observations.pointer("/properties/run").is_some(),
        "the observation read names its run: {observations}"
    );

    for name in [FABRO_WORK_RADIUS_TOOL_NAME, FABRO_WORK_COMMENTS_TOOL_NAME] {
        let parameters = &find(name).parameters;
        assert!(
            parameters.pointer("/properties/itemKey").is_some(),
            "{name} takes the provider-stable item key like every work tool: {parameters}"
        );
    }
}
