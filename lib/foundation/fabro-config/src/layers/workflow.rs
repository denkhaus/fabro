//! Sparse `[workflow]` settings layer definitions.

use fabro_types::Engine;
use serde::{Deserialize, Serialize};

use super::maps::ReplaceMap;

/// A sparse `[workflow]` layer as it appears in a single settings file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, fabro_macros::Combine)]
#[serde(deny_unknown_fields)]
pub struct WorkflowLayer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name:        Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional override for the default `workflow.fabro` graph path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph:       Option<String>,
    #[serde(default, skip_serializing_if = "ReplaceMap::is_empty")]
    pub metadata:    ReplaceMap<String>,
    /// The engine the workflow asks to run on: `"petri"` or `"legacy"`.
    /// Unset leaves the choice to the server's `[server.execution] engine`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine:      Option<Engine>,
}
