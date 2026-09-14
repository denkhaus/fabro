//! fabro-183f: stage retries (and resumed executions) of agent stages
//! continue the prior session with a minimal continuation message instead
//! of re-sending the full stage prompt + preamble per attempt.
//!
//! Table-driven over the three behaviors the seed demands: an executor
//! retry of an agent stage asks the backend to continue (not re-post), a
//! resumed stage execution carries continuation semantics, and non-agent
//! (one-shot prompt) retries keep the full re-send.

use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use fabro_graphviz::graph::{AttrValue, Edge, Graph, Node};
use fabro_types::{RunId, StageTiming, WorkflowSettings};
use tokio_util::sync::CancellationToken;

use crate::context::{Context, keys};
use crate::error::Error;
use crate::event::Emitter;
use crate::handler::agent::{
    AgentHandler, CodergenBackend, CodergenResult, CodergenRunRequest, OneShotRequest,
    RetryContinuation, continuation_message,
};
use crate::handler::exit::ExitHandler;
use crate::handler::start::StartHandler;
use crate::handler::{Handler as HandlerTrait, HandlerRegistry};
use crate::outcome::{Outcome, StageOutcome};
use crate::run_options::RunOptions;
use crate::test_support::WorkflowRunner;

/// One request as the backend saw it.
#[derive(Debug, Clone)]
struct RecordedPrompt {
    prompt:             String,
    retry_continuation: Option<RetryContinuation>,
}

impl RecordedPrompt {
    /// The message the backend contract sends for this request: the
    /// continuation when the request carries one, the prompt otherwise.
    fn effective_message(&self) -> String {
        self.retry_continuation
            .as_ref()
            .map_or_else(|| self.prompt.clone(), continuation_message)
    }
}

/// A backend that records every request and can fail the first N calls
/// with a retryable error.
struct RecordingBackend {
    calls:         Arc<Mutex<Vec<RecordedPrompt>>>,
    fail_first_n:  usize,
    error_message: String,
    one_shot:      bool,
}

impl RecordingBackend {
    fn new(
        calls: Arc<Mutex<Vec<RecordedPrompt>>>,
        fail_first_n: usize,
        error_message: &str,
        one_shot: bool,
    ) -> Self {
        Self {
            calls,
            fail_first_n,
            error_message: error_message.to_string(),
            one_shot,
        }
    }

    fn record(&self, prompt: &str, retry_continuation: Option<RetryContinuation>) {
        self.calls.lock().unwrap().push(RecordedPrompt {
            prompt: prompt.to_string(),
            retry_continuation,
        });
    }

    /// `Err` while within the fail budget, a plain success after.
    fn result(&self) -> Result<CodergenResult, Error> {
        if self.calls.lock().unwrap().len() <= self.fail_first_n {
            return Err(Error::handler(self.error_message.clone()));
        }
        Ok(CodergenResult::Text {
            text:              "done".to_string(),
            usage:             None,
            usage_by_model:    Vec::new(),
            files_touched:     Vec::new(),
            last_file_touched: None,
            timing:            StageTiming::default(),
        })
    }
}

#[async_trait]
impl CodergenBackend for RecordingBackend {
    async fn run(&self, request: CodergenRunRequest<'_>) -> Result<CodergenResult, Error> {
        self.record(request.prompt, request.retry_continuation.clone());
        self.result()
    }

    async fn one_shot(&self, request: OneShotRequest<'_>) -> Result<CodergenResult, Error> {
        assert!(
            self.one_shot,
            "one_shot called on an agent-mode backend (or vice versa)"
        );
        self.record(request.prompt, None);
        self.result()
    }
}

/// `start -> work -> exit`, where `work` carries `node_type` and
/// `max_retries = 1` so the executor retries exactly once.
fn retry_graph(name: &str, node_type: &str, prompt: &str) -> Graph {
    let mut graph = Graph::new(name);
    let mut start = Node::new("start");
    start.attrs.insert(
        "shape".to_string(),
        AttrValue::String("Mdiamond".to_string()),
    );
    graph.nodes.insert("start".to_string(), start);
    let mut work = Node::new("work");
    work.attrs
        .insert("type".to_string(), AttrValue::String(node_type.to_string()));
    work.attrs
        .insert("prompt".to_string(), AttrValue::String(prompt.to_string()));
    work.attrs.insert(
        "retry_policy".to_string(),
        AttrValue::String("linear".to_string()),
    );
    graph.nodes.insert("work".to_string(), work);
    let mut exit = Node::new("exit");
    exit.attrs.insert(
        "shape".to_string(),
        AttrValue::String("Msquare".to_string()),
    );
    graph.nodes.insert("exit".to_string(), exit);
    graph.edges.push(Edge::new("start", "work"));
    graph.edges.push(Edge::new("work", "exit"));
    graph
}

fn test_run_options(run_dir: &Path) -> RunOptions {
    RunOptions {
        settings:         WorkflowSettings::default(),
        run_dir:          run_dir.to_path_buf(),
        cancel_token:     CancellationToken::new(),
        run_id:           RunId::new(),
        labels:           std::collections::HashMap::new(),
        workflow_slug:    None,
        github_app:       None,
        base_branch:      None,
        display_base_sha: None,
        git_identity:     None,
        pre_run_git:      None,
        fork_source_ref:  None,
        git:              None,
    }
}

async fn run_with_backend(backend: Box<dyn CodergenBackend>, graph: &Graph) -> Outcome {
    let mut registry = HandlerRegistry::new(Box::new(AgentHandler::new(Some(backend))));
    registry.register("start", Box::new(StartHandler));
    registry.register("exit", Box::new(ExitHandler));
    let dir = tempfile::tempdir().unwrap();
    let sandbox = Arc::new(
        fabro_sandbox::local_sandbox(dir.path().to_path_buf())
            .await
            .expect("local sandbox should be created"),
    );
    let runner = WorkflowRunner::new(registry, Arc::new(Emitter::default()), sandbox);
    runner
        .run_with_state(graph, &test_run_options(dir.path()))
        .await
        .expect("workflow execution should complete")
        .0
}

// --- Engine-level table ------------------------------------------------------

/// Rows driven through the real executor retry loop.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn engine_retry_table() {
    struct Row {
        name:      &'static str,
        node_type: &'static str,
    }
    let rows = [
        Row {
            name:      "agent retry continues the session",
            node_type: "agent",
        },
        Row {
            name:      "prompt retry keeps the full re-send",
            node_type: "prompt",
        },
    ];

    for row in rows {
        let calls: Arc<Mutex<Vec<RecordedPrompt>>> = Arc::default();
        let one_shot = row.node_type == "prompt";
        // The prompt row needs the backend on the prompt handler; the agent
        // row on the default handler. One registry build per row below.
        let backend = RecordingBackend::new(Arc::clone(&calls), 1, "twin exploded", one_shot);

        let graph = retry_graph("RetryContinuation", row.node_type, "Do the important task");
        let outcome = if one_shot {
            run_prompt_with_backend(Box::new(backend), &graph).await
        } else {
            run_with_backend(Box::new(backend), &graph).await
        };

        assert_eq!(
            outcome.status,
            StageOutcome::Succeeded,
            "{}: {outcome:?}",
            row.name
        );
        let calls = calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 2, "{}: two attempts", row.name);

        if row.node_type == "agent" {
            {
                // Attempt 1: full prompt, no continuation.
                assert!(
                    calls[0].retry_continuation.is_none(),
                    "{}: attempt 1 must not carry a continuation",
                    row.name
                );
                assert!(
                    calls[0].prompt.contains("Do the important task"),
                    "{}: attempt 1 sends the stage prompt",
                    row.name
                );
                // Attempt 2: continuation marker naming the failure, and an
                // effective message that is NOT the full prompt re-post.
                let continuation = calls[1]
                    .retry_continuation
                    .as_ref()
                    .unwrap_or_else(|| panic!("{}: attempt 2 carries a continuation", row.name));
                assert_eq!(continuation.failed_attempt, Some(1), "{}", row.name);
                assert!(
                    continuation.failure.contains("twin exploded"),
                    "{}: failure summary carried, got {}",
                    row.name,
                    continuation.failure
                );
                let effective = calls[1].effective_message();
                assert!(
                    !effective.contains("Do the important task"),
                    "{}: attempt 2 must not re-send the stage prompt, got {effective}",
                    row.name
                );
                assert!(
                    effective.contains("Attempt 1 of this stage failed with: twin exploded"),
                    "{}: continuation names the failed attempt, got {effective}",
                    row.name
                );
                assert!(
                    effective.contains("Continue from your last state"),
                    "{}: continuation tells the session to continue, got {effective}",
                    row.name
                );
            }
        } else {
            // Non-agent stage: both attempts re-send the identical full
            // prompt; no continuation concept exists on one-shot.
            for (index, call) in calls.iter().enumerate() {
                assert!(
                    call.prompt.contains("Do the important task"),
                    "{}: attempt {} re-sends the full prompt",
                    row.name,
                    index + 1
                );
                assert!(
                    call.retry_continuation.is_none(),
                    "{}: one-shot requests never carry a continuation",
                    row.name
                );
            }
            assert_eq!(
                calls[0].prompt, calls[1].prompt,
                "{}: unchanged retry behavior for non-agent stages",
                row.name
            );
        }
    }
}

/// The prompt row installs its recording backend on the `prompt` handler.
async fn run_prompt_with_backend(backend: Box<dyn CodergenBackend>, graph: &Graph) -> Outcome {
    let mut registry = HandlerRegistry::new(Box::new(AgentHandler::new(None)));
    registry.register("start", Box::new(StartHandler));
    registry.register("exit", Box::new(ExitHandler));
    registry.register(
        "prompt",
        Box::new(crate::handler::prompt::PromptHandler::new(Some(backend))),
    );
    let dir = tempfile::tempdir().unwrap();
    let sandbox = Arc::new(
        fabro_sandbox::local_sandbox(dir.path().to_path_buf())
            .await
            .expect("local sandbox should be created"),
    );
    let runner = WorkflowRunner::new(registry, Arc::new(Emitter::default()), sandbox);
    runner
        .run_with_state(graph, &test_run_options(dir.path()))
        .await
        .expect("workflow execution should complete")
        .0
}

// --- Handler-level resume table ---------------------------------------------

#[tokio::test]
async fn resumed_execution_continuation_table() {
    struct Row {
        name:                &'static str,
        resumed_from:        Option<&'static str>,
        expect_continuation: bool,
    }
    let rows = [
        Row {
            name:                "fresh execution posts the full prompt",
            resumed_from:        None,
            expect_continuation: false,
        },
        Row {
            name:                "resumed execution carries continuation semantics",
            resumed_from:        Some("work@1"),
            expect_continuation: true,
        },
    ];

    for row in rows {
        let calls: Arc<Mutex<Vec<RecordedPrompt>>> = Arc::default();
        let backend = RecordingBackend::new(Arc::clone(&calls), 0, "unused", false);
        let handler = AgentHandler::new(Some(Box::new(backend)));

        let mut node = Node::new("work");
        node.attrs.insert(
            "prompt".to_string(),
            AttrValue::String("Do the important task".to_string()),
        );
        let context = Context::new();
        context.set(
            keys::INTERNAL_RUN_ID,
            serde_json::json!(fabro_types::fixtures::RUN_1.to_string()),
        );
        if let Some(prior) = row.resumed_from {
            context.set(keys::INTERNAL_STAGE_RESUMED_FROM, serde_json::json!(prior));
        }
        let graph = Graph::new("test");
        let tmp = tempfile::tempdir().unwrap();

        HandlerTrait::execute(
            &handler,
            &node,
            &context,
            &graph,
            tmp.path(),
            &crate::services::EngineServices::test_default(),
            &super::AttemptInfo::first(),
        )
        .await
        .unwrap_or_else(|e| panic!("{}: {e}", row.name));

        let calls = calls.lock().unwrap();
        let call = calls
            .first()
            .unwrap_or_else(|| panic!("{}: backend was called", row.name));
        assert!(
            call.prompt.contains("Do the important task"),
            "{}: the full prompt still travels with the request",
            row.name
        );
        match (row.expect_continuation, &call.retry_continuation) {
            (true, Some(continuation)) => {
                assert_eq!(continuation.failed_attempt, None, "{}", row.name);
                assert!(
                    continuation.failure.contains("work@1"),
                    "{}: names the superseded execution, got {}",
                    row.name,
                    continuation.failure
                );
                let effective = call.effective_message();
                assert!(
                    !effective.contains("Do the important task"),
                    "{}: the effective message is a continuation, not the prompt",
                    row.name
                );
            }
            (false, None) => {}
            other => panic!("{}: unexpected continuation {other:?}", row.name),
        }
    }
}

#[test]
fn continuation_message_shapes() {
    let retry = RetryContinuation {
        failed_attempt: Some(2),
        failure:        "gateway 502".to_string(),
    };
    let message = continuation_message(&retry);
    assert!(message.contains("Attempt 2 of this stage failed with: gateway 502"));
    assert!(message.contains("Continue from your last state"));

    let resumed = RetryContinuation {
        failed_attempt: None,
        failure:        "execution work@1 ended without completing".to_string(),
    };
    let message = continuation_message(&resumed);
    assert!(message.contains("was interrupted: execution work@1"));
    assert!(message.contains("original task is unchanged"));
}
