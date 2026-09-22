//! `fabro_ask`: ask one question to the Ask-Fabro analyst of another run
//! and wait for the final answer (ADR-0011).
//!
//! Carried to the Petri run-tools registry (fabro-43cf): the rebuild
//! catalog had dropped the tool while production graphs kept declaring
//! `x.fabro_tools="fabro_ask"`, and `register_named_fabro_run_tools`
//! silently ignored the unknown name — a rebuild deploy stripped the
//! revisor's core instrument with zero errors.

use std::sync::Arc;

use fabro_types::{SessionEvent, SessionEventBody};
use schemars::JsonSchema;
use serde::Serialize;

use super::common::{FabroToolBackend, ToolError, ToolResult};

const MAX_QUESTION_CHARS: usize = 8000;

/// Outcome of an Ask-Fabro turn, derived from the turn's terminal event.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    JsonSchema,
    strum::Display,
    strum::EnumString,
    strum::VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum AskTurnStatus {
    /// The analyst produced a final answer.
    Succeeded,
    /// The turn was interrupted before an answer completed.
    Interrupted,
    /// The turn failed or the stream ended without a terminal event.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AskTurnOutcome {
    pub status: AskTurnStatus,
    /// The analyst's final assistant message (empty unless `Succeeded`).
    pub answer: String,
    /// Terminal error text for `Failed` turns.
    pub error:  Option<String>,
}

#[derive(Debug, serde::Deserialize, JsonSchema)]
pub struct FabroAskParams {
    /// Target run selector (run id or unique prefix).
    pub run_id:   String,
    /// Single question for the Ask-Fabro analyst of the target run.
    pub question: String,
}

#[derive(Debug)]
pub struct ValidatedAsk {
    pub run_id:   String,
    pub question: String,
}

impl TryFrom<FabroAskParams> for ValidatedAsk {
    type Error = ToolError;

    fn try_from(params: FabroAskParams) -> Result<Self, Self::Error> {
        let run_id = params.run_id.trim();
        if run_id.is_empty() {
            return Err(ToolError::message("run_id is required"));
        }
        let question = params.question.trim();
        if question.is_empty() {
            return Err(ToolError::message("question is required"));
        }
        if question.chars().count() > MAX_QUESTION_CHARS {
            return Err(ToolError::message(format!(
                "question must be at most {MAX_QUESTION_CHARS} characters"
            )));
        }
        Ok(Self {
            run_id:   run_id.to_string(),
            question: question.to_string(),
        })
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AskResult {
    pub run_id:     String,
    pub session_id: String,
    pub status:     AskTurnStatus,
    pub answer:     String,
    pub error:      Option<String>,
}

/// Ask one question to the Ask-Fabro analyst of another run.
///
/// The server decides which actors may open sessions; this tool runs on
/// the stage worker's backend, whose token carries the server's
/// run-tool authority.
pub async fn ask_run(
    backend: Arc<dyn FabroToolBackend>,
    params: ValidatedAsk,
) -> ToolResult<AskResult> {
    let run = backend
        .resolve_run(&params.run_id)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))?;
    let title = session_title(&params.question);
    let session_id = backend
        .create_ask_session(&run.id, &title)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))?;
    let outcome = backend
        .submit_ask_turn(&run.id, &session_id, &params.question)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))?;
    match outcome.status {
        AskTurnStatus::Failed => {
            return Err(ToolError::message(
                outcome
                    .error
                    .unwrap_or_else(|| "Ask-Fabro turn failed".to_string()),
            ));
        }
        AskTurnStatus::Interrupted => {
            return Err(ToolError::message(
                "Ask-Fabro turn was interrupted before an answer completed",
            ));
        }
        AskTurnStatus::Succeeded => {}
    }
    Ok(AskResult {
        run_id: run.id.to_string(),
        session_id,
        status: outcome.status,
        answer: outcome.answer,
        error: outcome.error,
    })
}

pub fn ask_run_text(result: &AskResult) -> String {
    format!("asked Fabro run {}", result.run_id)
}

/// Incremental collector for Ask-Fabro turn events: feeds
/// [`SessionEvent`]s as they stream in and produces the final
/// [`AskTurnOutcome`]. Separated from the transport so the terminal
/// classification is unit-testable.
#[derive(Debug, Default)]
pub struct AskTurnCollector {
    answer: Option<String>,
    status: Option<AskTurnStatus>,
    error:  Option<String>,
}

impl AskTurnCollector {
    /// Absorb one streamed event. Non-terminal events are ignored.
    pub fn absorb(&mut self, event: &SessionEvent) {
        match &event.body {
            SessionEventBody::AssistantMessage(props) => {
                self.answer = Some(props.text.clone());
            }
            SessionEventBody::TurnSucceeded(_) => {
                self.status = Some(AskTurnStatus::Succeeded);
            }
            SessionEventBody::TurnInterrupted(_) => {
                self.status = Some(AskTurnStatus::Interrupted);
            }
            SessionEventBody::TurnFailed(props) => {
                self.status = Some(AskTurnStatus::Failed);
                self.error = Some(props.error.clone());
            }
            _ => {}
        }
    }

    /// Produce the outcome; a stream that ended without a terminal event
    /// counts as failed.
    #[must_use]
    pub fn finish(mut self) -> AskTurnOutcome {
        let status = self.status.take().unwrap_or_else(|| {
            self.error =
                Some("session turn ended before a terminal event was received".to_string());
            AskTurnStatus::Failed
        });
        let answer = if status == AskTurnStatus::Succeeded {
            self.answer.unwrap_or_default()
        } else {
            String::new()
        };
        AskTurnOutcome {
            status,
            answer,
            error: self.error,
        }
    }
}

fn session_title(question: &str) -> String {
    const MAX_CHARS: usize = 80;
    let trimmed = question.trim();
    if trimmed.chars().count() <= MAX_CHARS {
        return trimmed.to_string();
    }
    let mut title = trimmed.chars().take(MAX_CHARS - 3).collect::<String>();
    title.push_str("...");
    title
}

#[cfg(test)]
mod tests {
    use fabro_types::{SessionEvent, TurnId};

    use super::{
        AskTurnCollector, AskTurnOutcome, AskTurnStatus, FabroAskParams, MAX_QUESTION_CHARS,
        ValidatedAsk,
    };

    /// Build a session event from its wire JSON (the flattened
    /// `event`/`properties` shape the SSE stream serves), so the
    /// collector test pins the shape it consumes.
    fn wire_event(event: &str, properties: serde_json::Value) -> SessionEvent {
        let turn_id = TurnId::new();
        let mut properties = properties;
        properties["turn_id"] = serde_json::json!(turn_id.to_string());
        serde_json::from_value(serde_json::json!({
            "seq": 1,
            "session_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "run_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "ts": "1970-01-01T00:00:00Z",
            "event": event,
            "properties": properties,
        }))
        .expect("wire event should deserialize")
    }

    fn assistant_event(text: &str) -> SessionEvent {
        wire_event(
            "run.session.assistant_message",
            serde_json::json!({ "text": text }),
        )
    }

    #[test]
    fn validate_rejects_blank_run_id_and_question() {
        let err = ValidatedAsk::try_from(FabroAskParams {
            run_id:   "  ".to_string(),
            question: "what happened?".to_string(),
        })
        .unwrap_err();
        assert!(err.as_str().contains("run_id"), "{}", err.as_str());

        let err = ValidatedAsk::try_from(FabroAskParams {
            run_id:   "01J".to_string(),
            question: "   ".to_string(),
        })
        .unwrap_err();
        assert!(err.as_str().contains("question"), "{}", err.as_str());
    }

    #[test]
    fn validate_caps_question_length_and_trims() {
        let long = "x".repeat(MAX_QUESTION_CHARS + 1);
        let err = ValidatedAsk::try_from(FabroAskParams {
            run_id:   "01J".to_string(),
            question: long,
        })
        .unwrap_err();
        assert!(err.as_str().contains("8000"), "{}", err.as_str());

        let params = ValidatedAsk::try_from(FabroAskParams {
            run_id:   " 01J ".to_string(),
            question: "  why did the reviewer fail?  ".to_string(),
        })
        .unwrap();
        assert_eq!(params.run_id, "01J");
        assert_eq!(params.question, "why did the reviewer fail?");
    }

    #[test]
    fn collector_classifies_terminal_events() {
        // Succeeded: the last assistant message wins.
        let mut collector = AskTurnCollector::default();
        collector.absorb(&assistant_event("partial"));
        collector.absorb(&assistant_event("the gate timed out"));
        collector.absorb(&wire_event(
            "run.session.turn.succeeded",
            serde_json::json!({}),
        ));
        assert_eq!(collector.finish(), AskTurnOutcome {
            status: AskTurnStatus::Succeeded,
            answer: "the gate timed out".to_string(),
            error:  None,
        });

        // Failed: error text is preserved, the answer is dropped.
        let mut collector = AskTurnCollector::default();
        collector.absorb(&assistant_event("partial"));
        collector.absorb(&wire_event(
            "run.session.turn.failed",
            serde_json::json!({ "error": "llm unavailable" }),
        ));
        assert_eq!(collector.finish(), AskTurnOutcome {
            status: AskTurnStatus::Failed,
            answer: String::new(),
            error:  Some("llm unavailable".to_string()),
        });

        // Interrupted: no answer.
        let mut collector = AskTurnCollector::default();
        collector.absorb(&wire_event(
            "run.session.turn.interrupted",
            serde_json::json!({}),
        ));
        assert_eq!(collector.finish(), AskTurnOutcome {
            status: AskTurnStatus::Interrupted,
            answer: String::new(),
            error:  None,
        });

        // The stream ended without a terminal event: failed with a
        // diagnosis.
        let mut collector = AskTurnCollector::default();
        collector.absorb(&assistant_event("dangling"));
        let outcome = collector.finish();
        assert_eq!(outcome.status, AskTurnStatus::Failed);
        assert_eq!(outcome.answer, "");
        assert!(
            outcome
                .error
                .as_deref()
                .is_some_and(|error| error.contains("terminal"))
        );
    }

    #[test]
    fn ask_status_uses_snake_case_wire_names() {
        assert_eq!(AskTurnStatus::Succeeded.to_string(), "succeeded");
        assert_eq!(
            serde_json::to_value(AskTurnStatus::Interrupted).unwrap(),
            serde_json::json!("interrupted")
        );
    }

    #[test]
    fn session_title_truncates_long_questions() {
        assert_eq!(super::session_title("short question"), "short question");
        let long = "y".repeat(200);
        let title = super::session_title(&long);
        assert_eq!(title.chars().count(), 80);
        assert!(title.ends_with("..."));
    }
}
