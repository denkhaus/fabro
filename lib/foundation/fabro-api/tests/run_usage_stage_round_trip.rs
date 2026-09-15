use std::any::{TypeId, type_name};

use fabro_api::types::{RunUsageStage, Speed as ApiSpeed, UsageByModel, UsageModelRef};
use fabro_types::{ModelRef, StageState};
use lithos_llm::types::Speed;
use serde_json::{Value, json};

#[test]
fn usage_model_ref_reuses_domain_type() {
    assert_same_type::<UsageModelRef, ModelRef>();
    assert_same_type::<ApiSpeed, Speed>();
}

fn assert_same_type<A: 'static, B: 'static>() {
    assert_eq!(
        TypeId::of::<A>(),
        TypeId::of::<B>(),
        "{} should be the same type as {}",
        type_name::<A>(),
        type_name::<B>()
    );
}

fn zero_usage() -> Value {
    json!({
        "tokens": {
            "input": 0,
            "output": 0,
            "reasoning": 0,
            "cache_read": 0,
            "cache_write": 0
        }
    })
}

#[test]
fn run_usage_stage_model_accepts_required_null() {
    let value = json!({
        "stage": {
            "id": "start",
            "name": "start"
        },
        "model": null,
        "usage": zero_usage(),
        "timing": {"wall_time_ms": 0, "inference_time_ms": 0, "tool_time_ms": 0, "active_time_ms": 0}
    });
    assert_valid("RunUsageStage", &value);

    let stage: RunUsageStage =
        serde_json::from_value(value).expect("null stage model should deserialize");
    assert!(stage.model.is_none());

    let encoded = serde_json::to_value(stage).expect("stage should serialize");
    assert!(encoded.get("model").is_some());
    assert!(encoded["model"].is_null());
}

#[test]
fn run_usage_stage_round_trips_terminal_row_with_started_at_and_state() {
    let value = json!({
        "stage": {
            "id": "build",
            "name": "build"
        },
        "model": {
            "provider": "anthropic",
            "model_id": "claude-sonnet-4-5",
            "speed": "fast"
        },
        "usage": {
            "tokens": {
                "input": 12,
                "output": 34,
                "reasoning": 0,
                "cache_read": 0,
                "cache_write": 0
            },
            "cost": { "usd_micros": 123, "source": "catalog" }
        },
        "timing": {"wall_time_ms": 5500, "inference_time_ms": 0, "tool_time_ms": 0, "active_time_ms": 0},
        "started_at": "2026-04-29T12:34:56Z",
        "state": "succeeded"
    });
    assert_valid("RunUsageStage", &value);

    let stage: RunUsageStage =
        serde_json::from_value(value.clone()).expect("terminal stage row should deserialize");
    assert!(stage.started_at.is_some());
    assert_eq!(stage.state, Some(StageState::Succeeded));
    assert_eq!(stage.usage.cost.map(|cost| cost.usd_micros), Some(123));
    assert_eq!(serde_json::to_value(stage).unwrap(), value);
}

#[test]
fn usage_by_model_round_trips_provider_model_speed_identity() {
    let value = json!({
        "model": {
            "provider": "anthropic",
            "model_id": "claude-opus-4-6",
            "speed": "fast"
        },
        "stages": 2,
        "usage": {
            "tokens": {
                "input": 12,
                "output": 34,
                "reasoning": 0,
                "cache_read": 0,
                "cache_write": 0
            },
            "cost": { "usd_micros": 123, "source": "provider" }
        }
    });
    assert_valid("UsageByModel", &value);

    let row: UsageByModel =
        serde_json::from_value(value.clone()).expect("usage model ref should deserialize");
    assert_eq!(serde_json::to_value(row).unwrap(), value);
}

#[test]
fn run_usage_stage_round_trips_in_flight_row() {
    let value = json!({
        "stage": {
            "id": "build",
            "name": "build"
        },
        "model": null,
        "usage": zero_usage(),
        "timing": {"wall_time_ms": 1250, "inference_time_ms": 0, "tool_time_ms": 0, "active_time_ms": 0},
        "started_at": "2026-04-29T12:34:56Z",
        "state": "running"
    });
    assert_valid("RunUsageStage", &value);

    let stage: RunUsageStage =
        serde_json::from_value(value.clone()).expect("in-flight stage row should deserialize");
    assert!(stage.model.is_none());
    assert_eq!(stage.state, Some(StageState::Running));
    assert_eq!(serde_json::to_value(stage).unwrap(), value);
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
