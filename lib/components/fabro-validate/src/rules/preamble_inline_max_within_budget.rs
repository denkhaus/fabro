use fabro_graphviz::graph::Graph;
use fabro_types::graph::DEFAULT_PREAMBLE_BUDGET_KB;

use crate::{Diagnostic, LintRule, Severity};

pub(super) fn rule() -> Box<dyn LintRule> {
    Box::new(Rule)
}

struct Rule;

impl LintRule for Rule {
    fn name(&self) -> &'static str {
        "preamble_inline_max_within_budget"
    }

    fn apply(&self, graph: &Graph) -> Vec<Diagnostic> {
        let budget_kb = graph
            .preamble_budget_kb()
            .unwrap_or(DEFAULT_PREAMBLE_BUDGET_KB);
        let mut diagnostics = Vec::new();
        if let Some(graph_max) = graph.preamble_inline_max_kb() {
            if graph_max > budget_kb {
                diagnostics.push(Diagnostic {
                    rule: self.name().to_string(),
                    severity: Severity::Warning,
                    message: format!(
                        "Graph preamble_inline_max_kb={graph_max} exceeds the aggregate \
                         preamble_budget_kb={budget_kb}: the aggregate demote pass will \
                         re-demote values the raised inline ceiling kept inline"
                    ),
                    node_id: None,
                    edge: None,
                    fix: Some(format!(
                        "Raise preamble_budget_kb to at least {graph_max} or lower \
                         preamble_inline_max_kb"
                    )),
                    ..Diagnostic::default()
                });
            }
        }
        for node in graph.nodes.values() {
            let Some(node_max) = node.preamble_inline_max_kb() else {
                continue;
            };
            if node_max > budget_kb {
                diagnostics.push(Diagnostic {
                    rule: self.name().to_string(),
                    severity: Severity::Warning,
                    message: format!(
                        "Node '{}' has preamble_inline_max_kb={node_max} exceeding the \
                         aggregate preamble_budget_kb={budget_kb}: the aggregate demote \
                         pass will re-demote values the raised inline ceiling kept inline",
                        node.id
                    ),
                    node_id: Some(node.id.clone()),
                    edge: None,
                    fix: Some(format!(
                        "Raise preamble_budget_kb to at least {node_max} or lower the \
                         node's preamble_inline_max_kb"
                    )),
                    ..Diagnostic::default()
                });
            }
        }
        diagnostics
    }
}

#[cfg(test)]
mod tests {
    use fabro_graphviz::graph::{AttrValue, Node};

    use super::Rule;
    use crate::LintRule;
    use crate::rules::test_support::minimal_graph;

    fn int(v: i64) -> AttrValue {
        AttrValue::Integer(v)
    }

    #[test]
    fn node_max_within_budget_is_clean() {
        let mut g = minimal_graph();
        g.attrs.insert("preamble_budget_kb".to_string(), int(24));
        let mut work = Node::new("work");
        work.attrs
            .insert("preamble_inline_max_kb".to_string(), int(16));
        g.nodes.insert(work.id.clone(), work);
        assert!(Rule.apply(&g).is_empty());
    }

    #[test]
    fn node_max_over_budget_warns_with_default_budget() {
        let mut g = minimal_graph();
        let mut work = Node::new("work");
        work.attrs
            .insert("preamble_inline_max_kb".to_string(), int(16));
        g.nodes.insert(work.id.clone(), work);
        let diags = Rule.apply(&g);
        assert_eq!(diags.len(), 1, "expected one diagnostic: {diags:?}");
        assert_eq!(diags[0].rule, "preamble_inline_max_within_budget");
        assert!(
            diags[0].message.contains("preamble_budget_kb=12"),
            "default budget in message: {}",
            diags[0].message
        );
    }

    #[test]
    fn graph_max_over_budget_warns() {
        let mut g = minimal_graph();
        g.attrs.insert("preamble_budget_kb".to_string(), int(8));
        g.attrs
            .insert("preamble_inline_max_kb".to_string(), int(16));
        let diags = Rule.apply(&g);
        assert_eq!(diags.len(), 1, "expected one diagnostic: {diags:?}");
        assert!(
            diags[0].message.contains("Graph preamble_inline_max_kb=16"),
            "graph-level message: {}",
            diags[0].message
        );
    }

    #[test]
    fn node_max_below_one_is_ignored() {
        let mut g = minimal_graph();
        let mut work = Node::new("work");
        work.attrs
            .insert("preamble_inline_max_kb".to_string(), int(0));
        g.nodes.insert(work.id.clone(), work);
        assert!(Rule.apply(&g).is_empty());
    }
}
