//! Static context-key vocabulary shared by the workflow engine and validators.
//!
//! The engine injects, stamps, and hides these keys while a run executes.
//! This module is the single source of truth for their names, prefixes, and
//! classification predicates: `fabro-workflow` re-exports it as
//! `crate::context::keys` for its call sites, and `fabro-validate` uses the
//! classifiers to lint graph attributes (for example `preamble_allow_keys`)
//! against the same key semantics the render path applies.

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
pub const HUMAN_GATE_PREFIX: &str = "human.gate.";
pub const HUMAN_GATE_SELECTED: &str = "human.gate.selected";
pub const HUMAN_GATE_LABEL: &str = "human.gate.label";
pub const HUMAN_GATE_TEXT: &str = "human.gate.text";

// --- parallel.* keys ---
pub const PARALLEL_RESULTS: &str = "parallel.results";
pub const PARALLEL_BRANCH_COUNT: &str = "parallel.branch_count";

/// Runtime-only keys stripped from durable context projections.
pub const TRANSIENT_CONTEXT_KEYS: &[&str] = &[
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
pub fn is_preamble_hidden_key(key: &str) -> bool {
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

/// Returns `true` for keys the engine or its handlers stamp into
/// `context_updates` (or straight into context) regardless of model
/// output. The stage envelope (fabro-900e) never drops these:
/// workflow authors constrain agent-authored keys only, and dropping
/// an engine stamp would break templating, fan-in, and completion
/// payloads the engine itself depends on.
///
/// `node_id` scopes the `response.*` exemption to the completing
/// node's own response key: handlers stamp `response.{node_id}` for
/// the node they execute, so an agent-authored write to another
/// node's response key is cross-stage spoofing and stays droppable.
/// This is drift protection (ADR-0009), not adversarial containment:
/// engine-stamped keys an agent re-declares in its own updates are
/// still overwritten by the engine's own stamp on the same map.
#[must_use]
pub fn is_engine_stamped_key(key: &str, node_id: &str) -> bool {
    is_engine_internal_key(key)
        || key == response_key(node_id)
        || key.starts_with(HUMAN_GATE_PREFIX)
        || matches!(
            key,
            LAST_STAGE
                | LAST_RESPONSE
                | OUTCOME
                | FAILURE_CLASS
                | FAILURE_SIGNATURE
                | PREFERRED_LABEL
                | COMMAND_OUTPUT
                | PARALLEL_RESULTS
                | PARALLEL_BRANCH_COUNT
                | SEED_CYCLES
        )
}

/// Returns `true` for context keys the engine itself can write into a run
/// context that still render in a prompt preamble's `## Context` section:
/// engine-stamped or handler-written keys minus the preamble-hidden ones.
///
/// `preamble_allow_keys` validation treats these as the always-producible
/// core. A listed key that is neither engine-renderable nor declared in any
/// node's `context_allow_keys` is a probable typo (warning, never blocking).
#[must_use]
pub fn is_engine_renderable_key(key: &str) -> bool {
    // The empty `node_id` deliberately disables the response-key exemption
    // in `is_engine_stamped_key`: no node's `response.{id}` may count as
    // engine-written here. Response keys are preamble-hidden anyway, so the
    // hidden-key check in front makes that arm unreachable for them.
    !is_preamble_hidden_key(key) && (is_engine_stamped_key(key, "") || key == REVIEW_TARGET)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_engine_renderable_key_classifies_correctly() {
        assert!(is_engine_renderable_key(COMMAND_OUTPUT));
        assert!(is_engine_renderable_key(FAILURE_CLASS));
        assert!(is_engine_renderable_key(FAILURE_SIGNATURE));
        assert!(is_engine_renderable_key(REVIEW_TARGET));
        assert!(is_engine_renderable_key(SEED_CYCLES));
        assert!(is_engine_renderable_key(PARALLEL_RESULTS));
        assert!(is_engine_renderable_key(&format!(
            "{HUMAN_GATE_PREFIX}approve"
        )));

        // Hidden keys never render in a preamble, so they are not allowlistable.
        assert!(!is_engine_renderable_key(INTERNAL_RUN_ID));
        assert!(!is_engine_renderable_key(&response_key("plan")));
        assert!(!is_engine_renderable_key(OUTCOME));
        assert!(!is_engine_renderable_key(LAST_STAGE));

        // Agent-authored keys are producible only via context_allow_keys.
        assert!(!is_engine_renderable_key("current_seed_brief"));
    }

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
