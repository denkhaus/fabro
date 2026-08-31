use fabro_graphviz::graph::Graph;

use super::tool_catalog::is_known_tool_name;
use crate::{Diagnostic, LintRule, Severity};

pub(super) fn rule() -> Box<dyn LintRule> {
    Box::new(Rule)
}

struct Rule;

impl Rule {
    const FIX: &str = "Remove the unknown name, or fix the typo; mcp__-prefixed names are exempt                        because they depend on run configuration";
}

impl LintRule for Rule {
    fn name(&self) -> &'static str {
        "tools_attribute_known"
    }

    fn apply(&self, graph: &Graph) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for node in graph.nodes.values() {
            for (attribute, names) in [("tools", node.tools()), ("fabro_tools", node.fabro_tools())]
            {
                for name in names {
                    if !is_known_tool_name(name, attribute == "fabro_tools") {
                        diagnostics.push(Diagnostic {
                            rule: self.name().to_string(),
                            severity: Severity::Warning,
                            message: format!(
                                "Node '{}' lists unknown tool '{}' in '{attribute}'",
                                node.id, name
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
    use super::super::tool_catalog::{KNOWN_FABRO_RUN_TOOL_NAMES, KNOWN_NATIVE_TOOL_NAMES};
    use super::*;

    #[test]
    fn warns_on_unknown_native_and_fabro_tool_names() {
        let graph = test_support::graph_with_nodes([
            test_support::node_with_attrs("reviewer", &[("tools", "read_file,grep,made_up_tool")]),
            test_support::node_with_attrs("analyst", &[(
                "fabro_tools",
                "fabro_run_get,fabro_wrong_name",
            )]),
        ]);

        let diagnostics = rule().apply(&graph);
        let messages: Vec<&str> = diagnostics.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(messages.len(), 2, "{messages:?}");
        assert!(messages[0].contains("made_up_tool"), "{messages:?}");
        assert!(messages[1].contains("fabro_wrong_name"), "{messages:?}");
    }

    #[test]
    fn accepts_known_names_and_mcp_prefix() {
        let graph =
            test_support::graph_with_nodes([test_support::node_with_attrs("reviewer", &[(
                "tools",
                "read_file,Kimi-style-names-are-not-known,mcp__github__tool",
            )])]);

        let diagnostics = rule().apply(&graph);
        let unknown: Vec<&str> = diagnostics
            .iter()
            .map(|d| d.message.as_str())
            .filter(|m| m.contains("Kimi-style"))
            .collect();
        // Profile vocabularies (Read/Grep/...) are NOT canonical names: the
        // canonical form is required in tools lists, so they warn. MCP
        // names never warn.
        assert_eq!(unknown.len(), 1, "{diagnostics:?}");
        assert!(
            !diagnostics.iter().any(|d| d.message.contains("mcp__")),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn native_names_do_not_pass_for_fabro_tools_and_vice_versa() {
        let graph = test_support::graph_with_nodes([test_support::node_with_attrs("mixed", &[(
            "fabro_tools",
            "read_file,fabro_run_get",
        )])]);

        let diagnostics = rule().apply(&graph);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(
            diagnostics[0].message.contains("read_file"),
            "{diagnostics:?}"
        );
    }

    /// Guard the static catalogs against the live ones: the lists below
    /// must track fabro-agent's NativeTool and fabro-tool's catalog.
    #[test]
    fn static_catalogs_match_live_crates() {
        use strum::VariantArray as _;

        let mut native: Vec<&str> = KNOWN_NATIVE_TOOL_NAMES.to_vec();
        let mut live_native: Vec<String> = fabro_agent::NativeTool::VARIANTS
            .iter()
            .map(|tool| tool.canonical_name().to_owned())
            .collect();
        native.sort_unstable();
        live_native.sort_unstable();
        assert_eq!(
            native, live_native,
            "KNOWN_NATIVE_TOOL_NAMES drifted from fabro-agent NativeTool"
        );

        let mut fabro_run: Vec<&str> = KNOWN_FABRO_RUN_TOOL_NAMES.to_vec();
        let mut live_fabro_run: Vec<&str> = fabro_tool::tool_definitions()
            .iter()
            .map(|d| d.name)
            .collect();
        fabro_run.sort_unstable();
        live_fabro_run.sort_unstable();
        assert_eq!(
            fabro_run, live_fabro_run,
            "KNOWN_FABRO_RUN_TOOL_NAMES drifted from fabro-tool catalog"
        );
    }

    #[test]
    fn known_name_lookup_covers_both_catalogs() {
        assert!(is_known_tool_name("read_file", false));
        assert!(is_known_tool_name("fabro_run_get", true));
        assert!(!is_known_tool_name("fabro_run_get", false));
        assert!(!is_known_tool_name("read_file", true));
        assert!(is_known_tool_name("mcp__any__thing", false));
        assert!(is_known_tool_name("mcp__any__thing", true));
    }
}
