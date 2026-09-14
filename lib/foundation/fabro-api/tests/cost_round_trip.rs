use std::any::{TypeId, type_name};

use fabro_api::types::{Cost as ApiCost, CostSource as ApiCostSource};
use lithos_llm::types::{Cost, CostSource};
use serde_json::{Value, json};

#[test]
fn cost_types_reuse_lithos_types() {
    assert_same_type::<ApiCostSource, CostSource>();
    assert_same_type::<ApiCost, Cost>();
}

#[test]
fn cost_source_json_matches_openapi_shape() {
    for (source, wire) in [
        (CostSource::Catalog, "catalog"),
        (CostSource::Provider, "provider"),
        (CostSource::Application, "application"),
    ] {
        assert_eq!(serde_json::to_value(source).unwrap(), json!(wire));
        assert_valid("CostSource", &json!(wire));
        assert_eq!(
            serde_json::from_value::<ApiCostSource>(json!(wire)).unwrap(),
            source
        );
    }
}

#[test]
fn cost_json_matches_openapi_shape() {
    let cost = Cost {
        usd_micros: 125_000,
        source:     CostSource::Provider,
    };
    let json = serde_json::to_value(cost).unwrap();
    assert_eq!(json, json!({"usd_micros": 125000, "source": "provider"}));
    assert_valid("Cost", &json);
    assert_eq!(serde_json::from_value::<ApiCost>(json).unwrap(), cost);
}

fn assert_same_type<T: 'static, U: 'static>() {
    assert_eq!(
        TypeId::of::<T>(),
        TypeId::of::<U>(),
        "{} should be the same type as {}",
        type_name::<T>(),
        type_name::<U>()
    );
}

#[expect(
    clippy::disallowed_methods,
    reason = "a synchronous test reads the spec from the repository once"
)]
fn spec() -> Value {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../docs/public/api-reference/fabro-api.yaml"
    ))
    .expect("the OpenAPI spec is in the repository");
    let yaml: serde_yaml::Value = serde_yaml::from_str(&text).expect("the spec parses");
    serde_json::to_value(yaml).expect("the spec is JSON-compatible")
}

fn assert_valid(schema_name: &str, value: &Value) {
    let mut root = spec();
    root["$ref"] = json!(format!("#/components/schemas/{schema_name}"));
    let validator = jsonschema::validator_for(&root).expect("the spec compiles as a JSON schema");
    let errors: Vec<String> = validator
        .iter_errors(value)
        .map(|error| format!("{error} at {}", error.instance_path()))
        .collect();
    assert!(
        errors.is_empty(),
        "{schema_name} rejects {value:#}:\n{}",
        errors.join("\n")
    );
}
