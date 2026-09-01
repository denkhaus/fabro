pub mod keys {
    //! Static context-key vocabulary, now owned by `fabro_types::context_keys`
    //! (single source of truth shared with `fabro-validate`); re-exported here
    //! so existing `crate::context::keys::*` call sites stay unchanged.

    pub use fabro_graphviz::Fidelity;
    pub use fabro_types::context_keys::{
        COMMAND_OUTPUT, CURRENT_NODE, CURRENT_NODE_KEY, CURRENT_PREAMBLE, CURRENT_PREFIX,
        FAILURE_CLASS, FAILURE_SIGNATURE, GRAPH_GOAL, GRAPH_PREFIX, HUMAN_GATE_LABEL,
        HUMAN_GATE_PREFIX, HUMAN_GATE_SELECTED, HUMAN_GATE_TEXT, INTERNAL_EXIT_KIND,
        INTERNAL_FIDELITY, INTERNAL_NODE_VISIT_COUNT, INTERNAL_PARALLEL_BRANCH_ID,
        INTERNAL_PARALLEL_BRANCH_PREAMBLES, INTERNAL_PARALLEL_GROUP_ID, INTERNAL_PARENT_PREAMBLE,
        INTERNAL_PREFIX, INTERNAL_RETRY_COUNT_PREFIX, INTERNAL_RUN_ID, INTERNAL_SEED_CYCLE_ANCHOR,
        INTERNAL_SEED_CYCLES, INTERNAL_STAGE_EXECUTION_ORDINAL, INTERNAL_THREAD_ID,
        INTERNAL_WORK_DIR, LAST_RESPONSE, LAST_STAGE, OUTCOME, PARALLEL_BRANCH_COUNT,
        PARALLEL_RESULTS, PREFERRED_LABEL, RESPONSE_PREFIX, REVIEW_TARGET, SEED_CYCLES,
        THREAD_PREFIX, TRANSIENT_CONTEXT_KEYS, graph_attr_key, is_engine_internal_key,
        is_engine_stamped_key, is_preamble_hidden_key, response_key, retry_count_key,
        thread_current_node_key,
    };
}

use std::collections::HashMap;

pub use fabro_core::Context;
use fabro_graphviz::Fidelity;
use fabro_graphviz::graph::Node;
use fabro_types::{ParallelBranchId, RunId, StageId};
use serde::{Deserialize, Serialize};

use crate::error::{Error, FailureSignature, FailureSignatureExt};
use crate::event::StageScope;
use crate::outcome::{Outcome, OutcomeExt};

/// Applies the context values derived from a completed node result.
///
/// Edge-policy projection and the durable `after_record` lifecycle use this
/// same function so conditional routes observe identical values.
pub(crate) fn apply_recorded_outcome_context(
    context: &Context,
    node_id: &str,
    outcome: &Outcome,
    retry_count: u32,
) {
    let failure_class = outcome.classified_failure_category();
    let failure_signature = failure_class
        .map(|category| {
            let signature_hint = outcome
                .failure
                .as_ref()
                .and_then(|failure| failure.signature.as_deref());
            FailureSignature::new(node_id, category, signature_hint, outcome.failure_reason())
                .to_string()
        })
        .unwrap_or_default();

    context.set(
        keys::retry_count_key(node_id),
        serde_json::json!(retry_count),
    );
    context.set(keys::OUTCOME, serde_json::json!(outcome.status.to_string()));
    context.set(
        keys::FAILURE_CLASS,
        serde_json::json!(failure_class.map_or(String::new(), |class| class.to_string())),
    );
    context.set(
        keys::FAILURE_SIGNATURE,
        serde_json::json!(failure_signature),
    );
    if let Some(preferred_label) = &outcome.preferred_label {
        context.set(keys::PREFERRED_LABEL, serde_json::json!(preferred_label));
    }
}

/// Keys whose values changed or were added in `after` relative to `before`.
/// Takes `after` by value so changed entries move instead of clone.
pub(crate) fn context_diff(
    before: &HashMap<String, serde_json::Value>,
    after: HashMap<String, serde_json::Value>,
) -> HashMap<String, serde_json::Value> {
    after
        .into_iter()
        .filter(|(key, value)| before.get(key) != Some(value))
        .collect()
}

/// [`context_diff`] restricted to user-visible keys: the diff that should
/// propagate outside the executing scope (to a parent workflow or across a
/// parallel fork), with engine-internal keys removed.
pub(crate) fn context_diff_public(
    before: &HashMap<String, serde_json::Value>,
    after: HashMap<String, serde_json::Value>,
) -> HashMap<String, serde_json::Value> {
    context_diff(before, after)
        .into_iter()
        .filter(|(key, _)| !keys::is_engine_internal_key(key))
        .collect()
}

/// Apply the per-node stage envelope (fabro-900e, ADR-0009 family):
/// context key ownership and append-only accumulation.
///
/// * `context_allow_keys` (unset = every agent-authored key admitted,
///   default-open) drops updates for keys outside the list. Keys the engine
///   stamps itself are never dropped — the envelope constrains agent-authored
///   writes only.
/// * `context_append_keys` merges the emitted delta with the key's current
///   value into one absolute array. Stages emit only their new entries; the
///   merged absolute value is written back into `updates`, so `state.record`,
///   checkpoint replay, and edge conditions all keep last-writer-wins semantics
///   on the accumulated array. An append key outside a set allowlist is dropped
///   like any other key.
///
/// Returns the sorted list of dropped keys so the caller can emit a
/// `ContextUpdateDropped` notice; drops are never silent.
pub(crate) fn enforce_stage_envelope(
    node: &Node,
    context: &Context,
    updates: &mut HashMap<String, serde_json::Value>,
) -> Vec<String> {
    let allow = node.context_allow_keys();
    let append = node.context_append_keys();
    if allow.is_none() && append.is_empty() {
        return Vec::new();
    }

    let mut dropped = Vec::new();
    if let Some(allow) = allow {
        updates.retain(|key, _| {
            keys::is_engine_stamped_key(key, &node.id) || allow.contains(&key.as_str()) || {
                dropped.push(key.clone());
                false
            }
        });
        dropped.sort();
    }

    for key in append {
        // Engine-stamped keys keep replace semantics: the engine owns
        // their shape and overwrites them on the next record anyway.
        if keys::is_engine_stamped_key(key, &node.id) {
            continue;
        }
        let Some(delta) = updates.get(key).cloned() else {
            continue;
        };
        let mut merged = match context.get(key) {
            Some(serde_json::Value::Array(items)) => items,
            Some(serde_json::Value::Null) | None => Vec::new(),
            // A pre-envelope scalar (or object) becomes the first array
            // entry instead of being silently discarded.
            Some(other) => vec![other],
        };
        match delta {
            serde_json::Value::Array(items) => merged.extend(items),
            other => merged.push(other),
        }
        updates.insert(key.to_string(), serde_json::Value::Array(merged));
    }
    dropped
}

/// Read a context key the way workflow authors write one: the declared key
/// first, then the same key with a leading `context.` stripped.
///
/// The lookup is flat. `context.plan.title` reads the literal keys
/// `context.plan.title` and `plan.title`; it never walks into a nested object.
/// Maintain [`keys::SEED_CYCLES`] after a recorded stage (fabro-45d0).
///
/// Reads the reset key's CURRENT value from `context` (the completed
/// stage's updates are already applied when this runs): equal to the
/// anchor -> increment the node's count; different (or first
/// observation) -> reset all counts, then count the recording node as
/// its first visit. Anchor and counts live in `context` itself, so the
/// counter is deterministic across checkpoint resume and immune to
/// agent writes (every call recomputes and overwrites).
pub fn update_seed_cycles(context: &Context, reset_key: &str, node_id: &str) {
    let cur = context
        .get(reset_key)
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    let anchored = context
        .get(keys::INTERNAL_SEED_CYCLE_ANCHOR)
        .and_then(|v| v.as_str().map(str::to_owned))
        .is_some_and(|anchor| anchor == cur);
    let mut cycles = if anchored {
        context
            .get(keys::SEED_CYCLES)
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    } else {
        serde_json::Map::new()
    };
    let next = cycles
        .get(node_id)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
        + 1;
    cycles.insert(node_id.to_string(), serde_json::json!(next));
    context.set(keys::INTERNAL_SEED_CYCLE_ANCHOR, serde_json::json!(cur));
    context.set(keys::SEED_CYCLES, serde_json::Value::Object(cycles));
}
pub(crate) fn lookup_flat(context: &Context, key: &str) -> Option<serde_json::Value> {
    let bare = key.strip_prefix("context.").unwrap_or(key);
    context
        .get(key)
        .or_else(|| context.get(bare))
        // Dotted path: walk into nested objects (fabro-6baf). Engine-injected
        // values like seed_cycles are objects ({node -> visits}); edge
        // conditions address them as seed_cycles.reviewer. A bare key that
        // literally exists wins over decomposition, so dotted user keys keep
        // working unchanged.
        .or_else(|| {
            let mut segments = bare.split('.');
            let first = segments.next()?;
            let mut value = context.get(first)?;
            for segment in segments {
                value = value.get(segment)?.clone();
            }
            Some(value)
        })
}

/// One entry of the [`keys::INTERNAL_PARALLEL_BRANCH_PREAMBLES`] stash.
///
/// The stash is a JSON array indexed by the parallel node's outgoing-edge
/// order. `null` entries mean the branch inherits the fork's preamble.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParallelBranchPreamble {
    pub(crate) fidelity: Fidelity,
    pub(crate) preamble: String,
}

/// Domain-specific typed accessors for workflow context values.
pub trait WorkflowContext {
    fn fidelity(&self) -> Fidelity;
    fn thread_id(&self) -> Option<String>;
    fn preamble(&self) -> String;
    fn run_id(&self) -> String;
    /// Parse `internal.run_id`, failing when the engine did not seed a
    /// valid run ID.
    fn parsed_run_id(&self) -> Result<RunId, Error>;
    fn parallel_group_id(&self) -> Option<StageId>;
    fn parallel_branch_id(&self) -> Option<ParallelBranchId>;
    /// Build the stage-level emit scope from the currently-executing node and
    /// its execution ordinal. Returns `None` for run-level emissions
    /// where no stage is active (i.e., `CURRENT_NODE` is unset).
    fn current_stage_scope(&self) -> Option<StageScope>;
}

impl WorkflowContext for Context {
    fn fidelity(&self) -> Fidelity {
        self.get_string(keys::INTERNAL_FIDELITY, "")
            .parse()
            .unwrap_or_default()
    }

    fn thread_id(&self) -> Option<String> {
        self.get(keys::INTERNAL_THREAD_ID)
            .and_then(|v| v.as_str().map(String::from))
    }

    fn preamble(&self) -> String {
        self.get_string(keys::CURRENT_PREAMBLE, "")
    }

    fn run_id(&self) -> String {
        self.get_string(keys::INTERNAL_RUN_ID, "unknown")
    }

    fn parsed_run_id(&self) -> Result<RunId, Error> {
        self.run_id()
            .parse()
            .map_err(|err| Error::handler_with_source("invalid internal run_id", err))
    }

    fn parallel_group_id(&self) -> Option<StageId> {
        self.get(keys::INTERNAL_PARALLEL_GROUP_ID)
            .and_then(|value| serde_json::from_value(value).ok())
    }

    fn parallel_branch_id(&self) -> Option<ParallelBranchId> {
        self.get(keys::INTERNAL_PARALLEL_BRANCH_ID)
            .and_then(|value| serde_json::from_value(value).ok())
    }

    fn current_stage_scope(&self) -> Option<StageScope> {
        let node_id = self
            .get(keys::CURRENT_NODE)
            .and_then(|value| value.as_str().map(String::from))?;
        Some(StageScope::from_context(self, node_id))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn new_context_is_empty() {
        let ctx = Context::new();
        assert!(ctx.snapshot().is_empty());
    }

    #[test]
    fn set_and_get() {
        let ctx = Context::new();
        ctx.set("key", serde_json::json!("value"));
        assert_eq!(ctx.get("key"), Some(serde_json::json!("value")));
    }

    #[test]
    fn lookup_flat_prefers_the_exact_key_then_strips_the_context_prefix() {
        let ctx = Context::new();
        ctx.set("context.items", serde_json::json!(["exact"]));
        ctx.set("items", serde_json::json!(["fallback"]));

        assert_eq!(
            lookup_flat(&ctx, "context.items"),
            Some(serde_json::json!(["exact"]))
        );
        // An explicit null is a value, not a miss, so it wins over the bare key.
        ctx.set("context.items", serde_json::Value::Null);
        assert_eq!(
            lookup_flat(&ctx, "context.items"),
            Some(serde_json::Value::Null)
        );

        let bare_only = Context::new();
        bare_only.set("items", serde_json::json!(["fallback"]));
        assert_eq!(
            lookup_flat(&bare_only, "context.items"),
            Some(serde_json::json!(["fallback"]))
        );
        assert_eq!(
            lookup_flat(&bare_only, "items"),
            Some(serde_json::json!(["fallback"]))
        );
        assert_eq!(lookup_flat(&bare_only, "context.missing"), None);
    }

    #[test]
    fn get_missing_key() {
        let ctx = Context::new();
        assert_eq!(ctx.get("missing"), None);
    }

    #[test]
    fn context_diff_detects_additions() {
        let before = HashMap::new();
        let mut after = HashMap::new();
        after.insert("key".to_string(), serde_json::json!("value"));
        let diff = context_diff(&before, after);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff.get("key"), Some(&serde_json::json!("value")));
    }

    #[test]
    fn context_diff_detects_changes() {
        let mut before = HashMap::new();
        before.insert("key".to_string(), serde_json::json!("old"));
        let mut after = HashMap::new();
        after.insert("key".to_string(), serde_json::json!("new"));
        let diff = context_diff(&before, after);
        assert_eq!(diff.len(), 1);
        assert_eq!(diff.get("key"), Some(&serde_json::json!("new")));
    }

    #[test]
    fn context_diff_ignores_unchanged() {
        let mut before = HashMap::new();
        before.insert("key".to_string(), serde_json::json!("same"));
        let mut after = HashMap::new();
        after.insert("key".to_string(), serde_json::json!("same"));
        let diff = context_diff(&before, after);
        assert!(diff.is_empty());
    }

    #[test]
    fn context_diff_ignores_deletions() {
        let mut before = HashMap::new();
        before.insert("removed".to_string(), serde_json::json!("gone"));
        let after = HashMap::new();
        let diff = context_diff(&before, after);
        assert!(diff.is_empty());
    }

    #[test]
    fn context_diff_public_excludes_engine_internal_keys() {
        let before = HashMap::new();
        let mut after = HashMap::new();
        after.insert("graph.goal".to_string(), serde_json::json!("child goal"));
        after.insert(
            "internal.run_id".to_string(),
            serde_json::json!("child-run"),
        );
        after.insert(
            "thread.main.current_node".to_string(),
            serde_json::json!("exit"),
        );
        after.insert("current_node".to_string(), serde_json::json!("exit"));
        after.insert("response.plan".to_string(), serde_json::json!("the plan"));
        after.insert("review.result".to_string(), serde_json::json!("approved"));

        let filtered = context_diff_public(&before, after);

        assert_eq!(filtered.len(), 2);
        assert!(filtered.contains_key("response.plan"));
        assert!(filtered.contains_key("review.result"));
    }

    #[test]
    fn get_string_with_value() {
        let ctx = Context::new();
        ctx.set("name", serde_json::json!("alice"));
        assert_eq!(ctx.get_string("name", "default"), "alice");
    }

    #[test]
    fn get_string_missing_key() {
        let ctx = Context::new();
        assert_eq!(ctx.get_string("missing", "fallback"), "fallback");
    }

    #[test]
    fn get_string_non_string_value() {
        let ctx = Context::new();
        ctx.set("num", serde_json::json!(42));
        assert_eq!(ctx.get_string("num", "default"), "default");
    }

    #[test]
    fn snapshot_is_independent() {
        let ctx = Context::new();
        ctx.set("a", serde_json::json!(1));
        let snap = ctx.snapshot();
        ctx.set("b", serde_json::json!(2));
        assert!(snap.contains_key("a"));
        assert!(!snap.contains_key("b"));
    }

    #[test]
    fn fork_is_independent() {
        let ctx = Context::new();
        ctx.set("shared", serde_json::json!("original"));

        let forked = ctx.fork();
        forked.set("shared", serde_json::json!("modified"));

        assert_eq!(ctx.get("shared"), Some(serde_json::json!("original")));
        assert_eq!(forked.get("shared"), Some(serde_json::json!("modified")));
    }

    #[test]
    fn apply_updates() {
        let ctx = Context::new();
        ctx.set("existing", serde_json::json!("old"));

        let mut updates = HashMap::new();
        updates.insert("existing".to_string(), serde_json::json!("new"));
        updates.insert("added".to_string(), serde_json::json!(true));
        ctx.apply_updates(&updates);

        assert_eq!(ctx.get("existing"), Some(serde_json::json!("new")));
        assert_eq!(ctx.get("added"), Some(serde_json::json!(true)));
    }

    fn envelope_node(attrs: &[(&str, &str)]) -> Node {
        let mut node = Node::new("planner");
        for (key, value) in attrs {
            node.attrs.insert(
                key.to_string(),
                fabro_graphviz::graph::AttrValue::String(value.to_string()),
            );
        }
        node
    }

    #[test]
    fn envelope_unset_admits_everything_and_merges_nothing() {
        let ctx = Context::new();
        let mut updates = HashMap::from([
            (
                "current_seed_id".to_string(),
                serde_json::json!("fabro-900e"),
            ),
            ("last_stage".to_string(), serde_json::json!("plan")),
        ]);
        let dropped = enforce_stage_envelope(&envelope_node(&[]), &ctx, &mut updates);
        assert!(dropped.is_empty());
        assert_eq!(updates.len(), 2);
    }

    #[test]
    fn envelope_allowlist_drops_unlisted_agent_keys_sorted() {
        let ctx = Context::new();
        let node = envelope_node(&[("context_allow_keys", "current_seed_id, workflow_painpoints")]);
        let mut updates = HashMap::from([
            (
                "current_seed_id".to_string(),
                serde_json::json!("fabro-900e"),
            ),
            ("review_verdict".to_string(), serde_json::json!("approve")),
            (
                "implementation_summary".to_string(),
                serde_json::json!("done"),
            ),
        ]);
        let dropped = enforce_stage_envelope(&node, &ctx, &mut updates);
        assert_eq!(dropped, vec!["implementation_summary", "review_verdict"]);
        assert_eq!(
            updates,
            HashMap::from([(
                "current_seed_id".to_string(),
                serde_json::json!("fabro-900e")
            )])
        );
    }

    #[test]
    fn envelope_empty_allowlist_blocks_all_agent_keys() {
        let ctx = Context::new();
        let node = envelope_node(&[("context_allow_keys", "")]);
        let mut updates = HashMap::from([(
            "current_seed_id".to_string(),
            serde_json::json!("fabro-900e"),
        )]);
        let dropped = enforce_stage_envelope(&node, &ctx, &mut updates);
        assert_eq!(dropped, vec!["current_seed_id"]);
        assert!(updates.is_empty());
    }

    #[test]
    fn envelope_never_drops_engine_stamped_keys() {
        let ctx = Context::new();
        // envelope_node builds a node with id "planner".
        let node = envelope_node(&[("context_allow_keys", "current_seed_id")]);
        let mut updates = HashMap::from([
            (keys::LAST_STAGE.to_string(), serde_json::json!("plan")),
            (keys::LAST_RESPONSE.to_string(), serde_json::json!("resp")),
            (
                keys::response_key("planner"),
                serde_json::json!("planner output"),
            ),
            (keys::COMMAND_OUTPUT.to_string(), serde_json::json!("hi")),
            (keys::PARALLEL_RESULTS.to_string(), serde_json::json!([])),
            (
                keys::INTERNAL_RUN_ID.to_string(),
                serde_json::json!("run-1"),
            ),
            (
                keys::HUMAN_GATE_SELECTED.to_string(),
                serde_json::json!("approve"),
            ),
        ]);
        let dropped = enforce_stage_envelope(&node, &ctx, &mut updates);
        assert!(dropped.is_empty());
        assert_eq!(updates.len(), 7);
    }

    #[test]
    fn envelope_drops_foreign_response_keys() {
        // response.{other_node} is agent-authored cross-stage spoofing:
        // only the completing node's own response key is engine-stamped.
        let ctx = Context::new();
        let node = envelope_node(&[("context_allow_keys", "current_seed_id")]);
        let mut updates = HashMap::from([(
            keys::response_key("reviewer"),
            serde_json::json!("forged reviewer response"),
        )]);
        let dropped = enforce_stage_envelope(&node, &ctx, &mut updates);
        assert_eq!(dropped, vec![keys::response_key("reviewer")]);
        assert!(updates.is_empty());
    }

    #[test]
    fn envelope_append_merges_delta_onto_existing_array() {
        let ctx = Context::new();
        ctx.set(
            "workflow_painpoints",
            serde_json::json!(["first", "second"]),
        );
        let node = envelope_node(&[("context_append_keys", "workflow_painpoints")]);
        let mut updates = HashMap::from([(
            "workflow_painpoints".to_string(),
            serde_json::json!(["third"]),
        )]);
        let dropped = enforce_stage_envelope(&node, &ctx, &mut updates);
        assert!(dropped.is_empty());
        assert_eq!(
            updates["workflow_painpoints"],
            serde_json::json!(["first", "second", "third"])
        );
    }

    #[test]
    fn envelope_append_wraps_legacy_scalar_and_scalar_delta() {
        let ctx = Context::new();
        ctx.set("notes", serde_json::json!("legacy scalar"));
        let node = envelope_node(&[("context_append_keys", "notes, events")]);
        let mut updates = HashMap::from([("events".to_string(), serde_json::json!("first event"))]);
        assert!(enforce_stage_envelope(&node, &ctx, &mut updates).is_empty());
        assert_eq!(updates["events"], serde_json::json!(["first event"]));

        // A second append onto the merged key wraps nothing: the value is
        // already an array.
        ctx.apply_updates(&updates);
        updates.insert("events".to_string(), serde_json::json!(["second"]));
        assert!(enforce_stage_envelope(&node, &ctx, &mut updates).is_empty());
        assert_eq!(
            updates["events"],
            serde_json::json!(["first event", "second"])
        );
    }

    #[test]
    fn envelope_append_key_outside_allowlist_is_dropped() {
        let ctx = Context::new();
        ctx.set("workflow_painpoints", serde_json::json!(["kept"]));
        // `notes` is an append key but sits outside the allowlist: the
        // allowlist wins and the delta is dropped, not merged.
        let node = envelope_node(&[
            ("context_allow_keys", "current_seed_id"),
            ("context_append_keys", "notes"),
        ]);
        let mut updates = HashMap::from([
            (
                "current_seed_id".to_string(),
                serde_json::json!("fabro-900e"),
            ),
            ("notes".to_string(), serde_json::json!(["dropped"])),
        ]);
        let dropped = enforce_stage_envelope(&node, &ctx, &mut updates);
        assert_eq!(dropped, vec!["notes"]);
        assert_eq!(
            updates,
            HashMap::from([(
                "current_seed_id".to_string(),
                serde_json::json!("fabro-900e")
            )])
        );
    }

    #[test]
    fn envelope_append_skips_engine_stamped_keys() {
        let ctx = Context::new();
        ctx.set(keys::LAST_STAGE, serde_json::json!("plan"));
        let node = envelope_node(&[("context_append_keys", "last_stage")]);
        let mut updates =
            HashMap::from([(keys::LAST_STAGE.to_string(), serde_json::json!("review"))]);
        assert!(enforce_stage_envelope(&node, &ctx, &mut updates).is_empty());
        // Replace semantics preserved: no array wrap on engine keys.
        assert_eq!(updates[keys::LAST_STAGE], serde_json::json!("review"));
    }

    #[test]
    fn default_creates_empty_context() {
        let ctx = Context::default();
        assert!(ctx.snapshot().is_empty());
    }

    #[test]
    fn run_id_default() {
        let ctx = Context::new();
        assert_eq!(ctx.run_id(), "unknown");
    }

    #[test]
    fn run_id_set() {
        let ctx = Context::new();
        ctx.set(keys::INTERNAL_RUN_ID, serde_json::json!("abc-123"));
        assert_eq!(ctx.run_id(), "abc-123");
    }

    #[test]
    fn fidelity_default() {
        let ctx = Context::new();
        assert_eq!(ctx.fidelity(), keys::Fidelity::Compact);
    }

    #[test]
    fn fidelity_set() {
        let ctx = Context::new();
        ctx.set(keys::INTERNAL_FIDELITY, serde_json::json!("full"));
        assert_eq!(ctx.fidelity(), keys::Fidelity::Full);
    }

    #[test]
    fn preamble_default() {
        let ctx = Context::new();
        assert_eq!(ctx.preamble(), "");
    }

    #[test]
    fn preamble_set() {
        let ctx = Context::new();
        ctx.set(keys::CURRENT_PREAMBLE, serde_json::json!("hello"));
        assert_eq!(ctx.preamble(), "hello");
    }

    #[test]
    fn thread_id_default() {
        let ctx = Context::new();
        assert_eq!(ctx.thread_id(), None);
    }

    #[test]
    fn thread_id_null() {
        let ctx = Context::new();
        ctx.set(keys::INTERNAL_THREAD_ID, serde_json::Value::Null);
        assert_eq!(ctx.thread_id(), None);
    }

    #[test]
    fn thread_id_set() {
        let ctx = Context::new();
        ctx.set(keys::INTERNAL_THREAD_ID, serde_json::json!("main"));
        assert_eq!(ctx.thread_id(), Some("main".to_string()));
    }

    #[test]
    fn parallel_ids_default() {
        let ctx = Context::new();
        assert_eq!(ctx.parallel_group_id(), None);
        assert_eq!(ctx.parallel_branch_id(), None);
    }

    #[test]
    fn parallel_ids_set() {
        let ctx = Context::new();
        ctx.set(
            keys::INTERNAL_PARALLEL_GROUP_ID,
            serde_json::json!("fanout@2"),
        );
        ctx.set(
            keys::INTERNAL_PARALLEL_BRANCH_ID,
            serde_json::json!("fanout@2:1"),
        );
        assert_eq!(ctx.parallel_group_id(), Some(StageId::new("fanout", 2)));
        assert_eq!(
            ctx.parallel_branch_id(),
            Some(ParallelBranchId::new(StageId::new("fanout", 2), 1))
        );
    }

    #[test]
    fn node_visit_count_default() {
        let ctx = Context::new();
        // fabro-core returns 0 for missing; workflow code expects 1 as default
        // when used in workflow context. The raw core accessor returns 0.
        assert_eq!(ctx.node_visit_count(), 0);
    }

    #[test]
    fn node_visit_count_set() {
        let ctx = Context::new();
        ctx.set(keys::INTERNAL_NODE_VISIT_COUNT, serde_json::json!(3));
        assert_eq!(ctx.node_visit_count(), 3);
    }

    #[test]
    fn current_node_id_default() {
        let ctx = Context::new();
        assert_eq!(ctx.current_node_id(), "");
    }

    #[test]
    fn current_node_id_set() {
        let ctx = Context::new();
        ctx.set(keys::CURRENT_NODE, serde_json::json!("plan"));
        assert_eq!(ctx.current_node_id(), "plan");
    }
}

#[cfg(test)]
mod seed_cycles_tests {
    use fabro_core::Context;
    use serde_json::json;

    use super::keys::{INTERNAL_SEED_CYCLE_ANCHOR, SEED_CYCLES};
    use super::update_seed_cycles;

    fn claim(context: &Context, seed: &str) {
        // Mirrors record(): updates are applied BEFORE after_record runs.
        context.set("current_seed_id", json!(seed));
    }

    #[test]
    fn counts_increment_within_seed_and_reset_on_new_seed() {
        let context = Context::new();

        // Planner claims seed A: counter starts, planner = 1
        claim(&context, "seed-a");
        update_seed_cycles(&context, "current_seed_id", "planner");
        assert_eq!(context.get(SEED_CYCLES), Some(json!({"planner": 1})));

        // Reviewer twice within seed A
        update_seed_cycles(&context, "current_seed_id", "reviewer");
        update_seed_cycles(&context, "current_seed_id", "reviewer");
        assert_eq!(
            context.get(SEED_CYCLES),
            Some(json!({"planner": 1, "reviewer": 2}))
        );

        // Same seed re-emitted (re-plan): no reset, planner increments
        claim(&context, "seed-a");
        update_seed_cycles(&context, "current_seed_id", "planner");
        assert_eq!(
            context.get(SEED_CYCLES),
            Some(json!({"planner": 2, "reviewer": 2}))
        );

        // New seed: reset, only the claiming visit counts
        claim(&context, "seed-b");
        update_seed_cycles(&context, "current_seed_id", "planner");
        assert_eq!(context.get(SEED_CYCLES), Some(json!({"planner": 1})));
        assert_eq!(
            context.get(INTERNAL_SEED_CYCLE_ANCHOR),
            Some(json!("seed-b"))
        );
    }

    #[test]
    fn absent_reset_key_value_resets_like_first_observation() {
        // Before any claim, cur = "" — first observation anchors to "".
        let context = Context::new();
        update_seed_cycles(&context, "current_seed_id", "tester");
        update_seed_cycles(&context, "current_seed_id", "tester");
        assert_eq!(context.get(SEED_CYCLES), Some(json!({"tester": 2})));

        // A claim changes the value: reset
        claim(&context, "seed-a");
        update_seed_cycles(&context, "current_seed_id", "tester");
        assert_eq!(context.get(SEED_CYCLES), Some(json!({"tester": 1})));
    }

    #[test]
    fn agent_written_seed_cycles_is_overwritten_by_engine() {
        let context = Context::new();
        context.set(SEED_CYCLES, json!({"reviewer": 99}));
        claim(&context, "seed-a");
        update_seed_cycles(&context, "current_seed_id", "planner");
        // Anchor differs from the forged state's anchor (None) -> reset.
        assert_eq!(context.get(SEED_CYCLES), Some(json!({"planner": 1})));
    }

    #[test]
    fn counts_survive_context_fork_like_checkpoint_restore() {
        // Checkpoints persist context values; the counter recomputes purely
        // from them. A forked context keeps counting across the boundary.
        let context = Context::new();
        claim(&context, "seed-a");
        update_seed_cycles(&context, "current_seed_id", "planner");
        let resumed = context.fork();
        update_seed_cycles(&resumed, "current_seed_id", "reviewer");
        assert_eq!(
            resumed.get(SEED_CYCLES),
            Some(json!({"planner": 1, "reviewer": 1}))
        );
    }
}
