//! The Fabro run tools (`fabro_run_*`) as application tools a pebble coding
//! agent can call.

use std::sync::Arc;

use fabro_types::RunId;
use pebble_coding_agent::tools::{RegisteredTool, ToolError, ToolSource};
use serde::de::DeserializeOwned;

use crate::services::FabroRunToolServices;

/// Every Fabro run tool, bound to `services`.
#[must_use]
pub fn register_fabro_run_tools(services: &FabroRunToolServices) -> Vec<RegisteredTool> {
    fabro_tool::tool_definitions()
        .iter()
        .map(|definition| fabro_run_tool(definition, services.clone()))
        .collect()
}

/// Only the Fabro run tools whose names appear in `names`.
///
/// Unknown names are silently ignored so callers can list every tool they
/// care about without depending on the current `fabro_tool` catalog.
#[must_use]
pub fn register_named_fabro_run_tools(
    services: &FabroRunToolServices,
    names: &[&str],
) -> Vec<RegisteredTool> {
    fabro_tool::tool_definitions()
        .iter()
        .filter(|definition| names.contains(&definition.name))
        .map(|definition| fabro_run_tool(definition, services.clone()))
        .collect()
}

fn fabro_run_tool(
    definition: &fabro_tool::ToolDefinition,
    services: FabroRunToolServices,
) -> RegisteredTool {
    let name = definition.name.to_string();
    let services = Arc::new(services);
    RegisteredTool::function(
        name.clone(),
        definition.description.to_string(),
        definition.parameters.clone(),
        move |_context, arguments| {
            let name = name.clone();
            let services = Arc::clone(&services);
            async move {
                execute_fabro_run_tool(&name, arguments, &services)
                    .await
                    .map_err(|error| ToolError::execution(error.to_string()))
            }
        },
    )
    .with_source(ToolSource::Application)
    // A subagent spawned by a workflow stage does the same work under the
    // same run, so it keeps the same view of the run tree.
    .allow_in_subagents()
}

pub(crate) async fn execute_fabro_run_tool(
    name: &str,
    args: serde_json::Value,
    services: &FabroRunToolServices,
) -> fabro_tool::ToolResult<String> {
    match name {
        fabro_tool::FABRO_RUN_CREATE_TOOL_NAME => {
            let params = parse_fabro_tool_args::<fabro_tool::FabroRunCreateParams>(name, args)?;
            ensure_current_run_parent(&params, services.current_run_id)?;
            let validated = fabro_tool::ValidatedCreateRuns::try_from(params)?;
            let result = fabro_tool::create_runs_with_options(
                Arc::clone(&services.backend),
                &services.base_cwd,
                &services.user_settings_path,
                validated,
                fabro_tool::CreateRunOptions {
                    forced_parent_id: Some(services.current_run_id),
                },
            )
            .await?;
            let summary = fabro_tool::create_runs_text(&result);
            render_fabro_tool_result(&summary, &result)
        }
        fabro_tool::FABRO_RUN_SEARCH_TOOL_NAME => {
            let params = parse_fabro_tool_args::<fabro_tool::FabroRunSearchParams>(name, args)?;
            let result = fabro_tool::search_runs(
                Arc::clone(&services.backend),
                fabro_tool::ValidatedSearchRuns::try_from(params)?,
            )
            .await?;
            let summary = fabro_tool::search_runs_text(&result);
            render_fabro_tool_result(&summary, &result)
        }
        fabro_tool::FABRO_RUN_GET_TOOL_NAME => {
            let params = parse_fabro_tool_args::<fabro_tool::FabroRunGetParams>(name, args)?;
            let result = fabro_tool::run_get(
                Arc::clone(&services.backend),
                fabro_tool::ValidatedRunGet::try_from(params)?,
            )
            .await?;
            let summary = fabro_tool::run_get_text(&result);
            render_fabro_tool_result(&summary, &result)
        }
        fabro_tool::FABRO_RUN_INTERACT_TOOL_NAME => {
            let params = parse_fabro_tool_args::<fabro_tool::FabroRunInteractParams>(name, args)?;
            let validated = fabro_tool::ValidatedInteractRun::try_from(params)?;
            if validated.action.requires_user() {
                return Err(fabro_tool::ToolError::message(
                    "Run approval must be performed by a user through the API, CLI, web UI, or human MCP server.",
                ));
            }
            let result = fabro_tool::interact_run(Arc::clone(&services.backend), validated).await?;
            let summary = fabro_tool::interact_run_text(&result);
            render_fabro_tool_result(&summary, &result)
        }
        fabro_tool::FABRO_RUN_GATHER_TOOL_NAME => {
            let params = parse_fabro_tool_args::<fabro_tool::FabroRunGatherParams>(name, args)?;
            let result = fabro_tool::gather_runs(
                Arc::clone(&services.backend),
                fabro_tool::ValidatedGatherRuns::try_from(params)?,
            )
            .await?;
            let summary = fabro_tool::gather_runs_text(&result);
            render_fabro_tool_result(&summary, &result)
        }
        fabro_tool::FABRO_RUN_EVENTS_TOOL_NAME => {
            let params = parse_fabro_tool_args::<fabro_tool::FabroRunEventsParams>(name, args)?;
            let result = fabro_tool::run_events(
                Arc::clone(&services.backend),
                fabro_tool::ValidatedRunEvents::try_from(params)?,
            )
            .await?;
            let summary = fabro_tool::run_events_text(&result);
            render_fabro_tool_result(&summary, &result)
        }
        fabro_tool::FABRO_RUN_PAIR_TOOL_NAME => {
            let params = parse_fabro_tool_args::<fabro_tool::FabroRunPairParams>(name, args)?;
            let result = fabro_tool::pair_run(
                Arc::clone(&services.backend),
                fabro_tool::ValidatedPairRun::try_from(params)?,
            )
            .await?;
            let summary = fabro_tool::pair_run_text(&result);
            render_fabro_tool_result(&summary, &result)
        }
        _ => Err(fabro_tool::ToolError::message(format!(
            "unknown Fabro run tool `{name}`"
        ))),
    }
}

fn parse_fabro_tool_args<T>(name: &str, args: serde_json::Value) -> fabro_tool::ToolResult<T>
where
    T: DeserializeOwned,
{
    serde_json::from_value(args)
        .map_err(|err| fabro_tool::ToolError::message(format!("invalid {name} arguments: {err}")))
}

fn ensure_current_run_parent(
    params: &fabro_tool::FabroRunCreateParams,
    current_run_id: RunId,
) -> fabro_tool::ToolResult<()> {
    let current_parent = current_run_id.to_string();
    for run in &params.runs {
        let parent_id = match run {
            fabro_tool::CreateRunSpecInput::Workflow(_) => None,
            fabro_tool::CreateRunSpecInput::Spec(spec) => spec.parent_id.as_deref().map(str::trim),
        };
        match parent_id {
            None => {}
            Some("") => {
                return Err(fabro_tool::ToolError::message(
                    "parent_id must be omitted or match the current run; blank parent_id is invalid",
                ));
            }
            Some(parent_id) if parent_id == current_parent => {}
            Some(parent_id) => {
                return Err(fabro_tool::ToolError::message(format!(
                    "parent_id must be omitted or match the current run {current_parent}; got {parent_id}"
                )));
            }
        }
    }
    Ok(())
}

fn render_fabro_tool_result<T>(summary: &str, result: &T) -> fabro_tool::ToolResult<String>
where
    T: serde::Serialize,
{
    let json = serde_json::to_string_pretty(result).map_err(|err| {
        fabro_tool::ToolError::message(format!("failed to serialize tool result: {err}"))
    })?;
    Ok(format!("{summary}\n{json}"))
}
