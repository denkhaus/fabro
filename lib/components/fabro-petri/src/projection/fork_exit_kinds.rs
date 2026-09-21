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

use std::collections::BTreeMap;

use fabro_types::{FailureReason, RunStatus, SuccessReason};

/// Exit kinds by (from-node, to-node) parsed from the DOT `graph_source`.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct ExitKinds {
    edges: BTreeMap<(String, String), String>,
}

impl ExitKinds {
    /// Parse `x.kind="…"` edge attributes from raw DOT text. Handles the
    /// fabro files' edge syntax (`a -> b [label="…", x.kind="soft"]`,
    /// multi-line attributes included).
    pub(crate) fn parse(graph_source: &str) -> Self {
        let mut edges = BTreeMap::new();
        // Ein Kantenblock beginnt bei "name ->" und endet mit "]".
        let mut rest = graph_source;
        while let Some(arrow) = rest.find("->") {
            let head = &rest[..arrow];
            let from = head
                .lines()
                .last()
                .unwrap_or_default()
                .trim()
                .trim_matches(|c: char| c.is_whitespace() || c == '"' || c == ',')
                .to_string();
            let after = &rest[arrow + 2..];
            let Some(bracket_end) = after.find(']') else {
                break;
            };
            let block = &after[..bracket_end];
            let tail = &after[bracket_end + 1..];
            let to = block
                .lines()
                .next()
                .unwrap_or_default()
                .split('[')
                .next()
                .unwrap_or_default()
                .trim()
                .to_string();
            let to = to.split_whitespace().next().unwrap_or_default().to_string();
            if let Some(kind) = parse_x_kind(block) {
                if !from.is_empty() && !to.is_empty() {
                    edges.insert((from, to), kind.to_string());
                }
            }
            rest = tail;
        }
        Self { edges }
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
    pub(crate) fn classify(
        &self,
        finished: &str,
        last_stage: Option<&str>,
        to_exit: &str,
    ) -> Option<RunStatus> {
        let from = last_stage?;
        let kind = self.kind_of(from, to_exit)?;
        match (kind, finished) {
            ("boundary", "success") => None, // green needs no upgrade
            ("boundary", _) => Some(RunStatus::Succeeded {
                reason: SuccessReason::Boundary,
            }),
            ("deadlock", "success") => Some(RunStatus::Failed {
                reason: FailureReason::Deadlock,
            }),
            ("soft", "success") => Some(RunStatus::Failed {
                reason: FailureReason::SoftStop,
            }),
            _ => None,
        }
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
}
