//! The fabro half of the preamble family's enforcement (fabro-70af PART
//! 2b): the run's stage envelopes, lowered onto petri's
//! [`PreamblePolicy`] seam.
//!
//! The petri fork carries the rendering enforcement —
//! `petri_attractor_steps::fork_preamble_policy` consults a
//! [`PreamblePolicySource`] installed as the [`PreamblePolicyHandle`]
//! capability at `Preamble::render` / `AgentConfig::assemble`, and its
//! consume-keys half tombstones consumed context keys at merge. This
//! module is the source fabro installs: it maps a node's
//! `x.preamble_stages_ignore` / `x.preamble_stages_latest_only` /
//! `x.context_allow_keys` / `x.preamble_allow_keys` /
//! `x.context_consume_keys` / `x.preamble_budget_kb` /
//! `x.preamble_output_max_lines` envelope ([`crate::fork_stage_envelope`])
//! onto the policy, keyed by node name. A run without envelopes installs
//! nothing and the steps keep the default no-op policy.
//!
//! This file is fork-owned (new file, `fork_` prefix): an upstream merge
//! cannot silently absorb it. The upstream touch point is the single
//! capability install in `runtime.rs`, pinned by the tests here.

use std::sync::Arc;

use petri_attractor_steps::fork_preamble_policy::{
    PreamblePolicy, PreamblePolicyHandle, PreamblePolicySource,
};

use crate::fork_stage_envelope::StageEnvelopes;

/// The run's stage envelopes as a [`PreamblePolicySource`]: each node's
/// policy is its envelope lowered onto the fork seam.
pub struct FabroPreamblePolicy {
    envelopes: Arc<StageEnvelopes>,
}

impl FabroPreamblePolicy {
    /// The source over a run's parsed envelopes.
    #[must_use]
    pub fn new(envelopes: Arc<StageEnvelopes>) -> Self {
        Self { envelopes }
    }
}

impl PreamblePolicySource for FabroPreamblePolicy {
    fn policy(&self, node: &str) -> PreamblePolicy {
        policy_for(&self.envelopes, node)
    }
}

/// The capability the runtime installs: the envelopes as the fork seam's
/// `PreamblePolicyHandle`.
#[must_use]
pub fn capability(envelopes: Arc<StageEnvelopes>) -> PreamblePolicyHandle {
    PreamblePolicyHandle(Arc::new(FabroPreamblePolicy::new(envelopes)))
}

/// The policy for `node`: its envelope's preamble family lowered onto the
/// seam, or the no-op default for a node without one. Expansion clones
/// (`build#2`) carry the base node's policy, the same rule the envelope
/// lookup applies everywhere.
#[must_use]
pub fn policy_for(envelopes: &StageEnvelopes, node: &str) -> PreamblePolicy {
    let Some(envelope) = envelopes.envelope(node) else {
        return PreamblePolicy::default();
    };
    PreamblePolicy {
        node:                node.to_string(),
        stages_ignore:       envelope.preamble_stages_ignore.clone(),
        stages_latest_only:  envelope.preamble_stages_latest_only,
        context_allow_keys:  envelope.context_allow_keys.clone(),
        preamble_allow_keys: envelope.preamble_allow_keys.clone(),
        consume_keys:        envelope.context_consume_keys.clone(),
        budget_kb:           envelopes.preamble_budget_kb(node),
        output_max_lines:    envelope
            .preamble_output_max_lines
            .and_then(|lines| usize::try_from(lines).ok()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelopes(source: &str) -> Arc<StageEnvelopes> {
        Arc::new(StageEnvelopes::parse(source))
    }

    const WORKFLOW: &str = r#"digraph W {
        graph [goal="G", x.preamble_budget_kb=24]
        planner [
            x.preamble_stages_ignore="survey,develop",
            x.preamble_stages_latest_only=true,
            x.context_allow_keys="current_seed_id,journal",
            x.preamble_allow_keys="journal",
            x.context_consume_keys="review_verdict",
            x.preamble_output_max_lines=200
        ]
        implementer [x.preamble_budget_kb=8]
    }"#;

    /// Presence pin (fork feature, fabro-70af PART 2b): the planner's
    /// envelope lowers onto the seam policy field for field.
    #[test]
    fn the_preamble_family_lowers_onto_the_seam_policy() {
        let policy = policy_for(&envelopes(WORKFLOW), "planner");
        assert_eq!(policy.node, "planner");
        assert_eq!(policy.stages_ignore, ["survey", "develop"]);
        assert!(policy.stages_latest_only);
        assert_eq!(
            policy.context_allow_keys.as_deref(),
            Some(&["current_seed_id".to_string(), "journal".to_string()][..])
        );
        assert_eq!(
            policy.preamble_allow_keys.as_deref(),
            Some(&["journal".to_string()][..])
        );
        assert_eq!(policy.consume_keys, ["review_verdict"]);
        assert_eq!(policy.output_max_lines, Some(200));
        // No node budget: the graph's explicit one governs.
        assert_eq!(policy.budget_kb, Some(24));
    }

    /// A node's own budget wins over the graph's; a node without an
    /// envelope gets the no-op default, and a clone carries its base
    /// node's policy.
    #[test]
    fn budgets_and_node_resolution_follow_the_envelope_rules() {
        let parsed = envelopes(WORKFLOW);
        assert_eq!(policy_for(&parsed, "implementer").budget_kb, Some(8));
        assert_eq!(policy_for(&parsed, "implementer#3").budget_kb, Some(8));
        assert_eq!(
            policy_for(&parsed, "reviewer"),
            PreamblePolicy::default(),
            "no envelope: the no-op default, exactly upstream rendering"
        );
    }

    /// Presence pin: the capability is the fabro source over the
    /// envelopes — what `runtime.rs` installs as the seam handle.
    #[test]
    fn the_capability_answers_with_the_envelope_policy() {
        let parsed = envelopes(WORKFLOW);
        let FabroPreamblePolicy { envelopes } = FabroPreamblePolicy::new(parsed.clone());
        let handle = capability(parsed);
        let PreamblePolicyHandle(source) = &handle;
        assert_eq!(source.policy("planner"), policy_for(&envelopes, "planner"));
    }
}
