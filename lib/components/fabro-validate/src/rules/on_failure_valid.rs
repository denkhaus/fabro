use fabro_graphviz::graph::{AttrValue, Graph};
use fabro_types::OnFailure;

use crate::{Diagnostic, LintRule, Severity};

pub(super) fn rule() -> Box<dyn LintRule> {
    Box::new(Rule)
}

struct Rule;

impl LintRule for Rule {
    fn name(&self) -> &'static str {
        "on_failure_valid"
    }

    fn apply(&self, graph: &Graph) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        let expected_values = OnFailure::expected_values();

        match graph.attrs.get("on_failure") {
            None => {}
            Some(AttrValue::String(value)) if value.parse::<OnFailure>().is_ok() => {}
            Some(AttrValue::String(value)) => diagnostics.push(Diagnostic {
                rule: self.name().to_string(),
                severity: Severity::Error,
                message: format!("Graph has invalid on_failure value '{value}'"),
                fix: Some(format!("Use one of: {expected_values}")),
                ..Diagnostic::default()
            }),
            Some(_) => diagnostics.push(Diagnostic {
                rule: self.name().to_string(),
                severity: Severity::Error,
                message: "Graph attribute 'on_failure' must be a string".to_string(),
                fix: Some(format!("Use one of: {expected_values}")),
                ..Diagnostic::default()
            }),
        }

        for node in graph.nodes.values() {
            if node.attrs.contains_key("on_failure") {
                diagnostics.push(Diagnostic {
                    rule: self.name().to_string(),
                    severity: Severity::Warning,
                    message: format!(
                        "Node '{}' sets 'on_failure', which has no effect outside graph scope",
                        node.id
                    ),
                    node_id: Some(node.id.clone()),
                    fix: Some("Move 'on_failure' to the graph attributes".to_string()),
                    ..Diagnostic::default()
                });
            }
        }

        for edge in &graph.edges {
            if edge.attrs.contains_key("on_failure") {
                diagnostics.push(Diagnostic {
                    rule: self.name().to_string(),
                    severity: Severity::Warning,
                    message: format!(
                        "Edge '{} -> {}' sets 'on_failure', which has no effect outside graph scope",
                        edge.from, edge.to
                    ),
                    edge: Some((edge.from.clone(), edge.to.clone())),
                    fix: Some("Move 'on_failure' to the graph attributes".to_string()),
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

    #[test]
    fn accepts_absent_and_supported_graph_values() {
        let mut graph = minimal_graph();
        assert!(Rule.apply(&graph).is_empty());

        for value in ["route", "exit"] {
            graph.attrs.insert(
                "on_failure".to_string(),
                AttrValue::String(value.to_string()),
            );
            assert!(Rule.apply(&graph).is_empty());
        }
    }

    #[test]
    fn rejects_unsupported_graph_value() {
        let mut graph = minimal_graph();
        graph.attrs.insert(
            "on_failure".to_string(),
            AttrValue::String("stop".to_string()),
        );

        let diagnostics = Rule.apply(&graph);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert_eq!(
            diagnostics[0].message,
            "Graph has invalid on_failure value 'stop'"
        );
        assert_eq!(
            diagnostics[0].fix.as_deref(),
            Some("Use one of: route, exit")
        );
    }

    #[test]
    fn rejects_non_string_graph_value() {
        let mut graph = minimal_graph();
        graph
            .attrs
            .insert("on_failure".to_string(), AttrValue::Boolean(true));

        let diagnostics = Rule.apply(&graph);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert_eq!(
            diagnostics[0].message,
            "Graph attribute 'on_failure' must be a string"
        );
    }

    #[test]
    fn warns_for_node_and_edge_placement() {
        let mut graph = minimal_graph();
        let mut node = Node::new("work");
        node.attrs.insert(
            "on_failure".to_string(),
            AttrValue::String("exit".to_string()),
        );
        graph.nodes.insert("work".to_string(), node);

        let mut edge = Edge::new("start", "work");
        edge.attrs.insert(
            "on_failure".to_string(),
            AttrValue::String("exit".to_string()),
        );
        graph.edges.push(edge);

        let diagnostics = Rule.apply(&graph);

        assert_eq!(diagnostics.len(), 2);
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.severity == Severity::Warning)
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.message.contains("graph scope"))
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.node_id.as_deref() == Some("work"))
        );
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.edge == Some(("start".to_string(), "work".to_string()))
        }));
    }
}
