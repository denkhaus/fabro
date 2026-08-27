use std::sync::Arc;

use async_trait::async_trait;
use fabro_core::NodeSpec;
use fabro_core::error::Result as CoreResult;
use fabro_core::lifecycle::RunLifecycle;
use fabro_core::outcome::NodeResult;
use fabro_core::state::ExecutionState;
use fabro_types::{RunNoticeCode, RunNoticeLevel};

use super::event::stage_scope_for;
use crate::context;
use crate::event::Emitter;
use crate::graph::{WorkflowGraph, WorkflowNode};
use crate::outcome::BilledModelUsage;
use crate::stage_execution::StageExecutionTracker;

type WfRunState = ExecutionState<Option<BilledModelUsage>>;
type WfNodeResult = NodeResult<Option<BilledModelUsage>>;

/// Stage envelope lifecycle (fabro-900e, ADR-0009 family): enforces the
/// per-node context key ownership (`context_allow_keys`) and append-only
/// accumulation (`context_append_keys`) on every completed node result,
/// before anything downstream observes the updates.
///
/// Runs first in `WorkflowLifecycle::after_node`, so the completion event,
/// stage hooks, checkpoint, and `state.record` all see the admitted
/// updates only. Dropped keys are reported with a `ContextUpdateDropped`
/// warning notice — drops are drift protection, never silent and never a
/// stage failure.
pub(crate) struct EnvelopeLifecycle {
    emitter:          Arc<Emitter>,
    stage_executions: StageExecutionTracker,
}

impl EnvelopeLifecycle {
    pub(crate) fn new(emitter: &Arc<Emitter>, stage_executions: StageExecutionTracker) -> Self {
        Self {
            emitter: Arc::clone(emitter),
            stage_executions,
        }
    }
}

#[async_trait]
impl RunLifecycle<WorkflowGraph> for EnvelopeLifecycle {
    async fn after_node(
        &self,
        node: &WorkflowNode,
        result: &mut WfNodeResult,
        state: &WfRunState,
    ) -> CoreResult<()> {
        let dropped = context::enforce_stage_envelope(
            node.inner(),
            &state.context,
            &mut result.outcome.context_updates,
        );
        if dropped.is_empty() {
            return Ok(());
        }
        self.emitter.notice_scoped(
            RunNoticeLevel::Warn,
            RunNoticeCode::ContextUpdateDropped,
            format!("context_allow_keys dropped: {}", dropped.join(", ")),
            &stage_scope_for(&self.stage_executions, state, node.id()),
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use fabro_core::context::Context;
    use fabro_core::outcome::Outcome;
    use fabro_graphviz::graph::{AttrValue, Node as GvNode};
    use fabro_types::{EventBody, RunEvent, RunId, RunNoticeLevel};

    use super::*;

    fn envelope_node(attrs: &[(&str, &str)]) -> WorkflowNode {
        let mut node = GvNode::new("planner");
        for (key, value) in attrs {
            node.attrs
                .insert(key.to_string(), AttrValue::String(value.to_string()));
        }
        WorkflowNode(Arc::new(node))
    }

    fn node_result(updates: &[(&str, serde_json::Value)]) -> WfNodeResult {
        let mut outcome = Outcome::success();
        for (key, value) in updates {
            outcome
                .context_updates
                .insert(key.to_string(), value.clone());
        }
        NodeResult::new(
            outcome,
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
            1,
            1,
        )
    }

    fn run_state() -> WfRunState {
        // Built literally: `ExecutionState::new` needs a graph with a start
        // node, and the envelope only reads `state.context`.
        ExecutionState {
            context:          Context::new(),
            current_node_id:  "planner".to_string(),
            completed_nodes:  Vec::new(),
            node_outcomes:    std::collections::HashMap::new(),
            node_retries:     std::collections::HashMap::new(),
            node_visits:      std::collections::HashMap::new(),
            stage_index:      0,
            previous_node_id: None,
            cancelled:        false,
        }
    }

    #[tokio::test]
    async fn after_node_drops_keys_and_emits_notice() {
        let emitter = Arc::new(Emitter::new(RunId::new()));
        let notices = Arc::new(std::sync::Mutex::new(Vec::<RunEvent>::new()));
        let capture = Arc::clone(&notices);
        emitter.on_event(move |event| {
            if matches!(event.body, EventBody::RunNotice(_)) {
                capture.lock().expect("test mutex").push(event.clone());
            }
        });
        let lifecycle = EnvelopeLifecycle::new(&emitter, StageExecutionTracker::default());

        let node = envelope_node(&[("context_allow_keys", "current_seed_id")]);
        let mut result = node_result(&[
            ("current_seed_id", serde_json::json!("fabro-900e")),
            ("review_verdict", serde_json::json!("approve")),
        ]);
        let state = run_state();

        lifecycle
            .after_node(&node, &mut result, &state)
            .await
            .unwrap();

        assert_eq!(
            result.outcome.context_updates,
            std::collections::HashMap::from([(
                "current_seed_id".to_string(),
                serde_json::json!("fabro-900e")
            )])
        );
        let notices = notices.lock().expect("test mutex");
        assert_eq!(notices.len(), 1, "exactly one drop notice");
        match &notices[0].body {
            EventBody::RunNotice(props) => {
                assert_eq!(props.level, RunNoticeLevel::Warn);
                assert_eq!(props.code, "context_update_dropped");
                // Scoped emission: the node lands in the event envelope,
                // the dropped keys in the message.
                assert_eq!(notices[0].node_id.as_deref(), Some("planner"));
                assert!(
                    props.message.contains("review_verdict"),
                    "message names the dropped key: {}",
                    props.message
                );
            }
            other => panic!("expected RunNotice, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn after_node_without_envelope_attrs_is_a_noop() {
        let emitter = Arc::new(Emitter::new(RunId::new()));
        let lifecycle = EnvelopeLifecycle::new(&emitter, StageExecutionTracker::default());

        let node = envelope_node(&[]);
        let mut result = node_result(&[("any_key", serde_json::json!("kept"))]);
        let state = run_state();

        lifecycle
            .after_node(&node, &mut result, &state)
            .await
            .unwrap();

        assert_eq!(result.outcome.context_updates.len(), 1);
    }
}
