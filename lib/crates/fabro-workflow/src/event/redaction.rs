use ::fabro_types::{RunEvent, RunId};
use anyhow::{Context, Result};
use fabro_redact::SecretRedactor;
use fabro_store::EventPayload;
use fabro_util::json::normalize_json_value;
use serde_json::Value;

pub fn build_redacted_event_payload(
    event: &RunEvent,
    run_id: &RunId,
    redactor: &SecretRedactor,
) -> Result<EventPayload> {
    let value = redacted_event_value(event, redactor)?;
    EventPayload::new(value, run_id).map_err(anyhow::Error::from)
}

pub fn redacted_event_json(event: &RunEvent, redactor: &SecretRedactor) -> Result<String> {
    serde_json::to_string(&redacted_event_value(event, redactor)?).map_err(anyhow::Error::from)
}

pub(super) fn redacted_run_event(event: &RunEvent, redactor: &SecretRedactor) -> Result<RunEvent> {
    let value = redacted_event_value(event, redactor)?;
    RunEvent::from_ref(&value).context("Failed to reparse redacted event payload")
}

fn normalized_event_value(event: &RunEvent) -> Result<Value> {
    let value = event.to_value()?;
    Ok(normalize_json_value(value))
}

fn redacted_event_value(event: &RunEvent, redactor: &SecretRedactor) -> Result<Value> {
    let mut value = fabro_redact::redact_json_value(normalized_event_value(event)?);
    redact_registered_secrets_in_event_value(&mut value, redactor);
    Ok(value)
}

fn redact_registered_secrets_in_event_value(value: &mut Value, redactor: &SecretRedactor) {
    if redactor.is_empty() {
        return;
    }

    if let Some(Value::String(node_label)) = value.get_mut("node_label") {
        let redacted = redactor.redact_into(node_label);
        if redacted != *node_label {
            *node_label = redacted;
        }
    }

    if let Some(properties) = value.get_mut("properties") {
        redact_registered_secrets_in_properties(properties, redactor);
    }
}

fn redact_registered_secrets_in_properties(value: &mut Value, redactor: &SecretRedactor) {
    match value {
        Value::Object(properties) => {
            for (key, child) in properties {
                // Registered values can be low-entropy words. Redacting them in
                // every event field would corrupt structural values such as ids,
                // enum strings, and event names. Until event-field redaction is
                // derived from typed field metadata, add new free-form text
                // properties here when they are introduced.
                if is_free_form_text_property(key) {
                    *child = redactor.redact_json(std::mem::take(child));
                } else {
                    redact_registered_secrets_in_properties(child, redactor);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_registered_secrets_in_properties(item, redactor);
            }
        }
        _ => {}
    }
}

fn is_free_form_text_property(key: &str) -> bool {
    matches!(
        key,
        // command/script I/O
        "command"
            | "script"
            | "stdout"
            | "stderr"
            | "output"
            | "input"
            | "arguments"
            | "exec_output_tail"
            | "tool_input"
            | "tool_output"
            // agent/LLM text
            | "prompt"
            | "response"
            | "answer"
            | "question"
            | "delta"
            | "text"
            | "message"
            | "reason"
            | "notes"
            | "preview"
            // errors
            | "error"
            | "error_message"
            | "failure"
            | "causes"
            | "details"
            | "description"
            // diffs/config
            | "diff"
            | "final_patch"
            | "workflow_config"
            | "workflow_source"
            // metadata text
            | "goal"
            | "subject"
            | "title"
    )
}

pub fn event_payload_from_redacted_json(line: &str, run_id: &RunId) -> Result<EventPayload> {
    let value = serde_json::from_str(line).context("Failed to parse redacted event payload")?;
    EventPayload::new(value, run_id).map_err(anyhow::Error::from)
}

#[cfg(test)]
mod tests {
    use ::fabro_types::{fixtures, run_event as fabro_types};
    use fabro_redact::SecretRedactor;

    use super::*;
    use crate::event::{Event, to_run_event};

    #[test]
    fn build_redacted_event_payload_requires_id() {
        let stored = to_run_event(&fixtures::RUN_8, &Event::RunSubmitted {
            definition_blob: None,
        });
        let payload =
            build_redacted_event_payload(&stored, &fixtures::RUN_8, &SecretRedactor::default())
                .unwrap();
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

        let payload =
            build_redacted_event_payload(&stored, &fixtures::RUN_8, &SecretRedactor::default())
                .unwrap();
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
    fn build_redacted_event_payload_redacts_declared_low_entropy_values() {
        let stored = to_run_event(&fixtures::RUN_8, &Event::RunNotice {
            level:            fabro_types::RunNoticeLevel::Warn,
            code:             "deploy".to_string(),
            message:          "deploy to staging".to_string(),
            exec_output_tail: None,
        });
        let redactor = SecretRedactor::default();
        redactor.register("staging");

        let payload = build_redacted_event_payload(&stored, &fixtures::RUN_8, &redactor).unwrap();
        let payload_text = serde_json::to_string(payload.as_value()).unwrap();

        assert!(!payload_text.contains("staging"));
        assert!(payload_text.contains("REDACTED"));
    }

    #[test]
    fn setup_failed_event_redacts_declared_low_entropy_values() {
        let stored = to_run_event(&fixtures::RUN_8, &Event::SetupFailed {
            command:          "deploy staging".to_string(),
            index:            0,
            exit_code:        7,
            stderr:           "failed in staging".to_string(),
            exec_output_tail: Some(fabro_types::ExecOutputTail {
                stdout:           None,
                stderr:           Some("tail staging".to_string()),
                stdout_truncated: false,
                stderr_truncated: false,
            }),
        });
        let redactor = SecretRedactor::default();
        redactor.register("staging");

        let payload = build_redacted_event_payload(&stored, &fixtures::RUN_8, &redactor).unwrap();
        let payload_text = serde_json::to_string(payload.as_value()).unwrap();

        assert_eq!(payload.as_value()["event"], "setup.failed");
        assert!(!payload_text.contains("staging"));
        assert!(payload_text.contains("REDACTED"));
    }

    #[test]
    fn exact_redaction_preserves_structural_fields() {
        let mut stored = to_run_event(&fixtures::RUN_8, &Event::RunNotice {
            level:            fabro_types::RunNoticeLevel::Warn,
            code:             "staging".to_string(),
            message:          "deploy to staging".to_string(),
            exec_output_tail: None,
        });
        stored.node_id = Some("staging".to_string());
        stored.node_label = Some("Deploy staging".to_string());
        let redactor = SecretRedactor::default();
        redactor.register("staging");

        let payload = build_redacted_event_payload(&stored, &fixtures::RUN_8, &redactor).unwrap();
        let redacted = RunEvent::from_ref(payload.as_value()).unwrap();

        assert!(
            redacted
                .node_id
                .as_deref()
                .is_some_and(|value| value == "staging")
        );
        assert!(
            redacted
                .node_label
                .as_deref()
                .is_some_and(|value| value == "Deploy REDACTED")
        );
        assert!(
            payload.as_value()["properties"]["code"]
                .as_str()
                .is_some_and(|value| value == "staging")
        );
        assert!(
            payload.as_value()["properties"]["message"]
                .as_str()
                .is_some_and(|value| value == "deploy to REDACTED")
        );
    }

    #[test]
    fn empty_registry_is_content_only_identity() {
        let secret = "sk-ant-api03-xK9mZ2vL8nQ5rT1wY4bC7dF0gH3jE6pA";
        let stored = to_run_event(&fixtures::RUN_8, &Event::RunNotice {
            level:            fabro_types::RunNoticeLevel::Warn,
            code:             "example".to_string(),
            message:          format!("token={secret}"),
            exec_output_tail: None,
        });
        let content_only =
            fabro_redact::redact_json_value(normalized_event_value(&stored).unwrap());
        let content_only_json = serde_json::to_string(&content_only).unwrap();

        let with_empty_registry = redacted_event_json(&stored, &SecretRedactor::default()).unwrap();

        assert_eq!(with_empty_registry, content_only_json);
    }

    #[test]
    fn exact_redaction_is_per_registry() {
        let stored = to_run_event(&fixtures::RUN_8, &Event::RunNotice {
            level:            fabro_types::RunNoticeLevel::Warn,
            code:             "example".to_string(),
            message:          "deploy staging production".to_string(),
            exec_output_tail: None,
        });
        let staging = SecretRedactor::default();
        staging.register("staging");
        let production = SecretRedactor::default();
        production.register("production");

        let staging_payload =
            build_redacted_event_payload(&stored, &fixtures::RUN_8, &staging).unwrap();
        let production_payload =
            build_redacted_event_payload(&stored, &fixtures::RUN_8, &production).unwrap();
        let staging_text = serde_json::to_string(staging_payload.as_value()).unwrap();
        let production_text = serde_json::to_string(production_payload.as_value()).unwrap();

        assert!(!staging_text.contains("staging"));
        assert!(staging_text.contains("production"));
        assert!(production_text.contains("staging"));
        assert!(!production_text.contains("production"));
    }
}
