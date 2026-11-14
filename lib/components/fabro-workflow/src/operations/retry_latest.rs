//! The retry-on-latest operation (work#1320, user grill 2026-11-14).
//!
//! A plain retry replays the SOURCE's pinned workflow version and its
//! worktree base: a fault fixed on the target branch after the source
//! was created is unreachable (the retry-fix-lab evidence matrix,
//! 2026-11-14). Retry-on-latest re-drives the run on current bases:
//!
//! - the workflow version of the NEWEST run of the same workflow slug (or an
//!   explicit version the caller names), re-admitted fresh;
//! - the run branch's checkpoint commits REBASED onto the target branch's
//!   current tip, so the kept work rides on the fix;
//! - the entry stage mapped by NODE ID; a graph that dropped or renamed the
//!   faulting node parks the successor for a human instead of firing into a
//!   structure the seeded state does not fit.
//!
//! The pure half lives here (compatibility); the server-side wiring
//! resolves currency, re-admits through the create path, carries the
//! checkpoint work, and reports a failed successor (created-then-
//! failed, the seeded-fork pattern) when the graphs or the carry
//! refuse.

use fabro_types::RunGraph;

/// What retry-on-latest decided about the graphs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RetryLatestCompat {
    /// The new graph carries every node id the old graph named; the
    /// seeded state's node references stay meaningful.
    Compatible,
    /// The new graph lost node ids the old graph (and therefore the
    /// seeded petri state) still references. The successor must park.
    NodesMissing { missing: Vec<String> },
}

/// Compare the old run's graph with the re-admitted version's graph by
/// node id. The successor's seeded engine state names nodes of the OLD
/// graph; every one of those ids must exist in the NEW graph for the
/// mapping by node id to hold. New nodes are fine — the fix may add
/// stages the old state simply never visited.
pub fn graph_compat(old: &RunGraph, new: &RunGraph) -> RetryLatestCompat {
    let new_ids: std::collections::BTreeSet<&str> = new.nodes.keys().map(String::as_str).collect();
    let missing = old
        .nodes
        .keys()
        .map(String::as_str)
        .filter(|id| !new_ids.contains(*id))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if missing.is_empty() {
        RetryLatestCompat::Compatible
    } else {
        RetryLatestCompat::NodesMissing { missing }
    }
}

#[cfg(test)]
mod tests {
    use fabro_types::RunGraph;

    use super::*;

    fn graph_with_ids(ids: &[&str]) -> RunGraph {
        let mut graph = RunGraph::new("g");
        for id in ids {
            graph
                .nodes
                .insert((*id).to_string(), fabro_types::RunGraphNode {
                    label: (*id).to_string(),
                    kind:  fabro_types::StageHandler::Agent,
                });
        }
        graph
    }

    #[test]
    fn identical_node_sets_are_compatible() {
        let old = graph_with_ids(&["start", "work", "check", "exit"]);
        let new = graph_with_ids(&["start", "work", "check", "exit"]);
        assert_eq!(graph_compat(&old, &new), RetryLatestCompat::Compatible);
    }

    #[test]
    fn added_nodes_stay_compatible() {
        let old = graph_with_ids(&["start", "check"]);
        let new = graph_with_ids(&["start", "check", "report"]);
        assert_eq!(graph_compat(&old, &new), RetryLatestCompat::Compatible);
    }

    #[test]
    fn a_dropped_node_names_what_is_missing() {
        let old = graph_with_ids(&["start", "work", "check"]);
        let new = graph_with_ids(&["start", "work", "verify"]);
        assert_eq!(graph_compat(&old, &new), RetryLatestCompat::NodesMissing {
            missing: vec!["check".to_string()],
        });
    }
}
