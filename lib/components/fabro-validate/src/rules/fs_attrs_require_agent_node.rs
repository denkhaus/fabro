use fabro_graphviz::graph::Graph;

use crate::{Diagnostic, LintRule, Severity};

pub(super) fn rule() -> Box<dyn LintRule> {
    Box::new(Rule)
}

struct Rule;

impl Rule {
    const FIX: &str = "Remove 'fs_hide'/'fs_write' or move them to an agent node; only agent \
                       stages run tools against the sandbox filesystem";
}

impl LintRule for Rule {
    fn name(&self) -> &'static str {
        "fs_attrs_require_agent_node"
    }

    fn apply(&self, graph: &Graph) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for node in graph.nodes.values() {
            let has_fs_scope = !node.fs_hide().is_empty() || node.fs_write().is_some();
            if has_fs_scope && node.handler_type() != Some("agent") {
                diagnostics.push(Diagnostic {
                    rule: self.name().to_string(),
                    severity: Severity::Warning,
                    message: format!(
                        "Node '{}' sets fs_hide/fs_write but is not an agent node; the \
                         attributes have no effect",
                        node.id
                    ),
                    node_id: Some(node.id.clone()),
                    edge: None,
                    fix: Some(Self::FIX.to_string()),
                    ..Diagnostic::default()
                });
            }
        }
        diagnostics
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support;
    use super::*;

    #[test]
    fn warns_on_non_agent_nodes_with_fs_scope() {
        let graph = test_support::graph_with_nodes([
            test_support::node_with_attrs("judge", &[("shape", "tab"), ("fs_hide", ".seeds/**")]),
            test_support::node_with_attrs("build", &[("shape", "parallelogram"), ("fs_write", "")]),
            test_support::node_with_attrs("work", &[("fs_write", "*.go")]),
        ]);

        let diagnostics = rule().apply(&graph);
        let mut node_ids: Vec<&str> = diagnostics
            .iter()
            .filter_map(|d| d.node_id.as_deref())
            .collect();
        node_ids.sort_unstable();
        assert_eq!(node_ids, vec!["build", "judge"], "{diagnostics:?}");
    }

    #[test]
    fn silent_on_agent_nodes_and_unrelated_attributes() {
        let graph = test_support::graph_with_nodes([
            test_support::node_with_attrs("work", &[("fs_hide", ".fabro/**")]),
            test_support::node_with_attrs("judge", &[("shape", "tab"), ("prompt", "Review")]),
        ]);

        assert!(rule().apply(&graph).is_empty());
    }
}
