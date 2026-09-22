//! The Fabro workflow and run tools as application tools a pebble coding
//! agent can call.

use std::sync::Arc;

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
/// Unknown names are logged as a warning and then ignored (fabro-43cf):
/// a graph requesting a catalog-missing tool loses that capability on
/// deploy, and silent loss must not recur — the workflow's `x.fabro_tools`
/// declaration should name tools that resolve.
#[must_use]
pub fn register_named_fabro_run_tools(
    services: &FabroRunToolServices,
    names: &[&str],
) -> Vec<RegisteredTool> {
    let definitions = fabro_tool::tool_definitions();
    for missing in partition_unknown_tool_names(names, definitions) {
        tracing::warn!(
            tool = missing,
            known_tools = definitions.len(),
            current_run_id = %services.current_run_id,
            "requested run tool is not in the fabro tool catalog"
        );
    }
    definitions
        .iter()
        .filter(|definition| names.contains(&definition.name))
        .map(|definition| fabro_run_tool(definition, services.clone()))
        .collect()
}

/// The requested names that no catalog definition carries, in request
/// order. Extracted so the silent-loss guard is testable.
fn partition_unknown_tool_names<'a>(
    names: &'a [&'a str],
    definitions: &[fabro_tool::ToolDefinition],
) -> Vec<&'a str> {
    names
        .iter()
        .filter(|name| {
            !definitions
                .iter()
                .any(|definition| definition.name == **name)
        })
        .copied()
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
        fabro_tool::FABRO_WORKFLOW_VERSION_CREATE_TOOL_NAME => {
            let params =
                parse_fabro_tool_args::<fabro_tool::FabroWorkflowVersionCreateParams>(name, args)?;
            let source = fabro_tool::ValidatedWorkflowVersionCreate::try_from(params)?;
            let result =
                fabro_tool::create_workflow_version(Arc::clone(&services.backend), source).await?;
            let summary = fabro_tool::workflow_version_create_text(&result);
            render_fabro_tool_result(&summary, &result)
        }
        fabro_tool::FABRO_RUN_CREATE_TOOL_NAME => {
            let params = parse_fabro_tool_args::<fabro_tool::FabroRunCreateParams>(name, args)?;
            let result = fabro_tool::create_runs_with_options(
                Arc::clone(&services.backend),
                params,
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
        fabro_tool::FABRO_RUN_WAIT_TOOL_NAME => {
            let params = parse_fabro_tool_args::<fabro_tool::FabroRunWaitParams>(name, args)?;
            let result =
                fabro_tool::run_wait(Arc::clone(&services.backend), params.try_into()?).await?;
            let summary = fabro_tool::run_wait_text(&result);
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
        fabro_tool::FABRO_ASK_TOOL_NAME => {
            let params = parse_fabro_tool_args::<fabro_tool::FabroAskParams>(name, args)?;
            let result =
                fabro_tool::ask_run(Arc::clone(&services.backend), params.try_into()?).await?;
            let summary = fabro_tool::ask_run_text(&result);
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

fn render_fabro_tool_result<T>(summary: &str, result: &T) -> fabro_tool::ToolResult<String>
where
    T: serde::Serialize,
{
    let json = serde_json::to_string_pretty(result).map_err(|err| {
        fabro_tool::ToolError::message(format!("failed to serialize tool result: {err}"))
    })?;
    Ok(format!("{summary}\n{json}"))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use async_trait::async_trait;
    use fabro_tool::fabro_client::ClientBackend;
    use fabro_tool::{ValidatedWorkflowVersionCreate, WorkflowVersionPackager};
    use fabro_types::{RunId, WorkflowVersion};
    use fabro_workflow_version::{CollectedWorkflowClosure, ValidatedWorkflowVersion};
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn native_run_create_submits_intent_and_enforces_current_parent() {
        let server = httpmock::MockServer::start_async().await;
        let parent_id = fabro_types::RunId::new();
        let version_id: fabro_types::WorkflowVersionId =
            fabro_types::BlobHash::new(b"registered workflow").into();
        let create = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST)
                    .path("/api/v1/runs")
                    .json_body(json!({
                        "workflow_version_id": version_id,
                        "target": {"kind":"none"},
                        "parent_id": parent_id,
                        "args": {"auto_approve":false}
                    }));
                // Admission rejection proves the native dispatcher reached the
                // canonical API without registering or looking up a workflow.
                then.status(422).body("native admission rejection");
            })
            .await;
        let state = server
            .mock_async(|when, then| {
                when.path(format!("/api/v1/runs/{parent_id}/state"));
                then.status(500);
            })
            .await;
        // Fork seam (fabro-8ee1): the duplicate-child guard lists the
        // parent's children before creating — none here.
        server
            .mock_async(|when, then| {
                when.method("GET")
                    .path("/api/v1/runs")
                    .query_param("parent_id", parent_id.to_string());
                then.status(200).json_body(json!({
                    "data": [],
                    "meta": { "total": 0, "has_more": false }
                }));
            })
            .await;
        let client = fabro_client::Client::new_no_proxy(&server.url("")).unwrap();
        let services = FabroRunToolServices {
            backend:        Arc::new(ClientBackend::new(Arc::new(client))),
            current_run_id: parent_id,
        };
        let name = fabro_tool::FABRO_RUN_CREATE_TOOL_NAME;
        let mut args = json!({"runs":[{
            "workflow_version_id":version_id,
            "target":{"kind":"none"},
            "args":{"auto_approve":false}
        }]});
        let error = execute_fabro_run_tool(name, args.clone(), &services)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("native admission rejection"));
        args["runs"][0]["parent_id"] = json!(fabro_types::RunId::new());
        let error = execute_fabro_run_tool(name, args, &services)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("match the current run"));
        create.assert_calls_async(1).await;
        state.assert_calls_async(0).await;
    }

    struct SingleGraphPackager;

    #[async_trait]
    impl WorkflowVersionPackager for SingleGraphPackager {
        async fn package(
            &self,
            source: ValidatedWorkflowVersionCreate,
        ) -> anyhow::Result<CollectedWorkflowClosure> {
            let version = WorkflowVersion::new(source.entrypoint, source.files, BTreeMap::new())?;
            let id = version.id()?;
            Ok(CollectedWorkflowClosure::from_dependency_order(id, vec![(
                id,
                ValidatedWorkflowVersion::new(version)?,
            )]))
        }
    }

    #[tokio::test]
    async fn workflow_version_native_dispatch_registers_and_returns_version() {
        let server = httpmock::MockServer::start_async().await;
        let version = WorkflowVersion::new(
            "workflow".parse().unwrap(),
            BTreeMap::from([("workflow".parse().unwrap(), "digraph W {}".into())]),
            BTreeMap::new(),
        )
        .unwrap();
        let id = version.id().unwrap();
        let upload = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST)
                    .path("/api/v1/workflow-versions")
                    .json_body_obj(&version);
                then.status(201)
                    .json_body(json!({"workflow_version_id": id}));
            })
            .await;
        let client = fabro_client::Client::new_no_proxy(&server.url("")).unwrap();
        let services = FabroRunToolServices {
            backend:        Arc::new(
                ClientBackend::new(Arc::new(client))
                    .with_workflow_version_packager(Arc::new(SingleGraphPackager)),
            ),
            current_run_id: "01KRBZW4DW0000000000000002".parse().unwrap(),
        };
        let name = fabro_tool::FABRO_WORKFLOW_VERSION_CREATE_TOOL_NAME;
        assert_eq!(register_named_fabro_run_tools(&services, &[name]).len(), 1);
        let output = execute_fabro_run_tool(
            name,
            json!({"entrypoint":"workflow", "files":{"workflow":"digraph W {}"}}),
            &services,
        )
        .await
        .unwrap();
        let (summary, body) = output.split_once('\n').unwrap();
        assert_eq!(summary, format!("Registered workflow version {id}"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(body).unwrap(),
            json!({"workflow_version_id": id})
        );
        let error = execute_fabro_run_tool(
            name,
            json!({"entrypoint":"missing", "files":{"workflow":"digraph W {}"}}),
            &services,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("not present"));
        upload.assert_calls_async(1).await;
    }

    struct MockAskBackend {
        run_id: RunId,
    }

    fn ask_target_run(run_id: RunId) -> fabro_types::Run {
        use chrono::TimeZone;
        fabro_types::Run {
            id:               run_id,
            parent_id:        None,
            children_count:   0,
            title:            "Reviewed run".to_string(),
            goal:             "Reviewed run".to_string(),
            workflow:         fabro_types::WorkflowRef {
                slug:       Some("develop".to_string()),
                name:       Some("Develop".to_string()),
                graph_name: None,
                node_count: 0,
                edge_count: 0,
            },
            automation:       None,
            repository:       None,
            created_by:       fabro_types::test_support::test_principal(),
            origin:           fabro_types::RunOrigin::default(),
            labels:           std::collections::HashMap::new(),
            lifecycle:        fabro_types::RunLifecycle {
                conclusion_failure: None,
                status:             fabro_types::RunStatus::Pending {
                    reason: fabro_types::PendingReason::ApprovalRequired,
                },
                approval:           None,
                pending_control:    None,
                queue_position:     None,
                error:              None,
                archived:           false,
                archived_at:        None,
            },
            sandbox:          None,
            models:           Vec::new(),
            source_directory: None,
            timestamps:       fabro_types::RunTimestamps {
                created_at:    chrono::Utc.with_ymd_and_hms(2026, 9, 22, 8, 0, 0).unwrap(),
                started_at:    None,
                last_event_at: None,
                completed_at:  None,
            },
            timing:           None,
            usage:            fabro_api::types::Usage::default(),
            size:             fabro_types::RunSize::default(),
            ask_fabro:        fabro_types::AskFabro::default(),
            diff:             None,
            pull_request:     None,
            current_question: None,
            superseded_by:    None,
            retried_from:     None,
            links:            fabro_types::RunLinks { web: None },
        }
    }

    #[async_trait::async_trait]
    impl fabro_tool::FabroToolBackend for MockAskBackend {
        async fn create_run_from_intent(
            &self,
            _intent: fabro_types::RunIntent,
        ) -> anyhow::Result<RunId> {
            unreachable!("the ask test never creates runs")
        }
        async fn resolve_run(&self, selector: &str) -> anyhow::Result<fabro_types::Run> {
            assert_eq!(selector, self.run_id.to_string());
            Ok(ask_target_run(self.run_id))
        }
        async fn retrieve_run(&self, _run_id: &RunId) -> anyhow::Result<fabro_types::Run> {
            unreachable!("the ask test never retrieves runs directly")
        }
        async fn start_run(
            &self,
            _run_id: &RunId,
            _resume: bool,
        ) -> anyhow::Result<fabro_types::Run> {
            unreachable!("the ask test never starts runs")
        }
        async fn approve_run(&self, _run_id: &RunId) -> anyhow::Result<fabro_types::Run> {
            unreachable!("the ask test never approves runs")
        }
        async fn deny_run(
            &self,
            _run_id: &RunId,
            _reason: Option<String>,
        ) -> anyhow::Result<fabro_types::Run> {
            unreachable!("the ask test never denies runs")
        }
        async fn cancel_run(&self, _run_id: &RunId) -> anyhow::Result<fabro_types::Run> {
            unreachable!("the ask test never cancels runs")
        }
        async fn interrupt_run(&self, _run_id: &RunId) -> anyhow::Result<()> {
            unreachable!("the ask test never interrupts runs")
        }
        async fn steer_run(
            &self,
            _run_id: &RunId,
            _text: String,
            _interrupt: bool,
        ) -> anyhow::Result<()> {
            unreachable!("the ask test never steers runs")
        }
        async fn archive_run(&self, _run_id: &RunId) -> anyhow::Result<fabro_types::Run> {
            unreachable!("the ask test never archives runs")
        }
        async fn unarchive_run(&self, _run_id: &RunId) -> anyhow::Result<fabro_types::Run> {
            unreachable!("the ask test never unarchives runs")
        }
        async fn list_store_runs(&self) -> anyhow::Result<Vec<fabro_types::Run>> {
            unreachable!("the ask test never lists runs")
        }
        async fn list_store_runs_by_parent(
            &self,
            _parent_id: RunId,
        ) -> anyhow::Result<Vec<fabro_types::Run>> {
            unreachable!("the ask test never lists child runs")
        }
        async fn link_run_parent(
            &self,
            _child_id: &RunId,
            _parent_id: &RunId,
        ) -> anyhow::Result<fabro_types::Run> {
            unreachable!("the ask test never links runs")
        }
        async fn unlink_run_parent(&self, _child_id: &RunId) -> anyhow::Result<fabro_types::Run> {
            unreachable!("the ask test never unlinks runs")
        }
        async fn get_run_state(
            &self,
            _run_id: &RunId,
        ) -> anyhow::Result<fabro_types::RunProjection> {
            unreachable!("the ask test never reads run state")
        }
        async fn list_run_stream(
            &self,
            _run_id: &RunId,
            _after: u64,
            _limit: Option<usize>,
        ) -> anyhow::Result<Vec<fabro_types::RunStreamItem>> {
            unreachable!("the ask test never reads the run stream")
        }
        async fn list_run_questions(
            &self,
            _run_id: &RunId,
        ) -> anyhow::Result<Vec<fabro_api::types::ApiQuestion>> {
            unreachable!("the ask test never reads questions")
        }
        async fn submit_run_answer(
            &self,
            _run_id: &RunId,
            _question_id: &str,
            _body: fabro_api::types::SubmitAnswerRequest,
        ) -> anyhow::Result<()> {
            unreachable!("the ask test never answers questions")
        }

        async fn create_ask_session(&self, run_id: &RunId, title: &str) -> anyhow::Result<String> {
            assert_eq!(*run_id, self.run_id);
            assert_eq!(title, "why did the reviewer fail?");
            Ok("01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string())
        }

        async fn submit_ask_turn(
            &self,
            run_id: &RunId,
            session_id: &str,
            question: &str,
        ) -> anyhow::Result<fabro_tool::AskTurnOutcome> {
            assert_eq!(*run_id, self.run_id);
            assert_eq!(session_id, "01ARZ3NDEKTSV4RRFFQ69G5FAV");
            assert_eq!(question, "why did the reviewer fail?");
            Ok(fabro_tool::AskTurnOutcome {
                status: fabro_tool::AskTurnStatus::Succeeded,
                answer: "the gate timed out".to_string(),
                error:  None,
            })
        }
    }

    #[tokio::test]
    async fn ask_tool_resolves_the_target_and_returns_the_analyst_answer() {
        let run_id: RunId = "01KRBZW4DW0000000000000003".parse().unwrap();
        let services = FabroRunToolServices {
            backend:        Arc::new(MockAskBackend { run_id }),
            current_run_id: RunId::new(),
        };
        let output = execute_fabro_run_tool(
            fabro_tool::FABRO_ASK_TOOL_NAME,
            json!({
                "run_id": run_id.to_string(),
                "question": "  why did the reviewer fail?  "
            }),
            &services,
        )
        .await
        .unwrap();
        let (summary, body) = output.split_once('\n').unwrap();
        assert_eq!(summary, format!("asked Fabro run {run_id}"));
        let result: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(result["status"], json!("succeeded"));
        assert_eq!(result["answer"], json!("the gate timed out"));
        assert_eq!(result["session_id"], json!("01ARZ3NDEKTSV4RRFFQ69G5FAV"));
    }

    #[tokio::test]
    async fn ask_tool_reports_a_failed_analyst_turn_as_an_error() {
        struct MockFailingAskBackend {
            run_id: RunId,
        }

        #[async_trait::async_trait]
        impl fabro_tool::FabroToolBackend for MockFailingAskBackend {
            async fn create_run_from_intent(
                &self,
                _intent: fabro_types::RunIntent,
            ) -> anyhow::Result<RunId> {
                unreachable!()
            }
            async fn resolve_run(&self, _selector: &str) -> anyhow::Result<fabro_types::Run> {
                Ok(ask_target_run(self.run_id))
            }
            async fn retrieve_run(&self, _run_id: &RunId) -> anyhow::Result<fabro_types::Run> {
                unreachable!()
            }
            async fn start_run(
                &self,
                _run_id: &RunId,
                _resume: bool,
            ) -> anyhow::Result<fabro_types::Run> {
                unreachable!()
            }
            async fn approve_run(&self, _run_id: &RunId) -> anyhow::Result<fabro_types::Run> {
                unreachable!()
            }
            async fn deny_run(
                &self,
                _run_id: &RunId,
                _reason: Option<String>,
            ) -> anyhow::Result<fabro_types::Run> {
                unreachable!()
            }
            async fn cancel_run(&self, _run_id: &RunId) -> anyhow::Result<fabro_types::Run> {
                unreachable!()
            }
            async fn interrupt_run(&self, _run_id: &RunId) -> anyhow::Result<()> {
                unreachable!()
            }
            async fn steer_run(
                &self,
                _run_id: &RunId,
                _text: String,
                _interrupt: bool,
            ) -> anyhow::Result<()> {
                unreachable!()
            }
            async fn archive_run(&self, _run_id: &RunId) -> anyhow::Result<fabro_types::Run> {
                unreachable!()
            }
            async fn unarchive_run(&self, _run_id: &RunId) -> anyhow::Result<fabro_types::Run> {
                unreachable!()
            }
            async fn list_store_runs(&self) -> anyhow::Result<Vec<fabro_types::Run>> {
                unreachable!()
            }
            async fn list_store_runs_by_parent(
                &self,
                _parent_id: RunId,
            ) -> anyhow::Result<Vec<fabro_types::Run>> {
                unreachable!()
            }
            async fn link_run_parent(
                &self,
                _child_id: &RunId,
                _parent_id: &RunId,
            ) -> anyhow::Result<fabro_types::Run> {
                unreachable!()
            }
            async fn unlink_run_parent(
                &self,
                _child_id: &RunId,
            ) -> anyhow::Result<fabro_types::Run> {
                unreachable!()
            }
            async fn get_run_state(
                &self,
                _run_id: &RunId,
            ) -> anyhow::Result<fabro_types::RunProjection> {
                unreachable!()
            }
            async fn list_run_stream(
                &self,
                _run_id: &RunId,
                _after: u64,
                _limit: Option<usize>,
            ) -> anyhow::Result<Vec<fabro_types::RunStreamItem>> {
                unreachable!()
            }
            async fn list_run_questions(
                &self,
                _run_id: &RunId,
            ) -> anyhow::Result<Vec<fabro_api::types::ApiQuestion>> {
                unreachable!()
            }
            async fn submit_run_answer(
                &self,
                _run_id: &RunId,
                _question_id: &str,
                _body: fabro_api::types::SubmitAnswerRequest,
            ) -> anyhow::Result<()> {
                unreachable!()
            }
            async fn create_ask_session(
                &self,
                _run_id: &RunId,
                _title: &str,
            ) -> anyhow::Result<String> {
                Ok("01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string())
            }
            async fn submit_ask_turn(
                &self,
                _run_id: &RunId,
                _session_id: &str,
                _question: &str,
            ) -> anyhow::Result<fabro_tool::AskTurnOutcome> {
                Ok(fabro_tool::AskTurnOutcome {
                    status: fabro_tool::AskTurnStatus::Failed,
                    answer: String::new(),
                    error:  Some("llm unavailable".to_string()),
                })
            }
        }

        let run_id: RunId = "01KRBZW4DW0000000000000004".parse().unwrap();
        let services = FabroRunToolServices {
            backend:        Arc::new(MockFailingAskBackend { run_id }),
            current_run_id: RunId::new(),
        };
        let error = execute_fabro_run_tool(
            fabro_tool::FABRO_ASK_TOOL_NAME,
            json!({"run_id": run_id.to_string(), "question": "why?"}),
            &services,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("llm unavailable"), "{error}");
    }

    #[test]
    fn partition_flags_requested_names_missing_from_the_catalog() {
        let definitions = fabro_tool::tool_definitions();
        assert!(
            partition_unknown_tool_names(&[fabro_tool::FABRO_ASK_TOOL_NAME], definitions,)
                .is_empty()
        );

        // `fabro_runs_list` is the pre-rebuild revisor graphs' name; the
        // rebuild catalog renamed it to `fabro_run_search`, so the old
        // declaration is exactly the silent-loss case the warning guards.
        let missing = partition_unknown_tool_names(
            &["fabro_runs_list", fabro_tool::FABRO_ASK_TOOL_NAME],
            definitions,
        );
        assert_eq!(missing, vec!["fabro_runs_list"]);
    }
}
