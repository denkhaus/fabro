use fabro_graphviz::graph::Graph;

use crate::{Diagnostic, LintRule, Severity};

pub(super) fn rule() -> Box<dyn LintRule> {
    Box::new(Rule)
}

struct Rule;

impl LintRule for Rule {
    fn name(&self) -> &'static str {
        "preamble_stages_ignore_targets_exist"
    }

    fn apply(&self, graph: &Graph) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for node in graph.nodes.values() {
            for target in node.preamble_stages_ignore() {
                if !graph.nodes.contains_key(target) {
                    diagnostics.push(Diagnostic {
                        rule: self.name().to_string(),
                        severity: Severity::Warning,
                        message: format!(
                            "Node '{}' has preamble_stages_ignore entry '{}' that does not exist",
                            node.id, target
                        ),
                        node_id: Some(node.id.clone()),
                        edge: None,
                        fix: Some(format!(
                            "Define node '{target}' or remove it from preamble_stages_ignore"
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

    #[test]
    fn warns_on_nonexistent_stage_entry() {
        let mut g = minimal_graph();
        let mut planner = Node::new("planner");
        planner
            .attrs
            .insert("prompt".to_string(), AttrValue::String("plan".to_string()));
        let mut node = Node::new("reviewer");
        node.attrs.insert(
            "preamble_stages_ignore".to_string(),
            AttrValue::String("planner,ghost".to_string()),
        );
        g.nodes.insert("planner".to_string(), planner);
        g.nodes.insert("reviewer".to_string(), node);
        let rule = Rule;
        let d = rule.apply(&g);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].severity, Severity::Warning);
        assert!(d[0].message.contains("ghost"), "{}", d[0].message);
        assert!(!d[0].message.contains("planner"), "{}", d[0].message);
    }

    #[test]
    fn passes_when_all_entries_exist() {
        let mut g = minimal_graph();
        let mut planner = Node::new("planner");
        planner.attrs.insert(
            "prompt".to_string(),
            AttrValue::String("do work".to_string()),
        );
        let mut reviewer = Node::new("reviewer");
        reviewer.attrs.insert(
            "prompt".to_string(),
            AttrValue::String("review".to_string()),
        );
        reviewer.attrs.insert(
            "preamble_stages_ignore".to_string(),
            AttrValue::String("planner, reviewer".to_string()),
        );
        g.nodes.insert("planner".to_string(), planner);
        g.nodes.insert("reviewer".to_string(), reviewer);
        let rule = Rule;
        assert!(rule.apply(&g).is_empty());
    }

    #[test]
    fn ignores_self_reference_free_nodes_without_attribute() {
        let g = minimal_graph();
        let rule = Rule;
        assert!(rule.apply(&g).is_empty());
    }
}
