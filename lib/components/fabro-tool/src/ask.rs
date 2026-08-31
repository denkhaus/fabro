use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::common::{FabroToolBackend, ToolError, ToolResult};

/// Outcome of an Ask-Fabro turn, derived from the turn's terminal event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FabroAskParams {
    /// Target run selector (run id or unique prefix).
    pub run_id:   String,
    /// Single question for the Ask-Fabro analyst of the target run.
    pub question: String,
}

const MAX_QUESTION_CHARS: usize = 8000;

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
/// `inspects` is the caller's declared authority (ADR-0011): the target
/// run's workflow slug must appear in it. The server re-checks the scope;
/// this check rejects obvious violations before the round trip.
pub async fn ask_run(
    backend: Arc<dyn FabroToolBackend>,
    params: ValidatedAsk,
    inspects: &[String],
) -> ToolResult<AskResult> {
    if inspects.is_empty() {
        return Err(ToolError::message(
            "fabro_ask requires a graph that declares inspects",
        ));
    }
    let run = backend
        .resolve_run(&params.run_id)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))?;
    let allowed = run
        .workflow
        .slug
        .as_deref()
        .is_some_and(|slug| inspects.iter().any(|declared| declared == slug));
    if !allowed {
        return Err(ToolError::message(format!(
            "run {} is not in this workflow's inspects scope",
            run.id
        )));
    }
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
/// [`RunEvent`] names and properties as they stream in and produces the
/// final [`AskTurnOutcome`]. Separated from the transport so the terminal
/// classification is unit-testable.
#[derive(Debug, Default)]
pub struct AskTurnCollector {
    answer: Option<String>,
    status: Option<AskTurnStatus>,
    error:  Option<String>,
}

impl AskTurnCollector {
    /// Absorb one streamed event. Unknown events are ignored.
    pub fn absorb(&mut self, event_name: &str, properties: &serde_json::Value) {
        match event_name {
            "run.session.assistant_message" => {
                if let Some(text) = properties.get("text").and_then(serde_json::Value::as_str) {
                    self.answer = Some(text.to_string());
                }
            }
            "run.session.turn.succeeded" => {
                self.status = Some(AskTurnStatus::Succeeded);
            }
            "run.session.turn.interrupted" => {
                self.status = Some(AskTurnStatus::Interrupted);
            }
            "run.session.turn.failed" => {
                self.status = Some(AskTurnStatus::Failed);
                if let Some(error) = properties.get("error").and_then(serde_json::Value::as_str) {
                    self.error = Some(error.to_string());
                }
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
    use super::*;

    #[test]
    fn validate_rejects_blank_run_id_and_question() {
        let err = ValidatedAsk::try_from(FabroAskParams {
            run_id:   "  ".to_string(),
            question: "what happened?".to_string(),
        })
        .unwrap_err();
        assert!(err.as_str().contains("run_id"));

        let err = ValidatedAsk::try_from(FabroAskParams {
            run_id:   "01J".to_string(),
            question: "   ".to_string(),
        })
        .unwrap_err();
        assert!(err.as_str().contains("question"));
    }

    #[test]
    fn validate_caps_question_length_and_trims() {
        let long = "x".repeat(MAX_QUESTION_CHARS + 1);
        let err = ValidatedAsk::try_from(FabroAskParams {
            run_id:   "01J".to_string(),
            question: long,
        })
        .unwrap_err();
        assert!(err.as_str().contains("8000"));

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
        // Succeeded: last assistant message wins.
        let mut collector = AskTurnCollector::default();
        collector.absorb(
            "run.session.assistant_message",
            &serde_json::json!({ "text": "partial" }),
        );
        collector.absorb(
            "run.session.assistant_message",
            &serde_json::json!({ "text": "the gate timed out" }),
        );
        collector.absorb("run.session.turn.succeeded", &serde_json::json!({}));
        assert_eq!(collector.finish(), AskTurnOutcome {
            status: AskTurnStatus::Succeeded,
            answer: "the gate timed out".to_string(),
            error:  None,
        });

        // Failed: error text is preserved, answer dropped.
        let mut collector = AskTurnCollector::default();
        collector.absorb(
            "run.session.assistant_message",
            &serde_json::json!({ "text": "partial" }),
        );
        collector.absorb(
            "run.session.turn.failed",
            &serde_json::json!({ "error": "llm unavailable" }),
        );
        assert_eq!(collector.finish(), AskTurnOutcome {
            status: AskTurnStatus::Failed,
            answer: String::new(),
            error:  Some("llm unavailable".to_string()),
        });

        // Interrupted: no answer.
        let mut collector = AskTurnCollector::default();
        collector.absorb("run.session.turn.interrupted", &serde_json::json!({}));
        assert_eq!(collector.finish(), AskTurnOutcome {
            status: AskTurnStatus::Interrupted,
            answer: String::new(),
            error:  None,
        });

        // Stream ended without a terminal event: failed with a diagnosis.
        let mut collector = AskTurnCollector::default();
        collector.absorb(
            "run.session.assistant_message",
            &serde_json::json!({ "text": "dangling" }),
        );
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
    fn session_title_truncates_long_questions() {
        assert_eq!(session_title("short question"), "short question");
        let long = "y".repeat(200);
        let title = session_title(&long);
        assert_eq!(title.chars().count(), 80);
        assert!(title.ends_with("..."));
    }
}
