use std::sync::Arc;

use fabro_store::platform_records::StagePosition;
use fabro_types::RunId;

/// What Fabro's run tools bind to when an agent session calls them: the
/// tool backend (the server's API through a run-scoped client), the run
/// the session belongs to, and — for a workflow stage session — the
/// Petri stage position the session serves.
///
/// `stage` is `None` on surfaces without a stage (Ask Fabro sessions,
/// `fabro exec`); the one tool that needs it (`fabro_observation_record`,
/// ADR-0038) refuses there with a teaching error instead of writing an
/// unattributed record.
#[derive(Clone)]
pub struct FabroRunToolServices {
    pub backend:        Arc<dyn fabro_tool::FabroToolBackend>,
    pub current_run_id: RunId,
    /// The stage the calling session serves, when there is one: the
    /// `(execution, firing)` pair the engine's `HostToolContext` names
    /// when it asks for the session's tools.
    pub stage:          Option<StagePosition>,
}

impl FabroRunToolServices {
    /// This backend and run, serving the stage `position`. The Petri
    /// host-tools seam builds one services value per stage session this
    /// way (fabro-petri `host_tools::tools_for_stage`).
    #[must_use]
    pub fn for_stage(&self, position: StagePosition) -> Self {
        Self {
            backend:        Arc::clone(&self.backend),
            current_run_id: self.current_run_id,
            stage:          Some(position),
        }
    }
}
