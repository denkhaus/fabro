//! Workflow domain.
//!
//! `[workflow]` is descriptive: `name`, `description`, optional `graph` (a
//! path override for the default `workflow.fabro` file) and `metadata`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A structurally resolved `[workflow]` view for consumers.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkflowNamespace {
    pub name:             Option<String>,
    pub description:      Option<String>,
    pub graph:            String,
    pub metadata:         HashMap<String, String>,
    /// The command the server executes in the run's sandbox when the run
    /// turns terminal (work#1478), before the sandbox is released. A shell
    /// line, run through `bash -c` from the run's workspace; `None` means
    /// no cleanup. `default` keeps specs stored before the key existed
    /// deserializable; `skip_serializing_if` keeps undeclared workflows'
    /// specs byte-stable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_cleanup: Option<String>,
}
