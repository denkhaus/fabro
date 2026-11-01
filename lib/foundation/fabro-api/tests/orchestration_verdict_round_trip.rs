//! JSON parity and type-identity tests for `OrchestrationVerdict`
//! (ADR-0039 orchestration fields).
//!
//! Asserts that the API-side `OrchestrationVerdict` is the canonical
//! Rust type (via `with_replacement` in `build.rs`) and that both names
//! round-trip through the same JSON shape: the three verdict values,
//! and that `pending` is deliberately absent (it is the ABSENCE of a
//! verdict, never a wire value).

use fabro_api::types::OrchestrationVerdict as ApiOrchestrationVerdict;
use fabro_types::OrchestrationVerdict;
use serde_json::json;

/// Type-identity witness: the generated API name is the canonical Rust
/// type, not a parallel DTO. Compiles only when they are the same type.
#[expect(dead_code, reason = "compile-time type-identity witness")]
fn verdict_type_identity(value: ApiOrchestrationVerdict) -> OrchestrationVerdict {
    value
}

#[test]
fn orchestration_verdict_round_trips_every_value() {
    for (value, wire) in [
        (OrchestrationVerdict::Parked, json!("parked")),
        (OrchestrationVerdict::Enabled, json!("enabled")),
        (OrchestrationVerdict::Refused, json!("refused")),
    ] {
        let canonical = serde_json::to_value(value).expect("canonical serializes");
        assert_eq!(canonical, wire);
        let api: ApiOrchestrationVerdict =
            serde_json::from_value(wire.clone()).expect("api type parses the wire value");
        assert_eq!(serde_json::to_value(api).expect("api serializes"), wire);
    }
}

#[test]
fn orchestration_verdict_refuses_pending_on_the_wire() {
    // `pending` is the ABSENCE of a record for the field (ADR-0039),
    // never a wire value; a `pending` string must not decode.
    assert!(
        serde_json::from_value::<OrchestrationVerdict>(json!("pending")).is_err(),
        "pending is not a verdict value"
    );
}
