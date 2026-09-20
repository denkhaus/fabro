use chrono::{DateTime, Utc};
use lithos_llm::types::Usage;
use serde::{Deserialize, Serialize};

use crate::outcome::StageOutcome;
use crate::{RunDiff, RunFailure, RunTiming, StageTiming};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageSummary {
    pub stage_id:    String,
    pub stage_label: String,
    /// Per-node timing summed across every visit of the node within this
    /// conclusion. `wall_time_ms` is the sum of visit wall times.
    pub timing:      StageTiming,
    /// Per-node usage summed across every visit of the node.
    #[serde(default)]
    pub usage:       Usage,
    pub retries:     u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conclusion {
    pub timestamp:            DateTime<Utc>,
    pub status:               StageOutcome,
    /// Run-level timing. `wall_time_ms` is the run's clock duration; active
    /// fields sum work across stage visits and can exceed wall time.
    pub timing:               RunTiming,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure:              Option<RunFailure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_git_commit_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stages:               Vec<StageSummary>,
    /// The run's usage summed across every stage visit; `None` for a run
    /// that made no model calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage:                Option<Usage>,
    #[serde(default)]
    pub total_retries:        u32,
    #[serde(default)]
    pub diff:                 RunDiff,
}

impl Conclusion {
    /// A conclusion that records only how the run ended: no timing, stages,
    /// usage or diff. What a terminal lifecycle record gives when the
    /// engine recorded no finish of its own.
    #[must_use]
    pub fn outcome_only(
        timestamp: DateTime<Utc>,
        status: StageOutcome,
        failure: Option<RunFailure>,
    ) -> Self {
        Self {
            timestamp,
            status,
            timing: RunTiming::default(),
            failure,
            final_git_commit_sha: None,
            stages: Vec::new(),
            usage: None,
            total_retries: 0,
            diff: RunDiff::default(),
        }
    }
}
