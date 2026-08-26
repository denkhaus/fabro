pub mod keys {
    //! Static context key constants and helper functions for dynamic keys.
    //!
    //! All context keys used across the engine, handlers, and preamble are
    //! defined here to prevent typos and improve discoverability.

    // --- Top-level keys ---
    pub const CURRENT_NODE: &str = "current_node";
    pub const OUTCOME: &str = "outcome";
    pub const FAILURE_CLASS: &str = "failure_class";
    pub const FAILURE_SIGNATURE: &str = "failure_signature";
    pub const PREFERRED_LABEL: &str = "preferred_label";
    pub const LAST_STAGE: &str = "last_stage";
    pub const LAST_RESPONSE: &str = "last_response";
    pub const REVIEW_TARGET: &str = "review_target";

    // --- graph.* keys ---
    pub const GRAPH_GOAL: &str = "graph.goal";

    // --- internal.* keys ---
    pub const INTERNAL_RUN_ID: &str = "internal.run_id";
    pub const INTERNAL_WORK_DIR: &str = "internal.work_dir";
    pub const INTERNAL_FIDELITY: &str = "internal.fidelity";
    pub const INTERNAL_THREAD_ID: &str = "internal.thread_id";
    pub const INTERNAL_NODE_VISIT_COUNT: &str = "internal.node_visit_count";
    /// 1-based stage execution ordinal for the currently-executing node — the
    /// numeric component of the external `StageId`. Runtime-only: reserved by
    /// the lifecycle when a stage execution first becomes observable and
    /// stripped from durable context snapshots, unlike
    /// [`INTERNAL_NODE_VISIT_COUNT`], which remains the checkpointed graph
    /// visit.
    pub const INTERNAL_STAGE_EXECUTION_ORDINAL: &str = "internal.stage_execution_ordinal";
    pub const INTERNAL_PARENT_PREAMBLE: &str = "internal.parent_preamble";
    pub const INTERNAL_PARALLEL_GROUP_ID: &str = "internal.parallel_group_id";
    pub const INTERNAL_PARALLEL_BRANCH_ID: &str = "internal.parallel_branch_id";
    /// Stash of pre-rendered per-branch preambles for a parallel node; see
    /// [`super::ParallelBranchPreamble`] for the entry shape and the
    /// producer/consumer contract.
    pub const INTERNAL_PARALLEL_BRANCH_PREAMBLES: &str = "internal.parallel_branch_preambles";
    /// Last observed value of the graph's `cycle_counter_reset_key` context
    /// key. Engine bookkeeping for [`SEED_CYCLES`]: a value change resets
    /// the cycle counts. Hidden from preambles (internal.*), durable.
    pub const INTERNAL_SEED_CYCLE_ANCHOR: &str = "internal.seed_cycle_anchor";
    /// Exit-kind capture (fabro-b907): set when an edge targets the exit
    /// node; carries the exit node's `kind` attribute (default "natural").
    /// Consumed by the terminal-event builder to classify soft stops
    /// (deadlock-for-human, infrastructure) vs natural completion. Hidden
    /// from preambles (internal.*).
    pub const INTERNAL_EXIT_KIND: &str = "internal.exit_kind";

    /// Per-node stage visits since the last value change of the graph's
    /// `cycle_counter_reset_key` context key (e.g. the seed id the planner
    /// claimed). Object: { node_id -> visits_since_baseline }. Only set
    /// when the graph declares the attribute; agents use it for
    /// deterministic cycle guards instead of counting preamble history.
    /// Set after `record`, so a freshly recorded visit IS included.
    pub const INTERNAL_SEED_CYCLES: &str = "internal.seed_cycles";

    // --- current.* keys ---
    pub const CURRENT_PREAMBLE: &str = "current.preamble";

    // --- public engine-injected keys (preamble-visible) ---
    /// Per-node completed-visit counts since the graph's
    /// `cycle_counter_reset_key` context key last changed value (see
    /// [`INTERNAL_SEED_CYCLE_ANCHOR`]). Injected after each record when the
    /// graph declares the attribute; plain key on purpose so agents see it
    /// in their `## Context` section (internal.* is preamble-hidden).
    /// Agents cannot corrupt it — every record overwrites it.
    pub const SEED_CYCLES: &str = "seed_cycles";

    // --- command.* keys ---
    pub const COMMAND_OUTPUT: &str = "command.output";

    // --- human.gate.* keys ---
    pub const HUMAN_GATE_SELECTED: &str = "human.gate.selected";
    pub const HUMAN_GATE_LABEL: &str = "human.gate.label";
    pub const HUMAN_GATE_TEXT: &str = "human.gate.text";

    // --- parallel.* keys ---
    pub const PARALLEL_RESULTS: &str = "parallel.results";
    pub const PARALLEL_BRANCH_COUNT: &str = "parallel.branch_count";

    /// Runtime-only keys stripped from durable context projections.
    pub(crate) const TRANSIENT_CONTEXT_KEYS: &[&str] = &[
        CURRENT_PREAMBLE,
        INTERNAL_PARALLEL_BRANCH_PREAMBLES,
        INTERNAL_STAGE_EXECUTION_ORDINAL,
    ];

    // --- Prefix constants (for filtering and dynamic keys) ---
    pub const GRAPH_PREFIX: &str = "graph.";
    pub const INTERNAL_PREFIX: &str = "internal.";
    /// Engine-reserved key names starting with "current": the singular
    /// `current_node` plus the dotted `current.*` namespace. Deliberately NOT
    /// the bare prefix "current" — user keys like `current_seed_id` or
    /// `current_task` are natural names and must not be swallowed by the
    /// engine-internal filter (observed in run 01M0NJ3QZ1FK53X9DK3BBAN2ED:
    /// planner-emitted `current_seed_id`/`current_seed_brief` never reached
    /// the next stage's preamble at any fidelity).
    pub const CURRENT_NODE_KEY: &str = "current_node";
    pub const CURRENT_PREFIX: &str = "current.";
    pub const THREAD_PREFIX: &str = "thread.";
    pub const RESPONSE_PREFIX: &str = "response.";
    pub const INTERNAL_RETRY_COUNT_PREFIX: &str = "internal.retry_count.";

    /// Keys the prompt preamble never renders as context values: engine
    /// bookkeeping, per-thread cursors, and values the per-stage sections
    /// already present.
    #[must_use]
    pub(crate) fn is_preamble_hidden_key(key: &str) -> bool {
        is_engine_internal_key(key)
            || key.starts_with(RESPONSE_PREFIX)
            || key == OUTCOME
            || key == LAST_STAGE
            || key == LAST_RESPONSE
            || key == PREFERRED_LABEL
    }

    // --- Helper functions for dynamic keys ---

    #[must_use]
    pub fn response_key(node_id: &str) -> String {
        format!("{RESPONSE_PREFIX}{node_id}")
    }

    #[must_use]
    pub fn thread_current_node_key(thread_id: &str) -> String {
        format!("{THREAD_PREFIX}{thread_id}.current_node")
    }

    #[must_use]
    pub fn graph_attr_key(attr: &str) -> String {
        format!("{GRAPH_PREFIX}{attr}")
    }

    #[must_use]
    pub fn retry_count_key(node_id: &str) -> String {
        format!("{INTERNAL_RETRY_COUNT_PREFIX}{node_id}")
    }

    /// Returns `true` for engine-internal keys that should not propagate from
    /// child to parent workflow contexts.
    #[must_use]
    pub fn is_engine_internal_key(key: &str) -> bool {
        key.starts_with(INTERNAL_PREFIX)
            || key.starts_with(GRAPH_PREFIX)
            || key.starts_with(THREAD_PREFIX)
            || key == CURRENT_NODE_KEY
            || key.starts_with(CURRENT_PREFIX)
    }

    pub use fabro_graphviz::Fidelity;

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn response_key_formats_correctly() {
            assert_eq!(response_key("plan"), "response.plan");
        }

        #[test]
        fn thread_current_node_key_formats_correctly() {
            assert_eq!(thread_current_node_key("main"), "thread.main.current_node");
        }

        #[test]
        fn graph_attr_key_formats_correctly() {
            assert_eq!(graph_attr_key("goal"), "graph.goal");
        }

        #[test]
        fn retry_count_key_formats_correctly() {
            assert_eq!(retry_count_key("plan"), "internal.retry_count.plan");
        }

        #[test]
        fn is_engine_internal_key_classifies_correctly() {
            // Keys that ARE engine-internal (should not propagate)
            assert!(is_engine_internal_key("internal.run_id"));
            assert!(is_engine_internal_key("internal.fidelity"));
            assert!(is_engine_internal_key("internal.parent_preamble"));
            assert!(is_engine_internal_key("graph.goal"));
            assert!(is_engine_internal_key("thread.main.current_node"));
            assert!(is_engine_internal_key("current.preamble"));
            assert!(is_engine_internal_key("current_node"));
            // Natural user keys starting with "current" are NOT internal
            assert!(!is_engine_internal_key("current_seed_id"));
            assert!(!is_engine_internal_key("current_task"));
            assert!(!is_engine_internal_key("currently_running"));

            // Keys that are NOT engine-internal (should propagate)
            assert!(!is_engine_internal_key("response.plan"));
            assert!(!is_engine_internal_key("command.output"));
            assert!(!is_engine_internal_key("outcome"));
            assert!(!is_engine_internal_key("last_stage"));
            assert!(!is_engine_internal_key("review.result"));
            assert!(!is_engine_internal_key(REVIEW_TARGET));
            assert!(!is_engine_internal_key("user.name"));
        }
    }
}

use std::collections::HashMap;

pub use fabro_core::Context;
use fabro_graphviz::Fidelity;
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
