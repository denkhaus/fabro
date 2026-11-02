//! Work-item vocabulary shared across the platform: the ADR-0032
//! tracker seam's comment type, reused by the API surface through
//! `with_replacement` (AGENTS.md, API type ownership) so no parallel
//! DTO exists.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One comment on a work item, as its forge sees it — the answer
/// channel for parked runs (ADR-0039 stage D part 2): the adjudicator
/// parks with a question, the operator answers on the work item, and
/// the next pass reads the answer here. Chronological order, oldest
/// first (the forges' own order).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkItemComment {
    /// The comment's author as the forge names it (login/handle).
    pub author:     String,
    pub body:       String,
    /// When the forge recorded the comment, when it names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    /// The forge's permalink to the comment, when it names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url:        Option<String>,
}
