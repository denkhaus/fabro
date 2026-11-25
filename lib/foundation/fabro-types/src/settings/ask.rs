//! The Ask-Fabro chat surface.
//!
//! `[ask]` carries the defaults of the chat surfaces a run does not
//! scope. The run-scoped dock seeds its model from the AskFabro
//! readiness door (the catalog default); the runless supervisor chat
//! reads this namespace instead (work#1690).

use serde::{Deserialize, Serialize};

/// `[ask]` — the runless supervisor chat's settings (work#1690).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskNamespace {
    /// The model the runless supervisor chat pins at session creation
    /// (`ask.supervisor_model`), as a catalog selector or
    /// `provider:model` reference. Absent: session creation falls back
    /// to the catalog default, exactly as before the key existed. A
    /// value that names no catalog model fails the session creation
    /// with a 400 naming the key — deploy-rendered config fails fast
    /// and loud, not on the operator's first message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supervisor_model: Option<String>,
}
