use std::any::{TypeId, type_name};

use fabro_api::types::{
    AgentToolsAvailableProps as ApiAgentToolsAvailableProps,
    ContextWindowBreakdownItem as ApiContextWindowBreakdownItem,
    ContextWindowCategory as ApiContextWindowCategory,
    ContextWindowCountMethod as ApiContextWindowCountMethod,
    ContextWindowSnapshot as ApiContextWindowSnapshot,
    ContextWindowStaleness as ApiContextWindowStaleness,
    ContextWindowWarning as ApiContextWindowWarning, LlmOutputKind as ApiLlmOutputKind,
    ModelUsage as ApiModelUsage, ParallelBranchResult as ApiParallelBranchResult,
    PermissionLevel as ApiPermissionLevel, SkillActivationSource as ApiSkillActivationSource,
    SkillSummary as ApiSkillSummary, StageContextWindow as ApiStageContextWindow,
    StageContextWindowUnavailableReason as ApiStageContextWindowUnavailableReason,
    StageInferenceProjection as ApiStageInferenceProjection, StageProjection as ApiStageProjection,
    StageToolBatchProjection as ApiStageToolBatchProjection,
    TodoListProjection as ApiTodoListProjection, ToolCategory as ApiToolCategory,
    ToolSource as ApiToolSource, ToolSummary as ApiToolSummary,
};
use fabro_types::{
    AgentToolsAvailableProps, ContextWindowBreakdownItem, ContextWindowCategory,
    ContextWindowCountMethod, ContextWindowSnapshot, ContextWindowStaleness, ContextWindowWarning,
    LlmOutputKind, ModelRef, ModelUsage, ParallelBranchId, ParallelBranchResult, PermissionLevel,
    SkillActivationSource, SkillSummary, StageContextWindow, StageContextWindowUnavailableReason,
    StageId, StageInferenceProjection, StageProjection, StageToolBatchProjection, TodoListKind,
    TodoListProjection, ToolCategory, ToolSource, ToolSummary,
};
use lithos_llm::catalog::{ModelId, ProviderId};
use lithos_llm::types::{Cost, CostSource, TokenCounts, Usage};
use serde_json::json;

#[test]
fn stage_projection_reuses_canonical_type() {
    assert_same_type::<ApiStageProjection, StageProjection>();
    assert_same_type::<ApiModelUsage, ModelUsage>();
}

#[test]
fn usage_by_model_rows_match_openapi_json_shape() {
    let row = ModelUsage::new(
        ModelRef::new(ProviderId::new("openai"), ModelId::new("gpt-5.4")),
        Usage {
            tokens: TokenCounts {
                input: 107,
                output: 51,
                ..TokenCounts::default()
            },
            cost:   Some(Cost {
                usd_micros: 321,
                source:     CostSource::Catalog,
            }),
        },
    );
    let value = serde_json::to_value(&row).unwrap();
    assert_eq!(
        value,
        json!({
            "model": { "provider": "openai", "model_id": "gpt-5.4" },
            "usage": {
                "tokens": {
                    "input": 107,
                    "output": 51,
                    "reasoning": 0,
                    "cache_read": 0,
                    "cache_write": 0
                },
                "cost": { "usd_micros": 321, "source": "catalog" }
            }
        })
    );
    let api_row: ApiModelUsage = serde_json::from_value(value).unwrap();
    assert_eq!(api_row, row);

    let mut stage = StageProjection::new(std::num::NonZeroU32::new(1).unwrap());
    stage.usage_by_model = vec![row.clone()];
    let stage_json = serde_json::to_value(&stage).unwrap();
    assert_eq!(
        stage_json["usage_by_model"],
        json!([serde_json::to_value(&row).unwrap()])
    );
    let without: StageProjection = serde_json::from_value(json!({
        "first_event_seq": 1,
        "prompt": null,
        "response": null,
        "completion": null,
        "provider_used": null,
        "diff": null,
        "script_invocation": null,
        "script_timing": null,
        "parallel_results": null,
        "output": null,
        "usage": {
            "tokens": {
                "input": 0,
                "output": 0,
                "reasoning": 0,
                "cache_read": 0,
                "cache_write": 0
            }
        },
        "state": "running"
    }))
    .unwrap();
    assert!(without.usage_by_model.is_empty());
    assert!(
        serde_json::to_value(&without)
            .unwrap()
            .get("usage_by_model")
            .is_none(),
        "no rows, nothing on the wire"
    );
}

#[test]
fn stage_projection_reuses_nested_agent_state_types() {
    assert_same_type::<ApiParallelBranchResult, ParallelBranchResult>();
    assert_same_type::<ApiTodoListProjection, TodoListProjection>();
    assert_same_type::<ApiSkillSummary, SkillSummary>();
    assert_same_type::<ApiSkillActivationSource, SkillActivationSource>();
    assert_same_type::<ApiToolSummary, ToolSummary>();
    assert_same_type::<ApiToolSource, ToolSource>();
    assert_same_type::<ApiToolCategory, ToolCategory>();
    assert_same_type::<ApiAgentToolsAvailableProps, AgentToolsAvailableProps>();
    assert_same_type::<ApiPermissionLevel, PermissionLevel>();
    assert_same_type::<ApiStageContextWindow, StageContextWindow>();
    assert_same_type::<ApiContextWindowSnapshot, ContextWindowSnapshot>();
    assert_same_type::<ApiContextWindowBreakdownItem, ContextWindowBreakdownItem>();
    assert_same_type::<ApiContextWindowCategory, ContextWindowCategory>();
    assert_same_type::<ApiContextWindowCountMethod, ContextWindowCountMethod>();
    assert_same_type::<ApiContextWindowStaleness, ContextWindowStaleness>();
    assert_same_type::<ApiStageContextWindowUnavailableReason, StageContextWindowUnavailableReason>(
    );
    assert_same_type::<ApiContextWindowWarning, ContextWindowWarning>();
    assert_same_type::<ApiStageInferenceProjection, StageInferenceProjection>();
    assert_same_type::<ApiStageToolBatchProjection, StageToolBatchProjection>();
    assert_same_type::<ApiLlmOutputKind, LlmOutputKind>();
}

#[test]
fn stage_tool_batch_projection_matches_openapi_json_shape() {
    let batch = StageToolBatchProjection {
        session_id:    "ses_root".to_string(),
        started_at:    "2026-04-29T12:34:00Z".parse().unwrap(),
        open_call_ids: ["call_1".to_string(), "call_2".to_string()]
            .into_iter()
            .collect(),
    };
    let value = serde_json::to_value(&batch).unwrap();
    assert_eq!(
        value,
        json!({
            "session_id": "ses_root",
            "started_at": "2026-04-29T12:34:00Z",
            "open_call_ids": ["call_1", "call_2"]
        })
    );
    let api_batch: ApiStageToolBatchProjection = serde_json::from_value(value).unwrap();
    assert_eq!(api_batch, batch);
}

#[test]
fn stage_inference_projection_matches_openapi_json_shape() {
    let inference = StageInferenceProjection {
        session_id:        "ses_root".to_string(),
        started_at:        "2026-04-29T12:34:00Z".parse().unwrap(),
        requested_model:   "claude-fable-5".to_string(),
        first_output_at:   Some("2026-04-29T12:34:07Z".parse().unwrap()),
        first_output_kind: Some(LlmOutputKind::Reasoning),
        retries:           1,
    };
    let value = serde_json::to_value(&inference).unwrap();
    assert_eq!(
        value,
        json!({
            "session_id": "ses_root",
            "started_at": "2026-04-29T12:34:00Z",
            "requested_model": "claude-fable-5",
            "first_output_at": "2026-04-29T12:34:07Z",
            "first_output_kind": "reasoning",
            "retries": 1
        })
    );
    let api_inference: ApiStageInferenceProjection = serde_json::from_value(value).unwrap();
    assert_eq!(api_inference, inference);
}

#[test]
fn llm_enums_match_openapi_json_shape() {
    for (kind, wire) in [
        (LlmOutputKind::Reasoning, "reasoning"),
        (LlmOutputKind::Text, "text"),
        (LlmOutputKind::ToolCall, "tool_call"),
    ] {
        let value = serde_json::to_value(kind).unwrap();
        assert_eq!(value, json!(wire));
        let api_kind: ApiLlmOutputKind = serde_json::from_value(value).unwrap();
        assert_eq!(api_kind, kind);
    }
}

/// A stage projection written before `inference` existed must still
/// deserialize, and must not gain a phantom open bracket.
#[test]
fn stage_projection_without_inference_round_trips() {
    let value = json!({
        "first_event_seq": 1,
        "prompt": null,
        "response": null,
        "completion": null,
        "provider_used": null,
        "diff": null,
        "script_invocation": null,
        "script_timing": null,
        "parallel_results": null,
        "output": null,
        "usage": {
            "tokens": {
                "input": 0,
                "output": 0,
                "reasoning": 0,
                "cache_read": 0,
                "cache_write": 0
            }
        },
        "state": "running"
    });

    let stage: StageProjection = serde_json::from_value(value.clone()).unwrap();
    assert!(stage.inference.is_none());
    assert!(stage.acp_started_at.is_none());
    assert!(stage.tool_batch.is_none());
    assert_eq!(stage.live_inference_ms, 0);
    assert_eq!(stage.live_tool_ms, 0);
    assert_eq!(serde_json::to_value(stage).unwrap(), value);
}

#[test]
fn stage_projection_round_trips_representative_json() {
    let value = json!({
        "first_event_seq": 1,
        "prompt": "build it",
        "response": "done",
        "completion": {
            "outcome": "succeeded",
            "notes": null,
            "failure_reason": null,
            "timestamp": "2026-04-29T12:34:56Z"
        },
        "provider_used": {
            "mode": "prompt",
            "provider": "openai",
            "model": "gpt-5.2",
            "reasoning_effort": "high",
            "speed": "fast"
        },
        "diff": "diff --git a/file b/file",
        "script_invocation": { "command": "cargo test" },
        "script_timing": { "duration_ms": 42 },
        "parallel_results": [
            {
                "id": "review_api",
                "index": 0,
                "item_label": "api",
                "status": "succeeded",
                "context_updates": {
                    "response.review_api": "looks good",
                    "score": 0.95
                }
            },
            {
                "id": "review_ux",
                "index": 1,
                "status": "failed",
                "context_updates": {}
            }
        ],
        "parallel_branch_id": "review_fork@3:1",
        "output": "ok",
        "termination": "exited",
        "started_at": "2026-04-29T12:34:00Z",
        "timing": {
            "wall_time_ms": 56000,
            "inference_time_ms": 0,
            "tool_time_ms": 0,
            "active_time_ms": 0
        },
        "usage": {
            "tokens": {
                "input": 0,
                "output": 0,
                "reasoning": 0,
                "cache_read": 0,
                "cache_write": 0
            }
        },
        "permission_level": "read-only",
        "agent_tools": [
            {
                "name": "apply_patch",
                "description": "Apply a unified diff patch",
                "source": { "kind": "native" },
                "category": "write",
                "invoked": true
            },
            {
                "name": "mcp__filesystem__read_file",
                "description": "Read a file through MCP",
                "source": {
                    "kind": "mcp",
                    "server_name": "filesystem",
                    "original_name": "read_file"
                },
                "category": "other",
                "invoked": false
            }
        ],
        "inference": {
            "session_id": "ses_root",
            "started_at": "2026-04-29T12:34:00Z",
            "requested_model": "claude-fable-5",
            "first_output_at": "2026-04-29T12:34:07Z",
            "first_output_kind": "text",
            "retries": 0
        },
        "acp_started_at": "2026-04-29T12:34:00Z",
        "state": "succeeded"
    });

    let state: StageProjection = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(
        state.parallel_branch_id,
        Some(ParallelBranchId::new(StageId::new("review_fork", 3), 1))
    );
    assert_eq!(serde_json::to_value(state).unwrap(), value);
}

#[test]
fn stage_context_window_response_round_trips_representative_json() {
    let value = json!({
        "stage_id": "implement@1",
        "available": true,
        "unavailable_reason": null,
        "provider": "openai",
        "model": "gpt-5.4",
        "context_window_tokens": 400000,
        "input_tokens": 123456,
        "usage_percent": 30.864,
        "count_method": "provider_api_scaled_breakdown",
        "staleness": "live",
        "generated_at": "2026-05-23T12:34:56Z",
        "event_seq": 42,
        "breakdown": [
            {
                "category": "system_prompt",
                "tokens": 30000,
                "usage_percent": 7.5
            }
        ],
        "warnings": [
            {
                "code": "local_token_estimate",
                "message": "input token count is a local estimate"
            }
        ]
    });

    let response: StageContextWindow = serde_json::from_value(value.clone()).unwrap();
    let api_response: ApiStageContextWindow = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(api_response, response);
    assert_eq!(serde_json::to_value(response).unwrap(), value);
}

#[test]
fn permission_level_matches_openapi_json_shape() {
    let permission_json = serde_json::to_value(PermissionLevel::ReadOnly).unwrap();
    assert_eq!(permission_json, json!("read-only"));
    let api_permission: ApiPermissionLevel = serde_json::from_value(permission_json).unwrap();
    assert_eq!(api_permission, PermissionLevel::ReadOnly);
}

#[test]
fn todo_list_and_skill_types_match_openapi_json_shape() {
    for (kind, list_id, wire_kind) in [
        (
            TodoListKind::OpenAiPlan,
            "openai_plan:ses_root",
            "openai_plan",
        ),
        (TodoListKind::KimiTodos, "kimi_todos:ses_root", "kimi_todos"),
    ] {
        let todo_list = TodoListProjection::new(kind, list_id);
        let todo_json = serde_json::to_value(&todo_list).unwrap();
        assert_eq!(
            todo_json,
            json!({
                "kind": wire_kind,
                "list_id": list_id,
                "items": []
            })
        );
        let api_todo_list: ApiTodoListProjection = serde_json::from_value(todo_json).unwrap();
        assert_eq!(api_todo_list, todo_list);
    }

    let skill = SkillSummary {
        name:        "rust".to_string(),
        description: "Rust workflow help".to_string(),
    };
    let skill_json = serde_json::to_value(&skill).unwrap();
    assert_eq!(
        skill_json,
        json!({
            "name": "rust",
            "description": "Rust workflow help"
        })
    );
    let api_skill: ApiSkillSummary = serde_json::from_value(skill_json).unwrap();
    assert_eq!(api_skill, skill);

    let source_json = serde_json::to_value(SkillActivationSource::Slash).unwrap();
    assert_eq!(source_json, json!("slash"));
    let api_source: ApiSkillActivationSource = serde_json::from_value(source_json).unwrap();
    assert_eq!(api_source, SkillActivationSource::Slash);
}

#[test]
fn agent_tool_summary_matches_openapi_json_shape_without_parameter_schema() {
    let tool = ToolSummary {
        name:        "mcp__filesystem__read_file".to_string(),
        description: "Read a file through MCP".to_string(),
        source:      ToolSource::Mcp {
            server_name:   "filesystem".to_string(),
            original_name: "read_file".to_string(),
        },
        category:    ToolCategory::Other,
        invoked:     false,
    };

    let tool_json = serde_json::to_value(&tool).unwrap();
    assert_eq!(
        tool_json,
        json!({
            "name": "mcp__filesystem__read_file",
            "description": "Read a file through MCP",
            "source": {
                "kind": "mcp",
                "server_name": "filesystem",
                "original_name": "read_file"
            },
            "category": "other",
            "invoked": false
        })
    );
    assert!(tool_json.as_object().unwrap().get("parameters").is_none());
    let api_tool: ApiToolSummary = serde_json::from_value(tool_json).unwrap();
    assert_eq!(api_tool, tool);

    let props = AgentToolsAvailableProps {
        tools: vec![tool],
        visit: 2,
    };
    let props_json = serde_json::to_value(&props).unwrap();
    let api_props: ApiAgentToolsAvailableProps = serde_json::from_value(props_json).unwrap();
    assert_eq!(api_props, props);
}

fn assert_same_type<T: 'static, U: 'static>() {
    assert_eq!(
        TypeId::of::<T>(),
        TypeId::of::<U>(),
        "{} should be the same type as {}",
        type_name::<T>(),
        type_name::<U>()
    );
}
