//! Fabro DOT re-emitted as DOT Graphviz accepts.
//!
//! Graphviz rejects unquoted dotted attribute keys such as `acp.command`,
//! which Fabro's language allows. The render paths parse the source with
//! Petri's parser and print it back with every id quoted that needs quoting.

use std::borrow::Cow;

use petri_frontend_attractor::dot::{
    self, AstValue, Attr, AttrBlock, DotGraph, EdgeStmt, NodeStmt, Statement, SubgraphStmt,
};

/// Convert Fabro DOT into DOT Graphviz accepts.
///
/// If the source is outside the subset Petri parses, it is returned unchanged
/// so Graphviz can judge it itself: it may be valid Graphviz that is not a
/// workflow.
#[must_use]
pub fn normalize_for_graphviz(source: &str) -> Cow<'_, str> {
    match dot::parse("workflow.fabro", source) {
        Ok(graph) => Cow::Owned(emit_graph(&graph)),
        Err(_) => Cow::Borrowed(source),
    }
}

fn emit_graph(graph: &DotGraph) -> String {
    let mut out = String::new();
    out.push_str("digraph");
    if !graph.name.name.is_empty() {
        out.push(' ');
        out.push_str(&dot_id(&graph.name.name));
    }
    out.push_str(" {\n");
    emit_statements(&mut out, &graph.statements, 1);
    out.push_str("}\n");
    out
}

fn emit_statements(out: &mut String, statements: &[Statement], indent: usize) {
    for statement in statements {
        emit_statement(out, statement, indent);
    }
}

fn emit_statement(out: &mut String, statement: &Statement, indent: usize) {
    match statement {
        Statement::GraphAttrs(attrs) => emit_defaults(out, "graph", attrs, indent),
        Statement::NodeDefaults(attrs) => emit_defaults(out, "node", attrs, indent),
        Statement::EdgeDefaults(attrs) => emit_defaults(out, "edge", attrs, indent),
        Statement::Subgraph(subgraph) => emit_subgraph(out, subgraph, indent),
        Statement::Node(node) => emit_node(out, node, indent),
        Statement::Edge(edge) => emit_edge(out, edge, indent),
        Statement::GraphAttr(attr) => {
            push_indent(out, indent);
            emit_attr(out, attr);
            out.push_str(";\n");
        }
    }
}

fn emit_defaults(out: &mut String, keyword: &str, attrs: &AttrBlock, indent: usize) {
    push_indent(out, indent);
    out.push_str(keyword);
    out.push(' ');
    emit_attr_block(out, attrs);
    out.push_str(";\n");
}

fn emit_subgraph(out: &mut String, subgraph: &SubgraphStmt, indent: usize) {
    push_indent(out, indent);
    out.push_str("subgraph");
    if let Some(name) = &subgraph.name {
        out.push(' ');
        out.push_str(&dot_id(&name.name));
    }
    out.push_str(" {\n");
    emit_statements(out, &subgraph.statements, indent + 1);
    push_indent(out, indent);
    out.push_str("}\n");
}

fn emit_node(out: &mut String, node: &NodeStmt, indent: usize) {
    push_indent(out, indent);
    out.push_str(&dot_id(&node.id.name));
    if let Some(attrs) = &node.attrs {
        out.push(' ');
        emit_attr_block(out, attrs);
    }
    out.push_str(";\n");
}

fn emit_edge(out: &mut String, edge: &EdgeStmt, indent: usize) {
    push_indent(out, indent);
    let mut nodes = edge.nodes.iter();
    if let Some(first) = nodes.next() {
        out.push_str(&dot_id(&first.name));
        for node in nodes {
            out.push_str(" -> ");
            out.push_str(&dot_id(&node.name));
        }
    }
    if let Some(attrs) = &edge.attrs {
        out.push(' ');
        emit_attr_block(out, attrs);
    }
    out.push_str(";\n");
}

fn emit_attr_block(out: &mut String, attrs: &AttrBlock) {
    out.push('[');
    for (index, attr) in attrs.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        emit_attr(out, attr);
    }
    out.push(']');
}

fn emit_attr(out: &mut String, attr: &Attr) {
    out.push_str(&dot_id(&attr.key));
    out.push('=');
    out.push_str(&dot_value(&attr.value));
}

fn dot_value(value: &AstValue) -> String {
    match value {
        AstValue::Str(value) => quoted_dot_string(value),
        AstValue::Int(value) => value.to_string(),
        AstValue::Float(value) => value.to_string(),
        AstValue::Bool(value) => value.to_string(),
        AstValue::Ident(value) => dot_id(value),
    }
}

fn dot_id(value: &str) -> String {
    if is_plain_dot_id(value) && !is_dot_keyword(value) {
        value.to_string()
    } else {
        quoted_dot_string(value)
    }
}

fn quoted_dot_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn is_plain_dot_id(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn is_dot_keyword(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "digraph" | "edge" | "graph" | "node" | "strict" | "subgraph"
    )
}

fn push_indent(out: &mut String, indent: usize) {
    for _ in 0..indent {
        out.push_str("    ");
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_for_graphviz;

    #[test]
    fn quotes_dotted_attribute_keys() {
        let source = r#"digraph X {
            a [label="A", acp.command="codex"]
        }"#;

        let normalized = normalize_for_graphviz(source);

        assert!(normalized.contains(r#""acp.command"="codex""#));
    }

    #[test]
    fn quotes_known_fabro_dotted_attribute_keys() {
        let source = r#"digraph X {
            approve [human.default_choice="deploy"]
            child [stack.child_workflow="child.fabro", manager.max_cycles=50]
            approve -> child
        }"#;

        let normalized = normalize_for_graphviz(source);

        assert!(normalized.contains(r#""human.default_choice"="deploy""#));
        assert!(normalized.contains(r#""stack.child_workflow"="child.fabro""#));
        assert!(normalized.contains(r#""manager.max_cycles"=50"#));
    }

    #[test]
    fn preserves_subgraphs_defaults_and_bare_graph_attributes() {
        let source = r##"digraph X {
            rankdir=LR
            node [color="#357f9e"]
            subgraph cluster_loop {
                label="Loop"
                a [acp.command="codex"]
            }
        }"##;

        let normalized = normalize_for_graphviz(source);

        assert!(normalized.contains("rankdir=LR;"));
        assert!(normalized.contains("node ["));
        assert!(normalized.contains("subgraph cluster_loop"));
        assert!(normalized.contains(r#""acp.command"="codex""#));
    }

    #[test]
    fn quotes_ids_that_collide_with_keywords_or_need_escaping() {
        let source = r#"digraph X {
            "my node" [label="say \"hi\""]
            "my node" -> Node
        }"#;

        let normalized = normalize_for_graphviz(source);

        assert!(normalized.contains(r#""my node" [label="say \"hi\""]"#));
        assert!(normalized.contains(r#""my node" -> "Node""#));
    }

    #[test]
    fn source_outside_the_subset_passes_through_unchanged() {
        let source = "graph G { a -- b }";

        assert_eq!(normalize_for_graphviz(source), source);
    }
}
