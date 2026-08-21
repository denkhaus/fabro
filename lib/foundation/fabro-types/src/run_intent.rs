use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{RunId, WorkflowVersionId};

/// A request to create a run from an immutable workflow version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunIntent {
    pub workflow_version_id: WorkflowVersionId,
    pub target:              RunTarget,
    pub args:                RunIntentArgs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_id:      Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id:           Option<RunId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title:               Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal:                Option<String>,
}

/// Structured run-setting overrides accepted by [`RunIntent`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunIntentArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model:    Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub inputs:   HashMap<String, Value>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub labels:   HashMap<String, String>,
}

/// Requested workspace content, independent of sandbox placement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunTarget {
    Git {
        repo:   String,
        branch: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha:    Option<String>,
    },
}
