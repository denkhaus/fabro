use fabro_graphviz::graph::Graph;

use crate::{Diagnostic, LintRule, Severity};

pub(super) fn rule() -> Box<dyn LintRule> {
    Box::new(Rule)
}

struct Rule;

impl LintRule for Rule {
    fn name(&self) -> &'static str {
        "context_append_keys_within_allow"
    }

    fn apply(&self, graph: &Graph) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for node in graph.nodes.values() {
            let Some(allow) = node.context_allow_keys() else {
                continue;
            };
            for key in node.context_append_keys() {
                if allow.contains(&key) {
                    continue;
                }
                diagnostics.push(Diagnostic {
                    rule: self.name().to_string(),
                    severity: Severity::Warning,
                    message: format!(
                        "Node '{}' lists '{}' in context_append_keys but not in \
                         context_allow_keys: the engine drops the update instead of merging it",
                        node.id, key
                    ),
                    node_id: Some(node.id.clone()),
                    edge: None,
                    fix: Some(format!(
                        "Add '{key}' to context_allow_keys or remove it from \
                         context_append_keys"
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

    fn node_with_attrs(id: &str, attrs: &[(&str, &str)]) -> Node {
        let mut node = Node::new(id);
        for (key, value) in attrs {
            node.attrs
                .insert(key.to_string(), AttrValue::String(value.to_string()));
        }
        node
    }

    #[test]
    fn warns_when_append_key_is_outside_allowlist() {
        let mut g = minimal_graph();
        g.nodes.insert(
            "reviewer".to_string(),
            node_with_attrs("reviewer", &[
                ("context_allow_keys", "current_seed_id"),
                ("context_append_keys", "workflow_painpoints"),
            ]),
        );
        let d = Rule.apply(&g);
        assert_eq!(d.len(), 1);
        assert_eq!(
            d[0].message,
            "Node 'reviewer' lists 'workflow_painpoints' in context_append_keys but not in \
             context_allow_keys: the engine drops the update instead of merging it"
        );
        assert_eq!(
            d[0].fix.as_deref(),
            Some(
                "Add 'workflow_painpoints' to context_allow_keys or remove it from \
                 context_append_keys"
            )
        );
        assert!(!d[0].message.contains("  "), "no double-space artifacts");
    }

    #[test]
    fn passes_when_append_keys_are_allowed() {
        let mut g = minimal_graph();
        g.nodes.insert(
            "reviewer".to_string(),
            node_with_attrs("reviewer", &[
                ("context_allow_keys", "current_seed_id, workflow_painpoints"),
                ("context_append_keys", "workflow_painpoints"),
            ]),
        );
        assert!(Rule.apply(&g).is_empty());
    }

    #[test]
    fn ignores_append_keys_without_an_allowlist() {
        // Default-open: with no allowlist every append key is admitted.
        let mut g = minimal_graph();
        g.nodes.insert(
            "reviewer".to_string(),
            node_with_attrs("reviewer", &[(
                "context_append_keys",
                "workflow_painpoints",
            )]),
        );
        assert!(Rule.apply(&g).is_empty());
    }

    #[test]
    fn ignores_nodes_without_envelope_attributes() {
        let g = minimal_graph();
        assert!(Rule.apply(&g).is_empty());
    }
}
