//! JSON parity and type-identity tests for the Ask Fabro capability flip
//! types (ADR-0046, denkhaus/work#1451): `AskFabroCapability` and
//! `SetSessionCapabilityRequest`.
//!
//! Asserts that the API-side names are the canonical `fabro-types` types
//! (via `with_replacement` in `build.rs` — no parallel DTO) and that the
//! snake_case wire shape the OpenAPI schema promises is exactly what the
//! canonical types serialize.

use fabro_api::types::{
    AskFabroCapability as ApiAskFabroCapability,
    SetSessionCapabilityRequest as ApiSetSessionCapabilityRequest,
};
use fabro_types::{AskFabroCapability, SetSessionCapabilityRequest};
use serde_json::json;

/// Type-identity witness: the generated API names are the canonical Rust
/// types, not parallel DTOs. Compiles only when they are the same types.
#[expect(dead_code, reason = "compile-time type-identity witness")]
fn ask_fabro_capability_type_identity(
    capability: ApiAskFabroCapability,
    request: ApiSetSessionCapabilityRequest,
) -> (AskFabroCapability, SetSessionCapabilityRequest) {
    (capability, request)
}

#[test]
fn ask_fabro_capability_round_trips_the_wire_shape() {
    let wire = json!("full_access");
    let parsed: AskFabroCapability =
        serde_json::from_value(wire.clone()).expect("full_access parses");
    assert_eq!(parsed, AskFabroCapability::FullAccess);
    assert_eq!(serde_json::to_value(parsed).unwrap(), wire);

    let wire = json!("read_only");
    let parsed: AskFabroCapability =
        serde_json::from_value(wire.clone()).expect("read_only parses");
    assert_eq!(parsed, AskFabroCapability::ReadOnly);
    assert_eq!(serde_json::to_value(parsed).unwrap(), wire);
}

#[test]
fn set_session_capability_request_round_trips_the_wire_shape() {
    let request = SetSessionCapabilityRequest {
        capability: AskFabroCapability::FullAccess,
    };
    let wire = json!({"capability": "full_access"});
    assert_eq!(serde_json::to_value(request).unwrap(), wire);
    let parsed: SetSessionCapabilityRequest =
        serde_json::from_value(wire).expect("the flip body parses");
    assert_eq!(parsed, request);
}
