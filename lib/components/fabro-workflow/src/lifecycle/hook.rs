use std::sync::Arc;

use async_trait::async_trait;
use fabro_core::error::{Error as CoreError, Result as CoreResult};
use fabro_core::lifecycle::{
    AttemptContext, EdgeContext, EdgeDecision, NodeDecision, RunLifecycle,
};
use fabro_core::outcome::NodeResult;
use fabro_core::state::ExecutionState;
use fabro_hooks::{HookContext, HookDecision, HookEvent, HookExecutionContext, HookRunner};
use fabro_sandbox::Sandbox;
use fabro_types::RunId;

use crate::graph::{WorkflowGraph, WorkflowNode};
use crate::hook_context::set_hook_node;
use crate::outcome::{BilledModelUsage, Outcome, OutcomeExt, StageOutcome};

type WfRunState = ExecutionState<Option<BilledModelUsage>>;
type WfNodeResult = NodeResult<Option<BilledModelUsage>>;
type WfNodeDecision = NodeDecision<Option<BilledModelUsage>>;

/// Sub-lifecycle responsible for running workflow hooks.
pub(crate) struct HookLifecycle {
    pub hook_runner:            Option<Arc<HookRunner>>,
    pub sandbox:                Arc<dyn Sandbox>,
    pub hook_execution_context: HookExecutionContext,
    pub run_id:                 RunId,
    pub graph_name:             String,
}

impl HookLifecycle {
    async fn run_hook(&self, hook_ctx: &HookContext) -> HookDecision {
        let Some(ref runner) = self.hook_runner else {
            return HookDecision::Proceed;
        };
        runner
            .run(
                hook_ctx,
                self.sandbox.clone(),
                self.hook_execution_context.clone(),
            )
            .await
    }
}

#[async_trait]
impl RunLifecycle<WorkflowGraph> for HookLifecycle {
    async fn on_run_start(&self, _graph: &WorkflowGraph, _state: &WfRunState) -> CoreResult<()> {
        let hook_ctx = HookContext::new(HookEvent::RunStart, self.run_id, self.graph_name.clone());
        let decision = self.run_hook(&hook_ctx).await;
        if let HookDecision::Block { reason } = decision {
            let msg = reason.unwrap_or_else(|| "blocked by RunStart hook".into());
            return Err(CoreError::blocked(msg));
        }
        Ok(())
    }

    async fn before_attempt(
        &self,
        ctx: &AttemptContext<'_, WorkflowGraph>,
        _state: &WfRunState,
    ) -> CoreResult<WfNodeDecision> {
        let gv = ctx.node.inner();
        let mut hook_ctx =
            HookContext::new(HookEvent::StageStart, self.run_id, self.graph_name.clone());
        hook_ctx.cwd = self
            .hook_execution_context
            .sandbox_work_dir
            .as_ref()
            .map(|path| path.display().to_string());
        set_hook_node(&mut hook_ctx, gv);
        hook_ctx.attempt = Some(ctx.attempt as usize);
        hook_ctx.max_attempts = Some(ctx.max_attempts as usize);
        let decision = self.run_hook(&hook_ctx).await;
        match decision {
            HookDecision::Skip { reason } => {
                let msg = reason.unwrap_or_else(|| "skipped by StageStart hook".into());
                Ok(NodeDecision::Skip(Box::new(Outcome::skipped(&msg))))
            }
            HookDecision::Block { reason } => {
                let msg = reason.unwrap_or_else(|| "blocked by StageStart hook".into());
                Err(CoreError::blocked(msg))
            }
            _ => Ok(NodeDecision::Continue),
        }
    }

    async fn after_node(
        &self,
        node: &WorkflowNode,
        result: &mut WfNodeResult,
        _state: &WfRunState,
    ) -> CoreResult<()> {
        let outcome = &result.outcome;
        // Skipped nodes had no StageStarted, so skip hooks (engine.rs:2080)
        if outcome.status == StageOutcome::Skipped {
            return Ok(());
        }
        let hook_event = if outcome.status.is_failure() {
            HookEvent::StageFailed
        } else {
            HookEvent::StageComplete
        };
        let mut hook_ctx = HookContext::new(hook_event, self.run_id, self.graph_name.clone());
        set_hook_node(&mut hook_ctx, node.inner());
        hook_ctx.status = Some(outcome.status.to_string());
        hook_ctx.failure_reason = outcome.failure_reason().map(String::from);
        // Journal bridge (seed fabro-31b2): hand the stage's declared
        // context_updates to the hook — both on success and failure; a
        // failed stage can carry painpoints too. Raw declared values,
        // pre-blob-normalization: journal payloads are small and must not
        // degrade to blob refs inside the hook context file.
        if !outcome.context_updates.is_empty() {
            hook_ctx.context_updates = Some(outcome.context_updates.clone());
        }
        let _ = self.run_hook(&hook_ctx).await;
        Ok(())
    }

    async fn on_edge_selected(
        &self,
        ctx: &EdgeContext<'_, WorkflowGraph>,
        _state: &WfRunState,
    ) -> CoreResult<EdgeDecision> {
        let mut hook_ctx = HookContext::new(
            HookEvent::EdgeSelected,
            self.run_id,
            self.graph_name.clone(),
        );
        hook_ctx.edge_from = Some(ctx.from.to_string());
        hook_ctx.edge_to = Some(ctx.to.to_string());
        hook_ctx.edge_label = ctx
            .edge
            .as_ref()
            .and_then(|edge| edge.inner().label().map(String::from));
        let decision = self.run_hook(&hook_ctx).await;
        match decision {
            HookDecision::Override { edge_to } => Ok(EdgeDecision::Override(edge_to)),
            HookDecision::Block { reason } => {
                let msg = reason.unwrap_or_else(|| "blocked by EdgeSelected hook".into());
                Err(CoreError::blocked(msg))
            }
            _ => Ok(EdgeDecision::Continue),
        }
    }

    async fn on_checkpoint(
        &self,
        node: &WorkflowNode,
        _result: &WfNodeResult,
        _next_node_id: Option<&str>,
        _state: &WfRunState,
    ) -> CoreResult<()> {
        let mut hook_ctx = HookContext::new(
            HookEvent::CheckpointSaved,
            self.run_id,
            self.graph_name.clone(),
        );
        hook_ctx.node_id = Some(node.inner().id.clone());
        let _ = self.run_hook(&hook_ctx).await;
        Ok(())
    }

    async fn on_run_end(&self, outcome: &Outcome, state: &WfRunState) {
        if state.cancelled {
            return;
        }
        if outcome.status == StageOutcome::Succeeded
            || outcome.status == StageOutcome::PartiallySucceeded
        {
            let hook_ctx =
                HookContext::new(HookEvent::RunComplete, self.run_id, self.graph_name.clone());
            let _ = self.run_hook(&hook_ctx).await;
        } else {
            let error_msg = outcome
                .failure
                .as_ref()
                .map_or_else(|| "run failed".to_string(), |f| f.message.clone());
            let mut hook_ctx =
                HookContext::new(HookEvent::RunFailed, self.run_id, self.graph_name.clone());
            hook_ctx.failure_reason = Some(error_msg);
            let _ = self.run_hook(&hook_ctx).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use fabro_graphviz::graph::{AttrValue, Graph, Node};
    use fabro_hooks::config::HookDefinition;
    use fabro_hooks::HookSettings;
    use fabro_model::Catalog;
    use fabro_types::fixtures;
    use fabro_types::outcome::Outcome;

    use super::*;
    use crate::graph::WorkflowGraph;

    fn test_node(id: &str) -> WorkflowNode {
        let mut node = Node::new(id);
        node.attrs.insert(
            "shape".to_string(),
            AttrValue::String("box".to_string()),
        );
        WorkflowNode(Arc::new(node))
    }

    /// Minimal two-node graph (start -> <id>) so ExecutionState can exist.
    fn test_state(id: &str) -> WfRunState {
        let mut graph = Graph::new("hook-test");
        let mut start = Node::new("start");
        start.attrs.insert(
            "shape".to_string(),
            AttrValue::String("Mdiamond".to_string()),
        );
        graph.nodes.insert(start.id.clone(), start);
        graph.nodes.insert(id.to_string(), Node::new(id));
        graph.edges.push(fabro_graphviz::graph::Edge::new("start", id));
        let workflow_graph = WorkflowGraph(Arc::new(graph));
        ExecutionState::new(&workflow_graph).unwrap()
    }

    /// HookLifecycle whose stage_complete hook runs `command` on the host
    /// (context JSON arrives on stdin). The command writes a verdict into
    /// `marker` for the test to assert on — this exercises the REAL
    /// executor pipe (HookContext -> serialization -> hook process).
    fn hook_lifecycle(command: String) -> HookLifecycle {
        let hook = HookDefinition {
            name:       Some("journal-probe".into()),
            event:      HookEvent::StageComplete,
            command:    Some(command.into()),
            hook_type:  None,
            matcher:    None,
            blocking:   None,
            timeout_ms: None,
            sandbox:    Some(false),
        };
        let runner = HookRunner::new(
            HookSettings { hooks: vec![hook] },
            fabro_auth::test_support::vault_only_credential_source(),
            Arc::new(Catalog::from_builtin().expect("default catalog should build")),
        );
        HookLifecycle {
            hook_runner:            Some(Arc::new(runner)),
            sandbox:                Arc::new(fabro_agent::LocalSandbox::new(
                std::env::temp_dir(),
            )),
            hook_execution_context: HookExecutionContext::default(),
            run_id:                 fixtures::RUN_1,
            graph_name:             "test-wf".to_string(),
        }
    }

    fn probe_command(needle: &str, marker: &std::path::Path) -> String {
        // sh: read HookContext from stdin; write the probe verdict to marker.
        let marker = marker.display();
        format!(
            "ctx=$(cat); case \"$ctx\" in *\"{needle}\"*) echo found > {marker};; *)              echo missing > {marker};; esac; exit 0"
        )
    }

    #[tokio::test]
    async fn after_node_carries_context_updates_to_stage_complete_hooks() {
        // Journal bridge (seed fabro-31b2): a completed stage's declared
        // context_updates must reach the hook — that is the only pipe the
        // stage-journal hook has to persist agent journal payloads.
        let marker = std::env::temp_dir().join(format!(
            "fabro-journal-bridge-{}.txt",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&marker);
        let lc = hook_lifecycle(probe_command("blob refs unreadable", &marker));

        let node = test_node("reviewer");
        let mut result = NodeResult::new(
            Outcome::success(),
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
            1,
            1,
        );
        result.outcome.context_updates.insert(
            "journal".to_string(),
            serde_json::json!({"painpoints": [{"text": "blob refs unreadable"}]}),
        );

        let state = test_state("reviewer");
        lc.after_node(&node, &mut result, &state).await.unwrap();

        let verdict = std::fs::read_to_string(&marker).unwrap_or_else(|_| "no-marker".into());
        let _ = std::fs::remove_file(&marker);
        assert_eq!(
            verdict.trim(),
            "found",
            "stage's journal payload must reach the hook via the executor pipe"
        );
    }

    #[tokio::test]
    async fn after_node_leaves_updates_absent_when_stage_declared_none() {
        // Absent, not an empty map: hooks must distinguish "stage declared
        // nothing" from "bridge dropped the payload" — an empty {} would
        // look like a lost journal.
        let marker = std::env::temp_dir().join(format!(
            "fabro-journal-absent-{}.txt",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&marker);
        let lc = hook_lifecycle(probe_command("context_updates", &marker));

        let node = test_node("planner");
        let mut result = NodeResult::new(
            Outcome::success(),
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
            1,
            1,
        );

        let state = test_state("planner");
        lc.after_node(&node, &mut result, &state).await.unwrap();

        let verdict = std::fs::read_to_string(&marker).unwrap_or_else(|_| "no-marker".into());
        let _ = std::fs::remove_file(&marker);
        assert_eq!(
            verdict.trim(),
            "missing",
            "no declared updates must serialize WITHOUT a context_updates field"
        );
    }
}
