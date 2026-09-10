//! Normalization of provider reasoning material into [`ReasoningOutput`].
//!
//! Every provider that returns readable reasoning does it differently, and
//! several return more than one channel at once. This module reduces a final
//! response's content parts to the two normalized fields without reaching
//! into opaque material (signatures, item ids, encrypted payloads) and
//! without failing a completion it cannot classify.
//!
//! Parsing is deliberately tolerant: provider payloads are read as
//! `serde_json::Value` with optional lookups, so unknown detail variants,
//! missing members, extra members, and unexpected member types are ignored
//! rather than surfaced as errors.

use fabro_types::{ContentPart, ReasoningOutput};

/// OpenAI Responses reasoning items, as lithos stores them.
pub const OPENAI_REASONING_KIND: &str = "openai.reasoning";
/// OpenAI Responses message items, as lithos stores them.
pub const OPENAI_MESSAGE_KIND: &str = "openai.message";
/// OpenAI-compatible `reasoning_details` arrays, as lithos stores them.
pub const OPENAI_COMPAT_REASONING_DETAILS_KIND: &str = "openai_compatible.reasoning_details";

/// Separator between distinct complete reasoning blocks.
const BLOCK_SEPARATOR: &str = "\n\n";

#[derive(Default)]
struct Blocks<'a> {
    explicit_summary: Vec<&'a str>,
    explicit_trace:   Vec<&'a str>,
    fallback_trace:   Vec<&'a str>,
}

impl Blocks<'_> {
    fn into_output(self) -> Option<ReasoningOutput> {
        let summary = join_blocks(&self.explicit_summary);
        let trace = join_blocks(&self.explicit_trace)
            .or_else(|| join_blocks(&self.fallback_trace))
            .filter(|trace| summary.as_ref() != Some(trace));

        match (summary, trace) {
            (Some(summary), Some(trace)) => Some(ReasoningOutput::new(summary, trace)),
            (Some(summary), None) => Some(ReasoningOutput::from_summary(summary)),
            (None, Some(trace)) => Some(ReasoningOutput::from_trace(trace)),
            (None, None) => None,
        }
    }
}

fn join_blocks(blocks: &[&str]) -> Option<String> {
    (!blocks.is_empty()).then(|| blocks.join(BLOCK_SEPARATOR))
}

fn push_block<'a>(blocks: &mut Vec<&'a str>, block: &'a str) {
    if !block.trim().is_empty() {
        blocks.push(block);
    }
}

fn readable_member<'a>(entry: &'a serde_json::Value, member: &str) -> Option<&'a str> {
    entry.get(member).and_then(serde_json::Value::as_str)
}

fn collect_openai_reasoning_item<'a>(item: &'a serde_json::Value, blocks: &mut Blocks<'a>) {
    if let Some(entries) = item.get("summary").and_then(serde_json::Value::as_array) {
        for entry in entries {
            if let Some(text) = entry.as_str() {
                push_block(&mut blocks.explicit_summary, text);
            } else if let Some(text) = entry.get("text").and_then(serde_json::Value::as_str) {
                push_block(&mut blocks.explicit_summary, text);
            }
        }
    }
    if let Some(entries) = item.get("content").and_then(serde_json::Value::as_array) {
        for entry in entries {
            let Some(text) = entry.get("text").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let entry_type = entry
                .get("type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if entry_type == "reasoning_text" {
                push_block(&mut blocks.explicit_trace, text);
            }
        }
    }
}

fn collect_reasoning_details<'a>(details: &'a serde_json::Value, blocks: &mut Blocks<'a>) {
    let Some(entries) = details.as_array() else {
        return;
    };
    for entry in entries {
        let detail_type = entry
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        match detail_type {
            "reasoning.text" => {
                if let Some(text) = readable_member(entry, "text") {
                    push_block(&mut blocks.explicit_trace, text);
                }
            }
            "reasoning.summary" => {
                if let Some(text) = readable_member(entry, "summary") {
                    push_block(&mut blocks.explicit_summary, text);
                }
            }
            _ => {}
        }
    }
}

/// Normalizes the content parts of a final response into readable reasoning.
///
/// Returns `None` when the response carries no readable reasoning.
#[must_use]
pub fn normalize(content: &[ContentPart]) -> Option<ReasoningOutput> {
    let mut blocks = Blocks::default();
    for part in content {
        match part {
            ContentPart::Reasoning(reasoning) if !reasoning.redacted => {
                push_block(&mut blocks.fallback_trace, &reasoning.text);
            }
            ContentPart::Opaque { kind, data } if kind == OPENAI_REASONING_KIND => {
                collect_openai_reasoning_item(data, &mut blocks);
            }
            ContentPart::Opaque { kind, data } if kind == OPENAI_COMPAT_REASONING_DETAILS_KIND => {
                collect_reasoning_details(data, &mut blocks);
            }
            _ => {}
        }
    }
    blocks.into_output()
}

/// Whether a part is provider-native replay material Fabro keeps in history
/// but never renders.
#[must_use]
pub fn is_provider_part(part: &ContentPart) -> bool {
    matches!(part, ContentPart::Reasoning(_) | ContentPart::Opaque { .. })
}

/// Whether a part is an OpenAI Responses item tied to one specific API
/// response. Such items become invalid once compaction replaces their
/// surrounding context.
#[must_use]
pub fn is_opaque_openai(part: &ContentPart) -> bool {
    matches!(
        part,
        ContentPart::Opaque { kind, .. }
            if kind == OPENAI_REASONING_KIND || kind == OPENAI_MESSAGE_KIND
    )
}

#[cfg(test)]
mod tests {
    use fabro_types::ReasoningContent;
    use serde_json::json;

    use super::*;

    fn thinking(text: &str) -> ContentPart {
        ContentPart::Reasoning(ReasoningContent {
            text:             text.to_string(),
            signature:        None,
            signature_origin: None,
            redacted:         false,
        })
    }

    fn openai_reasoning(item: serde_json::Value) -> ContentPart {
        ContentPart::opaque(OPENAI_REASONING_KIND, item)
    }

    fn reasoning_details(details: serde_json::Value) -> ContentPart {
        ContentPart::opaque(OPENAI_COMPAT_REASONING_DETAILS_KIND, details)
    }

    #[test]
    fn non_redacted_thinking_becomes_a_trace() {
        let output = normalize(&[thinking("weighing the options")]).unwrap();
        assert!(output.summary().is_none());
        assert_eq!(output.trace(), Some("weighing the options"));
    }

    #[test]
    fn redacted_thinking_yields_no_readable_reasoning() {
        let redacted = ContentPart::Reasoning(ReasoningContent {
            text:             "AAAAopaque".to_string(),
            signature:        Some("sig".to_string()),
            signature_origin: Some("anthropic".to_string()),
            redacted:         true,
        });
        assert!(normalize(&[redacted]).is_none());
    }

    #[test]
    fn responses_item_with_summary_and_reasoning_text_produces_both_fields() {
        let output = normalize(&[openai_reasoning(json!({
            "type": "reasoning",
            "id": "rs_1",
            "encrypted_content": "gAAAAA",
            "summary": [{"type": "summary_text", "text": "inspect first"}],
            "content": [{"type": "reasoning_text", "text": "step one"}],
        }))])
        .unwrap();
        assert_eq!(output.summary(), Some("inspect first"));
        assert_eq!(output.trace(), Some("step one"));
    }

    #[test]
    fn responses_blocks_join_in_provider_order() {
        let output = normalize(&[openai_reasoning(json!({
            "summary": [
                {"type": "summary_text", "text": "first"},
                {"type": "summary_text", "text": "second"},
            ],
        }))])
        .unwrap();
        assert_eq!(output.summary(), Some("first\n\nsecond"));
    }

    #[test]
    fn structured_details_produce_summary_and_trace() {
        let output = normalize(&[reasoning_details(json!([
            {"type": "reasoning.summary", "summary": "checked the parser"},
            {"type": "reasoning.text", "text": "read convert.rs", "signature": "sig"},
            {"type": "reasoning.encrypted", "data": "gAAAAAsecret"},
        ]))])
        .unwrap();
        assert_eq!(output.summary(), Some("checked the parser"));
        assert_eq!(output.trace(), Some("read convert.rs"));
    }

    #[test]
    fn malformed_details_are_ignored_without_failing() {
        assert!(normalize(&[reasoning_details(json!("not-an-array"))]).is_none());
        assert!(
            normalize(&[reasoning_details(json!([
                42,
                {"type": "reasoning.summary", "summary": 7},
                {"no_type": true},
            ]))])
            .is_none()
        );
    }

    #[test]
    fn structured_details_suppress_a_duplicate_flattened_value() {
        let output = normalize(&[
            reasoning_details(json!([
                {"type": "reasoning.summary", "summary": "checked the parser"},
            ])),
            thinking("checked the parser"),
        ])
        .unwrap();
        assert_eq!(output.summary(), Some("checked the parser"));
        assert!(output.trace().is_none());
    }

    #[test]
    fn structured_trace_takes_precedence_over_flattened_trace() {
        let output = normalize(&[
            reasoning_details(json!([{"type": "reasoning.text", "text": "verbatim"}])),
            thinking("flattened"),
        ])
        .unwrap();
        assert_eq!(output.trace(), Some("verbatim"));
    }

    #[test]
    fn whitespace_only_fragments_do_not_create_reasoning() {
        assert!(normalize(&[thinking("   \n ")]).is_none());
        let output = normalize(&[thinking("  indented thought\n")]).unwrap();
        assert_eq!(output.trace(), Some("  indented thought\n"));
    }

    #[test]
    fn opaque_openai_items_are_recognized() {
        assert!(is_opaque_openai(&openai_reasoning(json!({}))));
        assert!(is_opaque_openai(&ContentPart::opaque(
            OPENAI_MESSAGE_KIND,
            json!({})
        )));
        assert!(!is_opaque_openai(&thinking("x")));
        assert!(is_provider_part(&thinking("x")));
        assert!(!is_provider_part(&ContentPart::Text {
            text: "x".to_string(),
        }));
    }
}
