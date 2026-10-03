//! Fork exit-kind classification from the run's graph source (ADR-0010
//! rev, Option A; fabro-288d, W3).
//!
//! Petri accepts `x.`-namespaced attributes at check and drops them at
//! lowering — the admitted graph does not carry them. But every run's spec
//! carries the ORIGINAL DOT text (`graph_source`), and the fork's
//! `x.kind` edges ride it verbatim. This module parses those edges and
//! maps a conclusion's final stage to its exit kind:
//!
//! - `x.kind="boundary"` + a failed finish → `Succeeded { Boundary }`
//!   (fabro-08b4: the failure parked the loop at a boundary; green with an
//!   attached failure).
//! - `x.kind="deadlock"` + a success-shaped finish → `Failed { Deadlock }`
//!   (fabro-b907: the guard routed green work to a deadlock exit; work is
//!   preserved, a human decides).
//! - `x.kind="soft"` + a success-shaped finish → `Failed { SoftStop }`
//!   (infrastructure could not finish; the next run re-enters).

use std::collections::{BTreeMap, BTreeSet};

use fabro_types::{FailureReason, RunStatus, SuccessReason};

use crate::fork_dot_edges;

/// Exit kinds by (from-node, to-node) parsed from the DOT `graph_source`.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct ExitKinds {
    edges:       BTreeMap<(String, String), String>,
    /// Edges without an `x.kind` — the plain routes a success-shaped
    /// finish takes (fabro-843d: a kind edge beside a plain one is not
    /// the exit a green conclusion used).
    plain_edges: BTreeSet<(String, String)>,
}

impl ExitKinds {
    /// Parse `x.kind="…"` edge attributes from raw DOT text through the
    /// shared edge scan ([`crate::fork_dot_edges`]), which owns the fabro
    /// files' edge syntax (`a -> b [label="…", x.kind="soft"]`,
    /// multi-line attributes and comments included).
    pub(crate) fn parse(graph_source: &str) -> Self {
        let mut edges = BTreeMap::new();
        let mut plain_edges = BTreeSet::new();
        fork_dot_edges::for_each_edge(graph_source, |from, to, attrs| match parse_x_kind(attrs) {
            Some(kind) => {
                edges.insert((from.to_string(), to.to_string()), kind.to_string());
            }
            None => {
                plain_edges.insert((from.to_string(), to.to_string()));
            }
        });
        Self { edges, plain_edges }
    }

    /// The exit kind of the edge `from -> to`, when it carries one.
    pub(crate) fn kind_of(&self, from: &str, to: &str) -> Option<&str> {
        self.edges
            .get(&(from.to_string(), to.to_string()))
            .map(String::as_str)
    }

    /// Classify a conclusion by its exit route (ADR-0010 rev Option A).
    /// `finished` is Petri's finish word (`success`, `cancelled`, else
    /// failure); `last_stage` names the stage that routed to `exit`.
    ///
    /// A success-shaped finish takes an `x.kind` exit only when that edge
    /// is the node's SOLE route to `exit` (fabro-843d): a node with a
    /// plain edge beside the kind edge — the mini's closeout carries
    /// `Seed closed` next to its failure-gated soft exit — left through
    /// the plain route, and the kind classification must not fire on a
    /// route the run never took. Failure-shaped finishes keep the
    /// unconditional reading.
    pub(crate) fn classify(
        &self,
        finished: &str,
        last_stage: Option<&str>,
        to_exit: &str,
    ) -> Option<RunStatus> {
        let from = last_stage?;
        let kind = self.kind_of(from, to_exit)?;
        let sole_exit_route = !self.has_plain_edge(from, to_exit);
        match (kind, finished) {
            ("boundary", "success") => None, // green needs no upgrade
            ("boundary", _) => Some(RunStatus::Succeeded {
                reason: SuccessReason::Boundary,
            }),
            ("deadlock", "success") if sole_exit_route => Some(RunStatus::Failed {
                reason: FailureReason::Deadlock,
            }),
            ("soft", "success") if sole_exit_route => Some(RunStatus::Failed {
                reason: FailureReason::SoftStop,
            }),
            _ => None,
        }
    }

    /// Whether `from -> to_exit` exists WITHOUT an `x.kind` attribute:
    /// the plain route a success-shaped finish takes.
    fn has_plain_edge(&self, from: &str, to_exit: &str) -> bool {
        self.plain_edges
            .contains(&(from.to_string(), to_exit.to_string()))
    }
}

fn parse_x_kind(block: &str) -> Option<&str> {
    let marker = "x.kind";
    let start = block.find(marker)?;
    let rest = block[start + marker.len()..].trim_start();
    let rest = rest.strip_prefix('=')?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(&rest[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = r#"
digraph Develop {
    start [shape=Mdiamond]
    exit  [shape=Msquare]
    planner [label="Planner"]
    planner -> exit [label="Soft exit", x.kind="soft"]
    reviewer -> exit [label="Deadlock", x.kind="deadlock"]
    tester -> exit [label="Boundary", x.kind="boundary"]
    closeout -> exit [label="Cycle complete"]
    start -> planner
}
"#;

    #[test]
    fn parses_x_kind_edges_from_graph_source() {
        let kinds = ExitKinds::parse(SOURCE);
        assert_eq!(kinds.kind_of("planner", "exit"), Some("soft"));
        assert_eq!(kinds.kind_of("reviewer", "exit"), Some("deadlock"));
        assert_eq!(kinds.kind_of("tester", "exit"), Some("boundary"));
        assert_eq!(kinds.kind_of("closeout", "exit"), None);
    }

    #[test]
    fn boundary_failure_upgrades_to_success_boundary() {
        let kinds = ExitKinds::parse(SOURCE);
        let status = kinds
            .classify("failed", Some("tester"), "exit")
            .expect("boundary failure upgrades");
        assert!(matches!(status, RunStatus::Succeeded {
            reason: SuccessReason::Boundary,
        }));
    }

    #[test]
    fn a_kind_edge_beside_a_plain_edge_does_not_fire_on_success() {
        // fabro-843d: the mini's closeout carries `Seed closed` (plain)
        // next to its failure-gated soft exit — a green conclusion left
        // through the plain route and must stay green.
        let source = r#"
            closeout -> exit [label="Seed closed"]
            closeout -> exit [x.kind="soft", label="Closeout failed", condition="outcome=failed"]
        "#;
        let kinds = ExitKinds::parse(source);
        assert_eq!(
            kinds.classify("success", Some("closeout"), "exit"),
            None,
            "the plain edge is the success route"
        );
    }

    #[test]
    fn a_sole_kind_exit_still_fires_on_success() {
        // probe-06's shape: the deadlock edge is the only exit route.
        let source =
            "flaky -> exit [x.kind=\"deadlock\", condition=\"nodes.flaky.generation >= 2\"]";
        let kinds = ExitKinds::parse(source);
        assert_eq!(
            kinds.classify("success", Some("flaky"), "exit"),
            Some(RunStatus::Failed {
                reason: FailureReason::Deadlock,
            })
        );
    }

    #[test]
    fn deadlock_and_soft_success_downgrade_to_failure() {
        let kinds = ExitKinds::parse(SOURCE);
        assert!(matches!(
            kinds.classify("success", Some("reviewer"), "exit"),
            Some(RunStatus::Failed {
                reason: FailureReason::Deadlock,
            })
        ));
        assert!(matches!(
            kinds.classify("success", Some("planner"), "exit"),
            Some(RunStatus::Failed {
                reason: FailureReason::SoftStop,
            })
        ));
    }

    #[test]
    fn plain_exits_and_green_boundaries_stay_untouched() {
        let kinds = ExitKinds::parse(SOURCE);
        assert_eq!(kinds.classify("success", Some("closeout"), "exit"), None);
        assert_eq!(kinds.classify("success", Some("tester"), "exit"), None);
    }

    /// The regression pin for fabro-8615 on the graph that exposed it
    /// (`.fabro/workflows/revisor/workflow.fabro`): comment lines carry
    /// `->` and `]`, and `start -> select` is a bare edge. Both shapes
    /// used to swallow the plain `select -> exit` route, so a green
    /// "Nothing to revise" pass classified as `Failed { SoftStop }` and
    /// the run showed a misleading `failed` (fabro-79ba).
    #[expect(
        clippy::disallowed_methods,
        reason = "the regression test reads the checked-in revisor graph synchronously"
    )]
    #[test]
    fn the_real_revisor_graph_keeps_its_plain_selector_exit() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.fabro/workflows/revisor/workflow.fabro");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let kinds = ExitKinds::parse(&source);
        assert_eq!(kinds.kind_of("select", "exit"), Some("soft"));
        assert_eq!(kinds.kind_of("analyze", "exit"), Some("soft"));
        assert_eq!(kinds.kind_of("file", "exit"), Some("soft"));
        assert!(
            kinds.has_plain_edge("select", "exit"),
            "the plain 'Nothing to revise' route survives the comment lines"
        );
        assert_eq!(
            kinds.classify("success", Some("select"), "exit"),
            None,
            "a green 'Nothing to revise' pass stays green"
        );
        assert!(
            kinds
                .edges
                .keys()
                .chain(kinds.plain_edges.iter())
                .all(|(from, to)| !from.starts_with("//")
                    && !to.starts_with("//")
                    && !from.contains(' ')
                    && !to.contains(' ')),
            "no comment fragment becomes an edge key: {:?} {:?}",
            kinds.edges,
            kinds.plain_edges
        );
    }
}
