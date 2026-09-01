use fabro_graphviz::graph::Graph;
use fabro_types::context_keys;
/// The lone wildcard entry meaning "every key renders".
use fabro_types::graph::ATTR_LIST_WILDCARD as WILDCARD;

use crate::{Diagnostic, LintRule, Severity};

pub(super) fn rule() -> Box<dyn LintRule> {
    Box::new(Rule)
}

struct Rule;

impl LintRule for Rule {
    fn name(&self) -> &'static str {
        "preamble_allow_keys_exist"
    }

    fn apply(&self, graph: &Graph) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        // Keys any node declares it may write: the producer side of the
        // stage envelope (fabro-900e). A reader's input contract should
        // only name keys somebody produces.
        let declared_keys: Vec<String> = graph
            .nodes
            .values()
            .filter_map(fabro_types::Node::context_allow_keys)
            .flatten()
            .map(str::to_owned)
            .collect();

        for node in graph.nodes.values() {
            let Some(allow) = node.preamble_allow_keys() else {
                continue;
            };

            if allow.contains(&WILDCARD) {
                if allow.len() > 1 {
                    diagnostics.push(Diagnostic {
                        rule: self.name().to_string(),
                        severity: Severity::Warning,
                        message: format!(
                            "Node '{}' mixes '*' with named keys in preamble_allow_keys; '*' alone already renders every key",
                            node.id
                        ),
                        node_id: Some(node.id.clone()),
                        edge: None,
                        fix: Some(format!(
                            "Use either '*' alone or the explicit key list for node '{}'",
                            node.id
                        )),
                        ..Diagnostic::default()
                    });
                }
                continue;
            }

            for key in allow {
                if context_keys::is_preamble_hidden_key(key) {
                    diagnostics.push(Diagnostic {
                        rule: self.name().to_string(),
                        severity: Severity::Warning,
                        message: format!(
                            "Node '{}' lists '{}' in preamble_allow_keys, but that key is preamble-hidden and never renders in a Context section",
                            node.id, key
                        ),
                        node_id: Some(node.id.clone()),
                        edge: None,
                        fix: Some(format!(
                            "Remove '{key}' from preamble_allow_keys on node '{}'",
                            node.id
                        )),
                        ..Diagnostic::default()
                    });
                } else if !context_keys::is_engine_renderable_key(key)
                    && !declared_keys.iter().any(|d| d == key)
                {
                    diagnostics.push(Diagnostic {
                        rule: self.name().to_string(),
                        severity: Severity::Warning,
                        message: format!(
                            "Node '{}' lists '{}' in preamble_allow_keys, but no node declares producing it via context_allow_keys and it is not an engine-renderable key",
                            node.id, key
                        ),
                        node_id: Some(node.id.clone()),
                        edge: None,
                        fix: Some(format!(
                            "Fix the key name, or declare '{key}' in the producing node's context_allow_keys"
                        )),
                        ..Diagnostic::default()
                    });
                }
            }
        }
        diagnostics
    }
}

#[cfg(test)]
mod tests {
    use fabro_graphviz::graph::{AttrValue, Node};

    use super::Rule;
    use crate::rules::test_support::minimal_graph;
    use crate::{LintRule, Severity};

    fn node_with(id: &str, allow: &str) -> Node {
        let mut node = Node::new(id);
        node.attrs.insert(
            "prompt".to_string(),
            AttrValue::String("do work".to_string()),
        );
        node.attrs.insert(
            "preamble_allow_keys".to_string(),
            AttrValue::String(allow.to_string()),
        );
        node
    }

    fn producer_node(id: &str, keys: &str) -> Node {
        let mut node = Node::new(id);
        node.attrs.insert(
            "prompt".to_string(),
            AttrValue::String("produce".to_string()),
        );
        node.attrs.insert(
            "context_allow_keys".to_string(),
            AttrValue::String(keys.to_string()),
        );
        node
    }

    #[test]
    fn passes_on_engine_renderable_and_declared_keys() {
        let mut g = minimal_graph();
        g.nodes.insert(
            "planner".to_string(),
            producer_node("planner", "current_seed_brief"),
        );
        g.nodes.insert(
            "reviewer".to_string(),
            node_with("reviewer", "command.output, current_seed_brief"),
        );
        assert!(Rule.apply(&g).is_empty());
    }

    #[test]
    fn warns_on_undeclared_agent_key() {
        let mut g = minimal_graph();
        g.nodes.insert(
            "reviewer".to_string(),
            node_with("reviewer", "current_seed_brief"),
        );
        let d = Rule.apply(&g);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].severity, Severity::Warning);
        assert!(
            d[0].message.contains("current_seed_brief"),
            "{}",
            d[0].message
        );
    }

    #[test]
    fn warns_on_preamble_hidden_key() {
        let mut g = minimal_graph();
        g.nodes.insert(
            "reviewer".to_string(),
            node_with("reviewer", "internal.run_id"),
        );
        let d = Rule.apply(&g);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].severity, Severity::Warning);
        assert!(d[0].message.contains("preamble-hidden"), "{}", d[0].message);
    }

    #[test]
    fn warns_on_wildcard_mixed_with_named_keys() {
        let mut g = minimal_graph();
        g.nodes.insert(
            "reviewer".to_string(),
            node_with("reviewer", "*, command.output"),
        );
        let d = Rule.apply(&g);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].severity, Severity::Warning);
        assert!(d[0].message.contains("'*' alone"), "{}", d[0].message);
    }

    #[test]
    fn wildcard_alone_passes() {
        let mut g = minimal_graph();
        g.nodes
            .insert("reviewer".to_string(), node_with("reviewer", "*"));
        assert!(Rule.apply(&g).is_empty());
    }

    #[test]
    fn nodes_without_attribute_are_ignored() {
        let g = minimal_graph();
        assert!(Rule.apply(&g).is_empty());
    }
}
