//! Edge-condition presence from the run's graph source (fabro-b00c
//! option a, user decision 2026-09-26): a FAILED leg whose onward route
//! is an EXPLICIT conditional edge (`condition="…"`) is control flow and
//! a green conclusion stands; only a failure an UNCONDITIONAL (catch-all)
//! edge consumed refuses the green. Petri's TerminalNode completion drew
//! exactly this line first — this module brings the projection's
//! conclusion in line with it.

use std::collections::BTreeSet;

use crate::fork_dot_edges;

/// Edges by (from-node, to-node) that carry a `condition=` attribute.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct EdgeConditions {
    conditional: BTreeSet<(String, String)>,
}

impl EdgeConditions {
    /// Parse `condition=` presence per edge from raw DOT text through the
    /// shared edge scan ([`crate::fork_dot_edges`]): one edge statement
    /// at a time — a bare edge (`a -> b`) owns NO attributes, and a
    /// bracket-less edge never inherits the NEXT edge's block. Comments
    /// are stripped first, so a commented-out route (`// merge -> exit`)
    /// is not read as a conditional one.
    pub(crate) fn parse(graph_source: &str) -> Self {
        let mut conditional = BTreeSet::new();
        fork_dot_edges::for_each_edge(graph_source, |from, to, attrs| {
            if has_condition(attrs) {
                conditional.insert((from.to_string(), to.to_string()));
            }
        });
        Self { conditional }
    }

    /// Whether the edge `from -> to` carries a condition: an explicit,
    /// condition-gated route.
    pub(crate) fn is_conditional(&self, from: &str, to: &str) -> bool {
        self.conditional
            .contains(&(from.to_string(), to.to_string()))
    }
}

/// The block carries a `condition=` attribute (quoted value or bare).
fn has_condition(block: &str) -> bool {
    if let Some(at) = block.find("condition") {
        let rest = block[at + "condition".len()..].trim_start();
        rest.starts_with('=')
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = r#"
digraph Develop {
    start [shape=Mdiamond]
    exit  [shape=Msquare]
    work [script="false"]
    fix [script="true"]
    start -> work
    work -> fix [label="Route on failure", condition="outcome=failed"]
    work -> exit [label="Green exit"]
    fix -> exit
}
"#;

    #[test]
    fn conditional_and_unconditional_edges_classify() {
        let edges = EdgeConditions::parse(SOURCE);
        assert!(edges.is_conditional("work", "fix"));
        assert!(!edges.is_conditional("work", "exit"), "no condition");
        assert!(!edges.is_conditional("start", "work"));
        assert!(!edges.is_conditional("fix", "exit"));
    }

    #[test]
    fn multiline_attribute_blocks_parse() {
        let source = "a -> b [label=\"X\",\n  condition=\"outcome=failed\",\n  x.kind=\"soft\"]";
        assert!(EdgeConditions::parse(source).is_conditional("a", "b"));
    }

    /// The regression pin for fabro-8615: the conductor carries its merge
    /// leg as comments today (`// survey -> merge [...]`, `// merge -> exit
    /// [x.kind="soft", ...]`). A commented-out route is no route — the old
    /// walk read both as conditional ones.
    #[test]
    fn a_commented_out_leg_is_no_conditional_route() {
        let source = "\
digraph Conductor {
    graph [goal=\"Run the line\"]
    start [shape=Mdiamond]
    exit [shape=Msquare]
    survey [shape=tab]
    develop [shape=tab]
    // survey -> merge   [label=\"Merge needed\", condition=\"preferred_label=\\\"Merge needed\\\"\"]
    survey -> develop [label=\"Work\", condition=\"preferred_label=\\\"Work\\\"\"]
    survey -> exit    [label=\"Nothing to do\", condition=\"preferred_label=\\\"Nothing to do\\\"\"]
    // merge -> exit [x.kind=\"soft\", label=\"Merge child failed\", condition=\"preferred_label=\\\"Merge child failed\\\"\"]
    develop -> exit   [label=\"Tracker empty\", condition=\"preferred_label=\\\"Tracker empty\\\"\"]
}";
        let edges = EdgeConditions::parse(source);
        assert!(
            !edges.is_conditional("survey", "merge"),
            "the commented-out merge leg is no route"
        );
        assert!(!edges.is_conditional("merge", "exit"));
        assert!(edges.is_conditional("survey", "develop"));
        assert!(edges.is_conditional("survey", "exit"));
        assert!(edges.is_conditional("develop", "exit"));
    }
}
