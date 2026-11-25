//! `fabro_session_read` (ADR-0046 phase 2, denkhaus/work#1679): the
//! supervisor's read of ONE Ask Fabro session's transcript — folded and
//! compacted server-side (the frontend replay's semantics) under a token
//! budget, so a whole conversation lands in one bounded tool result.

use std::fmt::Write as _;
use std::sync::Arc;

use fabro_store::session_transcript::{DEFAULT_MAX_TOKENS, MAX_MAX_TOKENS};
use schemars::JsonSchema;
use serde::Deserialize;

use super::common::{FabroToolBackend, ToolError, ToolResult};

/// The budget's bounds: below the floor a transcript carries no usable
/// context; above the ceiling the result stops being compacted.
const MIN_MAX_TOKENS: u64 = 200;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FabroSessionReadParams {
    /// The session id (from fabro_session_search or the session index).
    pub session_id: String,
    /// The transcript's token budget (200-50000, default 4000). Messages
    /// are included whole, in order, until the budget is exhausted; the
    /// rest is dropped and counted.
    pub max_tokens: Option<u64>,
}

impl FabroSessionReadParams {
    /// Validate and normalize: an unknown-shaped session id is refused
    /// with the teaching error; the budget clamps to the door's bounds.
    pub fn validated(self) -> ToolResult<ValidatedSessionRead> {
        let session_id = self.session_id.trim().to_string();
        session_id
            .parse::<fabro_types::SessionId>()
            .map_err(|err| {
                ToolError::message(format!(
                    "session_id must be a session id from fabro_session_search: {err}"
                ))
            })?;
        Ok(ValidatedSessionRead {
            session_id,
            max_tokens: self
                .max_tokens
                .unwrap_or(DEFAULT_MAX_TOKENS)
                .clamp(MIN_MAX_TOKENS, MAX_MAX_TOKENS),
        })
    }
}

#[derive(Debug)]
pub struct ValidatedSessionRead {
    pub session_id: String,
    pub max_tokens: u64,
}

pub async fn read_session_transcript(
    backend: Arc<dyn FabroToolBackend>,
    params: ValidatedSessionRead,
) -> ToolResult<fabro_types::SessionTranscript> {
    backend
        .read_session_transcript(&params.session_id, params.max_tokens)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))
}

/// The transcript rendering: the header (anchor, title, budget verdict),
/// then the conversation, then the counts. The JSON payload carries the
/// structured form beside it.
pub fn read_session_transcript_text(transcript: &fabro_types::SessionTranscript) -> String {
    let title = transcript.title.as_deref().unwrap_or("(untitled)");
    let anchor = transcript
        .run_id
        .map_or_else(|| "runless".to_string(), |run_id| run_id.to_string());
    let mut rendered = format!(
        "Session {} — run {anchor} — \"{}\" — {} message(s)",
        transcript.session_id,
        title,
        transcript.messages.len()
    );
    let _ = write!(
        rendered,
        "\nTool calls: {} ({} errored)",
        transcript.tool_calls.total, transcript.tool_calls.errors
    );
    if transcript.budget.truncated {
        let _ = write!(
            rendered,
            "\nBudget truncated the transcript: {} of {} tokens used, {} message(s) omitted \
             past the cap.",
            transcript.budget.used_tokens,
            transcript.budget.max_tokens,
            transcript.budget.omitted_messages
        );
    }
    if transcript.messages.is_empty() {
        let _ = write!(rendered, "\nThe session has no recorded conversation.");
        return rendered;
    }
    rendered.push_str("\n\nTranscript:");
    for message in &transcript.messages {
        let _ = write!(rendered, "\n\n[{:?}] {}", message.role, message.text);
    }
    rendered
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn params(session_id: &str, max_tokens: Option<u64>) -> FabroSessionReadParams {
        FabroSessionReadParams {
            session_id: session_id.to_string(),
            max_tokens,
        }
    }

    #[test]
    fn a_session_id_validates_and_defaults_the_budget() {
        let session_id = fabro_types::SessionId::new().to_string();
        let validated = params(&session_id, None)
            .validated()
            .expect("a session id validates");
        assert_eq!(validated.session_id, session_id);
        assert_eq!(validated.max_tokens, DEFAULT_MAX_TOKENS);
    }

    #[test]
    fn an_unknown_shaped_id_is_refused_with_the_teaching_error() {
        let error = params("not-a-session-id", None)
            .validated()
            .expect_err("a non-id must fail");
        let message = error.as_str();
        assert!(
            message.contains("fabro_session_search"),
            "the refusal points at the search: {message}"
        );
    }

    #[test]
    fn the_budget_clamps_to_the_door_bounds() {
        let session_id = fabro_types::SessionId::new().to_string();
        let low = params(&session_id, Some(1))
            .validated()
            .expect("a low budget clamps");
        assert_eq!(low.max_tokens, MIN_MAX_TOKENS);
        let high = params(&session_id, Some(1_000_000))
            .validated()
            .expect("a high budget clamps");
        assert_eq!(high.max_tokens, MAX_MAX_TOKENS);
    }

    #[test]
    fn the_text_renders_header_counts_and_messages() {
        let transcript = fabro_types::SessionTranscript {
            session_id: fabro_types::SessionId::new(),
            run_id:     None,
            title:      Some("Ask Fabro".to_string()),
            messages:   vec![
                fabro_types::SessionTranscriptMessage {
                    role: fabro_types::SessionTranscriptRole::User,
                    text: "What happened?".to_string(),
                    seq:  2,
                    ts:   chrono::Utc::now(),
                },
                fabro_types::SessionTranscriptMessage {
                    role: fabro_types::SessionTranscriptRole::Assistant,
                    text: "All good.".to_string(),
                    seq:  4,
                    ts:   chrono::Utc::now(),
                },
            ],
            tool_calls: fabro_types::SessionTranscriptToolCalls {
                total:   3,
                errors:  1,
                by_tool: BTreeMap::from([("read_file".to_string(), 3)]),
            },
            budget:     fabro_types::SessionTranscriptBudget {
                max_tokens:         4000,
                used_tokens:        8,
                truncated:          false,
                omitted_messages:   0,
                omitted_tool_calls: 0,
            },
        };
        let text = read_session_transcript_text(&transcript);
        assert!(text.contains("run runless"));
        assert!(text.contains("Tool calls: 3 (1 errored)"));
        assert!(text.contains("What happened?"));
        assert!(text.contains("All good."));
        assert!(
            !text.contains("Budget truncated"),
            "an untruncated transcript says nothing about the cap: {text}"
        );
    }

    #[test]
    fn the_text_names_the_truncation_when_the_budget_cut() {
        let session_id = fabro_types::SessionId::new();
        let transcript = fabro_types::SessionTranscript {
            session_id,
            run_id: None,
            title: None,
            messages: Vec::new(),
            tool_calls: fabro_types::SessionTranscriptToolCalls::new(),
            budget: fabro_types::SessionTranscriptBudget {
                max_tokens:         10,
                used_tokens:        10,
                truncated:          true,
                omitted_messages:   4,
                omitted_tool_calls: 7,
            },
        };
        let text = read_session_transcript_text(&transcript);
        assert!(text.contains("Budget truncated"));
        assert!(text.contains("4 message(s) omitted"));
    }
}
