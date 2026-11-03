//! The merger's merge door (`fabro_run_merge`, ADR-0039 stage E,
//! denkhaus/work#1107).
//!
//! Door symmetry (decision 12): the same merge `fabro run merge` serves
//! on the CLI facade, over the SAME endpoint with the SAME
//! authorization — this module is the agent-tool half. The merger's
//! fast path merges the clean mass through the CLI twin from script
//! stages; the agent pass uses this tool. The door merges the target
//! run's pull request server-side; it writes NO verdict — the merger
//! marks `closable` through the verdict door after the merge settles.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;

use super::common::{FabroToolBackend, ToolError, ToolResult};

/// The merge write: the target run whose pull request merges.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FabroRunMergeParams {
    /// The TARGET run: the run whose pull request merges. The run must
    /// carry a pull request the adjudicator marked `merge: enabled` —
    /// the merger integrates content the adjudicator approved, it never
    /// judges.
    pub run: String,
}

/// A validated merge: the parsed target run id (a PRESENT-but-blank
/// value is refused — both twins refuse the same argv).
#[derive(Debug, PartialEq, Eq)]
pub struct ValidatedRunMerge {
    pub run: fabro_types::RunId,
}

impl TryFrom<FabroRunMergeParams> for ValidatedRunMerge {
    type Error = ToolError;

    fn try_from(params: FabroRunMergeParams) -> Result<Self, Self::Error> {
        let trimmed = params.run.trim();
        if trimmed.is_empty() {
            return Err(ToolError::message(
                "run must not be blank; pass the target run id whose pull request merges",
            ));
        }
        let run = trimmed
            .parse()
            .map_err(|error| ToolError::message(format!("`{trimmed}` is not a run id: {error}")))?;
        Ok(Self { run })
    }
}

/// The merge result as the pass reads it.
#[derive(Debug, serde::Serialize)]
pub struct RunMergeResult {
    /// The merged run's id.
    pub run:          String,
    /// True when the pull request merged (this call or a previous one —
    /// the sweep is idempotent).
    pub merged:       bool,
    /// The merged pull request's HTML URL.
    pub pull_request: String,
}

/// Merge the target run's pull request through the backend (the tool
/// twin of `fabro run merge`).
pub async fn run_merge(
    backend: Arc<dyn FabroToolBackend>,
    params: ValidatedRunMerge,
) -> ToolResult<RunMergeResult> {
    let merged = backend
        .merge_run(&params.run)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))?;
    Ok(merged)
}

/// The one-line rendering: what merged, where it lives.
pub fn run_merge_text(result: &RunMergeResult) -> String {
    format!("run {} merged — {}", result.run, result.pull_request)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_run_refuses_teaching_the_target() {
        let error = ValidatedRunMerge::try_from(FabroRunMergeParams {
            run: "   ".to_string(),
        })
        .expect_err("a blank run must fail");
        assert!(
            error.to_string().contains("target run id"),
            "the refusal teaches the target: {error}"
        );
    }

    #[test]
    fn a_non_run_id_refuses_with_the_parse_error() {
        let error = ValidatedRunMerge::try_from(FabroRunMergeParams {
            run: "nope".to_string(),
        })
        .expect_err("a non-run id must fail");
        assert!(
            error.to_string().contains("not a run id"),
            "the refusal names the shape: {error}"
        );
    }

    #[test]
    fn the_text_renderer_names_run_and_pr() {
        let text = run_merge_text(&RunMergeResult {
            run:          "01M5844J1FDD".to_string(),
            merged:       true,
            pull_request: "https://github.com/denkhaus/fabro/pull/501".to_string(),
        });
        assert_eq!(
            text,
            "run 01M5844J1FDD merged — https://github.com/denkhaus/fabro/pull/501"
        );
    }
}
