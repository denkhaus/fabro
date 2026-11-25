//! The compacted transcript of an Ask Fabro session (ADR-0046 phase 2,
//! denkhaus/work#1679): the recorded `run.session.*` events folded
//! server-side into user and assistant messages, with the session's tool
//! calls summarized as counts and a token budget applied. The fold is the
//! server-side port of the frontend transcript replay — a resumed
//! transcript reads exactly like the conversation did.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use strum::{Display, EnumString, IntoStaticStr};

use crate::{RunId, SessionId};

/// Which side of the conversation a transcript message holds.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum SessionTranscriptRole {
    User,
    Assistant,
}

/// One folded transcript message, in transcript order. `seq` is the
/// sequence number of the event the message folded from and `ts` its
/// recorded time — the anchors a reader needs to walk the raw events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionTranscriptMessage {
    pub role: SessionTranscriptRole,
    pub text: String,
    pub seq:  u32,
    pub ts:   DateTime<Utc>,
}

/// The session's tool-call noise, summarized: every recorded call counted
/// per tool name, with the error count beside it. Tool calls never enter
/// the message list and never consume budget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTranscriptToolCalls {
    pub total:   u64,
    pub errors:  u64,
    pub by_tool: BTreeMap<String, u64>,
}

impl SessionTranscriptToolCalls {
    #[must_use]
    pub fn new() -> Self {
        Self {
            total:   0,
            errors:  0,
            by_tool: BTreeMap::new(),
        }
    }
}

impl Default for SessionTranscriptToolCalls {
    fn default() -> Self {
        Self::new()
    }
}

/// How the token budget was applied to the transcript: the cap, the
/// estimated spend of the messages kept, and what was dropped when the
/// cap cut the transcript short.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTranscriptBudget {
    pub max_tokens:         u64,
    pub used_tokens:        u64,
    pub truncated:          bool,
    pub omitted_messages:   u64,
    pub omitted_tool_calls: u64,
}

/// One session's compacted transcript. `run_id` is `None` for a runless
/// session (work#1677) — the global supervisor's conversations carry no
/// run anchor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionTranscript {
    pub session_id: SessionId,
    pub run_id:     Option<RunId>,
    pub title:      Option<String>,
    pub messages:   Vec<SessionTranscriptMessage>,
    pub tool_calls: SessionTranscriptToolCalls,
    pub budget:     SessionTranscriptBudget,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SessionTranscript {
        SessionTranscript {
            session_id: SessionId::new(),
            run_id:     None,
            title:      Some("Ask Fabro".to_string()),
            messages:   vec![SessionTranscriptMessage {
                role: SessionTranscriptRole::User,
                text: "What happened?".to_string(),
                seq:  2,
                ts:   Utc::now(),
            }],
            tool_calls: SessionTranscriptToolCalls::new(),
            budget:     SessionTranscriptBudget {
                max_tokens:         4000,
                used_tokens:        4,
                truncated:          false,
                omitted_messages:   0,
                omitted_tool_calls: 0,
            },
        }
    }

    #[test]
    fn transcript_round_trips_the_wire_shape() {
        let transcript = sample();
        let value = serde_json::to_value(&transcript).unwrap();
        assert_eq!(value["messages"][0]["role"], "user");
        let parsed: SessionTranscript = serde_json::from_value(value).unwrap();
        assert_eq!(parsed, transcript);
    }

    #[test]
    fn role_serializes_snake_case() {
        assert_eq!(
            serde_json::to_value(SessionTranscriptRole::Assistant).unwrap(),
            "assistant"
        );
        assert_eq!(
            "assistant".parse::<SessionTranscriptRole>().unwrap(),
            SessionTranscriptRole::Assistant
        );
    }
}
