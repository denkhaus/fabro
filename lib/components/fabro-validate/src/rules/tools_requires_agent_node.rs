use fabro_graphviz::graph::Graph;

use crate::{Diagnostic, LintRule, Severity};

pub(super) fn rule() -> Box<dyn LintRule> {
    Box::new(Rule)
}

struct Rule;

impl Rule {
    const FIX: &str = "Remove 'tools'/'fabro_tools' or move them to an agent node; prompt nodes \
                       have no tool registry at all";
}

impl LintRule for Rule {
    fn name(&self) -> &'static str {
        "tools_requires_agent_node"
    }

    fn apply(&self, graph: &Graph) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for node in graph.nodes.values() {
            let has_tool_lists = !node.tools().is_empty() || !node.fabro_tools().is_empty();
            if has_tool_lists && node.handler_type() != Some("agent") {
                diagnostics.push(Diagnostic {
                    rule: self.name().to_string(),
                    severity: Severity::Warning,
                    message: format!(
                        "Node '{}' sets tools/fabro_tools but is not an agent node; the \
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
    fn warns_on_prompt_and_command_nodes_with_tool_lists() {
        let graph = test_support::graph_with_nodes([
            test_support::node_with_attrs("judge", &[("shape", "tab"), ("tools", "read_file")]),
            test_support::node_with_attrs("build", &[
                ("shape", "parallelogram"),
                ("fabro_tools", "fabro_run_get"),
            ]),
            test_support::node_with_attrs("work", &[("tools", "read_file,grep")]),
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
            test_support::node_with_attrs("work", &[("tools", "read_file")]),
            test_support::node_with_attrs("judge", &[
                ("shape", "tab"),
                ("prompt", "Review the diff"),
            ]),
        ]);

        assert!(rule().apply(&graph).is_empty());
    }
}
