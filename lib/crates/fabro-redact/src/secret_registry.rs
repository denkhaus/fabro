use std::sync::{Arc, PoisonError, RwLock};

use serde_json::Value;

use crate::Region;

/// Per-run registry of exact secret values to redact from strings and JSON.
///
/// This complements the crate's content-based redaction by redacting registered
/// values even when they do not look like credentials. Clones share the same
/// registry so callers can hand a redactor to another subsystem and continue to
/// register values through the original.
#[derive(Clone, Default)]
pub struct SecretRedactor {
    values: Arc<RwLock<Vec<String>>>,
}

impl SecretRedactor {
    /// Register a secret value for exact substring redaction.
    ///
    /// Empty or whitespace-only values are ignored so an accidental empty
    /// registration cannot redact every output boundary.
    pub fn register(&self, value: impl Into<String>) {
        let value = value.into();
        if value.trim().is_empty() {
            return;
        }

        let mut values = self.values.write().unwrap_or_else(PoisonError::into_inner);
        if !values.iter().any(|registered| registered == &value) {
            values.push(value);
        }
    }

    /// Return `true` when no secret values have been registered.
    pub fn is_empty(&self) -> bool {
        self.values
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    }

    /// Redact all registered secret values from `s`.
    pub fn redact_into(&self, s: &str) -> String {
        let values = self.registered_values_longest_first();
        redact_string_values(s, &values)
    }

    /// Redact registered secret values from every JSON string leaf.
    ///
    /// Object keys are left unchanged.
    pub fn redact_json(&self, mut value: Value) -> Value {
        let values = self.registered_values_longest_first();
        if values.is_empty() {
            return value;
        }

        redact_json_value(&mut value, &values);
        value
    }

    fn registered_values_longest_first(&self) -> Vec<String> {
        let values = self.values.read().unwrap_or_else(PoisonError::into_inner);
        let mut values = values.clone();
        values.sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
        values
    }
}

fn redact_json_value(value: &mut Value, values: &[String]) {
    match value {
        Value::Object(obj) => {
            for child in obj.values_mut() {
                redact_json_value(child, values);
            }
        }
        Value::Array(arr) => {
            for child in arr {
                redact_json_value(child, values);
            }
        }
        Value::String(text) => {
            let redacted = redact_string_values(text, values);
            if redacted != *text {
                *text = redacted;
            }
        }
        _ => {}
    }
}

fn redact_string_values(s: &str, values: &[String]) -> String {
    if values.is_empty() {
        return s.to_string();
    }

    let mut regions = Vec::new();
    for value in values {
        for (start, _) in s.match_indices(value) {
            let end = start + value.len();
            if !regions
                .iter()
                .any(|region: &Region| regions_overlap(start, end, region))
            {
                regions.push(Region { start, end });
            }
        }
    }

    if regions.is_empty() {
        return s.to_string();
    }

    regions.sort_by_key(|region| region.start);

    let mut result = String::with_capacity(s.len());
    let mut previous = 0;
    for region in &regions {
        result.push_str(&s[previous..region.start]);
        result.push_str(crate::REDACTION_MARKER);
        previous = region.end;
    }
    result.push_str(&s[previous..]);
    result
}

fn regions_overlap(start: usize, end: usize, region: &Region) -> bool {
    start < region.end && region.start < end
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::SecretRedactor;

    #[test]
    fn redacts_registered_low_entropy_value() {
        let redactor = SecretRedactor::default();
        redactor.register("staging");

        assert_eq!(
            crate::redact_string("deploy to staging"),
            "deploy to staging"
        );
        assert_eq!(
            redactor.redact_into("deploy to staging"),
            "deploy to REDACTED"
        );
    }

    #[test]
    fn ignores_empty_and_whitespace_values() {
        let redactor = SecretRedactor::default();
        redactor.register("");
        redactor.register("   ");

        assert_eq!(
            redactor.redact_into("deploy to staging"),
            "deploy to staging"
        );
    }

    #[test]
    fn redacts_overlapping_values_longest_first() {
        let redactor = SecretRedactor::default();
        redactor.register("abc");
        redactor.register("abcdef");

        assert_eq!(redactor.redact_into("token=abcdef"), "token=REDACTED");
    }

    #[test]
    fn empty_registry_is_identity() {
        let redactor = SecretRedactor::default();
        let value = json!({
            "env": "staging",
            "items": ["staging", 42],
        });

        assert_eq!(
            redactor.redact_into("deploy to staging"),
            "deploy to staging"
        );
        assert_eq!(redactor.redact_json(value.clone()), value);
        assert!(redactor.is_empty());
    }

    #[test]
    fn redact_json_redacts_nested_object_values_and_array_elements() {
        let redactor = SecretRedactor::default();
        redactor.register("staging");
        let value = json!({
            "environment": "staging",
            "items": [
                "keep",
                "deploy staging now"
            ],
            "staging": "object keys are not redacted",
        });

        assert_eq!(
            redactor.redact_json(value),
            json!({
                "environment": "REDACTED",
                "items": [
                    "keep",
                    "deploy REDACTED now"
                ],
                "staging": "object keys are not redacted",
            })
        );
    }

    #[test]
    fn clones_share_registered_values() {
        let redactor = SecretRedactor::default();
        let clone = redactor.clone();

        redactor.register("staging");

        assert_eq!(clone.redact_into("deploy to staging"), "deploy to REDACTED");
    }
}
