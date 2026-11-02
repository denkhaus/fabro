//! JSON parity and type-identity tests for `WorkItemComment`
//! (ADR-0039 stage D part 2, the answer channel's wire type).
//!
//! Asserts that the API-side name is the canonical `fabro-types` type
//! (via `with_replacement` in `build.rs` — no parallel DTO, no
//! `api_comment` adapter) and that the camelCase wire shape the
//! OpenAPI schema promises is exactly what the canonical type
//! serializes.

use fabro_api::types::WorkItemComment as ApiWorkItemComment;
use fabro_types::WorkItemComment;
use serde_json::json;

/// Type-identity witness: the generated API name is the canonical Rust
/// type, not a parallel DTO. Compiles only when they are the same type.
#[expect(dead_code, reason = "compile-time type-identity witness")]
fn work_item_comment_type_identity(value: ApiWorkItemComment) -> WorkItemComment {
    value
}

#[test]
fn work_item_comment_round_trips_the_wire_shape() {
    let canonical = WorkItemComment {
        author:     "denkhaus".to_string(),
        body:       "exempt — the residual ships as-is".to_string(),
        created_at: Some(
            "2026-11-02T18:00:00Z"
                .parse::<chrono::DateTime<chrono::Utc>>()
                .expect("a parseable timestamp"),
        ),
        url:        Some("https://example.invalid/c/1".to_string()),
    };
    // The camelCase wire the OpenAPI schema names (createdAt, url) —
    // the adapter this replacement retired existed only to case-flip
    // these fields; the canonical type now owns the wire shape.
    let wire = json!({
        "author": "denkhaus",
        "body": "exempt — the residual ships as-is",
        "createdAt": "2026-11-02T18:00:00Z",
        "url": "https://example.invalid/c/1"
    });
    let serialized = serde_json::to_value(&canonical).expect("canonical serializes");
    assert_eq!(serialized, wire);
    let parsed: WorkItemComment =
        serde_json::from_value(wire).expect("canonical parses the wire value");
    assert_eq!(parsed, canonical);
}

#[test]
fn work_item_comment_skips_absent_optionals() {
    // The forge names no time and no permalink: both optionals vanish
    // from the wire (skip_serializing_if on the generated schema's
    // Option fields).
    let bare = WorkItemComment {
        author:     "adjudicator-bot".to_string(),
        body:       "parked-fork: residual file — unfinished or exempt?".to_string(),
        created_at: None,
        url:        None,
    };
    let serialized = serde_json::to_value(&bare).expect("bare serializes");
    assert!(serialized.get("createdAt").is_none());
    assert!(serialized.get("url").is_none());
    let parsed: WorkItemComment = serde_json::from_value(serialized).expect("bare parses back");
    assert_eq!(parsed, bare);
}
