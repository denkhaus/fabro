use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use fabro_types::{SessionId, TurnId};
use pebble_coding_agent::CodingAgent;
use tokio::sync::{Mutex as AsyncMutex, MutexGuard as AsyncMutexGuard};
use tokio_util::sync::CancellationToken;

use crate::sandbox_access::InspectionSandbox;

#[derive(Default)]
pub(crate) struct SessionRuntimeManager {
    entries: Mutex<HashMap<SessionId, Arc<SessionRuntimeEntry>>>,
}

impl SessionRuntimeManager {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn load_or_create_runtime(&self, session_id: SessionId) -> Arc<SessionRuntimeEntry> {
        self.entry(session_id)
    }

    pub(crate) fn reserve_turn(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
    ) -> Result<SessionTurnLease, StartTurnError> {
        let entry = self.load_or_create_runtime(session_id);
        {
            let mut active = entry
                .active_turn
                .lock()
                .expect("session active turn lock poisoned");
            if let Some(active) = active.as_ref() {
                return Err(StartTurnError::ActiveTurn {
                    turn_id: active.turn_id,
                });
            }
            *active = Some(ActiveTurn {
                turn_id,
                cancel_token: None,
                interrupt_requested: false,
            });
        }
        Ok(SessionTurnLease { entry, turn_id })
    }

    pub(crate) fn request_interrupt(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
    ) -> Result<PendingTurnInterrupt, InterruptTurnError> {
        let Some(entry) = self.existing_entry(session_id) else {
            return Err(InterruptTurnError::NotActive);
        };
        {
            let active = entry
                .active_turn
                .lock()
                .expect("session active turn lock poisoned");
            let Some(active) = active.as_ref() else {
                return Err(InterruptTurnError::NotActive);
            };
            if active.turn_id != turn_id {
                return Err(InterruptTurnError::NotActive);
            }
        }
        Ok(PendingTurnInterrupt { entry, turn_id })
    }

    fn entry(&self, session_id: SessionId) -> Arc<SessionRuntimeEntry> {
        let mut entries = self.entries.lock().expect("session runtime map poisoned");
        Arc::clone(
            entries
                .entry(session_id)
                .or_insert_with(|| Arc::new(SessionRuntimeEntry::new())),
        )
    }

    fn existing_entry(&self, session_id: SessionId) -> Option<Arc<SessionRuntimeEntry>> {
        self.entries
            .lock()
            .expect("session runtime map poisoned")
            .get(&session_id)
            .cloned()
    }
}

/// The live coding agent behind one Ask Fabro session, when this process has
/// one. A process that has none resumes the agent from its stored record.
pub(crate) struct SessionRuntimeEntry {
    agent:       AsyncMutex<AgentSlot>,
    active_turn: Mutex<Option<ActiveTurn>>,
}

/// One loaded Ask Fabro agent and the liveness guard over the run sandbox
/// it was built on (fabro-afab; the legacy turn-scoped terminal-run
/// session): the guard is `terminal` exactly when the run had already
/// ended when the agent was built, and stopping that sandbox again
/// belongs to the moment the agent leaves this slot.
pub(crate) struct AgentSlot {
    pub(crate) agent: Option<CodingAgent>,
    pub(crate) guard: Option<InspectionSandbox>,
}

impl AgentSlot {
    fn empty() -> Self {
        Self {
            agent: None,
            guard: None,
        }
    }

    /// Shut the agent down and release the sandbox guard, so a slot's exit
    /// always stops a terminal run's reactivated sandbox (fabro-afab) and
    /// never touches a live run's.
    async fn evict(&mut self) {
        if let Some(mut agent) = self.agent.take() {
            let _ = agent
                .shutdown(pebble_coding_agent::ShutdownReason::Error)
                .await;
        }
        if let Some(guard) = self.guard.take() {
            guard.finish().await;
        }
    }
}

impl SessionRuntimeEntry {
    fn new() -> Self {
        Self {
            agent:       AsyncMutex::new(AgentSlot::empty()),
            active_turn: Mutex::new(None),
        }
    }

    pub(crate) async fn lock_agent(&self) -> AsyncMutexGuard<'_, AgentSlot> {
        self.agent.lock().await
    }

    /// Whether a live agent occupies the slot. Test seam: the eviction
    /// proof reads it; production code acts through [`Self::clear_agent`].
    #[cfg(test)]
    pub(crate) fn has_agent(&self) -> bool {
        self.agent.try_lock().is_ok_and(|slot| slot.agent.is_some())
    }

    /// Drop the live agent so the next turn resumes from the stored
    /// record, and stop a terminal run's reactivated sandbox again.
    pub(crate) async fn clear_agent(&self) {
        self.agent.lock().await.evict().await;
    }

    /// Evict the agent after a successful turn when — and only when — it
    /// guards a terminal run's sandbox (fabro-afab; the legacy turn-scoped
    /// session): a finished run's sandbox runs for the turn and stops with
    /// it, while a live run's agent stays cached and its run owns the
    /// sandbox.
    pub(crate) async fn evict_turn_scoped(&self) {
        let mut slot = self.agent.lock().await;
        if slot
            .guard
            .as_ref()
            .is_some_and(InspectionSandbox::stops_on_eviction)
        {
            slot.evict().await;
        }
    }
}

struct ActiveTurn {
    turn_id:             TurnId,
    cancel_token:        Option<CancellationToken>,
    interrupt_requested: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StartTurnError {
    ActiveTurn { turn_id: TurnId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InterruptTurnError {
    NotActive,
}

pub(crate) struct SessionTurnLease {
    entry:   Arc<SessionRuntimeEntry>,
    turn_id: TurnId,
}

pub(crate) struct PendingTurnInterrupt {
    entry:   Arc<SessionRuntimeEntry>,
    turn_id: TurnId,
}

impl PendingTurnInterrupt {
    pub(crate) fn cancel(self) {
        let cancel_token = {
            let mut active = self
                .entry
                .active_turn
                .lock()
                .expect("session active turn lock poisoned");
            let Some(active) = active
                .as_mut()
                .filter(|active| active.turn_id == self.turn_id)
            else {
                return;
            };
            active.interrupt_requested = true;
            active.cancel_token.clone()
        };
        if let Some(cancel_token) = cancel_token {
            cancel_token.cancel();
        }
    }
}

impl SessionTurnLease {
    pub(crate) fn entry(&self) -> Arc<SessionRuntimeEntry> {
        Arc::clone(&self.entry)
    }

    pub(crate) fn attach_cancel_token(&self, cancel_token: &CancellationToken) -> bool {
        let mut active = self
            .entry
            .active_turn
            .lock()
            .expect("session active turn lock poisoned");
        let Some(active) = active
            .as_mut()
            .filter(|active| active.turn_id == self.turn_id)
        else {
            return false;
        };
        active.cancel_token = Some(cancel_token.clone());
        if active.interrupt_requested {
            cancel_token.cancel();
            true
        } else {
            false
        }
    }

    pub(crate) fn interrupt_requested(&self) -> bool {
        self.entry
            .active_turn
            .lock()
            .expect("session active turn lock poisoned")
            .as_ref()
            .is_some_and(|active| active.turn_id == self.turn_id && active.interrupt_requested)
    }
}

impl Drop for SessionTurnLease {
    fn drop(&mut self) {
        let mut active = self
            .entry
            .active_turn
            .lock()
            .expect("session active turn lock poisoned");
        if active
            .as_ref()
            .is_some_and(|active| active.turn_id == self.turn_id)
        {
            *active = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sandbox_driver::{Sandbox, SandboxState};
    use sandbox_driver_testing::ScriptedSandbox;

    use super::*;

    fn running_sandbox() -> (Arc<ScriptedSandbox>, Arc<dyn Sandbox>) {
        let inner = Arc::new(
            ScriptedSandbox::with_id_and_working_dir("test-sandbox", "/workspace")
                .state(SandboxState::Running),
        );
        let handle: Arc<dyn Sandbox> = Arc::clone(&inner) as Arc<_>;
        (inner, handle)
    }

    /// A slot holding a terminal-run guard stops the run's reactivated
    /// sandbox when the agent leaves it (fabro-afab; the legacy
    /// turn-scoped terminal-run session).
    #[tokio::test(start_paused = true)]
    async fn clearing_the_agent_stops_a_terminal_run_sandbox() {
        let (inner, sandbox) = running_sandbox();
        let manager = SessionRuntimeManager::new();
        let entry = manager.load_or_create_runtime(SessionId::new());
        entry.agent.lock().await.guard = Some(InspectionSandbox::terminal(sandbox));
        entry.clear_agent().await;
        assert_eq!(inner.current_state(), SandboxState::Stopped);
        assert!(!entry.has_agent());
    }

    /// A slot on a live run never stops the run's sandbox, whatever leaves
    /// the slot: the run's lifecycle owns that liveness.
    #[tokio::test(start_paused = true)]
    async fn clearing_the_agent_never_stops_a_live_run_sandbox() {
        let (inner, sandbox) = running_sandbox();
        let manager = SessionRuntimeManager::new();
        let entry = manager.load_or_create_runtime(SessionId::new());
        entry.agent.lock().await.guard = Some(InspectionSandbox::live(sandbox));
        entry.clear_agent().await;
        assert_eq!(inner.current_state(), SandboxState::Running);
    }

    /// After a successful turn, only a terminal run's agent leaves the
    /// slot (turn-scoped): a live run's slot keeps what it holds.
    #[tokio::test(start_paused = true)]
    async fn turn_scoped_eviction_leaves_a_live_run_slot_alone() {
        let (inner, sandbox) = running_sandbox();
        let manager = SessionRuntimeManager::new();
        let entry = manager.load_or_create_runtime(SessionId::new());
        {
            let mut slot = entry.agent.lock().await;
            slot.guard = Some(InspectionSandbox::live(sandbox));
        }
        entry.evict_turn_scoped().await;
        tokio::time::advance(std::time::Duration::from_millis(1)).await;
        assert_eq!(inner.current_state(), SandboxState::Running);
        assert!(
            entry.agent.lock().await.guard.is_some(),
            "a live run's slot is not evicted by the turn-scoped path"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn turn_scoped_eviction_stops_a_terminal_run_sandbox() {
        let (inner, sandbox) = running_sandbox();
        let manager = SessionRuntimeManager::new();
        let entry = manager.load_or_create_runtime(SessionId::new());
        entry.agent.lock().await.guard = Some(InspectionSandbox::terminal(sandbox));
        entry.evict_turn_scoped().await;
        assert_eq!(inner.current_state(), SandboxState::Stopped);
        assert!(entry.agent.lock().await.guard.is_none());
    }
}
