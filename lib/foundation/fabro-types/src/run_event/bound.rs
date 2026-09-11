//! Budget enforcement for serialized run events.
//!
//! `POST /runs/{id}/events` rejects bodies larger than
//! [`MAX_RUN_EVENT_BODY_BYTES`]. The agent layer budgets serialized tool
//! output against that limit; workflow-emitted events must obey the same
//! contract. A `checkpoint.completed` that embeds a full merge diff or
//! oversized context values used to lose the canonical run store (HTTP 413)
//! and kill the run *after* the work had already succeeded (seed fabro-a723).
//! [`bound_run_event`] shrinks such events deterministically instead of
//! letting the append fail.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::{CheckpointCompletedProps, EventBody, RunNoticeLevel, RunNoticeProps};
use crate::{BilledModelUsage, Outcome, RunEvent};

/// Headroom subtracted from [`MAX_RUN_EVENT_BODY_BYTES`] when bounding an
/// event. Covers the envelope fields, redaction rewriting, and serializer
/// drift between the bound measurement and the sink write.
pub const RUN_EVENT_BODY_HEADROOM_BYTES: usize = 256 * 1024;

/// Largest single string field (diff, notes, error message) retained after
/// truncation.
const MAX_RETAINED_STRING_BYTES: usize = 512 * 1024;

/// Largest serialized map entry (context values, outcome context updates)
/// retained per key before the entry is replaced by a truncation marker.
const MAX_RETAINED_ENTRY_BYTES: usize = 64 * 1024;

/// Result of bounding one run event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunEventBodyBound {
    /// Serialized size before bounding.
    pub original_bytes: usize,
    /// Serialized size after bounding.
    pub bounded_bytes:  usize,
}

/// Budget a bounded run event must fit into.
#[must_use]
pub fn run_event_body_budget() -> usize {
    super::MAX_RUN_EVENT_BODY_BYTES.saturating_sub(RUN_EVENT_BODY_HEADROOM_BYTES)
}

/// Bound `event` to [`run_event_body_budget`]. Returns `None` when the event
/// already fits and was left untouched.
///
/// Bounding is best-effort observational degradation: the audit trail keeps
/// the event shape and routing-relevant fields, while bulky payloads are
/// truncated or replaced with markers that record how many bytes were
/// omitted. An event that still cannot fit is replaced by a `run.notice`
/// warning rather than risking the append failure that kills the run.
#[must_use]
pub fn bound_run_event(event: &mut RunEvent) -> Option<RunEventBodyBound> {
    bound_run_event_with_budget(event, run_event_body_budget())
}

/// Budget-aware variant of [`bound_run_event`] for transports with their own
/// body limits.
#[must_use]
pub fn bound_run_event_with_budget(
    event: &mut RunEvent,
    budget: usize,
) -> Option<RunEventBodyBound> {
    let original_bytes = serialized_len(event)?;
    if original_bytes <= budget {
        return None;
    }

    if let EventBody::CheckpointCompleted(props) = &mut event.body {
        bound_checkpoint_completed(props, budget);
    }
    if fits(event, budget) {
        return Some(bound_info(original_bytes, event));
    }

    shrink_body_strings(&mut event.body);
    if fits(event, budget) {
        return Some(bound_info(original_bytes, event));
    }

    // Last resort (fabro-a723): never let an oversized body kill the run.
    // Record what was dropped so the audit trail stays honest.
    let dropped_event = event.body.event_name().to_owned();
    event.body = EventBody::RunNotice(RunNoticeProps {
        level:            RunNoticeLevel::Warn,
        code:             "event_body_overflow".to_string(),
        message:          format!(
            "{dropped_event} event of {original_bytes} bytes exceeded the {budget} byte \
             run-event budget even after truncation and was replaced to protect the run \
             store append"
        ),
        exec_output_tail: None,
    });
    Some(bound_info(original_bytes, event))
}

fn bound_info(original_bytes: usize, event: &RunEvent) -> RunEventBodyBound {
    RunEventBodyBound {
        original_bytes,
        bounded_bytes: serialized_len(event).unwrap_or(original_bytes),
    }
}

fn serialized_len(event: &RunEvent) -> Option<usize> {
    serde_json::to_vec(event).ok().map(|body| body.len())
}

fn fits(event: &RunEvent, budget: usize) -> bool {
    serialized_len(event).is_some_and(|len| len <= budget)
}

/// Truncate `input` to at most `max_bytes` by keeping an equal-sized UTF-8
/// head and tail and inserting a marker that records the omitted byte count.
fn truncate_string(input: &str, max_bytes: usize) -> String {
    if input.len() <= max_bytes {
        return input.to_owned();
    }
    let head_budget = max_bytes / 2;
    let tail_budget = max_bytes - head_budget;
    let head_end = input.floor_char_boundary(head_budget);
    let tail_start = input.ceil_char_boundary(input.len() - tail_budget);
    let retained = head_end + (input.len() - tail_start);
    let omitted = input.len().saturating_sub(retained);
    format!(
        "{}\n[... fabro truncated {omitted} of {} bytes ...]\n{}",
        &input[..head_end],
        input.len(),
        &input[tail_start..]
    )
}

/// Shrink the known-unbounded payloads of a `checkpoint.completed` event.
fn bound_checkpoint_completed(props: &mut CheckpointCompletedProps, budget: usize) {
    props.diff = props
        .diff
        .take()
        .map(|diff| truncate_string(&diff, MAX_RETAINED_STRING_BYTES));

    bound_value_map(&mut props.context_values);
    bound_outcome_map(&mut props.node_outcomes);

    // Many medium-sized entries can still exceed the budget together even
    // though each one fits the per-entry cap. Replace the maps wholesale;
    // both fields deserialize with `#[serde(default)]`, so absent entries
    // stay valid.
    if serialized_props_len(props).is_some_and(|len| len > budget) {
        replace_value_map(&mut props.context_values);
    }
    if serialized_props_len(props).is_some_and(|len| len > budget) {
        replace_outcome_map(&mut props.node_outcomes);
    }
}

fn serialized_props_len(props: &CheckpointCompletedProps) -> Option<usize> {
    serde_json::to_vec(props).ok().map(|body| body.len())
}

fn bound_value_map(map: &mut BTreeMap<String, Value>) {
    for (key, value) in map.iter_mut() {
        let Some(len) = serde_json::to_vec(value).ok().map(|body| body.len()) else {
            continue;
        };
        if len > MAX_RETAINED_ENTRY_BYTES {
            *value = json!({
                "fabro_truncated": true,
                "omitted_bytes": len,
                "key": key,
            });
        }
    }
}

fn replace_value_map(map: &mut BTreeMap<String, Value>) {
    let omitted_bytes = serde_json::to_vec(map).ok().map_or(0, |body| body.len());
    let entries = map.len();
    map.clear();
    map.insert(
        "fabro_truncated_map".to_string(),
        json!({
            "entries": entries,
            "omitted_bytes": omitted_bytes,
        }),
    );
}

fn bound_outcome_map(map: &mut BTreeMap<String, Outcome<Option<BilledModelUsage>>>) {
    for outcome in map.values_mut() {
        let Some(len) = serde_json::to_vec(outcome).ok().map(|body| body.len()) else {
            continue;
        };
        if len <= MAX_RETAINED_ENTRY_BYTES {
            continue;
        }
        // Keep routing-relevant fields; drop the bulky payloads.
        outcome.context_updates.clear();
        outcome.notes = outcome
            .notes
            .take()
            .map(|notes| truncate_string(&notes, 2048));
        if let Some(failure) = outcome.failure.as_mut() {
            failure.message = truncate_string(&failure.message, 2048);
            failure.exec_output_tail = None;
        }
    }
}

fn replace_outcome_map(map: &mut BTreeMap<String, Outcome<Option<BilledModelUsage>>>) {
    let omitted_bytes = serde_json::to_vec(map).ok().map_or(0, |body| body.len());
    let entries = map.len();
    map.clear();
    let outcome = Outcome {
        notes: Some(format!(
            "fabro truncated {entries} node outcomes ({omitted_bytes} bytes) that exceeded the \
             run-event budget"
        )),
        ..Outcome::default()
    };
    map.insert("fabro_truncated_outcomes".to_string(), outcome);
}

/// Generic pass over the serialized body: truncate every oversized string in
/// place. Type-preserving by construction (strings stay strings), so the
/// result still deserializes into [`EventBody`].
fn shrink_body_strings(body: &mut EventBody) {
    let Ok(mut value) = serde_json::to_value(&*body) else {
        return;
    };
    shrink_value_strings(&mut value);
    if let Ok(bounded) = serde_json::from_value::<EventBody>(value) {
        *body = bounded;
    }
}

fn shrink_value_strings(value: &mut Value) {
    match value {
        Value::String(text) => {
            if text.len() > MAX_RETAINED_STRING_BYTES {
                *text = truncate_string(text, MAX_RETAINED_STRING_BYTES);
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                shrink_value_strings(item);
            }
        }
        Value::Object(map) => {
            for (_, item) in map.iter_mut() {
                shrink_value_strings(item);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;
    use ulid::Ulid;

    use super::*;
    use crate::RunId;
    use crate::run_event::MAX_RUN_EVENT_BODY_BYTES;

    fn checkpoint_event(diff: Option<String>) -> RunEvent {
        RunEvent {
            id:                 "0196b0ad-7d47-7c7f-8f2a-3f2b1a0c9d11".to_string(),
            ts:                 chrono::Utc::now(),
            run_id:             RunId::from(Ulid::new()),
            node_id:            None,
            node_label:         None,
            stage_id:           None,
            parallel_group_id:  None,
            parallel_branch_id: None,
            session_id:         None,
            parent_session_id:  None,
            tool_call_id:       None,
            actor:              None,
            body:               EventBody::CheckpointCompleted(CheckpointCompletedProps {
                status: "succeeded".to_string(),
                current_node: "merge".to_string(),
                completed_nodes: Vec::new(),
                node_retries: BTreeMap::new(),
                context_values: BTreeMap::new(),
                node_outcomes: BTreeMap::new(),
                next_node_id: None,
                git_commit_sha: None,
                loop_failure_signatures: BTreeMap::new(),
                restart_failure_signatures: BTreeMap::new(),
                node_visits: BTreeMap::new(),
                diff,
                diff_summary: None,
                graph_visit: None,
                resumed_from_stage_id: None,
            }),
        }
    }

    fn serialized(event: &RunEvent) -> usize {
        serde_json::to_vec(event).expect("event serializes").len()
    }

    #[test]
    fn small_event_is_left_untouched() {
        let mut event = checkpoint_event(Some("diff --git a/x b/x".to_string()));
        let before = serde_json::to_vec(&event).expect("serializes").clone();

        let bound = bound_run_event(&mut event);

        assert!(bound.is_none());
        assert_eq!(before, serde_json::to_vec(&event).expect("serializes"));
    }

    #[test]
    fn oversized_diff_is_truncated_to_the_budget() {
        let huge_diff = "x".repeat(4 * 1024 * 1024);
        let mut event = checkpoint_event(Some(huge_diff));

        let bound = bound_run_event(&mut event).expect("event was bounded");

        assert!(bound.original_bytes > MAX_RUN_EVENT_BODY_BYTES);
        assert!(serialized(&event) <= run_event_body_budget());
        let EventBody::CheckpointCompleted(props) = &event.body else {
            panic!("event stays a checkpoint.completed");
        };
        let diff = props.diff.as_deref().expect("diff is retained");
        assert!(diff.contains("[... fabro truncated"));
        assert!(diff.len() <= MAX_RETAINED_STRING_BYTES + 256);
    }

    #[test]
    fn oversized_context_values_are_replaced_by_markers() {
        let mut event = checkpoint_event(None);
        let props = match &mut event.body {
            EventBody::CheckpointCompleted(props) => props,
            other => panic!("unexpected body: {other:?}"),
        };
        props
            .context_values
            .insert("journal".to_string(), json!("blob".repeat(2 * 1024 * 1024)));

        bound_run_event(&mut event).expect("event was bounded");

        assert!(serialized(&event) <= run_event_body_budget());
        let EventBody::CheckpointCompleted(props) = &event.body else {
            panic!("event stays a checkpoint.completed");
        };
        let marker = &props.context_values["journal"];
        assert_eq!(marker["fabro_truncated"], json!(true));
        assert!(marker["omitted_bytes"].as_u64().expect("byte count") > 0);
    }

    #[test]
    fn many_medium_entries_replace_the_map_wholesale() {
        let mut event = checkpoint_event(None);
        let props = match &mut event.body {
            EventBody::CheckpointCompleted(props) => props,
            other => panic!("unexpected body: {other:?}"),
        };
        for i in 0..60 {
            props
                .context_values
                .insert(format!("key_{i}"), json!("x".repeat(50_000)));
        }

        bound_run_event(&mut event).expect("event was bounded");

        assert!(serialized(&event) <= run_event_body_budget());
        let EventBody::CheckpointCompleted(props) = &event.body else {
            panic!("event stays a checkpoint.completed");
        };
        let marker = &props.context_values["fabro_truncated_map"];
        assert_eq!(marker["entries"], json!(60));
    }

    #[test]
    fn oversized_unknown_body_falls_back_to_a_notice() {
        let mut event = checkpoint_event(None);
        event.body = EventBody::RunNotice(RunNoticeProps {
            level:            RunNoticeLevel::Warn,
            code:             "test".to_string(),
            message:          "y".repeat(8 * 1024 * 1024),
            exec_output_tail: None,
        });

        let bound = bound_run_event_with_budget(&mut event, 1024).expect("event was bounded");

        assert!(bound.bounded_bytes <= 1024);
        let EventBody::RunNotice(notice) = &event.body else {
            panic!("fallback keeps a run.notice body");
        };
        assert_eq!(notice.code, "event_body_overflow");
        assert!(notice.message.contains("run.notice"));
        assert!(notice.message.contains("run store"));
    }
}
