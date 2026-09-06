//! Static tool-name catalogs for graph validation (fabro-47b5).
//!
//! These lists intentionally live here as plain data: `fabro-validate`
//! must not depend on `fabro-agent` or `fabro-tool` just to lint name
//! lists. Drift is caught by
//! `tools_attribute_known::static_catalogs_match_live_crates`, which
//! cross-checks the lists against the live crates through
//! dev-dependencies.

/// Canonical names of fabro-agent's native tools.
pub(crate) const KNOWN_NATIVE_TOOL_NAMES: &[&str] = &[
    "AskUserQuestion",
    "TaskCreate",
    "TaskGet",
    "TaskList",
    "TaskUpdate",
    "TodoList",
    "agent_output",
    "apply_patch",
    "background_agent",
    "close_agent",
    "edit_file",
    "glob",
    "grep",
    "list_dir",
    "message_agent",
    "read_file",
    "read_many_files",
    "request_user_input",
    "send_input",
    "shell",
    "spawn_agent",
    "stop_agent",
    "update_plan",
    "use_skill",
    "wait",
    "web_fetch",
    "web_search",
    "write_file",
];

/// Canonical names of stage tools the workflow engine registers beyond
/// fabro-agent's native set (fabro-e804). These are advertiseable in a
/// node `tools` allowlist like any native tool.
pub(crate) const KNOWN_WORKFLOW_TOOL_NAMES: &[&str] = &["context_read"];

/// Names of the static `fabro_run_*` tool catalog.
pub(crate) const KNOWN_FABRO_RUN_TOOL_NAMES: &[&str] = &[
    "fabro_ask",
    "fabro_run_create",
    "fabro_run_events",
    "fabro_run_gather",
    "fabro_run_get",
    "fabro_run_interact",
    "fabro_run_logs",
    "fabro_run_pair",
    "fabro_run_search",
    "fabro_run_wait",
    "fabro_runs_list",
];

/// Whether `name` is a known tool for the given list family. MCP names
/// (`mcp__`-prefixed) are always known: their catalog depends on run
/// configuration, not on the graph.
pub(crate) fn is_known_tool_name(name: &str, fabro_run_list: bool) -> bool {
    if name.starts_with("mcp__") {
        return true;
    }
    if fabro_run_list {
        KNOWN_FABRO_RUN_TOOL_NAMES.contains(&name)
    } else {
        KNOWN_NATIVE_TOOL_NAMES.contains(&name) || KNOWN_WORKFLOW_TOOL_NAMES.contains(&name)
    }
}
