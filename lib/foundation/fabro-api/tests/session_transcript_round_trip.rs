//! JSON parity and type-identity tests for the session archaeology
//! transcript types (ADR-0046 phase 2, denkhaus/work#1679):
//! `SessionTranscript` and its message, role, tool-call and budget
//! parts.
//!
//! Asserts that the API-side names are the canonical `fabro-types` types
//! (via `with_replacement` in `build.rs` — no parallel DTO) and that the
//! snake_case wire shape the OpenAPI schema promises is exactly what the
//! canonical types serialize.

use fabro_api::types::{
    SessionTranscript as ApiSessionTranscript,
    SessionTranscriptBudget as ApiSessionTranscriptBudget,
    SessionTranscriptMessage as ApiSessionTranscriptMessage,
    SessionTranscriptRole as ApiSessionTranscriptRole,
    SessionTranscriptToolCalls as ApiSessionTranscriptToolCalls,
};
use fabro_types::{
    SessionTranscript, SessionTranscriptBudget, SessionTranscriptMessage, SessionTranscriptRole,
    SessionTranscriptToolCalls,
};
use serde_json::json;

/// Type-identity witness: the generated API names are the canonical Rust
/// types, not parallel DTOs. Compiles only when they are the same types.
#[expect(dead_code, reason = "compile-time type-identity witness")]
fn session_transcript_type_identity(
    transcript: ApiSessionTranscript,
    message: ApiSessionTranscriptMessage,
    role: ApiSessionTranscriptRole,
    tool_calls: ApiSessionTranscriptToolCalls,
    budget: ApiSessionTranscriptBudget,
) -> (
    SessionTranscript,
    SessionTranscriptMessage,
    SessionTranscriptRole,
    SessionTranscriptToolCalls,
    SessionTranscriptBudget,
) {
    (transcript, message, role, tool_calls, budget)
}

#[test]
fn session_transcript_message_round_trips_the_wire_shape() {
    let wire = json!({
        "role": "user",
        "text": "What happened to the run?",
        "seq": 2,
        "ts": "2026-11-25T12:00:00Z",
    });
    let parsed: SessionTranscriptMessage =
        serde_json::from_value(wire.clone()).expect("the message shape parses");
    assert_eq!(parsed.role, SessionTranscriptRole::User);
    assert_eq!(serde_json::to_value(&parsed).unwrap(), wire);
}

#[test]
fn session_transcript_tool_calls_round_trip_the_wire_shape() {
    let wire = json!({
        "total": 3,
        "errors": 1,
        "by_tool": { "fabro_run_events": 2, "read_file": 1 },
    });
    let parsed: SessionTranscriptToolCalls =
        serde_json::from_value(wire.clone()).expect("the tool-call shape parses");
    assert_eq!(parsed.total, 3);
    assert_eq!(parsed.errors, 1);
    assert_eq!(serde_json::to_value(&parsed).unwrap(), wire);
}

#[test]
fn session_transcript_budget_round_trips_the_wire_shape() {
    let wire = json!({
        "max_tokens": 4000,
        "used_tokens": 3810,
        "truncated": true,
        "omitted_messages": 2,
        "omitted_tool_calls": 5,
    });
    let parsed: SessionTranscriptBudget =
        serde_json::from_value(wire.clone()).expect("the budget shape parses");
    assert!(parsed.truncated);
    assert_eq!(serde_json::to_value(&parsed).unwrap(), wire);
}

#[test]
fn session_transcript_round_trips_the_wire_shape() {
    let transcript = SessionTranscript {
        session_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap(),
        run_id:     None,
        title:      Some("Ask Fabro".to_string()),
        messages:   vec![
            SessionTranscriptMessage {
                role: SessionTranscriptRole::User,
                text: "What happened?".to_string(),
                seq:  2,
                ts:   "2026-11-25T12:00:00Z".parse().unwrap(),
            },
            SessionTranscriptMessage {
                role: SessionTranscriptRole::Assistant,
                text: "The run finished.".to_string(),
                seq:  4,
                ts:   "2026-11-25T12:00:05Z".parse().unwrap(),
            },
        ],
        tool_calls: SessionTranscriptToolCalls::new(),
        budget:     SessionTranscriptBudget {
            max_tokens:         4000,
            used_tokens:        8,
            truncated:          false,
            omitted_messages:   0,
            omitted_tool_calls: 0,
        },
    };
    let wire = serde_json::to_value(&transcript).unwrap();
    assert_eq!(wire["run_id"], serde_json::Value::Null);
    assert_eq!(wire["messages"][0]["role"], "user");
    assert_eq!(wire["messages"][1]["role"], "assistant");
    let parsed: SessionTranscript = serde_json::from_value(wire).expect("the transcript parses");
    assert_eq!(parsed, transcript);
}
