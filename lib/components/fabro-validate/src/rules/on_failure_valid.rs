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

        let invalid_value = |subject: &str, value: &AttrValue| -> Option<Diagnostic> {
            let message = match value {
                AttrValue::String(value) if value.parse::<OnFailure>().is_ok() => return None,
                AttrValue::String(value) => {
                    format!("{subject} has invalid on_failure value '{value}'")
                }
                _ => format!("{subject} attribute 'on_failure' must be a string"),
            };
            Some(Diagnostic {
                rule: self.name().to_string(),
                severity: Severity::Error,
                message,
                fix: Some(format!("Use one of: {}", OnFailure::expected_values())),
                ..Diagnostic::default()
            })
        };

        if let Some(value) = graph.attrs.get("on_failure") {
            diagnostics.extend(invalid_value("Graph", value));
        }

        let mut node_ids: Vec<&String> = graph.nodes.keys().collect();
        node_ids.sort();
        for node_id in node_ids {
            let node = &graph.nodes[node_id];
            if let Some(value) = node.attrs.get("on_failure") {
                if let Some(diagnostic) = invalid_value(&format!("Node '{node_id}'"), value) {
                    diagnostics.push(Diagnostic {
                        node_id: Some(node_id.clone()),
                        ..diagnostic
                    });
                }
            }
        }

        for edge in &graph.edges {
            if edge.attrs.contains_key("on_failure") {
                diagnostics.push(Diagnostic {
                    rule: self.name().to_string(),
                    severity: Severity::Warning,
                    message: format!(
                        "Edge '{} -> {}' sets 'on_failure', which has no effect on edges",
                        edge.from, edge.to
                    ),
                    fix: Some("Set 'on_failure' on the graph or the source node".to_string()),
                    edge: Some((edge.from.clone(), edge.to.clone())),
                    ..Diagnostic::default()
                });
            }
        }

        diagnostics
    }
}

#[cfg(test)]
mod tests {
    use fabro_graphviz::graph::{AttrValue, Edge};

    use super::Rule;
    use crate::rules::test_support::{minimal_graph, node_with_attrs};
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
    fn accepts_supported_node_values() {
        let mut graph = minimal_graph();
        for value in ["route", "exit"] {
            graph.nodes.insert(
                "work".to_string(),
                node_with_attrs("work", &[("on_failure", value)]),
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
    fn rejects_unsupported_node_value() {
        let mut graph = minimal_graph();
        graph.nodes.insert(
            "work".to_string(),
            node_with_attrs("work", &[("on_failure", "stop")]),
        );

        let diagnostics = Rule.apply(&graph);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert_eq!(
            diagnostics[0].message,
            "Node 'work' has invalid on_failure value 'stop'"
        );
        assert_eq!(diagnostics[0].node_id.as_deref(), Some("work"));
        assert_eq!(
            diagnostics[0].fix.as_deref(),
            Some("Use one of: route, exit")
        );
    }

    #[test]
    fn rejects_non_string_node_value() {
        let mut graph = minimal_graph();
        let mut node = node_with_attrs("work", &[]);
        node.attrs
            .insert("on_failure".to_string(), AttrValue::Boolean(true));
        graph.nodes.insert("work".to_string(), node);

        let diagnostics = Rule.apply(&graph);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert_eq!(
            diagnostics[0].message,
            "Node 'work' attribute 'on_failure' must be a string"
        );
    }

    #[test]
    fn warns_for_edge_placement() {
        let mut graph = minimal_graph();
        let mut edge = Edge::new("start", "work");
        edge.attrs.insert(
            "on_failure".to_string(),
            AttrValue::String("exit".to_string()),
        );
        graph.edges.push(edge);

        let diagnostics = Rule.apply(&graph);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, Severity::Warning);
        assert_eq!(
            diagnostics[0].message,
            "Edge 'start -> work' sets 'on_failure', which has no effect on edges"
        );
        assert_eq!(
            diagnostics[0].fix.as_deref(),
            Some("Set 'on_failure' on the graph or the source node")
        );
        assert_eq!(
            diagnostics[0].edge,
            Some(("start".to_string(), "work".to_string()))
        );
    }
}
