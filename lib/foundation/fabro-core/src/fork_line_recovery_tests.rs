//! Fork-only executor end-path park preservation tests (fabro-986b).
//!
//! PIN FILE: upstream does not have this file. A merge can never conflict
//! it away; if the fork seams disappear (fabro-8ee1 class), these tests go
//! red on the next build. Each test names the seed that motivated it.
//!
//! Covered fork feature:
//! - fabro-986b LIVE-EVIDENCE GAP (run 01M2J1BA2JC4S6SA10YEJHHMVP): the
//!   executor's `NextStep::End` path rewrote every failed outcome into a "no
//!   outgoing fail edge" diagnostic, destroying the reset-window prose
//!   `pipeline::finalize` keys on — park-class runs surfaced as deterministic
//!   workflow errors, so neither the breaker exemption nor the recheck
//!   classifier ever saw them. The `Graph::failure_parks_run` seam preserves
//!   the original failure detail for park-class outcomes.

use std::collections::HashMap;
use std::result::Result as StdResult;
use std::sync::Arc;

use async_trait::async_trait;
use fabro_types::{OnFailure, ResolvedOnFailure};

use crate::context::Context;
use crate::error::Result;
use crate::executor::ExecutorBuilder;
use crate::graph::{EdgeSelection, Graph};
use crate::handler::{AttemptInfo, NodeHandler};
use crate::outcome::{FailureCategory, FailureDetail, Outcome, StageOutcome};
use crate::state::ExecutionState;
use crate::test_fixtures::{TestEdge, TestGraph, TestNode};

/// The zai 5-hour hard-cut shape from run 01M2E7VZYX8V: naive,
/// offset-less reset wallclock announcing a multi-hour usage window.
const ZAI_HARD_CUT: &str =
    "provider zai Usage limit reached for 5 hour. Your limit will reset at 2026-09-14 05:31:20";

/// A graph that parks exactly the outcomes whose failure detail carries
/// the usage-window prose — the test double for the workflow-side
/// `fork_line_recovery::failure_detail_parks` classification.
struct ParkingGraph {
    inner: TestGraph,
}

impl ParkingGraph {
    fn route_policy(node: &str) -> Self {
        Self {
            inner: TestGraph::new(vec![TestNode::new(node)], Vec::<TestEdge>::new(), node),
        }
    }

    fn exit_policy(node: &str) -> Self {
        Self {
            inner: TestGraph::new(
                vec![TestNode::new(node).with_on_failure(OnFailure::Exit)],
                Vec::<TestEdge>::new(),
                node,
            ),
        }
    }
}

impl Graph for ParkingGraph {
    type Node = TestNode;
    type Edge = TestEdge;
    type Meta = ();

    fn get_node(&self, id: &str) -> Option<Self::Node> {
        <TestGraph as Graph>::get_node(&self.inner, id)
    }

    fn find_start_node(&self) -> Result<Self::Node> {
        <TestGraph as Graph>::find_start_node(&self.inner)
    }

    fn outgoing_edges(&self, node_id: &str) -> Vec<Self::Edge> {
        <TestGraph as Graph>::outgoing_edges(&self.inner, node_id)
    }

    fn select_edge(
        &self,
        node: &Self::Node,
        outcome: &Outcome,
        context: &Context,
    ) -> Option<EdgeSelection<Self>> {
        <TestGraph as Graph>::select_edge(&self.inner, node, outcome, context).map(|selection| {
            EdgeSelection {
                edge:   selection.edge,
                reason: selection.reason,
            }
        })
    }

    fn check_goal_gates(&self, outcomes: &HashMap<String, Outcome>) -> StdResult<(), String> {
        <TestGraph as Graph>::check_goal_gates(&self.inner, outcomes)
    }

    fn get_retry_target(&self, failed_node_id: &str) -> Option<String> {
        <TestGraph as Graph>::get_retry_target(&self.inner, failed_node_id)
    }

    fn resolve_on_failure(&self, node: &Self::Node) -> ResolvedOnFailure {
        <TestGraph as Graph>::resolve_on_failure(&self.inner, node)
    }

    fn failure_parks_run(&self, outcome: &Outcome) -> bool {
        outcome
            .failure
            .as_ref()
            .is_some_and(|failure| failure.message.contains("will reset at"))
    }
}

/// A stage handler that fails with the given failure detail, mirroring how
/// LLM-stage failures surface: a failed outcome, not an engine error.
struct FailingDetailHandler(FailureDetail);

#[async_trait]
impl NodeHandler<ParkingGraph> for FailingDetailHandler {
    async fn execute(
        &self,
        _node: &TestNode,
        _context: &Context,
        _graph: &ParkingGraph,
        _attempt: &AttemptInfo,
    ) -> Result<Outcome> {
        let mut outcome = Outcome::fail("stage handler failed");
        outcome.failure = Some(self.0.clone());
        Ok(outcome)
    }
}

async fn run_graph(graph: &ParkingGraph, detail: FailureDetail) -> Outcome {
    let state = ExecutionState::new(graph).expect("start node resolves");
    let executor = ExecutorBuilder::new(Arc::new(FailingDetailHandler(detail))).build();
    executor
        .run(graph, state)
        .await
        .expect("executor returns the failed outcome, not an engine error")
        .0
}

fn zai_park_detail() -> FailureDetail {
    FailureDetail::new(
        format!("LLM error: {ZAI_HARD_CUT}"),
        FailureCategory::TransientInfra,
    )
}

#[tokio::test]
async fn park_class_failure_keeps_original_detail_at_the_end() {
    let outcome = run_graph(&ParkingGraph::route_policy("work"), zai_park_detail()).await;

    assert!(
        matches!(outcome.status, StageOutcome::Failed { .. }),
        "the parked run stays failed: {outcome:?}"
    );
    let failure = outcome.failure.expect("park detail survives the end path");
    assert!(
        failure.message.contains("will reset at"),
        "reset-window prose must reach the terminal outcome: {}",
        failure.message
    );
    assert_eq!(failure.category, FailureCategory::TransientInfra);
}

#[tokio::test]
async fn ordinary_failure_still_gets_the_routing_diagnostic() {
    let detail = FailureDetail::new("sandbox hiccup", FailureCategory::TransientInfra);
    let outcome = run_graph(&ParkingGraph::route_policy("work"), detail).await;

    let failure = outcome.failure.expect("ordinary failure keeps a detail");
    assert_eq!(
        failure.message, "stage work failed with no outgoing fail edge",
        "upstream rewrite behavior is unchanged for non-park failures"
    );
}

#[tokio::test]
async fn exit_policy_park_is_also_preserved() {
    let outcome = run_graph(&ParkingGraph::exit_policy("work"), zai_park_detail()).await;

    let failure = outcome.failure.expect("park detail survives the end path");
    assert!(
        failure.message.contains("will reset at"),
        "on_failure=exit must not rewrite park-class prose either: {}",
        failure.message
    );
}
