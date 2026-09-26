//! Edge-condition presence from the run's graph source (fabro-b00c
//! option a, user decision 2026-09-26): a FAILED leg whose onward route
//! is an EXPLICIT conditional edge (`condition="…"`) is control flow and
//! a green conclusion stands; only a failure an UNCONDITIONAL (catch-all)
//! edge consumed refuses the green. Petri's TerminalNode completion drew
//! exactly this line first — this module brings the projection's
//! conclusion in line with it.

use std::collections::BTreeMap;

/// Edges by (from-node, to-node) that carry a `condition=` attribute.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct EdgeConditions {
    conditional: BTreeMap<(String, String), ()>,
}

impl EdgeConditions {
    /// Parse `condition=` presence per edge from raw DOT text: a
    /// line-based scan that reads one edge statement at a time — a bare
    /// edge (`a -> b`) owns NO attributes, an edge whose statement opens
    /// `[` keeps collecting attribute lines until its `]`. A bracket-less
    /// edge must never inherit the NEXT edge's block.
    pub(crate) fn parse(graph_source: &str) -> Self {
        let mut conditional = BTreeMap::new();
        let lines: Vec<&str> = graph_source.lines().collect();
        let mut index = 0;
        while let Some(line) = lines.get(index) {
            index += 1;
            let Some((from, to, rest)) = Self::edge_ends(line) else {
                continue;
            };
            if from.is_empty() || to.is_empty() {
                continue;
            }
            // The attribute block: the rest of THIS line after the target,
            // extended while the brackets stay unbalanced.
            let mut block = rest.to_string();
            let mut open = block.matches('[').count();
            let mut close = block.matches(']').count();
            while open > close {
                let Some(next) = lines.get(index) else { break };
                index += 1;
                block.push(' ');
                block.push_str(next);
                open += next.matches('[').count();
                close += next.matches(']').count();
            }
            if has_condition(&block) {
                conditional.insert((from, to), ());
            }
        }
        Self { conditional }
    }

    /// The edge statement's `(from, to, attribute-rest)`, when the line is
    /// an edge: `a -> b [attrs…` carries the block's opening; `a -> b`
    /// carries none.
    fn edge_ends(line: &str) -> Option<(String, String, &str)> {
        let arrow = line.find("->")?;
        let head = line[..arrow].trim();
        let from = head.split_whitespace().last()?.to_string();
        let after = line[arrow + 2..].trim();
        let (to, rest) = match after.find('[') {
            Some(at) => (after[..at].trim(), &after[at..]),
            None => (after, ""),
        };
        let to = to.split_whitespace().next()?.to_string();
        Some((from, to, rest))
    }

    /// Whether the edge `from -> to` carries a condition: an explicit,
    /// condition-gated route.
    pub(crate) fn is_conditional(&self, from: &str, to: &str) -> bool {
        self.conditional
            .contains_key(&(from.to_string(), to.to_string()))
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
}
