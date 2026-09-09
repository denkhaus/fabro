use fabro_graphviz::graph::{AttrValue, Graph};

use crate::{Diagnostic, LintRule, Severity};

pub(super) fn rule() -> Box<dyn LintRule> {
    Box::new(Rule)
}

struct Rule;

impl LintRule for Rule {
    fn name(&self) -> &'static str {
        "routing_fallback_silent"
    }

    fn apply(&self, graph: &Graph) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for node in graph.nodes.values() {
            let outgoing = graph.outgoing_edges(&node.id);
            if outgoing.is_empty() {
                continue;
            }
            let routes_by_label = outgoing
                .iter()
                .any(|e| e.label().is_some_and(|label| !label.trim().is_empty()));
            if !routes_by_label {
                continue;
            }
            // An unlabeled unconditional edge without a kind is the silent
            // catcher for an unrecognized preferred_label: the label misses,
            // no condition matches, and the run follows the fallback without
            // any signal (fabro-de4d).
            let silent_catchers: Vec<_> = outgoing
                .iter()
                .filter(|e| e.condition().is_none_or(str::is_empty))
                .filter(|e| e.label().is_none_or(|label| label.trim().is_empty()))
                .filter(|e| {
                    e.attrs
                        .get("kind")
                        .and_then(AttrValue::as_str)
                        .is_none_or(str::is_empty)
                })
                .collect();
            for edge in silent_catchers {
                diagnostics.push(Diagnostic {
                    rule: self.name().to_string(),
                    severity: Severity::Warning,
                    message: format!(
                        "Node '{}' routes by edge label, but an unrecognized preferred_label \
                         will silently fall through its unconditional unlabeled edge to '{}'",
                        node.id, edge.to
                    ),
                    node_id: Some(node.id.clone()),
                    edge: Some((node.id.clone(), edge.to.clone())),
                    fix: Some(
                        "Give the fallback edge a label (so it is a deliberate route), a kind \
                         (e.g. kind=\"deadlock\" for an explicit guard), or use \
                         output_schema=\"routing\" so labels are validated"
                            .to_string(),
                    ),
                    ..Diagnostic::default()
                });
            }
        }
        diagnostics
    }
}

#[cfg(test)]
mod tests {
    use fabro_graphviz::graph::{AttrValue, Edge, Node};

    use super::Rule;
    use crate::rules::test_support::minimal_graph;
    use crate::{LintRule, Severity};

    fn labeled_edge(from: &str, to: &str, label: &str) -> Edge {
        let mut edge = Edge::new(from, to);
        edge.attrs
            .insert("label".to_string(), AttrValue::String(label.to_string()));
        edge
    }

    #[test]
    fn warns_when_unlabeled_unconditional_edge_catches_unrouted_labels() {
        let mut g = minimal_graph();
        g.nodes.insert("review".to_string(), Node::new("review"));
        g.edges.push(labeled_edge("review", "approve", "Approve"));
        g.edges
            .push(labeled_edge("review", "fix", "Changes requested"));
        g.edges.push(Edge::new("review", "parked"));

        let diagnostics = Rule.apply(&g);

        assert_eq!(diagnostics.len(), 1, "diagnostics: {diagnostics:?}");
        assert_eq!(diagnostics[0].severity, Severity::Warning);
        assert_eq!(diagnostics[0].node_id.as_deref(), Some("review"));
        assert!(diagnostics[0].message.contains("silently fall through"));
        assert!(diagnostics[0].message.contains('\''));
    }

    #[test]
    fn no_warning_when_fallback_edge_has_a_kind() {
        let mut g = minimal_graph();
        g.nodes.insert("review".to_string(), Node::new("review"));
        g.edges.push(labeled_edge("review", "approve", "Approve"));
        let mut guard = Edge::new("review", "exit");
        guard
            .attrs
            .insert("kind".to_string(), AttrValue::String("deadlock".into()));
        g.edges.push(guard);

        let diagnostics = Rule.apply(&g);

        assert!(
            diagnostics.is_empty(),
            "explicit kind guard should not warn: {diagnostics:?}"
        );
    }

    #[test]
    fn no_warning_when_fallback_edge_is_labeled() {
        let mut g = minimal_graph();
        g.nodes.insert("review".to_string(), Node::new("review"));
        g.edges.push(labeled_edge("review", "approve", "Approve"));
        g.edges.push(labeled_edge("review", "escalate", "Escalate"));

        let diagnostics = Rule.apply(&g);

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn no_warning_for_nodes_without_labeled_edges() {
        let mut g = minimal_graph();
        g.nodes.insert("work".to_string(), Node::new("work"));
        g.edges.push(Edge::new("work", "exit"));

        let diagnostics = Rule.apply(&g);

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
