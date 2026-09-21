//! Stage-envelope attributes from the run's graph source (fabro-aa5f,
//! ADR-0009 rev; the fork's second `graph_source` reader after
//! `projection::fork_exit_kinds`).
//!
//! Petri accepts `x.`-namespaced attributes at check and drops them at
//! lowering — the admitted graph does not carry them. But every run's
//! spec carries the original DOT text (`graph_source`), and the
//! stage-envelope values ride it verbatim. This module parses the
//! per-node `x.fs_write`/`x.fs_hide` lists and the `x.preamble_*`
//! numbers, hands a node's compiled [`FsScope`] to the checkpoint guard
//! (`hooks::FabroHooks`), and lints the values at create time
//! (`check`): glob validity, fs_write-inside-fs_hide consistency, and
//! preamble budget consistency — the stage-envelope subset of the
//! validation rules the legacy `fabro-validate` crate owned
//! (fabro-1392).
//!
//! The parser is deliberately in the shape of `fork_exit_kinds`: a
//! light scan over the raw DOT text, tolerant of the fabro files'
//! multi-line attribute blocks, never a full DOT grammar. It can misread
//! attribute text embedded in quoted labels; the lints then judge what
//! was read, and a misread hides an envelope rather than inventing one.

use std::collections::BTreeMap;

use fabro_pebble_sandbox::fs_scope::{FsScope, FsScopeError};
use fabro_util::workspace_glob::WorkspaceGlob;

/// The aggregate preamble budget when no `x.preamble_budget_kb` is set:
/// the value the legacy engine defaulted to (fabro-a85b).
const DEFAULT_PREAMBLE_BUDGET_KB: u64 = 48;

/// One node's stage envelope, as written.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeEnvelope {
    /// `x.fs_hide`: workspace-relative globs hidden from the stage.
    pub fs_hide:                Vec<String>,
    /// `x.fs_write`: when the attribute is present, the only writable
    /// globs. `Some([])` (an empty value) is a read-only stage.
    pub fs_write:               Option<Vec<String>>,
    /// `x.preamble_inline_max_kb` on the node, when set.
    pub preamble_inline_max_kb: Option<u64>,
}

/// The graph block's envelope numbers, as written.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GraphEnvelope {
    /// `x.preamble_budget_kb` on the graph block, when set.
    pub preamble_budget_kb:     Option<u64>,
    /// `x.preamble_inline_max_kb` on the graph block, when set.
    pub preamble_inline_max_kb: Option<u64>,
}

/// Stage envelopes by node, parsed from the DOT `graph_source`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StageEnvelopes {
    nodes: BTreeMap<String, NodeEnvelope>,
    graph: GraphEnvelope,
}

/// One lint finding over the parsed envelopes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageEnvelopeLint {
    /// Whether the finding refuses the workflow (`Error`) or warns
    /// (`Warning`).
    pub severity: LintSeverity,
    /// The stable code: `fork.fs_globs_valid`, `fork.fs_scope_consistency`,
    /// `fork.preamble_budget_consistency`.
    pub code:     &'static str,
    pub message:  String,
    /// The node the finding names, when it names one.
    pub node:     Option<String>,
}

/// Whether a lint refuses or warns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LintSeverity {
    Error,
    Warning,
}

impl StageEnvelopes {
    /// Parse node and graph blocks with their `x.*` envelope attributes
    /// from raw DOT text. Edge statements (`a -> b [...]`) are skipped;
    /// a node block's attributes may span lines.
    #[must_use]
    pub fn parse(graph_source: &str) -> Self {
        let mut nodes: BTreeMap<String, NodeEnvelope> = BTreeMap::new();
        let mut graph = GraphEnvelope::default();
        let mut rest = graph_source;
        while let Some(open) = rest.find('[') {
            let head = &rest[..open];
            let after = &rest[open + 1..];
            let Some(close) = after.find(']') else { break };
            let block = &after[..close];
            rest = &after[close + 1..];
            // The subject is the tail of the head: `name`, `graph`, or an
            // edge's `a -> b`. Edges carry their own `x.*` attributes
            // (exit kinds) and never an envelope.
            let subject = head
                .lines()
                .last()
                .unwrap_or_default()
                .trim()
                .trim_matches(|c: char| c.is_whitespace() || c == ';' || c == ',');
            if subject.contains("->") {
                continue;
            }
            let name = subject.split_whitespace().next().unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            if name == "graph" {
                graph.preamble_budget_kb = number(block, "x.preamble_budget_kb");
                graph.preamble_inline_max_kb = number(block, "x.preamble_inline_max_kb");
            } else {
                let envelope = NodeEnvelope {
                    fs_hide:                list(block, "x.fs_hide"),
                    fs_write:               attribute(block, "x.fs_write")
                        .map(|_| list(block, "x.fs_write")),
                    preamble_inline_max_kb: number(block, "x.preamble_inline_max_kb"),
                };
                let declares = !envelope.fs_hide.is_empty()
                    || envelope.fs_write.is_some()
                    || envelope.preamble_inline_max_kb.is_some();
                if declares {
                    nodes.insert(name.to_string(), envelope);
                }
            }
        }
        Self { nodes, graph }
    }

    /// The envelope of `node`: the exact name, or the base name when the
    /// node is an expansion clone (`build#2` carries `build`'s envelope).
    #[must_use]
    pub fn envelope(&self, node: &str) -> Option<&NodeEnvelope> {
        self.nodes.get(node).or_else(|| {
            let base = node.split('#').next().unwrap_or(node);
            self.nodes.get(base)
        })
    }

    /// The node's compiled write scope, when the node declares envelope
    /// attributes at all: `None` leaves the stage unrestricted, `Some`
    /// compiles (and may fail on an invalid glob; the create-time lint
    /// rejects those, so a failure here is fail-closed).
    #[must_use]
    pub fn fs_scope(&self, node: &str) -> Option<Result<FsScope, FsScopeError>> {
        let envelope = self.envelope(node)?;
        let hide = envelope
            .fs_hide
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let write = envelope
            .fs_write
            .as_ref()
            .map(|entries| entries.iter().map(String::as_str).collect::<Vec<_>>());
        Some(FsScope::try_new(&hide, write.as_deref()))
    }

    /// The stage-envelope lints: glob validity (error), fs_write entries
    /// under fs_hide roots (warning), preamble inline ceilings above the
    /// aggregate budget (warning).
    #[must_use]
    pub fn lint(&self) -> Vec<StageEnvelopeLint> {
        let mut findings = Vec::new();
        for (node, envelope) in &self.nodes {
            for (attribute, entries) in [
                ("fs_hide", &envelope.fs_hide),
                ("fs_write", &envelope.fs_write.clone().unwrap_or_default()),
            ] {
                for entry in entries {
                    if let Err(error) = WorkspaceGlob::try_new(entry) {
                        findings.push(StageEnvelopeLint {
                            severity: LintSeverity::Error,
                            code:     "fork.fs_globs_valid",
                            message:  format!(
                                "node '{node}' has an invalid {attribute} glob '{entry}': {error}"
                            ),
                            node:     Some(node.clone()),
                        });
                    }
                }
            }
            if let Some(write) = &envelope.fs_write {
                for entry in write {
                    if hide_covers(&envelope.fs_hide, entry) {
                        findings.push(StageEnvelopeLint {
                            severity: LintSeverity::Warning,
                            code:     "fork.fs_scope_consistency",
                            message:  format!(
                                "node '{node}': fs_write entry '{entry}' sits under a fs_hide \
                                 glob and is never writable"
                            ),
                            node:     Some(node.clone()),
                        });
                    }
                }
            }
            let budget = self
                .graph
                .preamble_budget_kb
                .unwrap_or(DEFAULT_PREAMBLE_BUDGET_KB);
            if let Some(max) = envelope.preamble_inline_max_kb {
                if max > budget {
                    findings.push(StageEnvelopeLint {
                        severity: LintSeverity::Warning,
                        code:     "fork.preamble_budget_consistency",
                        message:  format!(
                            "node '{node}' has x.preamble_inline_max_kb={max} above the aggregate \
                         x.preamble_budget_kb={budget}: the aggregate demote pass will re-demote \
                         values the raised ceiling kept inline"
                        ),
                        node:     Some(node.clone()),
                    });
                }
            }
        }
        if let Some(max) = self.graph.preamble_inline_max_kb {
            let budget = self
                .graph
                .preamble_budget_kb
                .unwrap_or(DEFAULT_PREAMBLE_BUDGET_KB);
            if max > budget {
                findings.push(StageEnvelopeLint {
                    severity: LintSeverity::Warning,
                    code:     "fork.preamble_budget_consistency",
                    message:  format!(
                        "the graph's x.preamble_inline_max_kb={max} is above the aggregate \
                         x.preamble_budget_kb={budget}: the aggregate demote pass will re-demote \
                         values the raised ceiling kept inline"
                    ),
                    node:     None,
                });
            }
        }
        findings
    }
}

/// Whether a hide-glob list covers a write entry: the entry's literal
/// traversal root sits under a hide root (or a hide entry hides
/// everything). Wildcard-rooted hide globs (`*.env`) have no literal root
/// and cover nothing but themselves.
fn hide_covers(hide: &[String], entry: &str) -> bool {
    let Some(write_root) = WorkspaceGlob::try_new(entry)
        .ok()
        .map(|glob| glob.traversal_root().to_string())
        .filter(|root| !root.is_empty())
    else {
        return false;
    };
    hide.iter().any(|pattern| {
        if pattern == "**" {
            return true;
        }
        let Some(hide_root) = WorkspaceGlob::try_new(pattern)
            .ok()
            .map(|glob| glob.traversal_root().to_string())
            .filter(|root| !root.is_empty())
        else {
            return false;
        };
        write_root == hide_root || write_root.starts_with(&format!("{hide_root}/"))
    })
}

/// The quoted-or-bare value of `name=` in an attribute block, when the
/// attribute is present.
fn attribute<'a>(block: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{name}=");
    let at = block.find(&key)?;
    let rest = &block[at + key.len()..];
    let trimmed = rest.trim_start();
    if let Some(value) = trimmed.strip_prefix('"') {
        return value.find('"').map(|end| &value[..end]);
    }
    let end = trimmed
        .find([',', ']', '\n', ' ', '\r'])
        .unwrap_or(trimmed.len());
    Some(&trimmed[..end])
}

/// A comma-separated list attribute: split, trimmed, empties dropped.
fn list(block: &str, name: &str) -> Vec<String> {
    attribute(block, name)
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect()
}

/// A numeric attribute, when present and well-formed.
fn number(block: &str, name: &str) -> Option<u64> {
    attribute(block, name)?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKFLOW: &str = r#"digraph W {
    graph [goal="G", x.preamble_budget_kb=24]
    start [shape=Mdiamond]
    exit [shape=Msquare]
    review [
        label="Review the pass",
        x.fs_hide="lib/**,.seeds/**",
        x.fs_write=".fabro/reviews/**",
        x.preamble_inline_max_kb=32
    ]
    build [x.fs_write="", x.fs_hide=""]
    start -> review -> build -> exit
    review -> exit [x.kind="soft"]
}"#;

    #[test]
    fn parses_node_envelopes_from_multiline_blocks() {
        let envelopes = StageEnvelopes::parse(WORKFLOW);
        let review = envelopes
            .envelope("review")
            .expect("review has an envelope");
        assert_eq!(review.fs_hide, vec!["lib/**", ".seeds/**"]);
        assert_eq!(review.fs_write, Some(vec![".fabro/reviews/**".to_string()]));
        assert_eq!(review.preamble_inline_max_kb, Some(32));
        assert_eq!(envelopes.graph.preamble_budget_kb, Some(24));
    }

    #[test]
    fn empty_fs_write_is_a_read_only_stage_not_an_unset_one() {
        let envelopes = StageEnvelopes::parse(WORKFLOW);
        let build = envelopes.envelope("build").expect("build has an envelope");
        assert_eq!(build.fs_write, Some(Vec::new()));
        let scope = envelopes
            .fs_scope("build")
            .expect("build declares envelopes")
            .expect("globs compile");
        assert!(scope.check_write("", "any/file").is_err());
    }

    #[test]
    fn nodes_without_attributes_are_unrestricted() {
        let envelopes = StageEnvelopes::parse(WORKFLOW);
        assert!(envelopes.envelope("start").is_none());
        assert!(envelopes.fs_scope("start").is_none());
    }

    #[test]
    fn expansion_clones_carry_the_base_nodes_envelope() {
        let envelopes = StageEnvelopes::parse(WORKFLOW);
        assert!(envelopes.envelope("review#2").is_some());
    }

    #[test]
    fn edge_blocks_are_not_nodes() {
        let envelopes = StageEnvelopes::parse(WORKFLOW);
        assert!(envelopes.envelope("x.kind").is_none());
        assert_eq!(envelopes.nodes.len(), 2);
    }

    #[test]
    fn lints_flag_budget_and_overlap_but_not_valid_scopes() {
        let envelopes = StageEnvelopes::parse(WORKFLOW);
        let findings = envelopes.lint();
        assert!(
            findings
                .iter()
                .any(|lint| lint.code == "fork.preamble_budget_consistency"
                    && lint.node.as_deref() == Some("review"))
        );
        let overlap = r#"digraph W { a [x.fs_hide="lib/**", x.fs_write="lib/**"] }"#;
        let findings = StageEnvelopes::parse(overlap).lint();
        assert!(
            findings
                .iter()
                .any(|lint| lint.code == "fork.fs_scope_consistency")
        );
    }

    #[test]
    fn invalid_glob_is_an_error_finding() {
        let source = r#"digraph W { a [x.fs_write="../escape"] }"#;
        let findings = StageEnvelopes::parse(source).lint();
        assert!(
            findings
                .iter()
                .any(|lint| lint.severity == LintSeverity::Error
                    && lint.code == "fork.fs_globs_valid")
        );
    }
}
