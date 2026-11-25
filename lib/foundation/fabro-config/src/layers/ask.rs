//! Sparse `[ask]` settings layer definitions.

use serde::{Deserialize, Serialize};

/// `[ask]` — the runless supervisor chat's settings (work#1690).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, fabro_macros::Combine)]
#[serde(deny_unknown_fields)]
pub struct AskLayer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supervisor_model: Option<String>,
}
