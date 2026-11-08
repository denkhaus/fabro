//! JSON parity and type-identity tests for `OrchestrationVerdict` and
//! `OrchestrationField` (ADR-0039 orchestration fields).
//!
//! Asserts that the API-side names are the canonical Rust types (via
//! `with_replacement` in `build.rs`) and that both round-trip through
//! the same JSON shape: the three verdict values (and that `pending` is
//! deliberately absent — it is the ABSENCE of a verdict, never a wire
//! value) and the two fields.

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

use fabro_api::types::OrchestrationField as ApiOrchestrationField;
use fabro_types::OrchestrationField;
/// Type-identity witness for the field enum (the same
/// `with_replacement` contract as the verdict).
#[expect(dead_code, reason = "compile-time type-identity witness")]
fn field_type_identity(value: ApiOrchestrationField) -> OrchestrationField {
    value
}

#[test]
fn orchestration_field_round_trips_both_fields() {
    for (value, wire) in [
        (OrchestrationField::Merge, json!("merge")),
        (OrchestrationField::Closable, json!("closable")),
    ] {
        let canonical = serde_json::to_value(value).expect("canonical serializes");
        assert_eq!(canonical, wire);
        let api: ApiOrchestrationField =
            serde_json::from_value(wire.clone()).expect("api type parses the wire value");
        assert_eq!(serde_json::to_value(api).expect("api serializes"), wire);
    }
}

use fabro_api::types::RunObservation;
use fabro_types::ObservationCategory;

/// The generated `RunObservation.category` IS the canonical ADR-0038
/// genus type (`with_replacement`): this construction compiles only
/// when the field's type is `fabro_types::ObservationCategory`, not a
/// parallel DTO.
fn observation_with_category(category: ObservationCategory) -> RunObservation {
    RunObservation {
        category,
        seq: 0,
        text: String::new(),
        execution: None,
        firing: None,
        // Adjacent repair (pre-existing drift, verified on the untouched
        // tree): the spec grew `stage` and `linked_work_items`
        // (denkhaus/work#1098's observations table) without this
        // initializer following.
        stage: None,
        linked_work_items: Vec::new(),
        recorded_at: None,
    }
}

#[test]
fn run_observation_category_round_trips_both_categories() {
    for (value, wire) in [
        (ObservationCategory::Painpoint, json!("painpoint")),
        (ObservationCategory::Note, json!("note")),
    ] {
        let canonical = serde_json::to_value(value).expect("canonical serializes");
        assert_eq!(canonical, wire);
        let observation = observation_with_category(value);
        let as_json = serde_json::to_value(&observation).expect("the observation serializes");
        assert_eq!(
            as_json["category"], wire,
            "the generated struct carries the canonical category on the wire"
        );
        let parsed: RunObservation =
            serde_json::from_value(as_json).expect("the observation parses back");
        assert_eq!(parsed.category, value);
    }
}
