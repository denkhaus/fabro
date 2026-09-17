//! Fork-only line-recovery presence tests (fabro-986b, fabro-0e11).
//!
//! PIN FILE: upstream does not have this file. A merge can never conflict
//! it away; if the fork seams disappear (fabro-8ee1 class), these tests go
//! red on the next build. Each test names the seed that motivated it.
//!
//! Covered fork features:
//! - fabro-986b decision (user 2026-09-14): usage-window rate limits park the
//!   run (SoftStop, resumable, quota signature intact) instead of burning stage
//!   retries into a deterministic goal-gate failure. The recovery cadence is a
//!   fixed 10-minute recheck in fabro-server — NEVER a backoff parsed from
//!   provider prose.
//! - fabro-5082: the run-event store sink offloads oversized payloads
//!   (checkpoint diffs, context values) to content-addressed blobs before the
//!   lossy truncation — 413 body-limit failures killed two 2h merge-upstream
//!   runs before this seam existed.
//! - fabro-0e11: the line graphs pin `stall_timeout` above the legal 60-minute
//!   fabro_run_wait so the default 1800s watchdog cannot kill a healthy pass
//!   mid-wait.
//! - fabro-9cb0: `fabro_workflow_version_create` rejects an entrypoint without
//!   a directory component — a bare `workflow.toml` collapses every run's
//!   workflow slug to the invisible fallback `workflow`, which blinded the
//!   revisor selector and backlog checks (run 01M2G8Q3SEN4 listed 0 runs while
//!   terminal runs existed). Placeholder-shaped paths (`<slug>/…`, verbatim
//!   copies from documentation) are rejected for the same reason.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use fabro_core::Graph as _;
use fabro_llm::{ErrorData, ErrorKind, RetryClassification};
use fabro_types::failure_signature::FailureSignature;
use fabro_types::outcome::FailureCategory;
use fabro_types::{FailureReason, RunStatus};

use crate::error::Error;
use crate::fork_line_recovery::{failure_detail_parks, parks_for_usage_window};
use crate::graph::WorkflowGraph;
use crate::outcome::{FailureDetail, Outcome, StageOutcome};

/// The exact zai 5-hour hard-cut message from run 01M2E7VZYX8V (naive,
/// offset-less reset wallclock — zai sends UTC without saying so).
const ZAI_HARD_CUT: &str =
    "provider zai Usage limit reached for 5 hour. Your limit will reset at 2026-09-14 05:31:20";

fn zai_rate_limit() -> ErrorData {
    ErrorData::from(
        fabro_llm::Error::new(ErrorKind::RateLimit, ZAI_HARD_CUT)
            .with_provider(lithos_llm::catalog::builtin::openai())
            .with_retry(RetryClassification::Safe),
    )
}

// --- fabro-986b: park classification ---

#[test]
fn usage_window_rate_limit_is_not_retryable() {
    let err = Error::Llm(Box::new(zai_rate_limit()));
    assert!(
        parks_for_usage_window(&zai_rate_limit()),
        "naive-prose usage window must classify as a park"
    );
    assert!(
        !err.is_retryable(),
        "stage retries cannot bridge a closed usage window (fabro-183f)"
    );
    assert_eq!(err.failure_reason(), FailureReason::SoftStop);
    assert_eq!(err.failure_category(), FailureCategory::TransientInfra);
}

#[test]
fn short_rate_limit_window_keeps_retry_behavior() {
    let short = ErrorData::from(
        fabro_llm::Error::new(ErrorKind::RateLimit, "429 too many requests")
            .with_provider(lithos_llm::catalog::builtin::openai())
            .with_retry(RetryClassification::Safe)
            .with_provider_retry_after(Duration::from_secs(20)),
    );
    assert!(!parks_for_usage_window(&short));
    assert!(Error::Llm(Box::new(short)).is_retryable());
}

#[test]
fn parked_run_failure_carries_quota_signature_and_soft_stop() {
    let err = Error::Llm(Box::new(zai_rate_limit()));
    let failure = err.to_failure_detail();
    assert!(failure_detail_parks(&failure));
    assert_eq!(
        failure
            .signature
            .as_ref()
            .map(fabro_types::FailureSignature::as_str),
        Some("api_transient|openai|rate_limit")
    );

    // End-to-end mapping: a failed outcome with a park-class failure maps
    // to a resumable SoftStop run, never a hard workflow_error.
    let mut outcome = Outcome::fail("llm");
    outcome.failure = Some(failure);
    let (stage, run_failure, status) = crate::pipeline::classify_engine_result(&Ok(outcome));
    assert!(matches!(stage, StageOutcome::Failed { .. }));
    assert_eq!(status, RunStatus::Failed {
        reason: FailureReason::SoftStop,
    });
    let run_failure = run_failure.expect("parked run keeps its failure");
    assert_eq!(run_failure.reason, FailureReason::SoftStop);
}

// --- fabro-986b: the no-edge park seam in WorkflowGraph::select_edge ---

#[test]
fn parked_failure_selects_no_edge_even_with_matching_condition() {
    let mut recovery = fabro_graphviz::graph::types::Edge::new("work", "recover");
    recovery.attrs.insert(
        "condition".to_string(),
        fabro_graphviz::graph::types::AttrValue::String("outcome=failed".to_string()),
    );
    let mut graph = fabro_graphviz::graph::types::Graph::new("test");
    graph.nodes.insert(
        "work".to_string(),
        fabro_graphviz::graph::types::Node::new("work"),
    );
    graph.nodes.insert(
        "recover".to_string(),
        fabro_graphviz::graph::types::Node::new("recover"),
    );
    graph.edges = vec![recovery];
    let workflow_graph = WorkflowGraph(Arc::new(graph));

    let node = workflow_graph.get_node("work").expect("work node exists");

    let mut detail = FailureDetail::new(
        format!("LLM error: {ZAI_HARD_CUT}"),
        FailureCategory::TransientInfra,
    );
    detail.signature = Some(FailureSignature("api_transient|zai|rate_limit".to_string()));
    let mut outcome = Outcome::fail("llm");
    outcome.failure = Some(detail);

    assert!(
        workflow_graph
            .select_edge(&node, &outcome, &crate::context::Context::new())
            .is_none(),
        "usage-window parks route to no edge — the run ends resumable, \
         it does not feed the failure loop (fabro-986b)"
    );

    // Control: an ordinary transient failure without a reset announcement
    // still routes (the seam must not swallow ordinary failures).
    let mut ordinary = Outcome::fail("sandbox hiccup");
    ordinary.failure = Some(FailureDetail::new(
        "sandbox hiccup",
        FailureCategory::TransientInfra,
    ));
    assert!(
        workflow_graph
            .select_edge(&node, &ordinary, &crate::context::Context::new())
            .is_some(),
        "ordinary transient failures keep their routing"
    );
}

// --- fabro-986b: the executor end-path park preservation seam ---

#[test]
fn failure_parks_run_hook_is_wired_to_the_park_classification() {
    let mut graph = fabro_graphviz::graph::types::Graph::new("test");
    graph.nodes.insert(
        "work".to_string(),
        fabro_graphviz::graph::types::Node::new("work"),
    );
    let workflow_graph = WorkflowGraph(Arc::new(graph));

    let mut detail = FailureDetail::new(
        format!("LLM error: {ZAI_HARD_CUT}"),
        FailureCategory::TransientInfra,
    );
    detail.signature = Some(FailureSignature("api_transient|zai|rate_limit".to_string()));
    let mut parked = Outcome::fail("llm");
    parked.failure = Some(detail);

    assert!(
        workflow_graph.failure_parks_run(&parked),
        "the executor seam must see the same park classification as select_edge"
    );

    let mut ordinary = Outcome::fail("sandbox hiccup");
    ordinary.failure = Some(FailureDetail::new(
        "sandbox hiccup",
        FailureCategory::TransientInfra,
    ));
    assert!(
        !workflow_graph.failure_parks_run(&ordinary),
        "ordinary failures must not park"
    );
}

/// A stage handler failing with a fixed park-class detail, mirroring how
/// LLM-stage failures surface: a failed outcome, not an engine error.
struct ParkFailHandler(FailureDetail);

#[async_trait::async_trait]
impl fabro_core::NodeHandler<WorkflowGraph> for ParkFailHandler {
    async fn execute(
        &self,
        _node: &crate::graph::WorkflowNode,
        _context: &crate::context::Context,
        _graph: &WorkflowGraph,
        _attempt: &fabro_core::handler::AttemptInfo,
    ) -> fabro_core::Result<Outcome> {
        let mut outcome = Outcome::fail("stage failed");
        outcome.failure = Some(self.0.clone());
        Ok(outcome)
    }
}

/// Drives the full chain that run 01M2J1BA2JC4S6SA10YEJHHMVP broke: a
/// park-class stage failure on a route-policy node without a fail edge
/// must end the run SoftStop with the quota signature — the executor's
/// end path may not rewrite the failure detail into a routing diagnostic.
#[tokio::test]
async fn parked_stage_failure_ends_soft_stop_with_quota_signature() {
    use fabro_core::{ExecutionState, ExecutorBuilder};

    // Node "start" resolves as the start node by id; no outgoing edges.
    let mut graph = fabro_graphviz::graph::types::Graph::new("test");
    graph.nodes.insert(
        "start".to_string(),
        fabro_graphviz::graph::types::Node::new("start"),
    );
    let workflow_graph = WorkflowGraph(Arc::new(graph));

    let mut detail = FailureDetail::new(
        format!("LLM error: {ZAI_HARD_CUT}"),
        FailureCategory::TransientInfra,
    );
    detail.signature = Some(FailureSignature("api_transient|zai|rate_limit".to_string()));

    let state = ExecutionState::new(&workflow_graph).expect("start node resolves");
    let executor = ExecutorBuilder::new(Arc::new(ParkFailHandler(detail))).build();
    let (outcome, _state) = executor
        .run(&workflow_graph, state)
        .await
        .expect("the parked run ends with a failed outcome, not an engine error");

    assert!(matches!(outcome.status, StageOutcome::Failed { .. }));
    let failure = outcome
        .failure
        .as_ref()
        .expect("the end path must not drop the failure detail");
    assert!(
        failure.message.contains("will reset at"),
        "reset-window prose must survive the executor end path: {}",
        failure.message
    );
    assert!(
        !failure.message.contains("no outgoing fail edge"),
        "the routing diagnostic rewrite must not mask the park class"
    );

    // Terminal classification: the preserved prose parks the run as a
    // resumable SoftStop carrying the quota signature.
    let (stage, run_failure, status) = crate::pipeline::classify_engine_result(&Ok(outcome));
    assert!(matches!(stage, StageOutcome::Failed { .. }));
    assert_eq!(status, RunStatus::Failed {
        reason: FailureReason::SoftStop,
    });
    let run_failure = run_failure.expect("parked run keeps its failure");
    assert_eq!(run_failure.reason, FailureReason::SoftStop);
    assert_eq!(
        run_failure
            .detail
            .signature
            .as_ref()
            .map(fabro_types::FailureSignature::as_str),
        Some("api_transient|zai|rate_limit")
    );
}

// --- fabro-0e11: line graphs pin stall_timeout above the legal wait ---

#[test]
fn line_graphs_pin_stall_timeout_above_the_legal_wait() {
    let root = repo_root();
    for workflow in ["conductor", "develop", "merge-upstream"] {
        let path = root
            .join(".fabro/workflows")
            .join(workflow)
            .join("workflow.fabro");
        #[expect(
            clippy::disallowed_methods,
            reason = "presence pin reads loop assets synchronously in a unit test"
        )]
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        assert!(
            text.contains("stall_timeout=\"63m\""),
            "{workflow} must pin stall_timeout=63m (fabro-0e11): \
             the default 1800s watchdog kills legal 60m fabro_run_wait passes"
        );
    }
}

// --- fabro-9cb0: bare entrypoints collapse run slugs ---

#[test]
fn workflow_version_tool_rejects_entrypoint_without_directory() {
    use std::collections::BTreeMap;

    use fabro_tool::{FabroWorkflowVersionCreateParams, ValidatedWorkflowVersionCreate};

    fn validated(entrypoint: &str) -> Result<ValidatedWorkflowVersionCreate, String> {
        let files = BTreeMap::from([(
            entrypoint.parse().expect("test entrypoint parses"),
            "digraph W { start [shape=Mdiamond] exit [shape=Msquare] start -> exit }".to_string(),
        )]);
        ValidatedWorkflowVersionCreate::try_from(FabroWorkflowVersionCreateParams {
            entrypoint: entrypoint.parse().expect("test entrypoint parses"),
            files,
            files_from: None,
        })
        .map_err(|err| err.as_str().to_string())
    }

    for bare in ["workflow.toml", "workflow.fabro", "workflow"] {
        let error = validated(bare).expect_err("a bare entrypoint must be rejected");
        assert!(
            error.contains("no directory component"),
            "the rejection must teach the fabro-9cb0 cause, got: {error}"
        );
    }
    for prefixed in ["develop/workflow.toml", "demo/workflow"] {
        validated(prefixed).unwrap_or_else(|error| {
            panic!("dir-prefixed entrypoint must pass: {error}");
        });
    }
    // Documentation teaches `<slug>/…` placeholders; a verbatim copy must
    // fail loudly instead of registering runs under the literal slug `<slug>`.
    let error =
        validated("<slug>/workflow.toml").expect_err("a placeholder entrypoint must be rejected");
    assert!(error.contains("placeholder"), "got: {error}");
}

// --- fabro-5082: oversized-event blob offload seam ---

#[test]
fn run_event_sink_keeps_the_blob_offload_seam() {
    // The offload module is fork-only, but its seam call lives in the
    // shared `event/sink.rs`: a merge that rewrites the sink can drop the
    // call together with its inline tests (the #832 incident class). This
    // pin goes red when the lossless path is no longer wired in.
    let sink = repo_root().join("lib/components/fabro-workflow/src/event/sink.rs");
    #[expect(
        clippy::disallowed_methods,
        reason = "presence pin reads shared sink source synchronously"
    )]
    let text = std::fs::read_to_string(&sink)
        .unwrap_or_else(|error| panic!("read {}: {error}", sink.display()));
    assert!(
        text.contains("offload_oversized_run_event(event, run_store)"),
        "the store sink must offload oversized event payloads to blobs \
         before lossy truncation (fabro-5082)"
    );
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("manifest lives at lib/components/fabro-workflow")
        .to_path_buf()
}
