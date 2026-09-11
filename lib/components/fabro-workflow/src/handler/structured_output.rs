use std::fmt::Write as _;
use std::sync::{Arc, LazyLock};

use fabro_graphviz::graph::{Graph, Node};
use fabro_llm::types::ResponseFormat;
use jsonschema::error::ValidationErrorKind;
use jsonschema::paths::Location;
use jsonschema::{ValidationError, Validator};
use serde_json::Value;

use crate::error::Error;
use crate::graph::routing::normalize_label;
use crate::outcome::{FailureCategory, FailureDetail, Outcome, StageOutcome};

pub(crate) const ROUTING_KEYWORD: &str = "routing";

pub(crate) const ROUTING_STATUS_FIELDS: &[&str] = &[
    "preferred_next_label",
    "outcome",
    "failure_reason",
    "suggested_next_ids",
    "context_updates",
];

const QUOTED_ROUTING_STATUS_FIELDS: &[&str] = &[
    "\"preferred_next_label\"",
    "\"outcome\"",
    "\"failure_reason\"",
    "\"suggested_next_ids\"",
    "\"context_updates\"",
];

/// Parsed `output_schema` declaration with a precompiled validator so that
/// repair turns don't recompile the schema on every iteration.
#[derive(Debug, Clone)]
pub(crate) enum OutputSchemaKind {
    /// Labels of the node's preferred-label-routable outgoing edges:
    /// unconditional labels (matched by the preferred-label loop in
    /// `graph::routing::select_edge`) plus labels of conditional edges whose
    /// condition routes on `preferred_label` naming that label (matched by
    /// the condition evaluator, fabro-27dc). Empty when no edge information
    /// is available, in which case the label stays free-form (fabro-de4d).
    Routing { allowed_labels: Vec<String> },
    JsonSchema {
        schema:    Value,
        validator: Arc<Validator>,
    },
}

impl OutputSchemaKind {
    /// Describes what a valid final response looks like. Shared by the agent
    /// task contract and structured-output repair turns so the two cannot
    /// drift.
    fn expectation(&self) -> String {
        match self {
            Self::Routing { allowed_labels } => {
                let mut expectation = format!(
                    "Return a single JSON object with at least one routing field: {}.",
                    ROUTING_STATUS_FIELDS.join(", ")
                );
                if !allowed_labels.is_empty() {
                    let _ = write!(
                        expectation,
                        "\npreferred_next_label must be one of this node's outgoing edge \
                         labels: {}.",
                        quote_join(allowed_labels),
                    );
                }
                expectation
            }
            Self::JsonSchema { schema, .. } => format!(
                "Return a single JSON object that satisfies this JSON Schema:\n\
                 <output_schema>\n\
                 {schema}\n\
                 </output_schema>"
            ),
        }
    }

    /// Appends the final-output contract to an agent task prompt. Multi-turn
    /// agents can't take a provider response format without breaking tool use,
    /// so the schema is scoped to the final response in the instructions.
    #[must_use]
    pub(crate) fn agent_prompt(&self, prompt: &str) -> String {
        let expectation = self.expectation();
        format!(
            "{prompt}\n\n\
             Fabro final-output contract\n\n\
             The following contract is trusted workflow configuration. It applies only to your final response, not to intermediate tool calls.\n\
             {expectation}\n\
             The contract is complete. Do not ask the user to provide or choose the output shape."
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StructuredOutputErrorKind {
    NoJsonObject,
    NoRelevantJsonObject,
    InvalidJson,
    SchemaValidation,
}

const MAX_SCHEMA_FRAGMENT_CHARS: usize = 320;

/// `additionalProperties` errors carry one entry per unexpected key, and the
/// keys come from model output. Cap them so a wide object can't turn the repair
/// prompt into megabytes.
const MAX_UNEXPECTED_PROPERTIES: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
struct SchemaValidationIssue {
    instance_path: Location,
    schema_path:   Location,
    detail:        SchemaValidationIssueDetail,
}

/// `Required` and `AdditionalProperties` get bespoke rendering because
/// `jsonschema` names the offending property without ever locating it. Every
/// other keyword already renders a message that names both the value and the
/// constraint, so it goes through `Other` with the schema fragment attached.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SchemaValidationIssueDetail {
    Required {
        property: String,
    },
    AdditionalProperties {
        unexpected: Vec<String>,
        total:      usize,
    },
    Other {
        message:         String,
        schema_fragment: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StructuredOutputErrorDetails {
    Message(String),
    SchemaValidation(Vec<SchemaValidationIssue>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StructuredOutputError {
    kind:    StructuredOutputErrorKind,
    details: StructuredOutputErrorDetails,
}

impl SchemaValidationIssue {
    fn from_error(error: &ValidationError<'_>, schema: Option<&Value>) -> Self {
        let detail = match error.kind() {
            ValidationErrorKind::Required { property } => SchemaValidationIssueDetail::Required {
                property: property
                    .as_str()
                    .map_or_else(|| property.to_string(), str::to_owned),
            },
            ValidationErrorKind::AdditionalProperties { unexpected } => {
                // `unexpected` arrives in the order the model emitted the keys,
                // so sort before truncating. That keeps the retained subset and
                // the rendered message stable, and lets two attempts that left
                // the same keys in place compare equal whatever order they used.
                let total = unexpected.len();
                let mut sorted = unexpected.clone();
                sorted.sort_unstable();
                sorted.truncate(MAX_UNEXPECTED_PROPERTIES);
                SchemaValidationIssueDetail::AdditionalProperties {
                    unexpected: sorted,
                    total,
                }
            }
            _ => SchemaValidationIssueDetail::Other {
                message:         error.to_string(),
                schema_fragment: schema
                    .and_then(|schema| schema.pointer(error.schema_path().as_str()))
                    .map(bounded_json),
            },
        };
        Self {
            instance_path: error.instance_path().clone(),
            schema_path: error.schema_path().clone(),
            detail,
        }
    }

    fn render(&self) -> String {
        let mut message = match &self.detail {
            SchemaValidationIssueDetail::Required { property } => format!(
                "Missing required property {} at JSON Pointer `{}`. Add it to the object at {}.",
                Value::String(property.clone()),
                self.instance_path.join(property),
                pointer_phrase(&self.instance_path),
            ),
            SchemaValidationIssueDetail::AdditionalProperties { unexpected, total } => {
                let mut properties = unexpected
                    .iter()
                    .map(|property| {
                        format!(
                            "{} at `{}`",
                            Value::String(property.clone()),
                            self.instance_path.join(property),
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let remaining = total - unexpected.len();
                if remaining > 0 {
                    let _ = write!(properties, ", and {remaining} more");
                }
                format!(
                    "Unexpected properties in the object at {}: {properties}.",
                    pointer_phrase(&self.instance_path),
                )
            }
            SchemaValidationIssueDetail::Other { message, .. } => format!(
                "At {}: {}.",
                pointer_phrase(&self.instance_path),
                message.trim_end_matches('.'),
            ),
        };

        let _ = write!(
            message,
            " Schema rule: {}",
            pointer_phrase(&self.schema_path)
        );
        if let SchemaValidationIssueDetail::Other {
            schema_fragment: Some(fragment),
            ..
        } = &self.detail
        {
            message.push_str(": ");
            message.push_str(fragment);
        }
        message.push('.');
        message
    }
}

impl StructuredOutputError {
    fn new(kind: StructuredOutputErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            details: StructuredOutputErrorDetails::Message(message.into()),
        }
    }

    fn validation(issues: Vec<SchemaValidationIssue>) -> Self {
        Self {
            kind:    StructuredOutputErrorKind::SchemaValidation,
            details: StructuredOutputErrorDetails::SchemaValidation(issues),
        }
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn kind(&self) -> StructuredOutputErrorKind {
        self.kind
    }

    #[must_use]
    pub(crate) fn messages(&self) -> Vec<String> {
        match &self.details {
            StructuredOutputErrorDetails::Message(message) => vec![message.clone()],
            StructuredOutputErrorDetails::SchemaValidation(issues) => {
                issues.iter().map(SchemaValidationIssue::render).collect()
            }
        }
    }

    #[must_use]
    pub(crate) fn allows_routing_fallback(&self) -> bool {
        matches!(
            self.kind,
            StructuredOutputErrorKind::NoJsonObject
                | StructuredOutputErrorKind::NoRelevantJsonObject
        )
    }

    #[must_use]
    pub(crate) fn repair_message(
        &self,
        schema: &OutputSchemaKind,
        previous_error: Option<&Self>,
    ) -> String {
        let expectation = schema.expectation();
        let errors = self
            .messages()
            .iter()
            .map(|message| format!("- {message}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut sections =
            vec!["Your previous response did not satisfy the node's output_schema.".to_string()];
        if previous_error.is_some_and(|previous| self.shares_schema_issue_with(previous)) {
            sections.push(
                "At least one validation problem below is unchanged from your previous repair."
                    .to_string(),
            );
        }
        sections.push(format!("Validation errors:\n{errors}"));
        sections.push(expectation);
        if self.kind == StructuredOutputErrorKind::SchemaValidation {
            sections.push(
                "Apply each correction at the exact JSON Pointer shown and return the complete object."
                    .to_string(),
            );
        }
        sections.push(
            "Do not include Markdown fences or explanatory prose; reply only with the corrected JSON object."
                .to_string(),
        );
        sections.join("\n\n")
    }

    fn shares_schema_issue_with(&self, other: &Self) -> bool {
        let (
            StructuredOutputErrorDetails::SchemaValidation(current),
            StructuredOutputErrorDetails::SchemaValidation(previous),
        ) = (&self.details, &other.details)
        else {
            return false;
        };
        current.iter().any(|issue| previous.contains(issue))
    }
}

fn pointer_phrase(path: &Location) -> String {
    if path.as_str().is_empty() {
        "the document root".to_string()
    } else {
        format!("JSON Pointer `{path}`")
    }
}

fn bounded_json(value: &Value) -> String {
    let mut rendered = value.to_string();
    if let Some((offset, _)) = rendered.char_indices().nth(MAX_SCHEMA_FRAGMENT_CHARS) {
        rendered.truncate(offset);
        rendered.push('…');
    }
    rendered
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ValidatedStructuredOutput {
    pub(crate) value: Value,
}

#[must_use]
pub(crate) fn output_key(node_id: &str) -> String {
    format!("output.{node_id}")
}

/// Response-payload deduplication (fabro-b907): when a stage's
/// context_updates already carry the structured data the response text
/// embeds (an `output.<node>` JSON payload from a validated schema, or a
/// user-contract key also set from the same response), the full response
/// text duplicates that data verbatim — up to four copies of the same
/// brief landed in one checkpoint, consuming preamble budget that then
/// blob-ref'd 1.5 KB facts. Returns a compact reference instead of the
/// full text.
#[must_use]
pub(crate) fn compact_response_value(node_id: &str, response_text: &str) -> serde_json::Value {
    let len = response_text.len();
    let head = truncate_chars(response_text, 200);
    serde_json::json!({
        "dedup": "response text omitted — payload lives in structured context keys",
        "output_key": output_key(node_id),
        "chars": len,
        "preview": head,
    })
}

/// Character-safe truncation for previews (cuts at char boundaries, never
/// mid-UTF-8).
fn truncate_chars(text: &str, max_chars: usize) -> String {
    super::agent::truncate(text, max_chars).to_string()
}

#[must_use]
pub(crate) fn exhausted_failure_reason(repair_attempts: i64) -> String {
    format!("output schema validation failed after {repair_attempts} repair attempt(s)")
}

#[must_use]
pub(crate) fn exhausted_failure_outcome(repair_attempts: i64) -> Outcome {
    Outcome {
        status: StageOutcome::Failed {
            retry_requested: false,
        },
        failure: Some(FailureDetail::new(
            exhausted_failure_reason(repair_attempts),
            FailureCategory::Deterministic,
        )),
        ..Outcome::default()
    }
}

pub(crate) fn parse_node_output_schema(
    graph: &Graph,
    node: &Node,
) -> Result<Option<OutputSchemaKind>, Error> {
    let Some(raw) = node.output_schema() else {
        return Ok(None);
    };
    let value = raw.trim();
    if value.is_empty() {
        return Err(Error::Validation(format!(
            "Invalid output_schema for node \"{}\": value must not be empty",
            node.id
        )));
    }
    if value == ROUTING_KEYWORD {
        return Ok(Some(OutputSchemaKind::Routing {
            allowed_labels: routable_edge_labels(graph, &node.id),
        }));
    }
    if value.starts_with('@') {
        return Err(Error::Validation(format!(
            "Invalid output_schema for node \"{}\": unresolved file reference {value}",
            node.id
        )));
    }

    let schema = serde_json::from_str::<Value>(value).map_err(|err| {
        Error::Validation(format!(
            "Invalid output_schema for node \"{}\": expected \"routing\" or a JSON Schema object: {err}",
            node.id
        ))
    })?;
    let validator = jsonschema::validator_for(&schema).map_err(|err| {
        Error::Validation(format!(
            "Invalid output_schema for node \"{}\": {err}",
            node.id
        ))
    })?;
    Ok(Some(OutputSchemaKind::JsonSchema {
        schema,
        validator: Arc::new(validator),
    }))
}

#[must_use]
/// The provider response format for a node's output schema.
///
/// Providers with native structured output enforce the JSON schema; every
/// provider still gets validated locally afterwards.
pub(crate) fn prompt_response_format(schema: &OutputSchemaKind) -> ResponseFormat {
    match schema {
        OutputSchemaKind::Routing { .. } => ResponseFormat::JsonObject,
        OutputSchemaKind::JsonSchema { schema, .. } => ResponseFormat::JsonSchema {
            name:   "output_schema".to_string(),
            schema: schema.clone(),
        },
    }
}

pub(crate) fn validate_response_text(
    schema: &OutputSchemaKind,
    text: &str,
) -> Result<ValidatedStructuredOutput, StructuredOutputError> {
    match schema {
        OutputSchemaKind::Routing { allowed_labels } => {
            validate_routing_response_text(allowed_labels, text)
        }
        OutputSchemaKind::JsonSchema { schema, validator } => {
            validate_custom_response_text(validator, schema, text)
        }
    }
}

pub(crate) fn apply_validated_output(
    node: &Node,
    schema: &OutputSchemaKind,
    validated: &ValidatedStructuredOutput,
    outcome: &mut Outcome,
) {
    match schema {
        OutputSchemaKind::Routing { .. } => apply_routing_fields(&validated.value, outcome),
        OutputSchemaKind::JsonSchema { .. } => {
            outcome
                .context_updates
                .insert(output_key(&node.id), validated.value.clone());
        }
    }
}

/// Find the outermost balanced `{...}` JSON object substrings in the text, in
/// document order. Objects nested inside a match are skipped.
///
/// An unbalanced `{` does not suppress complete objects around or inside it:
/// the scan only skips ahead past a *matched* object, so it still walks into a
/// region that failed to close.
fn find_json_objects(text: &str) -> Vec<&str> {
    let mut results = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let start = i;
            let mut depth = 0;
            let mut in_string = false;
            let mut escape = false;
            let mut j = i;
            while j < bytes.len() {
                let c = bytes[j];
                if escape {
                    escape = false;
                } else if c == b'\\' && in_string {
                    escape = true;
                } else if c == b'"' {
                    in_string = !in_string;
                } else if !in_string {
                    if c == b'{' {
                        depth += 1;
                    } else if c == b'}' {
                        depth -= 1;
                        if depth == 0 {
                            results.push(&text[start..=j]);
                            i = j;
                            break;
                        }
                    }
                }
                j += 1;
            }
        }
        i += 1;
    }
    results
}

/// Return the outermost balanced JSON object that ends the text, ignoring
/// trailing whitespace.
pub(crate) fn terminal_json_object(text: &str) -> Option<&str> {
    let trimmed = text.trim_end();
    find_json_objects(trimmed)
        .into_iter()
        .next_back()
        .filter(|candidate| trimmed.ends_with(candidate))
}

pub(crate) fn extract_status_fields(text: &str, outcome: &mut Outcome) -> bool {
    let candidates = find_json_objects(text);

    let parsed = candidates.iter().rev().find_map(|candidate| {
        let value: Value = serde_json::from_str(candidate).ok()?;
        if value.as_object().is_some_and(contains_routing_field) {
            Some(value)
        } else {
            None
        }
    });

    let Some(value) = parsed else { return false };
    apply_routing_fields(&value, outcome);
    true
}

fn validate_routing_response_text(
    allowed_labels: &[String],
    text: &str,
) -> Result<ValidatedStructuredOutput, StructuredOutputError> {
    let candidates = find_json_objects(text);
    if candidates.is_empty() {
        return Err(StructuredOutputError::new(
            StructuredOutputErrorKind::NoJsonObject,
            "no JSON object found in response",
        ));
    }

    for candidate in candidates.iter().rev() {
        let parsed = match serde_json::from_str::<Value>(candidate) {
            Ok(value) => value,
            Err(err) if raw_mentions_routing_field(candidate) => {
                return Err(StructuredOutputError::new(
                    StructuredOutputErrorKind::InvalidJson,
                    format!("invalid routing JSON object: {err}"),
                ));
            }
            Err(_) => continue,
        };
        let Some(obj) = parsed.as_object() else {
            continue;
        };
        if !contains_routing_field(obj) {
            continue;
        }
        validate_value_against_validator(routing_validator(), &parsed, None)?;
        validate_preferred_label(allowed_labels, &parsed)?;
        return Ok(ValidatedStructuredOutput { value: parsed });
    }

    Err(StructuredOutputError::new(
        StructuredOutputErrorKind::NoRelevantJsonObject,
        format!(
            "no JSON object contained any recognized routing field ({})",
            ROUTING_STATUS_FIELDS.join(", ")
        ),
    ))
}

fn validate_custom_response_text(
    validator: &Validator,
    schema: &Value,
    text: &str,
) -> Result<ValidatedStructuredOutput, StructuredOutputError> {
    // Prose after the object can contain braces, so the last candidate is not
    // always JSON. Take the last one that parses; report its schema errors
    // rather than falling back to an earlier object that happens to validate.
    let candidates = find_json_objects(text);
    let mut invalid_json = None;
    for candidate in candidates.iter().rev() {
        match serde_json::from_str::<Value>(candidate) {
            Ok(parsed) => {
                validate_value_against_validator(validator, &parsed, Some(schema))?;
                return Ok(ValidatedStructuredOutput { value: parsed });
            }
            Err(err) if invalid_json.is_none() => invalid_json = Some(err.to_string()),
            Err(_) => {}
        }
    }

    Err(match invalid_json {
        Some(message) => StructuredOutputError::new(
            StructuredOutputErrorKind::InvalidJson,
            format!("invalid JSON object: {message}"),
        ),
        None => StructuredOutputError::new(
            StructuredOutputErrorKind::NoJsonObject,
            "no JSON object found in response",
        ),
    })
}

fn validate_value_against_validator(
    validator: &Validator,
    value: &Value,
    schema: Option<&Value>,
) -> Result<(), StructuredOutputError> {
    let issues = validator
        .iter_errors(value)
        .take(5)
        .map(|error| SchemaValidationIssue::from_error(&error, schema))
        .collect::<Vec<_>>();
    if issues.is_empty() {
        Ok(())
    } else {
        Err(StructuredOutputError::validation(issues))
    }
}

fn contains_routing_field(obj: &serde_json::Map<String, Value>) -> bool {
    ROUTING_STATUS_FIELDS
        .iter()
        .any(|field| obj.contains_key(*field))
}

/// Labels of the node's unconditional outgoing edges — the only edges a
/// `preferred_label` can routing-match in `graph::routing::select_edge`.
/// Conditional edges and unlabeled edges never match by label, so their
/// labels are not part of the allowed vocabulary.
#[must_use]
pub(crate) fn routable_edge_labels(graph: &Graph, node_id: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    graph
        .outgoing_edges(node_id)
        .into_iter()
        .filter_map(|edge| {
            let label = edge.label().map(|l| l.trim().to_owned())?;
            if label.is_empty() {
                return None;
            }
            // Unconditional edges: `select_edge` matches preferred_label
            // against their labels directly. Conditional edges: routable when
            // the condition itself routes on preferred_label naming this
            // label (the condition evaluator matches those first, fabro-27dc).
            let routed = match edge.condition() {
                Some(cond) if !cond.trim().is_empty() => {
                    condition_routes_preferred_label(cond, &label)
                }
                _ => true,
            };
            routed.then_some(label).filter(|l| seen.insert(l.clone()))
        })
        .collect()
}

/// True when the condition expression contains a `preferred_label = <label>`
/// clause — `select_edge` routes such edges through the condition evaluator
/// when the outcome names that label, so the label is a valid
/// `preferred_next_label` choice even though the edge is conditional
/// (fabro-27dc: the conductor graph routes exclusively this way).
fn condition_routes_preferred_label(condition: &str, label: &str) -> bool {
    use fabro_graphviz::condition::{Clause, ConditionExpr, Op, parse_condition_expr};

    fn expr_routes(expr: &ConditionExpr, label: &str) -> bool {
        match expr {
            ConditionExpr::Clause(Clause { key, op, value }) => {
                key.trim() == "preferred_label"
                    && matches!(op, Op::Eq)
                    && normalize_label(value) == normalize_label(label)
            }
            ConditionExpr::Not(inner) => expr_routes(inner, label),
            ConditionExpr::And(parts) | ConditionExpr::Or(parts) => {
                parts.iter().any(|part| expr_routes(part, label))
            }
        }
    }

    parse_condition_expr(condition).is_ok_and(|parsed| expr_routes(&parsed, label))
}

/// Fail the OUTPUT (not the run) when `preferred_next_label` is not one of the
/// node's outgoing edge labels (fabro-de4d). Verbose validation: the error
/// names the offending value AND the full allowed-label list so a repair turn
/// can correct it without guessing. When no edge labels are available (no
/// graph edges for the node), the field stays free-form — the label is simply
/// dropped later because no edge matches it.
fn validate_preferred_label(
    allowed_labels: &[String],
    value: &Value,
) -> Result<(), StructuredOutputError> {
    let Some(obj) = value.as_object() else {
        return Ok(());
    };
    // Non-string values already fail the base routing schema's type check.
    let Some(label) = obj.get("preferred_next_label").and_then(Value::as_str) else {
        return Ok(());
    };
    if allowed_labels.is_empty() {
        return Ok(());
    }
    let matched = allowed_labels
        .iter()
        .any(|allowed| normalize_label(allowed) == normalize_label(label));
    if matched {
        Ok(())
    } else {
        Err(StructuredOutputError::new(
            StructuredOutputErrorKind::SchemaValidation,
            format!(
                "preferred_next_label {} is not one of this node's outgoing edge labels: {}. \
                 Use exactly one of the listed labels (case-insensitive; accelerator prefixes \
                 like \"[F] \" are ignored) or omit the field.",
                Value::String(label.to_string()),
                quote_join(allowed_labels),
            ),
        ))
    }
}

fn quote_join(values: &[String]) -> String {
    values
        .iter()
        .map(|value| Value::String(value.clone()).to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn raw_mentions_routing_field(candidate: &str) -> bool {
    QUOTED_ROUTING_STATUS_FIELDS
        .iter()
        .any(|quoted_field| candidate.contains(quoted_field))
}

fn routing_validator() -> &'static Validator {
    static ROUTING_VALIDATOR: LazyLock<Validator> = LazyLock::new(|| {
        let schema = serde_json::json!({
            "type": "object",
            "additionalProperties": true,
            "properties": {
                "preferred_next_label": { "type": "string" },
                "outcome": {
                    "type": "string",
                    "enum": ["succeeded", "partially_succeeded", "failed", "skipped"]
                },
                "failure_reason": { "type": "string" },
                "suggested_next_ids": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "context_updates": { "type": "object" }
            },
            "anyOf": ROUTING_STATUS_FIELDS
                .iter()
                .map(|field| serde_json::json!({ "required": [field] }))
                .collect::<Vec<_>>()
        });
        jsonschema::validator_for(&schema).expect("built-in routing schema must compile")
    });
    &ROUTING_VALIDATOR
}

fn apply_routing_fields(value: &Value, outcome: &mut Outcome) {
    let Some(obj) = value.as_object() else {
        return;
    };

    if let Some(label) = obj.get("preferred_next_label").and_then(Value::as_str) {
        outcome.preferred_label = Some(label.to_string());
    }

    if let Some(ids) = obj.get("suggested_next_ids").and_then(Value::as_array) {
        let string_ids: Vec<String> = ids
            .iter()
            .filter_map(|value| value.as_str().map(String::from))
            .collect();
        if !string_ids.is_empty() {
            outcome.suggested_next_ids = string_ids;
        }
    }

    if let Some(status_str) = obj.get("outcome").and_then(Value::as_str) {
        if let Ok(status) = status_str.parse::<StageOutcome>() {
            outcome.status = status;
            if outcome.status.is_failure() {
                if let Some(reason) = obj.get("failure_reason").and_then(Value::as_str) {
                    outcome.failure =
                        Some(FailureDetail::new(reason, FailureCategory::Deterministic));
                }
            }
        }
    }

    if let Some(updates) = obj.get("context_updates").and_then(Value::as_object) {
        for (key, value) in updates {
            outcome.context_updates.insert(key.clone(), value.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use fabro_graphviz::graph::{AttrValue, Node};

    use super::*;

    fn routing() -> OutputSchemaKind {
        OutputSchemaKind::Routing {
            allowed_labels: Vec::new(),
        }
    }

    fn routing_with_labels(labels: &[&str]) -> OutputSchemaKind {
        OutputSchemaKind::Routing {
            allowed_labels: labels.iter().map(ToString::to_string).collect(),
        }
    }

    fn schema(value: Value) -> OutputSchemaKind {
        let validator =
            jsonschema::validator_for(&value).expect("test schema should be a valid JSON Schema");
        OutputSchemaKind::JsonSchema {
            schema:    value,
            validator: Arc::new(validator),
        }
    }

    /// A schema whose required field is itself an object, so validating the
    /// innermost `{...}` in the response would fail.
    fn issue_schema() -> OutputSchemaKind {
        schema(serde_json::json!({
            "type": "object",
            "required": ["issue"],
            "properties": {
                "issue": {
                    "type": "object",
                    "required": ["number"],
                    "properties": {
                        "number": { "type": "integer" }
                    }
                }
            }
        }))
    }

    #[test]
    fn validates_routing_json_and_applies_fields() {
        let validated = validate_response_text(
            &routing(),
            r#"done {"outcome":"failed","failure_reason":"tests failed","preferred_next_label":"fix","suggested_next_ids":["a"],"context_updates":{"verified":true}}"#,
        )
        .unwrap();
        let mut outcome = Outcome::success();

        apply_routing_fields(&validated.value, &mut outcome);

        assert_eq!(outcome.status, StageOutcome::Failed {
            retry_requested: false,
        });
        assert_eq!(
            outcome.failure.as_ref().map(|f| f.message.as_str()),
            Some("tests failed")
        );
        assert_eq!(outcome.preferred_label.as_deref(), Some("fix"));
        assert_eq!(outcome.suggested_next_ids, vec!["a".to_string()]);
        assert_eq!(
            outcome.context_updates.get("verified"),
            Some(&serde_json::json!(true)),
        );
    }

    #[test]
    fn routing_json_missing_routing_fields_is_invalid() {
        let error = validate_response_text(&routing(), r#"{"summary":"ok"}"#).unwrap_err();

        assert_eq!(
            error.kind(),
            StructuredOutputErrorKind::NoRelevantJsonObject
        );
        assert!(error.messages()[0].contains("recognized routing field"));
    }

    #[test]
    fn terminal_json_object_accepts_final_object_after_prose() {
        let object =
            terminal_json_object("# Results\n\n{\"context_updates\":{\"verified\":true}}\n\n");

        assert_eq!(object, Some(r#"{"context_updates":{"verified":true}}"#),);
    }

    #[test]
    fn terminal_json_object_returns_outermost_nested_object() {
        let object = terminal_json_object(r#"Results: {"context_updates":{"verified":true}}"#);

        assert_eq!(object, Some(r#"{"context_updates":{"verified":true}}"#),);
    }

    #[test]
    fn terminal_json_object_rejects_object_followed_by_content() {
        assert_eq!(
            terminal_json_object(
                "{\"outcome\":\"failed\",\"failure_reason\":\"tests failed\"}\nMore details",
            ),
            None,
        );
    }

    #[test]
    fn routing_json_with_wrong_field_type_is_invalid() {
        let error =
            validate_response_text(&routing(), r#"{"suggested_next_ids":[1]}"#).unwrap_err();

        assert_eq!(error.kind(), StructuredOutputErrorKind::SchemaValidation);
        assert!(
            error
                .messages()
                .iter()
                .any(|message| message.contains("string")),
            "unexpected messages: {:?}",
            error.messages(),
        );
    }

    #[test]
    fn known_preferred_label_passes_validation() {
        let schema = routing_with_labels(&["Approve", "[F] Fix"]);

        let validated = validate_response_text(
            &schema,
            r#"{"outcome":"succeeded","preferred_next_label":"Approve"}"#,
        );

        assert_eq!(
            validated.unwrap().value,
            serde_json::json!({"outcome": "succeeded", "preferred_next_label": "Approve"}),
        );
    }

    #[test]
    fn known_preferred_label_matches_case_and_accelerator_insensitively() {
        let schema = routing_with_labels(&["[F] Fix"]);

        let validated = validate_response_text(&schema, r#"{"preferred_next_label":"fix"}"#);

        assert_eq!(
            validated.unwrap().value,
            serde_json::json!({"preferred_next_label": "fix"}),
        );
    }

    #[test]
    fn unknown_preferred_label_fails_the_output_with_verbose_error() {
        // fabro-de4d incident shape: reviewer emitted 'deploy', a label no
        // outgoing edge knows. The output must fail (routing repair), not the
        // run, and the error must name the offending value AND the full
        // allowed-label list.
        let schema = routing_with_labels(&["Approve", "Changes requested"]);

        let error = validate_response_text(
            &schema,
            r#"{"outcome":"succeeded","preferred_next_label":"deploy"}"#,
        )
        .unwrap_err();

        assert_eq!(error.kind(), StructuredOutputErrorKind::SchemaValidation);
        assert!(!error.allows_routing_fallback());
        let message = error.messages().join("\n");
        assert!(
            message.contains("\"deploy\""),
            "error must name the offending value: {message}"
        );
        assert!(
            message.contains("\"Approve\"") && message.contains("\"Changes requested\""),
            "error must list every allowed label: {message}"
        );
    }

    #[test]
    fn routing_repair_message_for_unknown_label_carries_the_allowed_list() {
        let schema = routing_with_labels(&["Approve", "Changes requested"]);

        let error =
            validate_response_text(&schema, r#"{"preferred_next_label":"deploy"}"#).unwrap_err();
        let repair = error.repair_message(&schema, None);

        assert!(
            repair.contains("\"deploy\""),
            "repair message must name the offending value: {repair}"
        );
        assert!(
            repair.contains("\"Changes requested\""),
            "repair message must carry the allowed-label list: {repair}"
        );
    }

    #[test]
    fn routing_without_edge_labels_keeps_free_form_drop_semantics() {
        // No edge information available: the label is unconstrained at the
        // output boundary and dropped later when no edge matches it.
        let schema = routing();

        let validated = validate_response_text(&schema, r#"{"preferred_next_label":"deploy"}"#);

        assert_eq!(
            validated.unwrap().value,
            serde_json::json!({"preferred_next_label": "deploy"}),
        );
    }

    #[test]
    fn routing_agent_prompt_lists_the_allowed_labels() {
        let prompt = routing_with_labels(&["Approve", "Changes requested"])
            .agent_prompt("Pick the next step");

        assert!(
            prompt.contains(
                "preferred_next_label must be one of this node's outgoing edge \
                             labels: \"Approve\", \"Changes requested\"."
            ),
            "unexpected prompt: {prompt}"
        );
    }

    #[test]
    fn parse_node_output_schema_threads_outgoing_edge_labels_into_routing() {
        let mut graph = fabro_graphviz::graph::Graph::new("test");
        graph
            .nodes
            .insert("review".to_string(), Node::new("review"));
        graph
            .nodes
            .insert("approve".to_string(), Node::new("approve"));
        graph
            .nodes
            .insert("recover".to_string(), Node::new("recover"));
        let mut labeled = fabro_graphviz::graph::Edge::new("review", "approve");
        labeled.attrs.insert(
            "label".to_string(),
            AttrValue::String("Approve".to_string()),
        );
        graph.edges.push(labeled);
        let mut conditional = fabro_graphviz::graph::Edge::new("review", "recover");
        conditional.attrs.insert(
            "condition".to_string(),
            AttrValue::String("outcome=failed".to_string()),
        );
        conditional.attrs.insert(
            "label".to_string(),
            AttrValue::String("Recover".to_string()),
        );
        graph.edges.push(conditional);

        let mut node = Node::new("review");
        node.attrs.insert(
            "output_schema".to_string(),
            AttrValue::String("routing".to_string()),
        );

        let parsed = parse_node_output_schema(&graph, &node).unwrap();

        // The unconditional edge's label is routable (preferred-label loop);
        // the outcome=failed conditional edge does not route on
        // preferred_label, so its label stays excluded (fabro-27dc).
        match parsed {
            Some(OutputSchemaKind::Routing { allowed_labels }) => {
                assert_eq!(allowed_labels, vec!["Approve".to_string()]);
            }
            other => panic!("expected routing schema, got {other:?}"),
        }
    }

    #[test]
    fn routable_labels_include_preferred_label_condition_edges() {
        // Regression shape of fabro-27dc (conductor survey): real choice
        // edges carry `preferred_label="..."` conditions; the only
        // unconditional edge is the safety net. Before the fix the allowed
        // list contained ONLY the fallback label, so the routing validation
        // forced every survey answer into "Unrouted survey outcome" and the
        // line parked.
        let mut graph = fabro_graphviz::graph::Graph::new("test");
        graph
            .nodes
            .insert("survey".to_string(), Node::new("survey"));
        graph
            .nodes
            .insert("develop".to_string(), Node::new("develop"));
        graph.nodes.insert("exit".to_string(), Node::new("exit"));
        let mut work = fabro_graphviz::graph::Edge::new("survey", "develop");
        work.attrs
            .insert("label".to_string(), AttrValue::String("Work".to_string()));
        work.attrs.insert(
            "condition".to_string(),
            AttrValue::String("preferred_label=\"Work\"".to_string()),
        );
        graph.edges.push(work);
        let mut failed = fabro_graphviz::graph::Edge::new("survey", "exit");
        failed.attrs.insert(
            "label".to_string(),
            AttrValue::String("Survey failed".to_string()),
        );
        failed.attrs.insert(
            "condition".to_string(),
            AttrValue::String("outcome=failed".to_string()),
        );
        graph.edges.push(failed);
        let mut safety_net = fabro_graphviz::graph::Edge::new("survey", "exit");
        safety_net.attrs.insert(
            "label".to_string(),
            AttrValue::String("Unrouted survey outcome".to_string()),
        );
        graph.edges.push(safety_net);

        let labels = routable_edge_labels(&graph, "survey");

        assert_eq!(labels, vec![
            "Work".to_string(),
            "Unrouted survey outcome".to_string()
        ]);
    }

    #[test]
    fn routable_labels_include_composite_preferred_label_conditions() {
        let mut graph = fabro_graphviz::graph::Graph::new("test");
        graph.nodes.insert("gate".to_string(), Node::new("gate"));
        graph.nodes.insert("next".to_string(), Node::new("next"));
        graph.nodes.insert("other".to_string(), Node::new("other"));
        let mut composite = fabro_graphviz::graph::Edge::new("gate", "next");
        composite.attrs.insert(
            "label".to_string(),
            AttrValue::String("Proceed".to_string()),
        );
        composite.attrs.insert(
            "condition".to_string(),
            AttrValue::String("preferred_label=\"Proceed\" && outcome=succeeded".to_string()),
        );
        graph.edges.push(composite);
        let mut other_label = fabro_graphviz::graph::Edge::new("gate", "other");
        other_label.attrs.insert(
            "label".to_string(),
            AttrValue::String("Elsewhere".to_string()),
        );
        other_label.attrs.insert(
            "condition".to_string(),
            AttrValue::String("preferred_label=\"Elsewhere\"".to_string()),
        );
        graph.edges.push(other_label);
        let mut mismatched = fabro_graphviz::graph::Edge::new("gate", "other");
        mismatched
            .attrs
            .insert("label".to_string(), AttrValue::String("Wrong".to_string()));
        mismatched.attrs.insert(
            "condition".to_string(),
            AttrValue::String("preferred_label=\"Elsewhere\"".to_string()),
        );
        graph.edges.push(mismatched);

        let labels = routable_edge_labels(&graph, "gate");

        assert_eq!(labels, vec!["Proceed".to_string(), "Elsewhere".to_string()]);
    }

    #[test]
    fn validates_custom_schema_against_last_json_object() {
        let schema = schema(serde_json::json!({
            "type": "object",
            "required": ["passed"],
            "properties": {
                "passed": { "type": "boolean" }
            }
        }));

        let validated =
            validate_response_text(&schema, r#"ignore {"other":1} final {"passed":true}"#).unwrap();

        assert_eq!(validated.value, serde_json::json!({"passed": true}));
    }

    #[test]
    fn validates_custom_schema_against_outermost_object() {
        let validated =
            validate_response_text(&issue_schema(), r#"{"issue":{"number":19}}"#).unwrap();

        assert_eq!(
            validated.value,
            serde_json::json!({"issue": {"number": 19}})
        );
    }

    #[test]
    fn validates_last_outermost_object_when_response_has_trailing_prose() {
        let validated = validate_response_text(
            &issue_schema(),
            r#"ignore {"issue":{"number":1}} final {"issue":{"number":19}} trailing"#,
        )
        .unwrap();

        assert_eq!(
            validated.value,
            serde_json::json!({"issue": {"number": 19}})
        );
    }

    #[test]
    fn validates_last_parsable_object_when_trailing_prose_contains_braces() {
        let schema = schema(serde_json::json!({
            "type": "object",
            "required": ["passed"],
            "properties": {
                "passed": { "type": "boolean" }
            }
        }));

        let validated = validate_response_text(
            &schema,
            "{\"passed\":true}\n\nLet me know if {this works} for you.",
        )
        .unwrap();

        assert_eq!(validated.value, serde_json::json!({"passed": true}));
    }

    #[test]
    fn find_json_objects_returns_outermost_objects_only() {
        let cases = [
            (r#"{"a":{"b":1}}"#, vec![r#"{"a":{"b":1}}"#]),
            (r#"{"a":1} {"b":2}"#, vec![r#"{"a":1}"#, r#"{"b":2}"#]),
            (r#"{"a":1}{"b":2}"#, vec![r#"{"a":1}"#, r#"{"b":2}"#]),
            // An unclosed outer brace must not hide the complete object inside it.
            (r#"{ {"a":1}"#, vec![r#"{"a":1}"#]),
            (r#"{"a":1} {"#, vec![r#"{"a":1}"#]),
            // An unterminated string swallows the rest of its own candidate.
            (r#"{"a": "x} {"b":2}"#, vec![r#"{"b":2}"#]),
            // Braces inside strings are not delimiters.
            (r#"{"a":"} {"}"#, vec![r#"{"a":"} {"}"#]),
            ("no json here", vec![]),
        ];

        for (text, expected) in cases {
            assert_eq!(find_json_objects(text), expected, "input: {text}");
        }
    }

    #[test]
    fn custom_schema_validation_errors_are_reported() {
        let schema = schema(serde_json::json!({
            "type": "object",
            "required": ["passed"],
            "properties": {
                "passed": { "type": "boolean" }
            }
        }));

        let error = validate_response_text(&schema, r#"{"passed":"yes"}"#).unwrap_err();

        assert_eq!(error.kind(), StructuredOutputErrorKind::SchemaValidation);
        assert!(
            error
                .messages()
                .iter()
                .any(|message| message.contains("boolean")),
            "unexpected messages: {:?}",
            error.messages(),
        );
    }

    #[test]
    fn missing_nested_property_reports_the_required_target_pointer() {
        let schema = schema(serde_json::json!({
            "type": "object",
            "required": ["findings"],
            "properties": {
                "findings": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["rationale"],
                        "properties": {
                            "rationale": { "type": "string" }
                        }
                    }
                }
            }
        }));

        let error = validate_response_text(&schema, r#"{"findings":[{}]}"#).unwrap_err();

        assert_eq!(error.messages(), vec![
            "Missing required property \"rationale\" at JSON Pointer `/findings/0/rationale`. \
             Add it to the object at JSON Pointer `/findings/0`. Schema rule: JSON Pointer \
             `/properties/findings/items/required`."
                .to_string(),
        ],);
    }

    #[test]
    fn type_and_enum_errors_report_instance_and_schema_pointers() {
        let schema = schema(serde_json::json!({
            "type": "object",
            "properties": {
                "line": { "type": "integer" },
                "severity": { "enum": ["HIGH", "MEDIUM", "LOW"] }
            }
        }));

        let error =
            validate_response_text(&schema, r#"{"line":"85","severity":"CRITICAL"}"#).unwrap_err();

        assert_eq!(error.messages(), vec![
            "At JSON Pointer `/line`: \"85\" is not of type \"integer\". \
             Schema rule: JSON Pointer `/properties/line/type`: \"integer\"."
                .to_string(),
            "At JSON Pointer `/severity`: \"CRITICAL\" is not one of \"HIGH\", \"MEDIUM\" or \
             \"LOW\". Schema rule: JSON Pointer `/properties/severity/enum`: \
             [\"HIGH\",\"MEDIUM\",\"LOW\"]."
                .to_string(),
        ],);
    }

    #[test]
    fn additional_property_error_reports_each_property_pointer() {
        let schema = schema(serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "findings": { "type": "array" }
            }
        }));

        let error = validate_response_text(&schema, r#"{"findings":[],"rationale":"wrong level"}"#)
            .unwrap_err();

        assert_eq!(error.messages(), vec![
            "Unexpected properties in the object at the document root: \"rationale\" at \
             `/rationale`. Schema rule: JSON Pointer `/additionalProperties`."
                .to_string(),
        ],);
    }

    #[test]
    fn repeated_schema_error_calls_out_the_unchanged_pointer() {
        let schema = schema(serde_json::json!({
            "type": "object",
            "required": ["findings"],
            "properties": {
                "findings": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["rationale"]
                    }
                }
            }
        }));
        let previous = validate_response_text(&schema, r#"{"findings":[{}]}"#).unwrap_err();
        let current = validate_response_text(&schema, r#"{"findings":[{}]}"#).unwrap_err();

        let repair = current.repair_message(&schema, Some(&previous));

        assert!(
            repair.contains(
                "At least one validation problem below is unchanged from your previous repair."
            ),
            "unexpected repair message: {repair}",
        );
        assert!(
            repair.contains("JSON Pointer `/findings/0/rationale`"),
            "unexpected repair message: {repair}",
        );
    }

    #[test]
    fn the_same_unexpected_properties_in_a_new_order_are_still_unchanged() {
        let schema = schema(serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "findings": { "type": "array" }
            }
        }));
        let previous = validate_response_text(&schema, r#"{"beta":1,"alpha":1}"#).unwrap_err();
        let current = validate_response_text(&schema, r#"{"alpha":1,"beta":1}"#).unwrap_err();

        let repair = current.repair_message(&schema, Some(&previous));

        assert!(
            repair.contains("unchanged from your previous repair"),
            "unexpected repair message: {repair}",
        );
    }

    #[test]
    fn a_different_problem_at_the_same_location_is_not_called_unchanged() {
        let schema = schema(serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "findings": { "type": "array" }
            }
        }));
        let previous = validate_response_text(&schema, r#"{"stray":1}"#).unwrap_err();
        let current = validate_response_text(&schema, r#"{"different":1}"#).unwrap_err();

        let repair = current.repair_message(&schema, Some(&previous));

        assert!(
            !repair.contains("unchanged from your previous repair"),
            "unexpected repair message: {repair}",
        );
    }

    #[test]
    fn invalid_custom_schema_is_rejected_when_parsing_node_attr() {
        let mut node = Node::new("audit");
        node.attrs.insert(
            "output_schema".to_string(),
            AttrValue::String(r#"{"type": 5}"#.to_string()),
        );

        let error = parse_node_output_schema(&Graph::new("test"), &node).unwrap_err();

        assert!(
            error.to_string().contains("Invalid output_schema"),
            "unexpected error: {error}",
        );
    }

    #[test]
    fn invalid_json_candidate_is_reported_for_custom_schema() {
        let schema = schema(serde_json::json!({"type": "object"}));

        let error = validate_response_text(&schema, r"{not json}").unwrap_err();

        assert_eq!(error.kind(), StructuredOutputErrorKind::InvalidJson);
        assert!(error.messages()[0].contains("invalid JSON object"));
    }

    #[test]
    fn no_json_object_is_reported() {
        let error = validate_response_text(&routing(), "plain text only").unwrap_err();

        assert_eq!(error.kind(), StructuredOutputErrorKind::NoJsonObject);
        assert!(error.messages()[0].contains("no JSON object"));
    }

    #[test]
    fn parse_node_output_schema_accepts_builtin_routing_keyword() {
        let mut node = Node::new("route");
        node.attrs.insert(
            "output_schema".to_string(),
            AttrValue::String("routing".to_string()),
        );

        let parsed = parse_node_output_schema(&Graph::new("test"), &node).unwrap();

        assert!(matches!(parsed, Some(OutputSchemaKind::Routing { .. })));
    }

    #[test]
    fn routing_agent_prompt_lists_routing_fields_instead_of_a_schema() {
        let prompt = routing().agent_prompt("Pick the next step");

        assert!(prompt.starts_with("Pick the next step\n\n"));
        assert!(prompt.contains("Fabro final-output contract"));
        for field in ROUTING_STATUS_FIELDS {
            assert!(prompt.contains(field), "{field} missing from: {prompt}");
        }
        assert!(
            !prompt.contains("<output_schema>"),
            "routing has no JSON Schema to embed, got: {prompt}"
        );
    }

    #[test]
    fn json_schema_agent_prompt_embeds_the_resolved_schema() {
        let prompt = schema(serde_json::json!({"type": "object", "required": ["passed"]}))
            .agent_prompt("Audit the result");

        assert!(prompt.starts_with("Audit the result\n\n"));
        assert!(prompt.contains("<output_schema>"));
        assert!(prompt.contains(r#""required":["passed"]"#));
        assert!(prompt.contains("</output_schema>"));
    }

    #[test]
    fn prompt_response_format_uses_json_schema_for_custom_schema() {
        let schema = schema(serde_json::json!({"type": "object"}));

        let format = prompt_response_format(&schema);

        assert_eq!(format, ResponseFormat::JsonSchema {
            name:   "output_schema".to_string(),
            schema: serde_json::json!({"type": "object"}),
        });
    }

    #[test]
    fn apply_validated_custom_output_updates_output_context_key() {
        let node = Node::new("audit");
        let schema = schema(serde_json::json!({"type": "object"}));
        let validated = ValidatedStructuredOutput {
            value: serde_json::json!({"passed": true}),
        };
        let mut outcome = Outcome::success();

        apply_validated_output(&node, &schema, &validated, &mut outcome);

        assert_eq!(
            outcome.context_updates.get("output.audit"),
            Some(&serde_json::json!({"passed": true})),
        );
    }

    #[test]
    fn compact_response_value_references_payload_key() {
        // fabro-b907: with a validated output payload under output.<node>,
        // the response echo must become a compact reference, not a verbatim
        // copy of the same data.
        let value = compact_response_value("planner", "FULL RESPONSE TEXT ".repeat(100).as_str());
        let obj = value.as_object().expect("compact reference is an object");
        assert_eq!(
            obj.get("output_key"),
            Some(&serde_json::json!("output.planner"))
        );
        assert_eq!(obj.get("chars"), Some(&serde_json::json!(1_900)));
        let preview = obj
            .get("preview")
            .and_then(|v| v.as_str())
            .expect("preview");
        assert!(preview.starts_with("FULL RESPONSE TEXT"));
        assert!(preview.chars().count() <= 200);
        assert!(
            !preview.contains(&"FULL RESPONSE TEXT ".repeat(50)[..]),
            "preview must not carry the full text"
        );
    }

    #[test]
    fn compact_response_value_short_text_is_fully_previewed() {
        let value = compact_response_value("reviewer", "short");
        assert_eq!(value.get("preview"), Some(&serde_json::json!("short")));
        assert_eq!(value.get("chars"), Some(&serde_json::json!(5)));
    }
}
