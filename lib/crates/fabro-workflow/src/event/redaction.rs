use ::fabro_types::{RunEvent, RunId};
use anyhow::{Context, Result};
use fabro_redact::{SecretRedactor, redact_json_value};
use fabro_store::EventPayload;
use fabro_util::json::normalize_json_value;
use serde_json::Value;

pub fn build_redacted_event_payload(event: &RunEvent, run_id: &RunId) -> Result<EventPayload> {
    EventPayload::new(redacted_event_value(event)?, run_id).map_err(anyhow::Error::from)
}

/// Redact an event and reconstruct it as a `RunEvent`.
///
/// This runs the event through the content-based pass plus the per-run
/// exact-match [`SecretRedactor`], then reparses the redacted payload back
/// into a `RunEvent` so downstream sinks that require a typed event never see
/// the raw value. An empty redactor is an identity for the exact-match pass.
pub fn redacted_run_event(event: &RunEvent, redactor: &SecretRedactor) -> Result<RunEvent> {
    let mut value = redacted_event_value(event)?;
    redact_event_payload_secrets(&mut value, redactor);
    let payload = EventPayload::new(value, &event.run_id)?;
    RunEvent::try_from(&payload).map_err(anyhow::Error::from)
}

pub fn redacted_event_json(event: &RunEvent) -> Result<String> {
    serde_json::to_string(&redacted_event_value(event)?).map_err(anyhow::Error::from)
}

/// Content-based redaction over the normalized event payload. Exact-match
/// per-run secret redaction is layered on top by [`redacted_run_event`].
fn redacted_event_value(event: &RunEvent) -> Result<Value> {
    Ok(redact_json_value(normalize_json_value(event.to_value()?)))
}

fn redact_event_payload_secrets(value: &mut Value, redactor: &SecretRedactor) {
    // No declared secrets (the common case): skip the recursive property walk
    // entirely. Content-based redaction already ran in `redacted_event_value`.
    if redactor.is_empty() {
        return;
    }
    if let Some(properties) = value.get_mut("properties") {
        redact_redactable_event_properties(properties, redactor);
    }
    if let Some(Value::String(label)) = value.get_mut("node_label") {
        let redacted = redactor.redact_into(label);
        if redacted != *label {
            *label = redacted;
        }
    }
}

fn redact_redactable_event_properties(value: &mut Value, redactor: &SecretRedactor) {
    match value {
        Value::Object(obj) => {
            for (key, child) in obj {
                if is_secret_redactable_event_property(key) {
                    *child = redactor.redact_json(std::mem::take(child));
                } else {
                    redact_redactable_event_properties(child, redactor);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_redactable_event_properties(item, redactor);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

// Exact-match secret values may be intentionally low entropy ("staging",
// "pause", "running"). Redacting every string in an event can therefore corrupt
// structural fields that are validated enum values or IDs. Keep this list to
// free-form text/blob fields where replacing a matched substring preserves the
// event schema and projection semantics.
fn is_secret_redactable_event_property(key: &str) -> bool {
    matches!(
        key,
        "active_form"
            | "answer"
            | "arguments"
            | "causes"
            | "command"
            | "context_display"
            | "delta"
            | "description"
            | "details"
            | "diff"
            | "error"
            | "error_message"
            | "exec_output_tail"
            | "failure"
            | "final_patch"
            | "goal"
            | "input"
            | "message"
            | "notes"
            | "output"
            | "preview"
            | "prompt"
            | "question"
            | "reason"
            | "response"
            | "script"
            | "stderr"
            | "stdout"
            | "subject"
            | "text"
            | "title"
            | "tool_input"
            | "tool_output"
            | "workflow_config"
            | "workflow_source"
    )
}

pub fn event_payload_from_redacted_json(line: &str, run_id: &RunId) -> Result<EventPayload> {
    let value = serde_json::from_str(line).context("Failed to parse redacted event payload")?;
    EventPayload::new(value, run_id).map_err(anyhow::Error::from)
}

#[cfg(test)]
mod tests {
    use ::fabro_types::{fixtures, run_event as fabro_types};

    use super::*;
    use crate::event::{Event, to_run_event};

    #[test]
    fn build_redacted_event_payload_requires_id() {
        let stored = to_run_event(&fixtures::RUN_8, &Event::RunSubmitted {
            definition_blob: None,
        });
        let payload = build_redacted_event_payload(&stored, &fixtures::RUN_8).unwrap();
        assert_eq!(payload.as_value()["id"], stored.id);
        assert_eq!(payload.as_value()["event"], "run.submitted");
    }

    #[test]
    fn build_redacted_event_payload_redacts_exec_output_tail_values() {
        let secret = "sk-ant-api03-xK9mZ2vL8nQ5rT1wY4bC7dF0gH3jE6pA";
        let stored = to_run_event(&fixtures::RUN_8, &Event::SetupFailed {
            command:          "setup".to_string(),
            index:            0,
            exit_code:        1,
            stderr:           "compat stderr".to_string(),
            exec_output_tail: Some(fabro_types::ExecOutputTail {
                stdout:           Some(format!("stdout {secret}")),
                stderr:           Some("plain stderr".to_string()),
                stdout_truncated: false,
                stderr_truncated: false,
            }),
        });

        let payload = build_redacted_event_payload(&stored, &fixtures::RUN_8).unwrap();
        let payload_text = serde_json::to_string(payload.as_value()).unwrap();

        assert!(!payload_text.contains(secret));
        assert!(payload_text.contains("REDACTED"));
        assert_eq!(payload.as_value()["event"], "setup.failed");
        assert_eq!(
            payload.as_value()["properties"]["exec_output_tail"]["stderr"],
            "plain stderr"
        );
    }

    #[test]
    fn redacted_run_event_redacts_registered_low_entropy_secret() {
        let redactor = fabro_redact::SecretRedactor::default();
        redactor.register("staging");
        let stored = to_run_event(&fixtures::RUN_8, &Event::SetupFailed {
            command:          "deploy staging".to_string(),
            index:            0,
            exit_code:        1,
            stderr:           "failed in staging".to_string(),
            exec_output_tail: None,
        });

        let redacted = redacted_run_event(&stored, &redactor).unwrap();
        let redacted_text = serde_json::to_string(&redacted.to_value().unwrap()).unwrap();

        assert!(!redacted_text.contains("staging"));
        assert!(redacted_text.contains("REDACTED"));
    }

    #[test]
    fn redacted_run_event_redactors_are_isolated_per_run() {
        let first = fabro_redact::SecretRedactor::default();
        first.register("alpha");
        let second = fabro_redact::SecretRedactor::default();
        second.register("bravo");
        let stored = to_run_event(&fixtures::RUN_8, &Event::SetupCommandStarted {
            command: "echo alpha bravo".to_string(),
            index:   0,
        });

        let first_text = serde_json::to_string(
            &redacted_run_event(&stored, &first)
                .unwrap()
                .to_value()
                .unwrap(),
        )
        .unwrap();
        let second_text = serde_json::to_string(
            &redacted_run_event(&stored, &second)
                .unwrap()
                .to_value()
                .unwrap(),
        )
        .unwrap();

        assert!(!first_text.contains("alpha"));
        assert!(first_text.contains("bravo"));
        assert!(second_text.contains("alpha"));
        assert!(!second_text.contains("bravo"));
    }

    #[test]
    fn redacted_run_event_preserves_structural_event_fields() {
        let redactor = fabro_redact::SecretRedactor::default();
        redactor.register("setup.failed");
        let stored = to_run_event(&fixtures::RUN_8, &Event::SetupFailed {
            command:          "echo setup.failed".to_string(),
            index:            0,
            exit_code:        1,
            stderr:           "setup.failed".to_string(),
            exec_output_tail: None,
        });

        let redacted = redacted_run_event(&stored, &redactor).unwrap();

        assert_eq!(redacted.event_name(), "setup.failed");
        let value = redacted.to_value().unwrap();
        assert_eq!(value["properties"]["command"], "echo REDACTED");
        assert_eq!(value["properties"]["stderr"], "REDACTED");
    }

    #[test]
    fn redacted_run_event_preserves_structural_property_values() {
        let redactor = fabro_redact::SecretRedactor::default();
        redactor.register("pause");
        let stored = to_run_event(&fixtures::RUN_8, &Event::RunPauseRequested { actor: None });

        let redacted = redacted_run_event(&stored, &redactor).unwrap();

        assert_eq!(redacted.event_name(), "run.pause.requested");
        assert_eq!(
            redacted.to_value().unwrap()["properties"]["action"],
            "pause"
        );
    }
}
