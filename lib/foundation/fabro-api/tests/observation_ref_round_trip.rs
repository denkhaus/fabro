//! JSON parity and type-identity tests for `ObservationRef`
//! (ADR-0044, the filing chain's wire type).
//!
//! Asserts that the API-side name is the canonical `fabro-types` type
//! (via `with_replacement` in `build.rs` — no parallel DTO, no
//! adapter) and that the camelCase wire shape the OpenAPI schema
//! promises is exactly what the canonical type serializes.

use fabro_api::types::ObservationRef as ApiObservationRef;
use fabro_types::ObservationRef;
use serde_json::json;

/// Type-identity witness: the generated API name is the canonical Rust
/// type, not a parallel DTO. Compiles only when they are the same type.
#[expect(dead_code, reason = "compile-time type-identity witness")]
fn observation_ref_type_identity(value: ApiObservationRef) -> ObservationRef {
    value
}

#[test]
fn observation_ref_round_trips_the_wire_shape() {
    let canonical = ObservationRef {
        run_id: "01M7DSR5FS6WXSEGT1JZRNHR3K"
            .parse()
            .expect("a parseable run id"),
        seq:    13,
    };
    // The camelCase wire the OpenAPI schema names (runId, seq) — the
    // canonical type owns the wire shape, no case-flipping adapter.
    let wire = json!({
        "runId": "01M7DSR5FS6WXSEGT1JZRNHR3K",
        "seq": 13
    });
    let serialized = serde_json::to_value(&canonical).expect("canonical serializes");
    assert_eq!(serialized, wire);
    let parsed: ObservationRef =
        serde_json::from_value(wire).expect("canonical parses the wire value");
    assert_eq!(parsed, canonical);
}
