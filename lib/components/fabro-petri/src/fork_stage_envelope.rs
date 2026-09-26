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
//!
//! Unknown `x.*` attribute names are REFUSED at create (`lint`,
//! fabro-70af): the petri rework dropped the whole legacy per-node
//! family silently, and only a loud admission keeps the next attribute
//! from going dark the same way. Names in the recognized-but-unenforced
//! family warn — each turns silent once its enforcement lands.

use std::collections::{BTreeMap, BTreeSet};

use fabro_redact::fs_scope::{FsScope, FsScopeError};
use fabro_util::workspace_glob::WorkspaceGlob;

/// The aggregate preamble budget when no `x.preamble_budget_kb` is set:
/// the value the legacy engine defaulted to (fabro-a85b).
const DEFAULT_PREAMBLE_BUDGET_KB: u64 = 48;

/// The `x.*` attributes parsed AND enforced today, on node or graph
/// blocks: the envelope set plus the graph preamble budget. Edges carry
/// [`EDGE_X`] only.
const ENFORCED_X: &[&str] = &[
    "x.fs_hide",
    "x.fs_write",
    "x.preamble_inline_max_kb",
    "x.preamble_budget_kb",
    "x.fabro_tools",
    "x.tools",
];

/// The legacy per-node family the petri rework dropped (fabro-70af):
/// recognized by the census — warned as unenforced, never silently
/// dropped — until each name lands its enforcement and moves up into
/// [`ENFORCED_X`].
const RECOGNIZED_X: &[&str] = &[
    "x.skills",
    "x.inspects",
    "x.preamble_stages_ignore",
    "x.preamble_stages_latest_only",
    "x.preamble_allow_keys",
    "x.context_allow_keys",
    "x.context_consume_keys",
    "x.preamble_output_max_lines",
];

/// The `x.*` attributes an edge block may carry: exit kinds.
const EDGE_X: &[&str] = &["x.kind"];

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
    /// `x.fabro_tools`: when present, the run tools the node's sessions
    /// may register (an empty value: none). Nodes without the attribute
    /// register none either — the per-node allowlist the legacy engine
    /// enforced, restored at the host-tools seam (fabro-96c6).
    pub fabro_tools:            Option<Vec<String>>,
    /// `x.tools`: when present, the only session tools the node's agent
    /// may call (an empty value: none — the read-only reviewer posture).
    /// Enforced mechanically at the tool boundary, Pebble's middleware
    /// included ([`crate::tool_policy`], fabro-1a41); `None` leaves the
    /// session's full toolset.
    pub tools:                  Option<Vec<String>>,
}

/// The graph block's envelope numbers, as written.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GraphEnvelope {
    /// `x.preamble_budget_kb` on the graph block, when set.
    pub preamble_budget_kb:     Option<u64>,
    /// `x.preamble_inline_max_kb` on the graph block, when set.
    pub preamble_inline_max_kb: Option<u64>,
}

/// One `x.*` attribute site the scan saw: the block's subject (a node
/// name, `graph`, or an edge's `a -> b` text) and the attribute name.
#[derive(Clone, Debug, PartialEq, Eq)]
struct XAttributeSite {
    subject: String,
    name:    String,
    on_edge: bool,
}

/// Stage envelopes by node, parsed from the DOT `graph_source`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StageEnvelopes {
    nodes:   BTreeMap<String, NodeEnvelope>,
    graph:   GraphEnvelope,
    x_sites: Vec<XAttributeSite>,
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
        let mut x_sites = Vec::new();
        let mut rest = graph_source;
        while let Some(open) = rest.find('[') {
            let head = &rest[..open];
            let after = &rest[open + 1..];
            let Some(close) = after.find(']') else { break };
            let block = &after[..close];
            rest = &after[close + 1..];
            // The subject is the tail of the head after the last `{` (a
            // graph header can share the line with the first statement):
            // `name`, `graph`, or an edge's `a -> b`. Edges carry their
            // own `x.*` attributes (exit kinds) and never an envelope.
            let after_brace = head.rsplit_once('{').map_or(head, |(_, after)| after);
            let subject = after_brace
                .lines()
                .last()
                .unwrap_or_default()
                .trim()
                .trim_matches(|c: char| c.is_whitespace() || c == ';' || c == ',');
            let on_edge = subject.contains("->");
            for attribute in x_attribute_names(block) {
                x_sites.push(XAttributeSite {
                    subject: subject.to_string(),
                    name: attribute,
                    on_edge,
                });
            }
            if on_edge {
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
                    fabro_tools:            attribute(block, "x.fabro_tools")
                        .map(|_| list(block, "x.fabro_tools")),
                    tools:                  attribute(block, "x.tools")
                        .map(|_| list(block, "x.tools")),
                };
                let declares = !envelope.fs_hide.is_empty()
                    || envelope.fs_write.is_some()
                    || envelope.preamble_inline_max_kb.is_some()
                    || envelope.fabro_tools.is_some()
                    || envelope.tools.is_some();
                if declares {
                    nodes.insert(name.to_string(), envelope);
                }
            }
        }
        Self {
            nodes,
            graph,
            x_sites,
        }
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
        self.lint_x_vocabulary(&mut findings);
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

    /// The `x.*` vocabulary findings (fabro-70af): an unknown name on a
    /// node, graph, or edge block refuses the workflow — the silent drop
    /// is how the whole legacy family went dark — while a recognized
    /// name without enforcement warns once per (subject, name).
    fn lint_x_vocabulary(&self, findings: &mut Vec<StageEnvelopeLint>) {
        let mut warned = BTreeSet::new();
        for site in &self.x_sites {
            if site.on_edge {
                if !EDGE_X.contains(&site.name.as_str()) {
                    findings.push(StageEnvelopeLint {
                        severity: LintSeverity::Error,
                        code:     "fork.x_attribute_known",
                        message:  format!(
                            "edge '{}' carries unknown attribute '{}': edges accept x.kind only \
                             (fabro-70af refuses unknown x.* instead of dropping them)",
                            site.subject, site.name
                        ),
                        node:     None,
                    });
                }
                continue;
            }
            if ENFORCED_X.contains(&site.name.as_str()) {
                continue;
            }
            if RECOGNIZED_X.contains(&site.name.as_str()) {
                if warned.insert((site.subject.clone(), site.name.clone())) {
                    findings.push(StageEnvelopeLint {
                        severity: LintSeverity::Warning,
                        code:     "fork.x_attribute_enforced",
                        message:  format!(
                            "'{}' declares '{}': recognized but NOT enforced on this engine yet \
                             (fabro-70af re-implementation pending)",
                            site.subject, site.name
                        ),
                        node:     Some(site.subject.clone()),
                    });
                }
                continue;
            }
            findings.push(StageEnvelopeLint {
                severity: LintSeverity::Error,
                code:     "fork.x_attribute_known",
                message:  format!(
                    "'{}' carries unknown attribute '{}': not in the x.* vocabulary — a typo or \
                     an unannounced name; refused instead of silently dropped (fabro-70af)",
                    site.subject, site.name
                ),
                node:     Some(site.subject.clone()),
            });
        }
    }
}

/// The `x.*` attribute names in a block, quoted spans stripped first so
/// label text cannot pose as an attribute. A name counts only when its
/// whole `x.<name>=` span sits at a delimiter boundary.
fn x_attribute_names(block: &str) -> Vec<String> {
    let stripped = strip_quoted_spans(block);
    let mut names = Vec::new();
    let mut rest = stripped.as_str();
    while let Some(at) = rest.find("x.") {
        let before_boundary = rest[..at]
            .chars()
            .next_back()
            .is_none_or(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | ',' | '[' | ';'));
        let after = &rest[at + 2..];
        if before_boundary {
            if let Some(equals) = after.find('=') {
                let name: String = after[..equals]
                    .chars()
                    .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')
                    .collect();
                if !name.is_empty() && name.len() == equals {
                    names.push(format!("x.{name}"));
                }
            }
        }
        rest = after;
    }
    names
}

/// The block with double-quoted spans (label text, attribute values)
/// removed, escape-aware.
fn strip_quoted_spans(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut quoted = false;
    let mut escaped = false;
    for character in text.chars() {
        if quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                quoted = false;
            }
        } else if character == '"' {
            quoted = true;
        } else {
            out.push(character);
        }
    }
    out
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
    use std::path::{Path, PathBuf};

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
    fn single_line_digraph_statements_parse() {
        let envelopes = StageEnvelopes::parse(
            "digraph W { graph [x.preamble_budget_kb=24] a [x.fabro_tools=\"fabro_run_get\"] }",
        );
        assert_eq!(envelopes.graph.preamble_budget_kb, Some(24));
        assert_eq!(
            envelopes
                .envelope("a")
                .and_then(|envelope| envelope.fabro_tools.clone()),
            Some(vec!["fabro_run_get".to_string()])
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

    #[test]
    fn unknown_x_attribute_refuses_admission() {
        let source = r#"digraph W { a [x.fs_write="lib/**", x.preamble_stage_ignore="b"] }"#;
        let findings = StageEnvelopes::parse(source).lint();
        let unknown = findings
            .iter()
            .find(|lint| lint.code == "fork.x_attribute_known")
            .expect("the typo'd name is refused");
        assert_eq!(unknown.severity, LintSeverity::Error);
        assert_eq!(unknown.node.as_deref(), Some("a"));
        assert!(unknown.message.contains("x.preamble_stage_ignore"));
    }

    #[test]
    fn recognized_but_unenforced_family_warns_once_per_subject() {
        let source = r#"digraph W {
            a [x.preamble_stages_ignore="b,c", x.context_allow_keys="k", x.skills="discover"]
            b [x.preamble_stages_ignore="a"]
        }"#;
        let findings = StageEnvelopes::parse(source).lint();
        let warnings: Vec<_> = findings
            .iter()
            .filter(|lint| lint.code == "fork.x_attribute_enforced")
            .collect();
        assert_eq!(warnings.len(), 4, "three names on a, one on b");
        assert!(
            warnings
                .iter()
                .all(|lint| lint.severity == LintSeverity::Warning)
        );
        assert!(
            findings
                .iter()
                .all(|lint| lint.code != "fork.x_attribute_known")
        );
    }

    #[test]
    fn edges_accept_only_exit_kinds() {
        let kinds = r#"digraph W { a -> b [x.kind="soft"] }"#;
        assert!(
            StageEnvelopes::parse(kinds)
                .lint()
                .iter()
                .all(|lint| lint.code != "fork.x_attribute_known")
        );
        let bogus = r#"digraph W { a -> b [x.kind="soft", x.fs_write="lib/**"] }"#;
        let findings = StageEnvelopes::parse(bogus).lint();
        assert!(
            findings
                .iter()
                .any(|lint| lint.severity == LintSeverity::Error
                    && lint.code == "fork.x_attribute_known"
                    && lint.message.contains("x.fs_write"))
        );
    }

    #[test]
    fn label_text_cannot_pose_as_an_x_attribute() {
        let source = r#"digraph W { a [label="the x.bogus=1 attribute is documented"] }"#;
        assert!(
            StageEnvelopes::parse(source)
                .lint()
                .iter()
                .all(|lint| lint.code != "fork.x_attribute_known")
        );
    }

    /// fabro-70af: the workspace's own graphs stay inside the x.*
    /// vocabulary — every attribute fabro's workflows carry is either
    /// enforced or a recognized member of the pending family. A new
    /// attribute name lands here first, red, before any workflow uses it
    /// silently.
    #[test]
    fn workspace_x_census_stays_within_the_vocabulary() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let mut graphs = Vec::new();
        collect_fabro_files(&root.join(".fabro"), &mut graphs);
        assert!(
            !graphs.is_empty(),
            "the census found no .fabro graphs under the repo root"
        );
        let mut offenders = Vec::new();
        for graph in &graphs {
            for lint in StageEnvelopes::parse(&read(graph)).lint() {
                if lint.code == "fork.x_attribute_known" {
                    offenders.push(format!(
                        "{}: {}",
                        graph.strip_prefix(&root).unwrap_or(graph).display(),
                        lint.message
                    ));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "unknown x.* attributes in the workspace graphs:\n{}",
            offenders.join("\n")
        );
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "test-only census I/O over the checkout"
    )]
    fn collect_fabro_files(dir: &Path, out: &mut Vec<PathBuf>) {
        let entries = std::fs::read_dir(dir)
            .unwrap_or_else(|error| panic!("read_dir {}: {error}", dir.display()));
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_fabro_files(&path, out);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "fabro")
            {
                out.push(path);
            }
        }
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "test-only census I/O over the checkout"
    )]
    fn read(path: &Path) -> String {
        std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
    }
}
