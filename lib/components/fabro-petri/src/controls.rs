//! The controls Fabro drives on a live Petri run: pause and unpause at
//! admission, a steer into the run's agent stage, and cancel.
//!
//! [`RunControls`] is Petri's `ControlService` as the run's worker holds it:
//! one per run, built before the run and handed to [`engine::run`] in its
//! [`RunRequest`], which installs the service's pause gate over the run's
//! hooks (Fabro's own [`FabroHooks`] over Petri's local hook service),
//! observes the run through it, and wires it to the coordinator once the
//! coordinator exists. A start and a resume install it the same way, so a
//! run that was paused when its worker died resumes paused: the service is
//! handed the replayed coordinator state before the first attempt is
//! admitted, and admission stays held until an unpause arrives through the
//! new worker's control channel.
//!
//! What each control does, and what the run's record says of it:
//!
//! - pause holds every attempt not yet admitted, at once; the coordinator
//!   records `run.paused`. Running work continues to its end.
//! - unpause records `run.unpaused` first and releases admission once the
//!   record is durable, so a crash between the two resumes paused.
//! - steer delivers a text to a live agent stage as guidance for its session:
//!   the stage's firing records `control.requested` with the `{"$steer": …}`
//!   value, and the agent runs the text as a follow-up turn once its current
//!   answer is reached. Fabro's steer names no stage, so the steer goes to the
//!   one live agent stage; with none, or several, it is refused with the
//!   reason, and nothing is recorded.
//! - cancel is the caller's cancellation token ([`RunRequest::cancel`]); the
//!   service's own cancel is here for a host that holds only this.
//!
//! The paused state is published as a watch ([`RunControls::paused_changes`])
//! so the worker can mirror it to Fabro's lifecycle (`run.paused` and
//! `run.unpaused` lifecycle events), including the flip a resume makes.
//!
//! [`engine::run`]: crate::engine::run
//! [`RunRequest`]: crate::engine::RunRequest
//! [`RunRequest::cancel`]: crate::engine::RunRequest::cancel
//! [`FabroHooks`]: crate::hooks::FabroHooks

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use petri_execution::controls::{ControlError, ControlService};
use petri_execution::{
    CoordinatorHandle, CoordinatorRecord, CoordinatorState, ExecutionId, ExecutionObserver,
};
use petri_frontend_attractor::kinds::AGENT_KIND;
use petri_runtime::driver::lifecycle::ExecutionHooks;
use petri_runtime::engine::{EngineState, Event, EventRecord};
use petri_runtime::ir::FiringId;
use tokio::sync::watch;

/// Why a steer was not delivered.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SteerError {
    /// No agent stage is running: the same refusal the legacy server gave a
    /// control that needs a live agent session.
    #[error("Run has no active steerable agent session.")]
    NoLiveAgent,
    /// More than one agent stage is running and the steer names none.
    #[error("Run has several active agent stages ({}); the steer names none.", .0.join(", "))]
    SeveralLiveAgents(Vec<String>),
    /// The named stage is not running, or the run has ended.
    #[error(transparent)]
    Control(#[from] ControlError),
}

/// The live agent firings, by node name: what a steer that names no stage
/// is routed by.
#[derive(Default)]
struct LiveAgents {
    stages:  BTreeMap<String, (ExecutionId, FiringId)>,
    firings: BTreeMap<(ExecutionId, FiringId), String>,
}

/// One run's controls. Clone freely: every clone drives the same service.
#[derive(Clone)]
pub struct RunControls {
    service: ControlService,
    agents:  Arc<Mutex<LiveAgents>>,
}

impl Default for RunControls {
    fn default() -> Self {
        Self::new()
    }
}

impl RunControls {
    #[must_use]
    pub fn new() -> Self {
        Self {
            service: ControlService::new(),
            agents:  Arc::new(Mutex::new(LiveAgents::default())),
        }
    }

    /// Hold every attempt not yet admitted. Running work is not interrupted.
    pub fn pause(&self) {
        self.service.pause();
    }

    /// Release held and future attempts, once the unpause is durable.
    pub async fn unpause(&self) {
        self.service.unpause().await;
    }

    #[must_use]
    pub fn is_paused(&self) -> bool {
        self.service.is_paused()
    }

    /// Every change of the paused state, the flip a resume makes included.
    #[must_use]
    pub fn paused_changes(&self) -> watch::Receiver<bool> {
        self.service.paused_changes()
    }

    /// The names of the agent stages running now.
    #[must_use]
    pub fn live_agents(&self) -> Vec<String> {
        self.agents().stages.keys().cloned().collect()
    }

    /// Deliver `text` to the named agent stage, or to the one live agent
    /// stage when `node` is `None`. The name of the stage steered.
    pub async fn steer(&self, node: Option<&str>, text: &str) -> Result<String, SteerError> {
        let node = if let Some(node) = node {
            node.to_owned()
        } else {
            let mut live = self.live_agents();
            match live.len() {
                0 => return Err(SteerError::NoLiveAgent),
                1 => live.remove(0),
                _ => return Err(SteerError::SeveralLiveAgents(live)),
            }
        };
        self.service.steer(&node, text).await?;
        Ok(node)
    }

    /// Cancel the whole run politely; a second call reaches the kill tier.
    pub fn cancel(&self) -> Result<(), ControlError> {
        self.service.cancel()
    }

    /// The pause gate over `inner`, for the runtime.
    pub(crate) fn hooks(&self, inner: Option<Arc<dyn ExecutionHooks>>) -> Arc<dyn ExecutionHooks> {
        self.service.hooks(inner)
    }

    /// Hand the service the run's coordinator handle.
    pub(crate) fn wire(&self, handle: CoordinatorHandle) {
        self.service.wire(handle);
    }

    /// The observer to register on the run: the service's own, which keeps
    /// the live firing of every stage and the paused state across a
    /// resume, and the live agent stages beside it.
    pub(crate) fn observer(&self) -> Arc<dyn ExecutionObserver> {
        Arc::new(self.clone())
    }

    fn agents(&self) -> MutexGuard<'_, LiveAgents> {
        self.agents.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl ExecutionObserver for RunControls {
    fn on_engine_record(
        &self,
        execution: ExecutionId,
        record: &EventRecord,
        recorded_at: u64,
        state: &EngineState,
    ) {
        self.service
            .on_engine_record(execution, record, recorded_at, state);
        match &record.event {
            Event::StepStarted { firing, .. } => {
                let Some(node) = state
                    .firing_node(*firing)
                    .and_then(|id| state.graph().node(id))
                else {
                    return;
                };
                if node.step.kind != AGENT_KIND {
                    return;
                }
                let name = node.name.to_string();
                let mut agents = self.agents();
                agents.stages.insert(name.clone(), (execution, *firing));
                agents.firings.insert((execution, *firing), name);
            }
            Event::StepFinished { firing, .. } => {
                let mut agents = self.agents();
                if let Some(name) = agents.firings.remove(&(execution, *firing)) {
                    if agents.stages.get(&name) == Some(&(execution, *firing)) {
                        agents.stages.remove(&name);
                    }
                }
            }
            _ => {}
        }
    }

    fn on_lifecycle(&self, record: &CoordinatorRecord) {
        self.service.on_lifecycle(record);
    }

    fn on_resumed(&self, state: &CoordinatorState) {
        self.service.on_resumed(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_steer_with_no_live_agent_is_refused_with_the_legacy_reason() {
        let controls = RunControls::new();
        assert_eq!(
            controls.steer(None, "hurry up").await,
            Err(SteerError::NoLiveAgent)
        );
        assert_eq!(
            SteerError::NoLiveAgent.to_string(),
            "Run has no active steerable agent session."
        );
    }

    #[tokio::test]
    async fn a_steer_to_a_named_stage_that_is_not_running_is_refused() {
        let controls = RunControls::new();
        assert_eq!(
            controls.steer(Some("work"), "hurry up").await,
            Err(SteerError::Control(ControlError::NoSuchStage(
                "work".to_string()
            )))
        );
    }

    #[test]
    fn a_pause_holds_before_the_run_is_wired() {
        let controls = RunControls::new();
        let mut changes = controls.paused_changes();
        assert!(!controls.is_paused());
        controls.pause();
        assert!(controls.is_paused());
        assert!(changes.has_changed().expect("the sender is alive"));
        assert!(*changes.borrow_and_update());
    }
}
