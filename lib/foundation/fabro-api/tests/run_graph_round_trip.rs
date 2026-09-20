use std::any::{TypeId, type_name};

use fabro_api::types::{
    RunGraph as ApiRunGraph, RunGraphEdge as ApiRunGraphEdge, RunGraphNode as ApiRunGraphNode,
};
use fabro_types::{RunGraph, RunGraphEdge, RunGraphNode, StageHandler};
use serde_json::json;

#[test]
fn the_run_graph_reuses_canonical_types() {
    assert_same_type::<ApiRunGraph, RunGraph>();
    assert_same_type::<ApiRunGraphNode, RunGraphNode>();
    assert_same_type::<ApiRunGraphEdge, RunGraphEdge>();
}

#[test]
fn the_run_graph_round_trips_the_schema_shape() {
    let value = json!({
        "name": "Ship",
        "goal": "Ship the feature",
        "nodes": {
            "plan": { "label": "Plan rollout", "kind": "agent" },
            "start": { "label": "Start", "kind": "start" }
        },
        "edges": [
            { "from": "start", "to": "plan" }
        ]
    });
    let graph: RunGraph = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(graph.name, "Ship");
    assert_eq!(graph.goal(), "Ship the feature");
    assert_eq!(
        graph.node("plan").map(|node| node.kind),
        Some(StageHandler::Agent)
    );
    assert_eq!(graph.edges.len(), 1);
    assert_eq!(serde_json::to_value(&graph).unwrap(), value);
}

#[test]
fn a_graph_with_only_a_name_is_the_schema_minimum() {
    let graph: RunGraph = serde_json::from_value(json!({ "name": "Bare" })).unwrap();
    assert_eq!(graph, RunGraph::new("Bare"));
}

fn assert_same_type<T: 'static, U: 'static>() {
    assert_eq!(
        TypeId::of::<T>(),
        TypeId::of::<U>(),
        "{} should be the same type as {}",
        type_name::<T>(),
        type_name::<U>()
    );
}
