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
//! - fabro-0e11: the line graphs pin `stall_timeout` above the legal 60-minute
//!   fabro_run_wait so the default 1800s watchdog cannot kill a healthy pass
//!   mid-wait.

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

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("manifest lives at lib/components/fabro-workflow")
        .to_path_buf()
}
