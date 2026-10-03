//! Fork presence pin (fabro-43cf): the `fabro_ask` stage tool must stay in
//! the rebuild run-tool catalog.
//!
//! Upstream merges cannot conflict away or silently drop a fork-only test
//! file: if the ask tool leaves the catalog (the fabro-43cf migration
//! hazard — `register_named_fabro_run_tools` skips unknown names), this
//! test reds instead of the revisor losing its core instrument quietly.

use fabro_tool::{FABRO_ASK_TOOL_NAME, tool_definitions};

#[test]
fn catalog_carries_the_fabro_ask_stage_tool() {
    let definition = tool_definitions()
        .iter()
        .find(|definition| definition.name == FABRO_ASK_TOOL_NAME)
        .expect(
            "fabro_ask must be in the run-tool catalog: a rebuild deploy without it silently \
             strips the revisor's ask capability (fabro-43cf)",
        );
    assert!(
        definition.description.contains("Ask-Fabro analyst"),
        "the catalog description must say what the tool does"
    );
    assert!(definition.parameters["properties"]["run_id"].is_object());
    assert!(definition.parameters["properties"]["question"].is_object());
}
