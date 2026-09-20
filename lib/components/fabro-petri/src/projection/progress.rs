//! A step's progress records folded into its stage: a command's log lines,
//! the payloads the Attractor steps emit (the prompt and its completion,
//! the fallback plan, the tools a session was offered, a parallel branch's
//! start), Pebble's coding-agent envelope, and a gate's question (VIEWS.md
//! "Agent activity", "Questions").

use chrono::{DateTime, Utc};
use fabro_types::{
    BlockedReason, CodingAgentEvent, CodingEvent, InterviewOption, InterviewQuestionRecord,
    PendingInterviewRecord, ReviewTarget, ReviewTargetKind, RunStatus, StageInferenceProjection,
    StageModelUsage, StageProjection, ToolCategory, ToolSource, ToolSummary, timing,
};
use petri_execution::ExecutionId;
use petri_execution::events::{Parsed, RunEvent};
use petri_runtime::ir::StepEvent;
use petri_runtime::steps::QuestionReference;
use serde_json::Value;
use tracing::debug;

use super::model::{model_ref, split_model, usage_of};
use super::{RunView, apply_status, stage_key};
use crate::interview::question_type;

impl RunView {
    pub(super) fn fold_progress(
        &mut self,
        execution: ExecutionId,
        event: &RunEvent,
        ev: &StepEvent,
        at: DateTime<Utc>,
    ) {
        match ev {
            StepEvent::Log { line, .. } => {
                if let Some(stage) = self.stage_of(execution, event.subject.as_ref()) {
                    let output = stage.output.get_or_insert_default();
                    output.push_str(line);
                    output.push('\n');
                    stage.output_bytes = Some(output.len() as u64);
                    stage.live_streaming = Some(true);
                }
            }
            StepEvent::Artifact { .. } => {}
            StepEvent::Custom(payload) => {
                if let Some(parsed) = event.parsed() {
                    self.fold_parsed(execution, event, parsed, at);
                    return;
                }
                let kind = payload.get("kind").and_then(Value::as_str).unwrap_or("");
                match kind {
                    "pebble" => self.fold_pebble(execution, event, payload, at),
                    "attractor.prompt" => {
                        if let Some(stage) = self.stage_of(execution, event.subject.as_ref()) {
                            stage.prompt = payload
                                .get("prompt")
                                .and_then(Value::as_str)
                                .map(str::to_string);
                            let model = payload.get("model").and_then(Value::as_str);
                            if let Some(model) = model {
                                let (provider, model_id) = split_model(model);
                                stage.provider_used = Some(StageModelUsage {
                                    mode:             StageModelUsage::MODE_PROMPT.to_string(),
                                    provider:         provider.map(str::to_string),
                                    model:            Some(model_id.to_string()),
                                    reasoning_effort: None,
                                    speed:            None,
                                });
                                stage.model = model_ref(provider, model_id);
                            }
                        }
                    }
                    "attractor.prompt.completed" => {
                        if let Some(stage) = self.stage_of(execution, event.subject.as_ref()) {
                            stage.response = payload
                                .get("response")
                                .and_then(Value::as_str)
                                .map(str::to_string);
                            if let Some(usage) = usage_of(payload.get("usage")) {
                                stage.usage = usage;
                            }
                        }
                    }
                    "attractor.fallback.plan" => {
                        if let Some(stage) = self.stage_of(execution, event.subject.as_ref()) {
                            let route = payload
                                .get("routes")
                                .and_then(Value::as_array)
                                .and_then(|routes| routes.first());
                            if let Some(route) = route {
                                let provider = route.get("provider").and_then(Value::as_str);
                                let model = route.get("model").and_then(Value::as_str);
                                stage.provider_used = Some(StageModelUsage {
                                    mode:             StageModelUsage::MODE_AGENT.to_string(),
                                    provider:         provider.map(str::to_string),
                                    model:            model.map(str::to_string),
                                    reasoning_effort: None,
                                    speed:            None,
                                });
                                if let Some(model) = model {
                                    stage.model = model_ref(provider, model);
                                }
                            }
                        }
                    }
                    // The tools a native session was offered, once per
                    // session (VIEWS.md "Agent activity", tools available):
                    // the stage's list is the union over its sessions, by
                    // name, in the order the sessions listed them.
                    "attractor.tools" => {
                        if let Some(stage) = self.stage_of(execution, event.subject.as_ref()) {
                            let tools = payload.get("tools").and_then(Value::as_array);
                            for tool in tools.into_iter().flatten() {
                                let Some(summary) = tool_summary(tool) else {
                                    continue;
                                };
                                if !stage
                                    .agent_tools
                                    .iter()
                                    .any(|known| known.name == summary.name)
                                {
                                    stage.agent_tools.push(summary);
                                }
                            }
                        }
                    }
                    "attractor.parallel.branch.started" => {
                        let invocation = payload.get("invocation").and_then(Value::as_u64);
                        let index = payload
                            .get("index")
                            .and_then(Value::as_u64)
                            .and_then(|index| u32::try_from(index).ok());
                        let fork_firing = payload
                            .get("occurrence")
                            .and_then(|occurrence| occurrence.get("firing"))
                            .and_then(Value::as_u64);
                        if let (Some(invocation), Some(index), Some(fork_firing)) =
                            (invocation, index, fork_firing)
                        {
                            let group = self
                                .state
                                .stages
                                .get(&stage_key(execution.raw(), fork_firing))
                                .map(|stage| stage.stage_id.clone());
                            if let Some(group) = group {
                                self.state.invocations.entry(invocation).or_default().branch =
                                    Some((group, index));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn fold_parsed(
        &mut self,
        execution: ExecutionId,
        event: &RunEvent,
        parsed: &Parsed,
        at: DateTime<Utc>,
    ) {
        match parsed {
            Parsed::Question { question } => {
                let Some(subject) = event.subject.as_ref() else {
                    return;
                };
                let Some(firing) = subject.firing else {
                    return;
                };
                let key = stage_key(execution.raw(), firing.raw());
                let label = self.state.stages.get(&key).map_or_else(
                    || subject.node.name.to_string(),
                    |stage| stage.stage_id.to_string(),
                );
                self.state.questions.insert(question.id.clone(), key);
                let Some(projection) = self.projection.as_mut() else {
                    return;
                };
                projection
                    .pending_interviews
                    .insert(question.id.clone(), PendingInterviewRecord {
                        question:   InterviewQuestionRecord {
                            id:              question.id.clone(),
                            text:            question.text.clone(),
                            stage:           label,
                            question_type:   question_type(question),
                            options:         question
                                .options
                                .iter()
                                .map(|option| InterviewOption {
                                    key:         option.key.clone(),
                                    label:       option.label.clone(),
                                    description: option.description.clone(),
                                    preview:     option.preview.clone(),
                                })
                                .collect(),
                            allow_freeform:  question.freeform,
                            timeout_seconds: question
                                .timeout_ms
                                .map(|timeout| timeout as f64 / 1000.0),
                            context_display: question.context.clone(),
                            review_target:   question.reference.as_ref().and_then(review_target),
                        },
                        started_at: at,
                    });
                apply_status(
                    projection,
                    RunStatus::Blocked {
                        blocked_reason: BlockedReason::HumanInputRequired,
                    },
                    at,
                );
            }
            Parsed::QuestionExpired { expired } => {
                self.close_questions(Some(expired.question.as_str()), None, at);
            }
            Parsed::Note { .. } => {}
        }
    }

    /// Close one question by id, or every question of a firing, and unblock

    /// Close one question by id, or every question of a firing, and unblock
    /// the run when none is left.
    pub(super) fn close_questions(
        &mut self,
        question: Option<&str>,
        firing_key: Option<&str>,
        at: DateTime<Utc>,
    ) {
        let closed: Vec<String> = match (question, firing_key) {
            (Some(question), _) => vec![question.to_string()],
            (None, Some(key)) => self
                .state
                .questions
                .iter()
                .filter(|(_, asked_by)| asked_by.as_str() == key)
                .map(|(id, _)| id.clone())
                .collect(),
            (None, None) => Vec::new(),
        };
        for id in &closed {
            self.state.questions.remove(id);
        }
        let Some(projection) = self.projection.as_mut() else {
            return;
        };
        for id in &closed {
            projection.pending_interviews.remove(id);
        }
        if projection.pending_interviews.is_empty()
            && matches!(projection.status, RunStatus::Blocked { .. })
        {
            apply_status(projection, RunStatus::Running, at);
        }
    }

    fn fold_pebble(
        &mut self,
        execution: ExecutionId,
        event: &RunEvent,
        payload: &Value,
        at: DateTime<Utc>,
    ) {
        let Some(envelope) = payload.get("event") else {
            return;
        };
        let envelope: CodingAgentEvent = match serde_json::from_value(envelope.clone()) {
            Ok(envelope) => envelope,
            Err(error) => {
                debug!(error = %error, "a pebble envelope did not decode; skipped");
                return;
            }
        };
        let Some(stage) = self.stage_of(execution, event.subject.as_ref()) else {
            return;
        };
        let agent = stage.agent.get_or_insert_default();
        agent.apply(&envelope);
        if stage.completion.is_none() {
            stage.usage = agent.usage.saturating_add(agent.descendant_usage());
        }
        // A tool the stage's list names was called, by any of its sessions.
        if let CodingEvent::ToolCallStarted { tool_name, .. } = &envelope.event {
            if let Some(tool) = stage
                .agent_tools
                .iter_mut()
                .find(|tool| tool.name == *tool_name)
            {
                tool.invoked = true;
            }
        }
        let is_root = envelope.parent_session_id.is_none();
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "pebble's event vocabulary is non-exhaustive and only some events project"
        )]
        match &envelope.event {
            CodingEvent::SessionStarted {
                provider, model, ..
            } if is_root => {
                stage.provider_used = Some(StageModelUsage {
                    mode:             StageModelUsage::MODE_AGENT.to_string(),
                    provider:         provider.clone(),
                    model:            model.clone(),
                    reasoning_effort: None,
                    speed:            None,
                });
                if let Some(model) = model.as_deref() {
                    stage.model = model_ref(provider.as_deref(), model);
                }
            }
            CodingEvent::LlmRequestStarted { requested_model } if is_root => {
                stage.inference = Some(StageInferenceProjection {
                    session_id:        envelope.session_id.clone(),
                    started_at:        at,
                    requested_model:   requested_model.clone(),
                    first_output_at:   None,
                    first_output_kind: None,
                    retries:           0,
                });
            }
            CodingEvent::LlmFirstOutput { kind } => {
                if let Some(inference) = stage.inference.as_mut() {
                    if inference.session_id == envelope.session_id {
                        inference.first_output_at = Some(at);
                        inference.first_output_kind = Some(*kind);
                    }
                }
            }
            CodingEvent::LlmRetry { .. } => {
                if let Some(inference) = stage.inference.as_mut() {
                    if inference.session_id == envelope.session_id {
                        inference.retries = inference.retries.saturating_add(1);
                        inference.first_output_at = None;
                        inference.first_output_kind = None;
                    }
                }
            }
            CodingEvent::AssistantMessage { model, .. } => {
                if is_root {
                    if let Some(provider) = stage
                        .provider_used
                        .as_ref()
                        .and_then(|used| used.provider.as_deref())
                    {
                        stage.model = model_ref(Some(provider), model);
                    }
                }
                close_inference(stage, &envelope.session_id, at);
            }
            CodingEvent::Error { .. } | CodingEvent::RoundInterrupted { .. } => {
                close_inference(stage, &envelope.session_id, at);
            }
            CodingEvent::SessionEnded => {
                close_inference(stage, &envelope.session_id, at);
                stage.close_tool_batch_for_session(&envelope.session_id, at);
            }
            CodingEvent::ToolCallStarted { tool_call_id, .. } if is_root => {
                stage.open_tool_call(envelope.session_id.clone(), tool_call_id.clone(), at);
            }
            CodingEvent::ToolCallCompleted { tool_call_id, .. } if is_root => {
                stage.close_tool_call(&envelope.session_id, tool_call_id, at);
            }
            _ => {}
        }
    }
}

/// One tool of an `attractor.tools` payload as the stage's list carries
/// it: the name and description as recorded, Pebble's `source` as it is,
/// and Pebble's behavioural category where Petri's says which (a
/// sub-agent tool); every other tool is `other`, because the payload
/// carries Petri's origin category (`builtin`, `mcp`, `host`, `question`),
/// not Pebble's permission class. `invoked` starts false and flips on the
/// session's `ToolCallStarted`.
fn tool_summary(tool: &Value) -> Option<ToolSummary> {
    let name = tool.get("name").and_then(Value::as_str)?;
    let description = tool
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let source = tool
        .get("source")
        .cloned()
        .and_then(|source| serde_json::from_value::<ToolSource>(source).ok())
        .unwrap_or_default();
    let category = match tool.get("category").and_then(Value::as_str) {
        Some("subagent") => ToolCategory::Subagent,
        _ => ToolCategory::Other,
    };
    Some(ToolSummary {
        name: name.to_string(),
        description: description.to_string(),
        source,
        category,
        invoked: false,
    })
}

/// The question's `reference` as Fabro's review target, when it is one
/// Fabro's validation admits (a `document`, or a reference without a kind,
/// with a label and an absolute HTTP URL within Fabro's limits).
fn review_target(reference: &QuestionReference) -> Option<ReviewTarget> {
    let kind = match reference.kind.as_deref() {
        Some("document") | None => ReviewTargetKind::Document,
        Some(_) => return None,
    };
    ReviewTarget::new(reference.label.clone(), reference.url.clone(), kind).ok()
}

fn close_inference(stage: &mut StageProjection, session_id: &str, at: DateTime<Utc>) {
    let open = stage
        .inference
        .as_ref()
        .is_some_and(|inference| inference.session_id == session_id);
    if !open {
        return;
    }
    if let Some(inference) = stage.inference.take() {
        stage.accumulate_inference_ms(timing::elapsed_ms(inference.started_at, at));
    }
}
