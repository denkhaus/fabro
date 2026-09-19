//! The graph a run displays: what Petri admitted, reduced to the nodes and
//! edges the read side names.
//!
//! Petri lowers and admits every run's workflow at create time
//! (`RunSpec::admission` names the lowered graphs). The read side needs far
//! less than the lowered graph: the workflow's name and goal, each stage's
//! label and handler kind, and the routing edges as written. `RunGraph` is
//! that projection, built once from the admitted graph by `fabro-petri` and
//! stored on the run spec. The DOT the workflow was written in is beside it
//! as `RunSpec::graph_source`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::stage_handler::StageHandler;

/// The display graph of a run: the admitted workflow's name, goal, stages
/// and edges.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunGraph {
    /// The workflow's name: the DOT `digraph` name.
    pub name:  String,
    /// The run's goal as the run displays it.
    #[serde(default)]
    pub goal:  String,
    /// The stages by node id, in id order.
    #[serde(default)]
    pub nodes: BTreeMap<String, RunGraphNode>,
    /// The routing edges as written, one per arm.
    #[serde(default)]
    pub edges: Vec<RunGraphEdge>,
}

/// One stage of the display graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunGraphNode {
    /// The node's display label: its `label` attribute, else its id.
    pub label: String,
    /// The handler the node runs as.
    pub kind:  StageHandler,
}

/// One routing edge of the display graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunGraphEdge {
    pub from: String,
    pub to:   String,
}

impl RunGraph {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// The run's goal; empty when the workflow has none.
    #[must_use]
    pub fn goal(&self) -> &str {
        &self.goal
    }

    #[must_use]
    pub fn node(&self, id: &str) -> Option<&RunGraphNode> {
        self.nodes.get(id)
    }

    /// Whether `id` names a `start` or `exit` boundary: a node that runs no
    /// work of its own. The test is the node's kind, never its name.
    #[must_use]
    pub fn is_boundary(&self, id: &str) -> bool {
        self.node(id)
            .is_some_and(|node| matches!(node.kind, StageHandler::Start | StageHandler::Exit))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph() -> RunGraph {
        let mut graph = RunGraph::new("Ship");
        graph.goal = "Ship it".to_string();
        graph.nodes.insert("start".to_string(), RunGraphNode {
            label: "Start".to_string(),
            kind:  StageHandler::Start,
        });
        graph.nodes.insert("plan".to_string(), RunGraphNode {
            label: "Plan".to_string(),
            kind:  StageHandler::Agent,
        });
        graph.edges.push(RunGraphEdge {
            from: "start".to_string(),
            to:   "plan".to_string(),
        });
        graph
    }

    #[test]
    fn boundaries_are_by_kind_not_name() {
        let mut graph = graph();
        assert!(graph.is_boundary("start"));
        assert!(!graph.is_boundary("plan"));
        assert!(!graph.is_boundary("missing"));
        graph.nodes.get_mut("start").unwrap().kind = StageHandler::Agent;
        assert!(!graph.is_boundary("start"));
    }

    #[test]
    fn the_wire_shape_round_trips_and_defaults_the_optional_parts() {
        let graph = graph();
        let json = serde_json::to_value(&graph).unwrap();
        assert_eq!(json["nodes"]["plan"]["kind"], "agent");
        assert_eq!(json["edges"][0]["to"], "plan");
        let decoded: RunGraph = serde_json::from_value(json).unwrap();
        assert_eq!(decoded, graph);

        let bare: RunGraph = serde_json::from_value(serde_json::json!({ "name": "Bare" })).unwrap();
        assert_eq!(bare, RunGraph::new("Bare"));
    }
}
