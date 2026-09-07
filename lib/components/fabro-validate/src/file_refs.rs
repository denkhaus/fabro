//! Resolve workflow-relative `@`-file references before lint rules run.
//!
//! The `unresolved_file_ref` rule treats any `@`-prefixed `prompt` or `goal`
//! as an error because the runtime pipeline normally inlines those files
//! before validation. Standalone graph validation (the `fabro-validate`
//! binary behind `just validate-workflows`) does have a real base directory —
//! the graph file's directory — so it inlines referenced files here first,
//! mirroring the runtime's file-inlining approach. References whose file does
//! not exist are left untouched so `unresolved_file_ref` still reports them:
//! resolution narrows the rule to genuinely broken references instead of
//! scoping the rule out wholesale.
//!
//! The runtime also renders templated `model_stylesheet` sources before the
//! `stylesheet_syntax` rule runs; standalone validation cannot render them,
//! so [`strip_templated_stylesheet`] blanks them instead of false-positiving
//! on template syntax.

use std::path::Path;

use fabro_graphviz::graph::{AttrValue, Graph};

/// Node attributes whose `@`-prefixed values are file references covered by
/// the `unresolved_file_ref` rule.
const NODE_REF_ATTRS: [&str; 1] = ["prompt"];

/// Graph attributes whose `@`-prefixed values are file references covered by
/// the `unresolved_file_ref` rule.
const GRAPH_REF_ATTRS: [&str; 1] = ["goal"];

#[expect(
    clippy::disallowed_methods,
    reason = "sync CLI linter inlines small prompt/goal files synchronously"
)]
fn inline_ref(value: &mut String, base_dir: &Path) {
    let Some(reference) = value.strip_prefix('@') else {
        return;
    };
    let path = base_dir.join(reference);
    if let Ok(contents) = std::fs::read_to_string(&path) {
        *value = contents;
    }
    // A missing file keeps the `@` reference so `unresolved_file_ref`
    // reports it instead of silently suppressing the error.
}

/// Inline every resolvable `@`-file reference in `graph` relative to
/// `base_dir`, leaving unresolvable references for `unresolved_file_ref`.
pub fn resolve_file_refs(graph: &mut Graph, base_dir: &Path) {
    for attr in GRAPH_REF_ATTRS {
        if let Some(AttrValue::String(value)) = graph.attrs.get_mut(attr) {
            inline_ref(value, base_dir);
        }
    }
    for node in graph.nodes.values_mut() {
        for attr in NODE_REF_ATTRS {
            if let Some(AttrValue::String(value)) = node.attrs.get_mut(attr) {
                inline_ref(value, base_dir);
            }
        }
    }
}

/// Blank a `model_stylesheet` whose source contains template syntax.
///
/// The runtime renders the stylesheet (MiniJinja, `inputs`/`vars`) before the
/// `stylesheet_syntax` rule runs. Raw template source is not valid stylesheet
/// syntax and cannot be rendered without a run context, so standalone
/// validation blanks templated stylesheets instead of false-positive
/// linting them; the rendered form is still validated at run creation.
/// Template-free stylesheets are kept and linted as-is.
pub fn strip_templated_stylesheet(graph: &mut Graph) {
    if let Some(AttrValue::String(stylesheet)) = graph.attrs.get_mut("model_stylesheet") {
        if stylesheet.contains("{{") || stylesheet.contains("{%") {
            stylesheet.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use fabro_graphviz::graph::{AttrValue, Edge, Graph, Node};

    use super::{resolve_file_refs, strip_templated_stylesheet};
    use crate::Severity;
    use crate::rules::test_support::minimal_graph;
    use crate::rules::unresolved_file_ref;

    fn graph_with_prompt(prompt: &str) -> Graph {
        let mut g = minimal_graph();
        let mut node = Node::new("work");
        node.attrs
            .insert("prompt".to_string(), AttrValue::String(prompt.to_string()));
        g.nodes.insert("work".to_string(), node);
        g.edges.push(Edge::new("start", "work"));
        g.edges.push(Edge::new("work", "exit"));
        g
    }

    fn unresolved_diagnostics(g: &Graph) -> Vec<String> {
        unresolved_file_ref::rule_for_tests()
            .apply(g)
            .into_iter()
            .filter(|d| d.severity == Severity::Error)
            .map(|d| d.message)
            .collect()
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "unit test creates a temporary prompt file synchronously"
    )]
    fn temp_dir_with_prompt(name: &str, contents: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        let prompts = dir.path().join("prompts");
        std::fs::create_dir_all(&prompts).expect("mkdir prompts");
        std::fs::write(prompts.join(name), contents).expect("write prompt");
        dir
    }

    #[test]
    fn resolves_existing_prompt_ref_relative_to_base_dir() {
        let base = temp_dir_with_prompt("work.md", "Do the work.");
        let mut g = graph_with_prompt("@prompts/work.md");

        resolve_file_refs(&mut g, base.path());

        let prompt = g.nodes["work"].attrs["prompt"].as_str().unwrap_or("");
        assert_eq!(prompt, "Do the work.");
        assert!(unresolved_diagnostics(&g).is_empty());
    }

    #[test]
    fn resolves_existing_goal_ref() {
        let base = temp_dir_with_prompt("goal.md", "Ship it.");
        let mut g = minimal_graph();
        g.attrs.insert(
            "goal".to_string(),
            AttrValue::String("@prompts/goal.md".to_string()),
        );

        resolve_file_refs(&mut g, base.path());

        let goal = g.attrs["goal"].as_str().unwrap_or("");
        assert_eq!(goal, "Ship it.");
        assert!(unresolved_diagnostics(&g).is_empty());
    }

    #[test]
    fn missing_prompt_ref_still_fails_unresolved_file_ref() {
        let base = tempfile::tempdir().expect("temp dir");
        let mut g = graph_with_prompt("@prompts/does-not-exist.md");

        resolve_file_refs(&mut g, base.path());

        assert_eq!(
            g.nodes["work"].attrs["prompt"].as_str().unwrap_or(""),
            "@prompts/does-not-exist.md",
            "missing reference must be left for the rule to report"
        );
        let diagnostics = unresolved_diagnostics(&g);
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].contains("@prompts/does-not-exist.md"));
    }

    #[test]
    fn inline_prompt_is_untouched() {
        let base = temp_dir_with_prompt("work.md", "Do the work.");
        let mut g = graph_with_prompt("Inline prompt, no file");

        resolve_file_refs(&mut g, base.path());

        assert_eq!(
            g.nodes["work"].attrs["prompt"].as_str().unwrap_or(""),
            "Inline prompt, no file"
        );
    }

    #[test]
    fn non_string_attr_values_are_ignored() {
        let base = Path::new("/nonexistent");
        let mut g = minimal_graph();
        g.attrs.insert("goal".to_string(), AttrValue::Integer(7));

        resolve_file_refs(&mut g, base);

        assert_eq!(g.attrs["goal"], AttrValue::Integer(7));
    }

    #[test]
    fn templated_stylesheet_is_blanked_for_standalone_lint() {
        let mut g = minimal_graph();
        g.attrs.insert(
            "model_stylesheet".to_string(),
            AttrValue::String(
                "{% set tiers = ['low'] %}\n* { reasoning_effort: low; }".to_string(),
            ),
        );

        strip_templated_stylesheet(&mut g);

        assert_eq!(g.attrs["model_stylesheet"].as_str().unwrap_or(""), "");
    }

    #[test]
    fn literal_stylesheet_is_kept() {
        let mut g = minimal_graph();
        g.attrs.insert(
            "model_stylesheet".to_string(),
            AttrValue::String("* { reasoning_effort: low; }".to_string()),
        );

        strip_templated_stylesheet(&mut g);

        assert_eq!(
            g.attrs["model_stylesheet"].as_str().unwrap_or(""),
            "* { reasoning_effort: low; }"
        );
    }
}
