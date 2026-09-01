use fabro_graphviz::graph::Graph;
use fabro_util::workspace_glob::WorkspaceGlob;

use crate::{Diagnostic, LintRule, Severity};

pub(super) fn rule() -> Box<dyn LintRule> {
    Box::new(Rule)
}

struct Rule;

/// Spellings of the shell tool across provider vocabularies. The `tools`
/// allow-list is written in canonical names, but alias spellings are
/// accepted for robustness.
const SHELL_TOOL_NAMES: &[&str] = &["shell", "Bash", "shell_command"];

/// Whether `prefix_root` covers `root` as a literal directory prefix
/// (equal, or a parent directory of it).
fn root_covers(prefix_root: &str, root: &str) -> bool {
    if prefix_root == root {
        return true;
    }
    root.starts_with(&format!("{prefix_root}/"))
}

impl LintRule for Rule {
    fn name(&self) -> &'static str {
        "fs_scope_consistency"
    }

    fn apply(&self, graph: &Graph) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for node in graph.nodes.values() {
            let hide = node.fs_hide();
            let write = node.fs_write();
            if hide.is_empty() && write.is_none() {
                continue;
            }

            // Hidden paths are never writable (they behave as if they did not
            // exist), so a fs_write entry inside a hide glob is ineffective —
            // almost always a configuration mistake. Overlap is decided on
            // literal traversal roots: a wildcard-rooted hide glob (for
            // example `*.env`) hides files we cannot enumerate statically, so
            // only the explicit hide-everything entry `**` is treated as
            // covering every write entry.
            if let Some(write) = write.as_deref() {
                // `traversal_root` is already the literal leading
                // segments, so wildcard-rooted globs like `*.env` yield an
                // empty root and drop out naturally.
                let literal_hide_roots: Vec<String> = hide
                    .iter()
                    .filter_map(|pattern| WorkspaceGlob::try_new(pattern).ok())
                    .map(|glob| glob.traversal_root().to_string())
                    .filter(|root| !root.is_empty())
                    .collect();
                let hides_everything = hide.contains(&"**");
                for write_entry in write {
                    let Ok(glob) = WorkspaceGlob::try_new(write_entry) else {
                        continue; // fs_globs_valid reports the syntax error
                    };
                    let write_root = glob.traversal_root();
                    let overlapping = hides_everything
                        || literal_hide_roots
                            .iter()
                            .any(|hide_root| root_covers(hide_root, write_root));
                    if overlapping {
                        diagnostics.push(Diagnostic {
                            rule: self.name().to_string(),
                            severity: Severity::Warning,
                            message: format!(
                                "Node '{}' lists '{}' in fs_write, but a fs_hide glob hides that \
                                 area: hidden paths are never writable, the entry has no effect",
                                node.id, write_entry
                            ),
                            node_id: Some(node.id.clone()),
                            edge: None,
                            fix: Some(
                                "Remove the overlapping fs_write entry or narrow the fs_hide glob"
                                    .to_string(),
                            ),
                            ..Diagnostic::default()
                        });
                    }
                }
            }

            // The scope is drift protection, not containment: shell stays
            // the documented escape hatch (ADR-0009 trust model). Point
            // authors at the pairing instead of failing the graph.
            let shell_available = node.tools().is_empty()
                || node
                    .tools()
                    .iter()
                    .any(|tool| SHELL_TOOL_NAMES.contains(tool));
            if shell_available {
                diagnostics.push(Diagnostic {
                    rule:     self.name().to_string(),
                    severity: Severity::Info,
                    message:  format!(
                        "Node '{}' restricts the filesystem but keeps shell available: shell \
                         bypasses fs_hide/fs_write (drift protection only)",
                        node.id
                    ),
                    node_id:  Some(node.id.clone()),
                    edge:     None,
                    fix:      Some(
                        "Exclude shell via a tools= allow-list when the stage needs real containment"
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
    use fabro_agent::native_tool::NativeTool;

    use super::super::test_support;
    use super::*;

    #[test]
    fn warns_on_write_entries_covered_by_hide() {
        let graph = test_support::graph_with_nodes([test_support::node_with_attrs("work", &[
            ("fs_hide", ".fabro/**"),
            ("fs_write", ".fabro/x.rs,ok.rs"),
        ])]);

        let diagnostics = rule().apply(&graph);
        let warnings: Vec<&Diagnostic> = diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .collect();
        assert_eq!(warnings.len(), 1, "{diagnostics:?}");
        assert!(warnings[0].message.contains(".fabro/x.rs"));
        assert!(warnings[0].message.contains("no effect"));
    }

    #[test]
    fn no_overlap_warning_for_wildcard_rooted_hide_globs() {
        let graph = test_support::graph_with_nodes([test_support::node_with_attrs("work", &[
            ("fs_hide", "*.env"),
            ("fs_write", "*.go"),
        ])]);

        let diagnostics = rule().apply(&graph);
        assert!(
            diagnostics.iter().all(|d| d.severity != Severity::Warning),
            "disjoint wildcard roots must not warn: {diagnostics:?}"
        );
    }

    #[test]
    fn hide_everything_entry_overlaps_any_write_entry() {
        let graph = test_support::graph_with_nodes([test_support::node_with_attrs("work", &[
            ("fs_hide", "**"),
            ("fs_write", "*.go"),
        ])]);

        let diagnostics = rule().apply(&graph);
        assert!(
            diagnostics.iter().any(|d| d.severity == Severity::Warning
                && d.message.contains("no effect")
                && d.message.contains("*.go")),
            "the `**` hide entry makes every write entry ineffective: {diagnostics:?}"
        );
    }

    #[test]
    fn no_overlap_warning_for_disjoint_roots() {
        let graph = test_support::graph_with_nodes([test_support::node_with_attrs("work", &[
            ("fs_hide", ".fabro/**"),
            ("fs_write", "*.go"),
        ])]);

        let diagnostics = rule().apply(&graph);
        assert!(
            diagnostics.iter().all(|d| d.severity != Severity::Warning),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn shell_pairing_is_info_and_tool_exclusion_silences_it() {
        let graph = test_support::graph_with_nodes([
            test_support::node_with_attrs("open", &[("fs_hide", ".seeds/**")]),
            test_support::node_with_attrs("contained", &[
                ("fs_hide", ".seeds/**"),
                ("tools", "read_file,grep"),
            ]),
        ]);

        let diagnostics = rule().apply(&graph);
        let infos: Vec<&Diagnostic> = diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Info)
            .collect();
        assert_eq!(infos.len(), 1, "{diagnostics:?}");
        assert_eq!(infos[0].node_id.as_deref(), Some("open"));
    }

    #[test]
    fn silent_without_fs_attrs() {
        let graph = test_support::graph_with_nodes([test_support::node_with_attrs("work", &[(
            "tools",
            "shell,read_file",
        )])]);

        assert!(rule().apply(&graph).is_empty());
    }

    /// The hardcoded shell spellings must stay inside the live alias set of
    /// `NativeTool::Shell`, so a new alias cannot silently diverge (same
    /// cross-check discipline as `tools_attribute_known`).
    #[test]
    fn shell_tool_names_stay_within_native_shell_aliases() {
        for name in SHELL_TOOL_NAMES {
            assert_eq!(
                NativeTool::from_any_name(name),
                Some(NativeTool::Shell),
                "spelling '{name}' no longer resolves to the shell tool"
            );
        }
    }
}
