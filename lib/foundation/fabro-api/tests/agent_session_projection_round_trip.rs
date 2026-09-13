//! `StageProjection.agent` is pebble's `SessionProjection`, reused verbatim
//! along with every type nested in it. These tests prove the API types are
//! pebble's own and that the OpenAPI schemas describe pebble's serde shape:
//! a populated projection validates against `AgentSessionProjection`, every
//! key it serializes is declared, and every enum variant this build knows is
//! in the spec. A pebble re-pin that adds a field or a variant fails here
//! until the spec is updated.

use std::any::{TypeId, type_name};
use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::time::{Duration, SystemTime};

use fabro_api::types::{
    AgentErrorData as ApiAgentErrorData, AgentErrorKind as ApiAgentErrorKind,
    AgentSessionActivatedSkill as ApiAgentSessionActivatedSkill,
    AgentSessionActivity as ApiAgentSessionActivity,
    AgentSessionCompaction as ApiAgentSessionCompaction,
    AgentSessionDescendantAccount as ApiAgentSessionDescendantAccount,
    AgentSessionFailoverStop as ApiAgentSessionFailoverStop,
    AgentSessionMcpServer as ApiAgentSessionMcpServer,
    AgentSessionProjection as ApiAgentSessionProjection,
    AgentSessionPromptDelta as ApiAgentSessionPromptDelta,
    AgentSessionRoute as ApiAgentSessionRoute,
    AgentSessionRouteFailover as ApiAgentSessionRouteFailover,
    AgentSessionSkills as ApiAgentSessionSkills, AgentSessionSubagent as ApiAgentSessionSubagent,
    AgentSessionSubagentCounts as ApiAgentSessionSubagentCounts,
    AgentSessionSubagentStatus as ApiAgentSessionSubagentStatus,
    AgentSessionToolActivity as ApiAgentSessionToolActivity,
    CompactionReason as ApiCompactionReason, FailoverContinuation as ApiFailoverContinuation,
    FailoverStop as ApiFailoverStop, LlmErrorKind as ApiLlmErrorKind,
    LlmRetryClassification as ApiLlmRetryClassification, McpToolSummary as ApiMcpToolSummary,
    StageProjection as ApiStageProjection, TokenUsage as ApiTokenUsage,
};
use fabro_types::StageProjection;
use lithos_llm::types::{ErrorKind as LlmErrorKind, RetryClassification};
use pebble_coding_agent::events::{
    CodingAgentEvent, CodingEvent, CompactionReason, ContextWindowCountMethod,
    ContextWindowSnapshot, ContextWindowStaleness, ErrorData, ErrorKind, FailoverContinuation,
    FailoverStop, InputSource, LlmRetryPhase, McpToolSummary, SkillActivationSource, SkillSummary,
    TodoCreatedProps, TodoListKind, TodoStatus, TokenUsage,
};
use pebble_coding_agent::projection::{
    ActivatedSkill, CompactionProjection, DescendantAccount, FailoverStopProjection,
    McpServerProjection, PromptDelta, RouteFailoverProjection, RouteProjection, SessionActivity,
    SessionProjection, SkillsProjection, SubagentCounts, SubagentProjection, SubagentStatus,
    ToolActivity,
};
use pebble_coding_agent::tools::ToolOutputMetadata;
use serde_json::{Value, json};

#[test]
fn agent_session_projection_reuses_pebbles_types() {
    assert_same_type::<ApiAgentSessionProjection, SessionProjection>();
    assert_same_type::<ApiAgentSessionActivity, SessionActivity>();
    assert_same_type::<ApiAgentSessionRoute, RouteProjection>();
    assert_same_type::<ApiAgentSessionDescendantAccount, DescendantAccount>();
    assert_same_type::<ApiAgentSessionToolActivity, ToolActivity>();
    assert_same_type::<ApiAgentSessionSubagentCounts, SubagentCounts>();
    assert_same_type::<ApiAgentSessionMcpServer, McpServerProjection>();
    assert_same_type::<ApiAgentSessionSkills, SkillsProjection>();
    assert_same_type::<ApiAgentSessionActivatedSkill, ActivatedSkill>();
    assert_same_type::<ApiAgentSessionSubagent, SubagentProjection>();
    assert_same_type::<ApiAgentSessionSubagentStatus, SubagentStatus>();
    assert_same_type::<ApiAgentSessionCompaction, CompactionProjection>();
    assert_same_type::<ApiAgentSessionRouteFailover, RouteFailoverProjection>();
    assert_same_type::<ApiAgentSessionFailoverStop, FailoverStopProjection>();
    assert_same_type::<ApiAgentSessionPromptDelta, PromptDelta>();
    assert_same_type::<ApiTokenUsage, TokenUsage>();
    assert_same_type::<ApiMcpToolSummary, McpToolSummary>();
    assert_same_type::<ApiCompactionReason, CompactionReason>();
    assert_same_type::<ApiFailoverContinuation, FailoverContinuation>();
    assert_same_type::<ApiFailoverStop, FailoverStop>();
    assert_same_type::<ApiAgentErrorData, ErrorData>();
    assert_same_type::<ApiAgentErrorKind, ErrorKind>();
    assert_same_type::<ApiLlmErrorKind, LlmErrorKind>();
    assert_same_type::<ApiLlmRetryClassification, RetryClassification>();
}

#[test]
fn a_populated_projection_matches_its_openapi_schema() {
    let projection = scripted_projection();
    // The script reached every section of the projection.
    assert_eq!(projection.root_session_id.as_deref(), Some("ses_root"));
    assert_eq!(projection.activity, SessionActivity::Idle);
    assert_eq!(projection.route.model.as_deref(), Some("claude-fable-5"));
    assert_eq!(projection.prompts, 1);
    assert!(projection.prompt.completed);
    assert_eq!(projection.messages, 2);
    assert_eq!(projection.retries, 1);
    assert_eq!(projection.descendants.len(), 1);
    assert_eq!(projection.tools.len(), 2);
    assert_eq!(projection.mcp_servers.len(), 2);
    assert_eq!(projection.skills.activated.len(), 1);
    assert_eq!(projection.todos.len(), 1);
    assert_eq!(projection.subagents.len(), 2);
    assert_eq!(projection.compactions.len(), 1);
    assert_eq!(projection.failovers.len(), 1);
    assert!(projection.failover_stopped.is_some());
    assert_eq!(projection.files_touched, ["/workspace/src/lib.rs"]);
    assert!(projection.context_window.is_some());

    let value = serde_json::to_value(&projection).unwrap();
    assert!(
        value.get("pending_writes").is_none(),
        "a settled projection keeps its bookkeeping off the wire: {value}"
    );
    assert_valid("AgentSessionProjection", &value);
    assert_declared(
        &spec(),
        &json!({ "$ref": "#/components/schemas/AgentSessionProjection" }),
        &value,
        "agent",
    );

    let api: ApiAgentSessionProjection = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(api, projection);
    assert_eq!(serde_json::to_value(&api).unwrap(), value);
}

#[test]
fn a_projection_taken_mid_write_declares_its_open_write() {
    let mut projection = scripted_projection();
    projection.apply(&root(CodingEvent::ToolCallStarted {
        tool_name:    "write_file".to_string(),
        tool_call_id: "call_open".to_string(),
        arguments:    json!({"file_path": "/workspace/README.md", "content": "x"}),
    }));
    let value = serde_json::to_value(&projection).unwrap();
    assert_eq!(
        value["pending_writes"]["call_open"],
        json!(["/workspace/README.md"])
    );
    assert_valid("AgentSessionProjection", &value);
    assert_declared(
        &spec(),
        &json!({ "$ref": "#/components/schemas/AgentSessionProjection" }),
        &value,
        "agent",
    );
}

#[test]
fn a_stage_projection_carrying_the_fold_matches_its_openapi_schema() {
    let mut stage = StageProjection::new(NonZeroU32::new(1).unwrap());
    stage.agent = Some(scripted_projection());
    let value = serde_json::to_value(&stage).unwrap();
    assert!(value["agent"].is_object());
    assert_valid("StageProjection", &value);
    assert_declared(
        &spec(),
        &json!({ "$ref": "#/components/schemas/StageProjection" }),
        &value,
        "stage",
    );
    let api: ApiStageProjection = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(&api).unwrap(), value);

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
            "input_tokens": 0,
            "output_tokens": 0,
            "total_tokens": 0,
            "reasoning_tokens": 0,
            "cache_read_tokens": 0,
            "cache_write_tokens": 0
        },
        "state": "running"
    }))
    .unwrap();
    assert!(
        without.agent.is_none(),
        "a stage written before the fold existed has none"
    );
    assert!(
        serde_json::to_value(&without)
            .unwrap()
            .get("agent")
            .is_none(),
        "a stage without a fold serializes none"
    );
}

#[test]
fn every_enum_variant_this_build_knows_is_in_the_spec() {
    for activity in [
        SessionActivity::Idle,
        SessionActivity::Running,
        SessionActivity::WaitingForSteer,
        SessionActivity::Ended,
    ] {
        assert_valid(
            "AgentSessionActivity",
            &serde_json::to_value(activity).unwrap(),
        );
    }
    for reason in [
        CompactionReason::Threshold,
        CompactionReason::Manual,
        CompactionReason::Overflow,
    ] {
        assert_valid("CompactionReason", &serde_json::to_value(reason).unwrap());
    }
    for continuation in [
        FailoverContinuation::ReplayPrompt,
        FailoverContinuation::ContinueTurn,
    ] {
        assert_valid(
            "FailoverContinuation",
            &serde_json::to_value(continuation).unwrap(),
        );
    }
    for stop in [FailoverStop::Ineligible, FailoverStop::Exhausted] {
        assert_valid("FailoverStop", &serde_json::to_value(stop).unwrap());
    }
    for kind in [
        ErrorKind::Llm,
        ErrorKind::Compaction,
        ErrorKind::Agent,
        ErrorKind::InvalidInput,
        ErrorKind::SessionClosed,
        ErrorKind::InvalidState,
        ErrorKind::ToolExecution,
        ErrorKind::Interrupted,
        ErrorKind::ToolRoundsExhausted,
        ErrorKind::Task,
        ErrorKind::EventStream,
    ] {
        assert_valid("AgentErrorKind", &serde_json::to_value(kind).unwrap());
    }
    for retry in [
        RetryClassification::Never,
        RetryClassification::Safe,
        RetryClassification::after(Duration::from_millis(1500)),
    ] {
        let value = serde_json::to_value(retry).unwrap();
        assert_valid("LlmRetryClassification", &value);
        assert_declared(
            &spec(),
            &json!({ "$ref": "#/components/schemas/LlmRetryClassification" }),
            &value,
            "retry",
        );
    }
    for kind in [
        LlmErrorKind::RateLimit,
        LlmErrorKind::ContextLength,
        LlmErrorKind::Unknown("later".to_string()),
    ] {
        assert_valid("LlmErrorKind", &serde_json::to_value(kind).unwrap());
    }
    for status in [
        SubagentStatus::Running,
        SubagentStatus::Completed {
            success:    true,
            turns_used: 3,
        },
        SubagentStatus::Failed { error: llm_error() },
        SubagentStatus::Closed,
    ] {
        let value = serde_json::to_value(&status).unwrap();
        assert_valid("AgentSessionSubagentStatus", &value);
        assert_declared(
            &spec(),
            &json!({ "$ref": "#/components/schemas/AgentSessionSubagentStatus" }),
            &value,
            "status",
        );
    }
}

// --- The scripted session -------------------------------------------------

fn scripted_projection() -> SessionProjection {
    let mut projection = SessionProjection::new();
    projection.apply_all(&scripted_events());
    projection
}

/// One prompt that touches every section: MCP servers up and down, skills,
/// a todo, a completed and a failed child, a retry, a failover with a
/// compaction on the new route, and a stop.
fn scripted_events() -> Vec<CodingAgentEvent> {
    vec![
        root(CodingEvent::SessionStarted {
            provider: Some("openai".to_string()),
            model:    Some("gpt-5.2".to_string()),
        }),
        root(CodingEvent::McpServerReady {
            server:     "github".to_string(),
            tools:      vec![McpToolSummary {
                name:          "mcp__github__list_issues".to_string(),
                original_name: "list_issues".to_string(),
            }],
            startup_ms: 842,
        }),
        root(CodingEvent::McpServerFailed {
            server:     "broken".to_string(),
            error:      "could not launch".to_string(),
            startup_ms: 3,
        }),
        root(CodingEvent::SkillsDiscovered {
            profile:     "anthropic".to_string(),
            source_dirs: Vec::new(),
            skills:      vec![SkillSummary {
                name:        "rust".to_string(),
                description: "Rust workflow help".to_string(),
            }],
            skipped:     Vec::new(),
        }),
        root(CodingEvent::UserInput {
            text:    "build it".to_string(),
            content: None,
            source:  InputSource::Prompt,
        }),
        root(message("openai", "gpt-5.2", 100, 10, Some(500))),
        root(CodingEvent::ToolCallStarted {
            tool_name:    "mcp__github__list_issues".to_string(),
            tool_call_id: "call_1".to_string(),
            arguments:    json!({}),
        }),
        root(completed("mcp__github__list_issues", "call_1", false)),
        root(CodingEvent::ToolCallStarted {
            tool_name:    "write_file".to_string(),
            tool_call_id: "call_2".to_string(),
            arguments:    json!({"file_path": "/workspace/src/lib.rs", "content": "pub fn x() {}"}),
        }),
        root(completed("write_file", "call_2", false)),
        root(CodingEvent::SkillActivated {
            skill_name: "rust".to_string(),
            source:     SkillActivationSource::Tool,
        }),
        root(CodingEvent::TodoCreated(TodoCreatedProps {
            list_id:     TodoListKind::AnthropicTasks.list_id("ses_root"),
            list_kind:   TodoListKind::AnthropicTasks,
            todo_id:     "t1".to_string(),
            status:      TodoStatus::InProgress,
            order:       0,
            subject:     "write tests".to_string(),
            description: String::new(),
            active_form: Some("Writing tests".to_string()),
            owner:       None,
            blocks:      Vec::new(),
            blocked_by:  Vec::new(),
            metadata:    BTreeMap::new(),
        })),
        root(CodingEvent::SubAgentSpawned {
            agent_id:   "sub-1".to_string(),
            depth:      1,
            task:       "review".to_string(),
            generation: 1,
        }),
        child(CodingEvent::SessionStarted {
            provider: Some("openai".to_string()),
            model:    Some("gpt-5.2-mini".to_string()),
        }),
        child(message("openai", "gpt-5.2-mini", 7, 1, None)),
        root(CodingEvent::SubAgentCompleted {
            agent_id:   "sub-1".to_string(),
            depth:      1,
            generation: 1,
            success:    true,
            turns_used: 1,
        }),
        root(CodingEvent::SubAgentSpawned {
            agent_id:   "sub-2".to_string(),
            depth:      1,
            task:       "check the tests".to_string(),
            generation: 1,
        }),
        root(CodingEvent::SubAgentFailed {
            agent_id:   "sub-2".to_string(),
            depth:      1,
            generation: 1,
            error:      ErrorData::new(ErrorKind::Agent, "boom"),
        }),
        root(CodingEvent::LlmRetry {
            provider:   "openai".to_string(),
            model:      "gpt-5.2".to_string(),
            attempt:    0,
            delay_secs: 0.1,
            error:      ErrorData::new(ErrorKind::Llm, "slow down"),
            phase:      LlmRetryPhase::Open,
        }),
        root(CodingEvent::RouteFailover {
            from:            "openai/gpt-5.2".to_string(),
            to:              "anthropic/claude-fable-5".to_string(),
            attempt:         1,
            error:           llm_error(),
            usage:           TokenUsage {
                input: 100,
                output: 10,
                ..TokenUsage::default()
            },
            cost_usd_micros: Some(500),
            inference_ms:    120,
            tool_ms:         30,
            continuation:    FailoverContinuation::ContinueTurn,
        }),
        root(CodingEvent::CompactionCompleted {
            original_turn_count:    20,
            preserved_turn_count:   6,
            summary_token_estimate: 500,
            tracked_file_count:     1,
            reason:                 CompactionReason::Threshold,
            usage:                  TokenUsage {
                input: 30,
                ..TokenUsage::default()
            },
            cost_usd_micros:        Some(2),
        }),
        root(message("anthropic", "claude-fable-5", 50, 5, Some(300))),
        root(CodingEvent::RouteFailoverStopped {
            route:   "anthropic/claude-fable-5".to_string(),
            attempt: 1,
            reason:  FailoverStop::Exhausted,
            error:   llm_error(),
        }),
        root(CodingEvent::ProcessingEnd),
    ]
}

fn root(event: CodingEvent) -> CodingAgentEvent {
    CodingAgentEvent::new("ses_root".to_string(), event, SystemTime::UNIX_EPOCH)
}

fn child(event: CodingEvent) -> CodingAgentEvent {
    CodingAgentEvent::new("ses_child".to_string(), event, SystemTime::UNIX_EPOCH)
        .with_parent_session_id("ses_root".to_string())
}

fn message(provider: &str, model: &str, input: u64, output: u64, cost: Option<u64>) -> CodingEvent {
    CodingEvent::AssistantMessage {
        text:            "ok".to_string(),
        model:           model.to_string(),
        usage:           TokenUsage {
            input,
            output,
            ..TokenUsage::default()
        },
        cost_usd_micros: cost,
        cost_source:     None,
        tool_call_count: 0,
        context_window:  Some(ContextWindowSnapshot {
            provider:              provider.to_string(),
            model:                 model.to_string(),
            context_window_tokens: 400_000,
            input_tokens:          123_456,
            usage_percent:         30.864,
            count_method:          ContextWindowCountMethod::LocalEstimate,
            staleness:             ContextWindowStaleness::Live,
            generated_at:          SystemTime::UNIX_EPOCH,
            event_seq:             Some(9),
            breakdown:             Vec::new(),
            warnings:              Vec::new(),
        }),
        reasoning:       None,
    }
}

fn completed(tool_name: &str, tool_call_id: &str, is_error: bool) -> CodingEvent {
    CodingEvent::ToolCallCompleted {
        tool_name: tool_name.to_string(),
        tool_call_id: tool_call_id.to_string(),
        output: json!("done"),
        metadata: ToolOutputMetadata::default(),
        is_error,
        error_kind: None,
        output_bytes_observed: 4,
        output_bytes_retained: 4,
        output_bytes_omitted: 0,
    }
}

/// A model-layer failure with every optional member set.
fn llm_error() -> ErrorData {
    let mut error = ErrorData::new(ErrorKind::Llm, "rate limited: 429 Too Many Requests")
        .with_provider("openai")
        .with_model("gpt-5.2");
    error.llm_kind = Some(LlmErrorKind::RateLimit);
    error.retry = Some(RetryClassification::after(Duration::from_millis(1500)));
    error.status = Some(429);
    error.provider_code = Some("rate_limit_exceeded".to_string());
    error.provider_retry_after_millis = Some(1500);
    error.source_chain = vec!["429 Too Many Requests".to_string()];
    error
}

// --- The spec as a JSON schema --------------------------------------------

#[expect(
    clippy::disallowed_methods,
    reason = "a synchronous test reads the spec from the repository once"
)]
fn spec() -> Value {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../docs/public/api-reference/fabro-api.yaml"
    ))
    .expect("the OpenAPI spec is in the repository");
    let yaml: serde_yaml::Value = serde_yaml::from_str(&text).expect("the spec parses");
    serde_json::to_value(yaml).expect("the spec is JSON-compatible")
}

/// Validates `value` against one component schema, with the whole document
/// as the root so `$ref`s resolve.
fn assert_valid(schema_name: &str, value: &Value) {
    let mut root = spec();
    root["$ref"] = json!(format!("#/components/schemas/{schema_name}"));
    let validator = jsonschema::validator_for(&root).expect("the spec compiles as a JSON schema");
    let errors: Vec<String> = validator
        .iter_errors(value)
        .map(|error| format!("{error} at {}", error.instance_path()))
        .collect();
    assert!(
        errors.is_empty(),
        "{schema_name} rejects {value:#}:\n{}",
        errors.join("\n")
    );
}

fn resolve<'a>(spec: &'a Value, schema: &'a Value) -> &'a Value {
    match schema.get("$ref").and_then(Value::as_str) {
        Some(reference) => resolve(
            spec,
            spec.pointer(reference.trim_start_matches('#'))
                .unwrap_or_else(|| panic!("unresolved {reference}")),
        ),
        None => schema,
    }
}

/// Every key `value` serializes is a declared property of `schema`, and
/// every required property is present, recursively. This is what catches a
/// pebble field the spec does not know yet, since the schemas do not forbid
/// additional properties.
fn assert_declared(spec: &Value, schema: &Value, value: &Value, path: &str) {
    let schema = resolve(spec, schema);
    if let Some(variants) = schema.get("oneOf").and_then(Value::as_array) {
        if value.is_null() {
            return;
        }
        let chosen = match schema.get("discriminator") {
            Some(discriminator) => {
                let property = discriminator["propertyName"]
                    .as_str()
                    .expect("a discriminator names its property");
                let tag = value[property]
                    .as_str()
                    .unwrap_or_else(|| panic!("{path}: no `{property}` tag in {value}"));
                let target = discriminator["mapping"][tag]
                    .as_str()
                    .unwrap_or_else(|| panic!("{path}: `{tag}` is not a mapped variant"));
                spec.pointer(target.trim_start_matches('#'))
                    .unwrap_or_else(|| panic!("unresolved {target}"))
            }
            None => variants
                .iter()
                .map(|variant| resolve(spec, variant))
                .find(|variant| variant.get("type").and_then(Value::as_str) != Some("null"))
                .unwrap_or_else(|| panic!("{path}: no non-null variant")),
        };
        assert_declared(spec, chosen, value, path);
        return;
    }
    match value {
        Value::Object(object) => {
            if let Some(additional) = schema.get("additionalProperties") {
                if additional.is_object() {
                    for (key, member) in object {
                        assert_declared(spec, additional, member, &format!("{path}.{key}"));
                    }
                }
                return;
            }
            let properties = schema
                .get("properties")
                .and_then(Value::as_object)
                .unwrap_or_else(|| panic!("{path}: the schema declares no properties"));
            for (key, member) in object {
                let property = properties
                    .get(key)
                    .unwrap_or_else(|| panic!("{path}.{key} is serialized but not declared"));
                assert_declared(spec, property, member, &format!("{path}.{key}"));
            }
            for required in schema
                .get("required")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let key = required.as_str().expect("required names are strings");
                assert!(
                    object.contains_key(key),
                    "{path}.{key} is required but not serialized"
                );
            }
        }
        Value::Array(items) => {
            if let Some(item_schema) = schema.get("items") {
                for (index, item) in items.iter().enumerate() {
                    assert_declared(spec, item_schema, item, &format!("{path}[{index}]"));
                }
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
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
