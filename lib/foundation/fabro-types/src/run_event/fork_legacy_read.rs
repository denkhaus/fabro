//! Fork-only legacy read tolerance (extracted per ADR-0021 D7 in the
//! v0.357 merge; user directive 2026-09-15: fork features live in fork-only
//! files upstream cannot overwrite).
//!
//! Everything here rewrites pre-v0.354 stored rows into the current event
//! shapes so run-history activation replays real production stores without
//! hard errors. Upstream deliberately reads old shapes back with zero
//! usage; the fork keeps the data. This file is fork-only on purpose: an
//! upstream merge can no longer overwrite or drop the normalizers, and the
//! tests below are the presence pin for the whole class.

use serde_json::{Value, json};

/// Event names that no longer carry an `EventBody` variant but may still
/// appear in stored run history from before the sandbox-driver adoption
/// (upstream #849 removed the variants, the names stayed in
/// [`is_known_event_name`]). Reads must tolerate them as
/// [`EventBody::Unknown`] instead of aborting startup: run-history
/// activation replays every stored event, and one legacy name would
/// otherwise crash-loop the server on real production data
/// (2026-09-12, same class as the v0.353 billing normalizer). Delete
/// together with the other legacy normalizers once no pre-adoption store
/// can be read.
pub(super) fn is_legacy_variantless_event_name(event: &str) -> bool {
    matches!(
        event,
        "sandbox.git.started"
            | "sandbox.git.completed"
            | "sandbox.git.failed"
            | "sandbox.cleanup.started"
            | "sandbox.cleanup.completed"
            | "sandbox.cleanup.failed"
    )
}

/// Whether `event` names a payload-less CodingEvent and `properties` is a
/// flat pre-v0.354 row for it (no envelope `event` key; `{}` or
/// bookkeeping-only).
pub(super) fn is_legacy_payloadless_coding_row(event: &str, properties: &Value) -> bool {
    if !matches!(
        event,
        "agent.session.ended" | "agent.loop.detected" | "agent.processing.end"
    ) {
        return false;
    }
    properties
        .as_object()
        .is_some_and(|object| !object.contains_key("event"))
}

/// Whether `properties` is a flat pre-v0.354 agent event row: no envelope
/// `event` key, and either no fields at all (the payload-less events —
/// `agent.session.ended`, `agent.loop.detected` — stored `{}`) or at least
/// one field beyond the envelope's own `stage`/`visit`/`seq` bookkeeping.
/// Every event family the old writer stored carried its own props beside
/// those; a row that has ONLY the bookkeeping keys is a malformed envelope,
/// not a legacy row, and strict reading keeps rejecting it.
pub(super) fn is_legacy_flat_agent_row(properties: &Value) -> bool {
    let Some(object) = properties.as_object() else {
        return false;
    };
    if object.contains_key("event") {
        return false;
    }
    object.is_empty()
        || object
            .keys()
            .any(|key| !matches!(key.as_str(), "stage" | "visit" | "seq"))
}

/// Rewrite one flat legacy `agent.message` row into a
/// `CodingAgentEvent`-shaped envelope the current `EventBody::Agent`
/// reader accepts. Called from the value and parts readers alike, after
/// the property normalizers.
///
/// Legacy: `{text, model: {provider, model_id}, billing: {flat counts},
/// tool_call_count}`. Target: AssistantMessage with a `provider/model`
/// catalog id string, a pebble `TokenUsage`, and the cost total hoisted to
/// `cost_usd_micros`. Rows that already carry an `event` key are envelopes
/// and pass through untouched.
pub(super) fn normalize_legacy_agent_message(properties: &mut Value, timestamp: Value) {
    if !is_legacy_flat_agent_row(properties)
        || !properties
            .as_object()
            .is_some_and(|object| object.contains_key("text"))
    {
        return;
    }
    let Some(object) = properties.as_object_mut() else {
        return;
    };
    let text = object
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let model = match object.get("model") {
        Some(Value::String(model)) => model.clone(),
        Some(Value::Object(model)) => {
            // The legacy row carries the full ModelRef (provider + model_id);
            // the current AssistantMessage contract is the bare catalog model
            // id, with the provider sourced from the session-activation
            // evidence on the stage (run_state's stage_model_ref). Joining
            // "provider/model_id" here produced a wrong model name on replay
            // and aborted run-history activation on healthy stored rows
            // (observed live 2026-09-13, run 01M0NGQXB67674XQ5YCR1MB4BN).
            model
                .get("model_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        }
        _ => String::new(),
    };
    let mut usage = serde_json::Map::new();
    let mut cost_usd_micros = None;
    if let Some(billing) = object.get("billing").and_then(Value::as_object) {
        for (legacy, current) in [
            ("input_tokens", "input"),
            ("output_tokens", "output"),
            ("reasoning_tokens", "reasoning"),
            ("cache_read_tokens", "cache_read"),
            ("cache_write_tokens", "cache_write"),
        ] {
            if let Some(count) = billing.get(legacy).and_then(Value::as_u64) {
                usage.insert(current.to_string(), Value::from(count));
            }
        }
        cost_usd_micros = billing
            .get("total_usd_micros")
            .and_then(Value::as_u64)
            .or_else(|| {
                billing
                    .get("input")
                    .and_then(Value::as_object)
                    .and_then(|input| input.get("total_usd_micros"))
                    .and_then(Value::as_u64)
            });
    }
    let tool_call_count = object
        .get("tool_call_count")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let visit = object.get("visit").and_then(Value::as_u64).unwrap_or(1);
    let mut usage_value = serde_json::Map::new();
    usage_value.insert("tokens".to_string(), Value::Object(usage));
    if let Some(cost) = cost_usd_micros {
        // The legacy `estimated` source (or an explicit cost_source) is
        // rewritten to `catalog` by the cost-source pass below.
        let source = object
            .get("cost_source")
            .cloned()
            .unwrap_or_else(|| Value::String("catalog".to_string()));
        usage_value.insert(
            "cost".to_string(),
            json!({ "usd_micros": cost, "source": source }),
        );
    }
    let mut message = serde_json::Map::new();
    message.insert("text".to_string(), Value::String(text));
    message.insert("model".to_string(), Value::String(model));
    message.insert("usage".to_string(), Value::Object(usage_value));
    message.insert("tool_call_count".to_string(), Value::from(tool_call_count));
    let mut envelope = serde_json::Map::new();
    // AgentEventProps flattens the CodingAgentEvent beside `stage`/`visit`;
    // legacy rows carry `visit` but never `stage` (the old props had none),
    // so the stage is empty and the projection's node attribution comes
    // from the run's stage sequencing instead.
    envelope.insert("stage".to_string(), Value::String(String::new()));
    envelope.insert("visit".to_string(), Value::from(visit));
    envelope.insert("seq".to_string(), Value::from(0_u64));
    envelope.insert("stream_id".to_string(), Value::String(String::new()));
    envelope.insert(
        "event".to_string(),
        serde_json::json!({ "AssistantMessage": Value::Object(message) }),
    );
    envelope.insert("timestamp".to_string(), timestamp);
    envelope.insert("session_id".to_string(), Value::String(String::new()));
    *properties = Value::Object(envelope);
}

/// Rewrite the legacy lithos billing wrapper stored before v0.353:
///
/// ```json
/// {"input": {"usage": {"model": {"provider": "zai", "model_id": "glm-5.3"},
///                       "tokens": {"input_tokens": 1, ...}, "facts": ...}},
///  "total_usd_micros": 45294}
/// ```
///
/// into the shapes the current billing types expect: a `provider:model`
/// ModelRef string plus flat token counts. Run-history activation replays
/// every stored event; one legacy wrapper aborts server startup
/// (2026-09-11, v0.353.0 upgrade, fabro-7893 follow-up). Delete together
/// with the other legacy normalizers once no pre-v0.353 store can be read.
/// Older provider codecs could persist a negative disjoint bucket when a
/// detail count exceeded its inclusive parent total. The typed read model
/// (`lithos` TokenCounts) is nonnegative, and a negative would fail
/// deserialization and abort activation on real production stores — clamp
/// the legacy buckets at the JSON boundary instead. Same semantics the
/// read-model clamp applied pre-v0.357: caches fold into the input total,
/// reasoning absorbs a negative output, totals stay nonnegative.
pub(super) fn clamp_legacy_token_counts(map: &mut serde_json::Map<String, Value>) {
    let number = |map: &serde_json::Map<String, Value>, key: &str| {
        map.get(key).and_then(Value::as_i64).unwrap_or(0)
    };
    let input_total = (number(map, "input_tokens")
        + number(map, "cache_read_tokens")
        + number(map, "cache_write_tokens"))
    .max(0);
    let cache_read = number(map, "cache_read_tokens").clamp(0, input_total);
    let cache_write = number(map, "cache_write_tokens").clamp(0, input_total - cache_read);
    let input = input_total - cache_read - cache_write;

    let output_total = (number(map, "output_tokens") + number(map, "reasoning_tokens")).max(0);
    let reasoning = number(map, "reasoning_tokens").clamp(0, output_total);
    let output = output_total - reasoning;

    for (key, value) in [
        ("input_tokens", input),
        ("output_tokens", output),
        ("reasoning_tokens", reasoning),
        ("cache_read_tokens", cache_read),
        ("cache_write_tokens", cache_write),
    ] {
        map.insert(key.to_string(), Value::from(value));
    }
}

pub(super) fn normalize_legacy_billing(event: &str, value: &mut Value) {
    let Some(map) = value.as_object_mut() else {
        return;
    };
    let mut rename_billing_to_usage = false;
    for (key, item) in map.iter_mut() {
        if key != "billing" && key != "usage" {
            normalize_legacy_billing(event, item);
            continue;
        }
        let Some(wrapper) = item.as_object_mut() else {
            continue;
        };
        let Some(usage) = wrapper
            .get_mut("input")
            .and_then(|input| input.get_mut("usage"))
            .and_then(|usage| usage.as_object_mut())
        else {
            continue;
        };
        // billing::ModelRef (the struct) already matches the stored
        // {provider, model_id} object - keep it verbatim.
        let model_ref = usage.get("model").cloned();
        let Some(mut tokens) = usage.get("tokens").cloned() else {
            continue;
        };
        let total_usd_micros = wrapper.get("total_usd_micros").cloned();
        if let Some(map) = tokens.as_object_mut() {
            clamp_legacy_token_counts(map);
        }
        if event == "agent.message" {
            // The envelope's AssistantMessage carries lithos `Usage`
            // (TokenCounts + optional Cost): rename the legacy suffixed
            // buckets into it; the fork's catalog pricing made any legacy
            // total a catalog cost.
            let mut tokens_map = match tokens {
                Value::Object(map) => map,
                other => {
                    *item = other;
                    continue;
                }
            };
            for (from, to) in [
                ("input_tokens", "input"),
                ("output_tokens", "output"),
                ("reasoning_tokens", "reasoning"),
                ("cache_read_tokens", "cache_read"),
                ("cache_write_tokens", "cache_write"),
            ] {
                if let Some(value) = tokens_map.remove(from) {
                    tokens_map.insert(to.to_string(), value);
                }
            }
            let mut usage_out = serde_json::Map::new();
            usage_out.insert("tokens".to_string(), Value::Object(tokens_map));
            if let Some(total) = total_usd_micros {
                usage_out.insert(
                    "cost".to_string(),
                    json!({ "usd_micros": total, "source": "catalog" }),
                );
            }
            *item = Value::Object(usage_out);
            rename_billing_to_usage = true;
        } else {
            // ModelUsage: ModelRef plus lithos Usage, whose TokenCounts
            // field names dropped the `_tokens` suffix.
            let mut tokens_map = match tokens {
                Value::Object(map) => map,
                other => {
                    *item = other;
                    continue;
                }
            };
            for (from, to) in [
                ("input_tokens", "input"),
                ("output_tokens", "output"),
                ("reasoning_tokens", "reasoning"),
                ("cache_read_tokens", "cache_read"),
                ("cache_write_tokens", "cache_write"),
            ] {
                if let Some(value) = tokens_map.remove(from) {
                    tokens_map.insert(to.to_string(), value);
                }
            }
            // A usage entry without an extractable model cannot satisfy
            // ModelUsage's required ModelRef; null keeps the enclosing
            // Option field valid instead of failing deserialization.
            let Some(model_ref) = model_ref else {
                *item = Value::Null;
                continue;
            };
            // ModelUsage { model, usage: Usage { tokens, cost } }; the
            // fork's catalog pricing made any legacy total a catalog cost.
            let mut inner = serde_json::Map::new();
            inner.insert("tokens".to_string(), Value::Object(tokens_map));
            if let Some(total) = total_usd_micros {
                inner.insert(
                    "cost".to_string(),
                    json!({ "usd_micros": total, "source": "catalog" }),
                );
            }
            let mut usage_out = serde_json::Map::new();
            usage_out.insert("model".to_string(), model_ref);
            usage_out.insert("usage".to_string(), Value::Object(inner));
            *item = Value::Object(usage_out);
            rename_billing_to_usage = true;
        }
    }
    // The current field name is `usage` on every carrier (PromptCompleted,
    // stage outcomes, the pebble envelope); rename the key so the
    // transformed value lands where the deserializer now reads it.
    if rename_billing_to_usage {
        if let Some(billing) = map.remove("billing") {
            map.insert("usage".to_string(), billing);
        }
    }
}

/// Rewrite the removed `CostSource::Estimated` variant (`estimated`) to
/// `catalog`.
///
/// lithos-llm replaced `estimated` with `catalog`/`provider`/`application`
/// semantics; legacy events priced by Fabro from the model catalog carry
/// `estimated`, which the current deserializer rejects. Run-history
/// activation replays every stored event through the current types, so one
/// legacy value aborts server startup (2026-09-11, v0.353.0 upgrade).
/// Catalog pricing is the correct mapping for estimated costs.
///
/// Delete together with the other legacy normalizers once no pre-v0.353
/// event sources can be read anymore.
pub(super) const LEGACY_COST_SOURCE: &str = "estimated";
pub(super) const LEGACY_COST_SOURCE_REPLACEMENT: &str = "catalog";

pub(super) fn normalize_legacy_cost_source(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                if (key == "cost_source" || key == "source")
                    && item.as_str() == Some(LEGACY_COST_SOURCE)
                {
                    *item = Value::String(LEGACY_COST_SOURCE_REPLACEMENT.to_string());
                } else {
                    normalize_legacy_cost_source(item);
                }
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                normalize_legacy_cost_source(item);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use pebble_coding_agent::events::{CodingEvent, CostSource};

    use super::super::{EventBody, RunEvent};

    #[test]
    fn legacy_variantless_event_names_read_back_as_unknown() {
        // Pre-sandbox-driver stores (before upstream #849) wrote
        // sandbox.git.* / sandbox.cleanup.* progress events. The variants
        // are gone; the names stayed "known", so the strict read path
        // would abort startup on real production history
        // (2026-09-12). They must read back as Unknown instead.
        for name in [
            "sandbox.git.started",
            "sandbox.git.completed",
            "sandbox.git.failed",
            "sandbox.cleanup.started",
            "sandbox.cleanup.completed",
            "sandbox.cleanup.failed",
        ] {
            let line = format!(
                "{{\"id\":\"00000000-0000-0000-0000-000000000002\",\"ts\":\"2026-01-01T14:25:00Z\",                 \"run_id\":\"01M20DMYEK5B3GDQAYFR83DNGN\",\"event\":\"{name}\",\"properties\":{{}}}}"
            );
            let event = RunEvent::from_json_str(&line)
                .unwrap_or_else(|err| panic!("{name} should read back as Unknown: {err}"));
            assert!(
                matches!(event.body, EventBody::Unknown { .. }),
                "{name} should be Unknown, got {:?}",
                event.body
            );
        }
    }

    #[test]
    fn legacy_billing_wrapper_parses_through_prompt_completed() {
        // Exact stored row (run 01M20DMYEK, 2026-09-11 crash-loop incident):
        // the legacy lithos billing wrapper must normalize into
        // BilledModelUsage (ModelRef string + token counts).
        let raw = r#"{"id":"01a080e1-d4fa-7d82-afc5-b53695397b4c","ts":"2026-09-08T08:00:00Z","run_id":"01M20DMYEK5B3GDQAYFR83DNGN","event":"prompt.completed","properties":{"response":"{\"preferred_next_label\":\"Merge needed\"}","model":"glm-5.3","provider":"zai","billing":{"input":{"usage":{"model":{"provider":"zai","model_id":"glm-5.3"},"tokens":{"input_tokens":22308,"output_tokens":681,"reasoning_tokens":73,"cache_read_tokens":41344,"cache_write_tokens":0}}},"total_usd_micros":45294}}}"#;
        let event = RunEvent::from_value(
            serde_json::from_str::<serde_json::Value>(raw).expect("fixture should be valid JSON"),
        )
        .expect("legacy billing wrapper should normalize and parse");
        match event.body {
            EventBody::PromptCompleted(props) => {
                let usage = props.usage.expect("usage should survive normalization");
                assert_eq!(usage.model.provider.to_string(), "zai");
                assert_eq!(usage.model.model_id.to_string(), "glm-5.3");
                assert_eq!(usage.usage.tokens.input, 22308);
                let cost = usage
                    .usage
                    .cost
                    .expect("legacy total becomes a catalog cost");
                assert_eq!(cost.usd_micros, 45294);
                assert_eq!(cost.source, CostSource::Catalog);
            }
            other => panic!("expected PromptCompleted, got {other:?}"),
        }
    }

    #[test]
    fn legacy_negative_token_buckets_read_back_clamped() {
        // Pre-v0.357 codecs could persist a negative disjoint bucket (a
        // detail count exceeding its inclusive parent). The typed read
        // model is nonnegative; the clamp at the JSON boundary keeps real
        // stored rows parsing instead of aborting activation (read-model
        // clamp semantics, ported from the store layer in the v0.357
        // usage merge).
        let raw = r#"{"id":"01a080e1-d4fa-7d82-afc5-b53695397b4c","ts":"2026-09-08T08:00:00Z","run_id":"01M20DMYEK5B3GDQAYFR83DNGN","event":"prompt.completed","properties":{"response":"ok","model":"glm-5.3","provider":"zai","billing":{"input":{"usage":{"model":{"provider":"zai","model_id":"glm-5.3"},"tokens":{"input_tokens":53,"output_tokens":-7,"reasoning_tokens":66,"cache_read_tokens":0,"cache_write_tokens":0}}},"total_usd_micros":45294}}}"#;
        let event = RunEvent::from_value(
            serde_json::from_str::<serde_json::Value>(raw).expect("fixture should be valid JSON"),
        )
        .expect("legacy negative buckets clamp instead of failing activation");
        match event.body {
            EventBody::PromptCompleted(props) => {
                let usage = props.usage.expect("usage should survive normalization");
                assert_eq!(usage.usage.tokens.input, 53);
                assert_eq!(usage.usage.tokens.output, 0);
                assert_eq!(usage.usage.tokens.reasoning, 59);
            }
            other => panic!("expected PromptCompleted, got {other:?}"),
        }
    }

    #[test]
    fn legacy_billing_normalizer_leaves_current_agent_message_untouched() {
        // Stored agent.message rows already use the current shapes: since
        // the pebble adoption they are CodingAgentEvent envelopes whose
        // AssistantMessage carries a flat TokenUsage. The normalizer must
        // rewrite the pre-v0.354 flat row into that envelope (exact row
        // shape from the 2026-09-11 incident store).
        let raw = r#"{"id":"00000000-0000-0000-0000-000000000001","ts":"2026-09-08T08:00:00Z","run_id":"01M20DMYEK5B3GDQAYFR83DNGN","event":"agent.message","properties":{"text":"ok","model":{"provider":"zai","model_id":"glm-5.3"},"billing":{"input_tokens":10,"output_tokens":2,"total_tokens":12,"reasoning_tokens":0,"cache_read_tokens":0,"cache_write_tokens":0,"total_usd_micros":7},"tool_call_count":0,"visit":1}}"#;
        let event = RunEvent::from_value(
            serde_json::from_str::<serde_json::Value>(raw).expect("fixture should be valid JSON"),
        )
        .expect("legacy agent.message row should read back as an envelope");
        match event.body {
            EventBody::Agent(props) => match props.event.event {
                CodingEvent::AssistantMessage {
                    ref model,
                    ref usage,
                    ..
                } => {
                    // The bare catalog id, NOT "zai/glm-5.3": the fold's
                    // stage_model_ref pairs it with the session-activation
                    // provider, and a slash-joined name failed the
                    // run-history activation verification on healthy rows
                    // (2026-09-13, run 01M0NGQXB67674XQ5YCR1MB4BN).
                    assert_eq!(model, "glm-5.3");
                    assert_eq!(usage.tokens.input, 10);
                    assert_eq!(usage.tokens.output, 2);
                    let cost = usage.cost.expect("legacy total becomes a catalog cost");
                    assert_eq!(cost.usd_micros, 7);
                }
                other => panic!("expected AssistantMessage, got {other:?}"),
            },
            other => panic!("expected Agent, got {other:?}"),
        }
    }

    #[test]
    fn flat_legacy_coding_rows_degrade_to_unknown_instead_of_failing() {
        // A flat pre-v0.354 agent.* row that has no targeted rewriter (a
        // tool event) must not abort run-history activation: the row reads
        // back as Unknown, the same tolerance variantless names got in
        // v0.353.
        let raw = r#"{"id":"00000000-0000-0000-0000-000000000003","ts":"2026-09-08T08:00:00Z","run_id":"01M20DMYEK5B3GDQAYFR83DNGN","event":"agent.tool.started","properties":{"call":{"id":"call_1","name":"read_file","arguments":{"file_path":"/w/x"}}}}"#;
        let event = RunEvent::from_value(
            serde_json::from_str::<serde_json::Value>(raw).expect("fixture should be valid JSON"),
        )
        .expect("flat legacy tool row should degrade to Unknown");
        assert!(
            matches!(event.body, EventBody::Unknown { .. }),
            "expected Unknown, got {:?}",
            event.body
        );
    }
}
