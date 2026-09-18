//! Workflow domain.
//!
//! `[workflow]` is descriptive: `name`, `description`, optional `graph` (a
//! path override for the default `workflow.fabro` file), `metadata`, and the
//! optional `engine` the workflow asks to run on.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::Engine;

/// A structurally resolved `[workflow]` view for consumers.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkflowNamespace {
    pub name:        Option<String>,
    pub description: Option<String>,
    pub graph:       String,
    pub metadata:    HashMap<String, String>,
    /// The engine the workflow names; `None` leaves the choice to the
    /// server's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine:      Option<Engine>,
}
