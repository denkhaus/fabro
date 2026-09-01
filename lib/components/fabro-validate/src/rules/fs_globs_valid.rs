use fabro_graphviz::graph::Graph;
use fabro_util::workspace_glob::WorkspaceGlob;

use crate::{Diagnostic, LintRule, Severity};

pub(super) fn rule() -> Box<dyn LintRule> {
    Box::new(Rule)
}

struct Rule;

impl Rule {
    const FIX: &str = "Fix the glob: workspace-relative, '/' separators, '*' within one \
                       segment, '**' recursive; absolute paths, backslashes, and '..' are \
                       rejected";
}

impl LintRule for Rule {
    fn name(&self) -> &'static str {
        "fs_globs_valid"
    }

    fn apply(&self, graph: &Graph) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for node in graph.nodes.values() {
            // The Node accessors split and trim exactly like the runtime
            // (`split_key_list`), so validation judges the same entries
            // the engine will compile.
            let entries: [(&str, Vec<&str>); 2] = [
                ("fs_hide", node.fs_hide()),
                ("fs_write", node.fs_write().unwrap_or_default()),
            ];
            for (attribute, attribute_entries) in entries {
                for entry in attribute_entries {
                    if let Err(error) = WorkspaceGlob::try_new(entry) {
                        diagnostics.push(Diagnostic {
                            rule: self.name().to_string(),
                            severity: Severity::Error,
                            message: format!(
                                "Node '{}' has an invalid {} glob '{}': {}",
                                node.id, attribute, entry, error
                            ),
                            node_id: Some(node.id.clone()),
                            edge: None,
                            fix: Some(Self::FIX.to_string()),
                            ..Diagnostic::default()
                        });
                    }
                }
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
    fn errors_on_invalid_globs() {
        let graph = test_support::graph_with_nodes([
            test_support::node_with_attrs("ok", &[("fs_hide", ".fabro/**,*.tmp")]),
            test_support::node_with_attrs("bad", &[("fs_write", "/abs/path,ok.rs")]),
            test_support::node_with_attrs("worse", &[("fs_hide", "../*"), ("fs_write", "a\\b")]),
        ]);

        let diagnostics = rule().apply(&graph);
        // Node iteration follows a HashMap; assert order-independently.
        let mut messages: Vec<&str> = diagnostics.iter().map(|d| d.message.as_str()).collect();
        messages.sort_unstable();
        assert_eq!(diagnostics.len(), 3, "{messages:?}");
        assert!(
            messages
                .iter()
                .any(|m| m.contains("bad") && m.contains("/abs/path"))
        );
        assert!(
            messages
                .iter()
                .any(|m| m.contains("worse") && m.contains("../*"))
        );
        assert!(
            messages
                .iter()
                .any(|m| m.contains("worse") && m.contains("a\\b"))
        );
        assert!(diagnostics.iter().all(|d| d.severity == Severity::Error));
    }

    #[test]
    fn silent_on_valid_or_unset() {
        let graph = test_support::graph_with_nodes([
            test_support::node_with_attrs("ok", &[("fs_hide", "**"), ("fs_write", "")]),
            test_support::node_with_attrs("none", &[("prompt", "hi")]),
        ]);

        assert!(rule().apply(&graph).is_empty());
    }
}
