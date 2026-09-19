//! The workflow graph as written: the typed model `fabro_graphviz::parser`
//! produces from a DOT file.
//!
//! This is the graph the bundler and workflow version registration walk to
//! find what a workflow references (`import`, `stack.child_workflow`,
//! `@file` prompts, the goal, the model stylesheet). Petri compiles and
//! admits the workflow; the graph a run displays is [`crate::RunGraph`],
//! read off Petri's admitted graph, not this model.

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Typed attribute values for nodes, edges, and graph-level attributes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AttrValue {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Duration(Duration),
}

impl AttrValue {
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Integer(n) => Some(*n),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Float(n) => Some(*n),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_duration(&self) -> Option<Duration> {
        match self {
            Self::Duration(d) => Some(*d),
            _ => None,
        }
    }
}

/// Maps Graphviz shapes to handler type strings (Section 2.8).
#[must_use]
pub fn shape_to_handler_type(shape: &str) -> Option<&'static str> {
    match shape {
        "Mdiamond" => Some("start"),
        "Msquare" => Some("exit"),
        "box" => Some("agent"),
        "tab" => Some("prompt"),
        "hexagon" => Some("human"),
        "diamond" => Some("conditional"),
        "component" => Some("parallel"),
        "tripleoctagon" => Some("parallel.fan_in"),
        "parallelogram" => Some("command"),
        "house" => Some("stack.manager_loop"),
        "insulator" => Some("wait"),
        _ => None,
    }
}

/// A node in the workflow graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id:      String,
    pub attrs:   HashMap<String, AttrValue>,
    /// CSS-like classes for model stylesheet targeting (from `class` attr and
    /// subgraph derivation).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub classes: Vec<String>,
}

impl Node {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id:      id.into(),
            attrs:   HashMap::new(),
            classes: Vec::new(),
        }
    }

    /// Appends a class, ignoring blank names and ones already present.
    ///
    /// Classes accumulate from several sources — the `class` attribute and
    /// enclosing subgraphs — so every caller needs the same de-duplicating
    /// append. The name is trimmed, and a name that is empty or only
    /// whitespace is dropped. Order is preserved.
    pub fn add_class(&mut self, class: &str) {
        let class = class.trim();
        if !class.is_empty() && !self.classes.iter().any(|existing| existing == class) {
            self.classes.push(class.to_string());
        }
    }

    fn str_attr(&self, key: &str) -> Option<&str> {
        self.attrs.get(key).and_then(AttrValue::as_str)
    }

    #[must_use]
    pub fn label(&self) -> &str {
        self.str_attr("label").unwrap_or(&self.id)
    }

    /// The node's Graphviz shape, which contributes to handler selection.
    ///
    /// An explicit `shape` or `type` attribute disables inference. Otherwise,
    /// the presence of `script` infers `parallelogram`. Everything else falls
    /// back to `box`.
    #[must_use]
    pub fn shape(&self) -> &str {
        if let Some(shape) = self.str_attr("shape") {
            return shape;
        }
        if self.node_type().is_none() && self.attrs.contains_key("script") {
            return "parallelogram";
        }
        "box"
    }

    #[must_use]
    pub fn node_type(&self) -> Option<&str> {
        self.str_attr("type")
    }

    #[must_use]
    pub fn prompt(&self) -> Option<&str> {
        self.str_attr("prompt")
    }

    /// Resolve the handler type for this node using explicit type or shape
    /// mapping.
    #[must_use]
    pub fn handler_type(&self) -> Option<&str> {
        match self.node_type() {
            Some("tool") => return Some("command"),
            Some(node_type) => return Some(node_type),
            None => {}
        }
        shape_to_handler_type(self.shape())
    }
}

/// An edge connecting two nodes in the workflow graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub from:  String,
    pub to:    String,
    pub attrs: HashMap<String, AttrValue>,
}

impl Edge {
    pub fn new(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            from:  from.into(),
            to:    to.into(),
            attrs: HashMap::new(),
        }
    }

    fn str_attr(&self, key: &str) -> Option<&str> {
        self.attrs.get(key).and_then(AttrValue::as_str)
    }

    #[must_use]
    pub fn label(&self) -> Option<&str> {
        self.str_attr("label")
    }

    #[must_use]
    pub fn condition(&self) -> Option<&str> {
        self.str_attr("condition")
    }
}

/// The parsed workflow graph containing nodes, edges, and graph-level
/// attributes.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Graph {
    pub name:  String,
    pub nodes: HashMap<String, Node>,
    pub edges: Vec<Edge>,
    pub attrs: HashMap<String, AttrValue>,
}

impl Graph {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name:  name.into(),
            nodes: HashMap::new(),
            edges: Vec::new(),
            attrs: HashMap::new(),
        }
    }

    /// Graph-level goal attribute.
    pub fn goal(&self) -> &str {
        self.attrs
            .get("goal")
            .and_then(AttrValue::as_str)
            .unwrap_or("")
    }

    /// Graph-level model stylesheet attribute.
    pub fn model_stylesheet(&self) -> &str {
        self.attrs
            .get("model_stylesheet")
            .and_then(AttrValue::as_str)
            .unwrap_or("")
    }
}

/// Where an attribute appears in a workflow graph.
#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::Display, strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum AttributeScope {
    Graph,
    Node,
    Edge,
}

/// Kinds of static (non-templated) workflow-owned file references.
#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::Display)]
pub enum ReferenceKind {
    #[strum(to_string = "file inline reference")]
    FileInline,
    #[strum(to_string = "import reference")]
    Import,
    #[strum(to_string = "child workflow reference")]
    ChildWorkflow,
    #[strum(to_string = "Dockerfile reference")]
    Dockerfile,
    #[strum(to_string = "graph goal file reference")]
    GraphGoalFile,
    #[strum(to_string = "run goal file reference")]
    RunGoalFile,
}

/// Kinds of static file references that graph attributes can carry: the
/// subset of [`ReferenceKind`] that [`reference_kind_for_attribute`] can
/// classify. Config-sourced kinds (Dockerfiles, run goal files) are
/// unrepresentable here by construction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphReferenceKind {
    FileInline,
    Import,
    ChildWorkflow,
    GraphGoalFile,
}

impl From<GraphReferenceKind> for ReferenceKind {
    fn from(kind: GraphReferenceKind) -> Self {
        match kind {
            GraphReferenceKind::FileInline => Self::FileInline,
            GraphReferenceKind::Import => Self::Import,
            GraphReferenceKind::ChildWorkflow => Self::ChildWorkflow,
            GraphReferenceKind::GraphGoalFile => Self::GraphGoalFile,
        }
    }
}

/// Classify a graph attribute as a static file reference, if it is one.
#[must_use]
pub fn reference_kind_for_attribute(
    scope: AttributeScope,
    key: &str,
    value: &str,
) -> Option<GraphReferenceKind> {
    match key {
        "import" if matches!(scope, AttributeScope::Node) => Some(GraphReferenceKind::Import),
        "stack.child_workflow" if matches!(scope, AttributeScope::Node) => {
            Some(GraphReferenceKind::ChildWorkflow)
        }
        "goal" if matches!(scope, AttributeScope::Graph) && value.starts_with('@') => {
            Some(GraphReferenceKind::GraphGoalFile)
        }
        "prompt" | "output_schema"
            if matches!(scope, AttributeScope::Node) && value.starts_with('@') =>
        {
            Some(GraphReferenceKind::FileInline)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attr_value_accessors_match_their_variant() {
        assert_eq!(
            AttrValue::String("hello".to_string()).as_str(),
            Some("hello")
        );
        assert_eq!(AttrValue::Integer(1).as_str(), None);
        assert_eq!(AttrValue::Integer(42).as_i64(), Some(42));
        assert_eq!(AttrValue::String("x".to_string()).as_i64(), None);
        assert_eq!(AttrValue::Float(3.15).as_f64(), Some(3.15));
        assert_eq!(AttrValue::Integer(1).as_f64(), None);
        assert_eq!(AttrValue::Boolean(true).as_bool(), Some(true));
        assert_eq!(AttrValue::String("true".to_string()).as_bool(), None);
        let ten = Duration::from_secs(10);
        assert_eq!(AttrValue::Duration(ten).as_duration(), Some(ten));
        assert_eq!(AttrValue::Integer(10).as_duration(), None);
    }

    #[test]
    fn shape_to_handler_type_mappings() {
        assert_eq!(shape_to_handler_type("Mdiamond"), Some("start"));
        assert_eq!(shape_to_handler_type("Msquare"), Some("exit"));
        assert_eq!(shape_to_handler_type("box"), Some("agent"));
        assert_eq!(shape_to_handler_type("tab"), Some("prompt"));
        assert_eq!(shape_to_handler_type("hexagon"), Some("human"));
        assert_eq!(shape_to_handler_type("diamond"), Some("conditional"));
        assert_eq!(shape_to_handler_type("component"), Some("parallel"));
        assert_eq!(
            shape_to_handler_type("tripleoctagon"),
            Some("parallel.fan_in")
        );
        assert_eq!(shape_to_handler_type("parallelogram"), Some("command"));
        assert_eq!(shape_to_handler_type("house"), Some("stack.manager_loop"));
        assert_eq!(shape_to_handler_type("insulator"), Some("wait"));
        assert_eq!(shape_to_handler_type("unknown"), None);
    }

    #[test]
    fn node_defaults() {
        let node = Node::new("test");
        assert_eq!(node.id, "test");
        assert_eq!(node.label(), "test");
        assert_eq!(node.shape(), "box");
        assert_eq!(node.node_type(), None);
        assert_eq!(node.prompt(), None);
        assert!(node.classes.is_empty());
        assert_eq!(node.handler_type(), Some("agent"));
    }

    #[test]
    fn add_class_trims_names_and_drops_blanks_and_duplicates() {
        let mut node = Node::new("work");
        node.add_class("coding");
        node.add_class(" coding ");
        node.add_class("");
        node.add_class("   ");
        node.add_class("\tcritical\n");

        assert_eq!(node.classes, ["coding", "critical"]);
    }

    fn node_with(id: &str, attrs: &[(&str, &str)]) -> Node {
        let mut node = Node::new(id);
        for (key, value) in attrs {
            node.attrs
                .insert((*key).to_string(), AttrValue::String((*value).to_string()));
        }
        node
    }

    #[test]
    fn shapeless_script_node_infers_command() {
        let node = node_with("build", &[("script", "cargo build")]);
        assert_eq!(node.shape(), "parallelogram");
        assert_eq!(node.handler_type(), Some("command"));
    }

    #[test]
    fn shapeless_node_without_script_stays_agent() {
        let node = node_with("plan", &[("prompt", "Plan the work")]);
        assert_eq!(node.shape(), "box");
        assert_eq!(node.handler_type(), Some("agent"));
        assert_eq!(node.prompt(), Some("Plan the work"));
    }

    #[test]
    fn explicit_shape_or_type_wins_over_script_inference() {
        let shaped = node_with("odd", &[("shape", "box"), ("script", "cargo build")]);
        assert_eq!(shaped.shape(), "box");
        assert_eq!(shaped.handler_type(), Some("agent"));

        let typed = node_with("odd", &[("type", "agent"), ("script", "cargo build")]);
        assert_eq!(typed.shape(), "box");
        assert_eq!(typed.handler_type(), Some("agent"));
    }

    #[test]
    fn any_script_attribute_value_infers_command() {
        let empty = node_with("empty", &[("script", "")]);
        assert_eq!(empty.shape(), "parallelogram");
        assert_eq!(empty.handler_type(), Some("command"));

        let mut non_string = Node::new("non_string");
        non_string
            .attrs
            .insert("script".to_string(), AttrValue::Integer(123));
        assert_eq!(non_string.shape(), "parallelogram");
        assert_eq!(non_string.handler_type(), Some("command"));
    }

    #[test]
    fn explicit_types_and_shapes_resolve_handler_types() {
        assert_eq!(
            node_with("build", &[("type", "tool")]).handler_type(),
            Some("command")
        );
        assert_eq!(
            node_with("gate", &[("type", "human")]).handler_type(),
            Some("human")
        );
        assert_eq!(
            node_with("entry", &[("shape", "Mdiamond")]).handler_type(),
            Some("start")
        );
        assert_eq!(node_with("odd", &[("shape", "star")]).handler_type(), None);
    }

    #[test]
    fn edge_attributes_are_read_as_written() {
        let bare = Edge::new("a", "b");
        assert_eq!(bare.from, "a");
        assert_eq!(bare.to, "b");
        assert_eq!(bare.label(), None);
        assert_eq!(bare.condition(), None);

        let mut edge = Edge::new("a", "b");
        edge.attrs
            .insert("label".to_string(), AttrValue::String("next".to_string()));
        edge.attrs.insert(
            "condition".to_string(),
            AttrValue::String("outcome=succeeded".to_string()),
        );
        assert_eq!(edge.label(), Some("next"));
        assert_eq!(edge.condition(), Some("outcome=succeeded"));
    }

    #[test]
    fn graph_goal_and_stylesheet_default_to_empty() {
        let mut graph = Graph::new("test");
        assert_eq!(graph.name, "test");
        assert_eq!(graph.goal(), "");
        assert_eq!(graph.model_stylesheet(), "");

        graph.attrs.insert(
            "goal".to_string(),
            AttrValue::String("Run tests".to_string()),
        );
        graph.attrs.insert(
            "model_stylesheet".to_string(),
            AttrValue::String("* { model: gpt-5.4; }".to_string()),
        );
        assert_eq!(graph.goal(), "Run tests");
        assert_eq!(graph.model_stylesheet(), "* { model: gpt-5.4; }");
    }

    #[test]
    fn output_schema_at_value_is_file_inline_reference() {
        assert_eq!(
            reference_kind_for_attribute(
                AttributeScope::Node,
                "output_schema",
                "@schemas/result.schema.json",
            ),
            Some(GraphReferenceKind::FileInline),
        );
    }

    #[test]
    fn output_schema_builtin_keyword_is_not_file_inline_reference() {
        assert_eq!(
            reference_kind_for_attribute(AttributeScope::Node, "output_schema", "routing"),
            None,
        );
    }
}
