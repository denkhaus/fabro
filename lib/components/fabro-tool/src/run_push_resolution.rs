//! The merger's resolve push door (`fabro_run_push_resolution`,
//! ADR-0047, denkhaus/work#1471).
//!
//! The agent-tool twin of `POST /runs/{id}/resolution`: after the merge
//! door declined with `merge_conflict`, the resolve pass commits its
//! resolution on the TARGET run's branch in its own checkout and names
//! the commit here. The server transfers the commit's objects itself
//! (the sandbox never ships credentials — only the door moves git state
//! toward the forge) and pushes the run's OWN branch with the same
//! server-side credentials `run_publish` uses. The door refuses any
//! branch but `fabro/run/<target>`, refuses non-fast-forward pushes,
//! and lands a `run.branch_pushed` audit record in the target run's
//! stream per push.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;

use super::common::{FabroToolBackend, ToolError, ToolResult};

/// The resolve push: the target run whose branch receives the
/// resolution, and the resolution commit itself.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FabroRunPushResolutionParams {
    /// The TARGET run: the run whose own branch (`fabro/run/<id>`) the
    /// resolution lands on — the packet's run, the one whose merge the
    /// door declined. The branch is the server's to name; there is no
    /// branch parameter to pass.
    pub run:    String,
    /// The resolution commit's sha, as committed on the run branch in
    /// THIS checkout (a short or full sha the checkout resolves). The
    /// commit must sit on the checkout's branch for the run branch —
    /// resolve first, then name the commit.
    pub commit: String,
}

/// A validated push: the parsed target run id and a PRESENT commit
/// (blank values are refused — both twins refuse the same argv).
#[derive(Debug, PartialEq, Eq)]
pub struct ValidatedRunPushResolution {
    pub run:    fabro_types::RunId,
    pub commit: String,
}

impl TryFrom<FabroRunPushResolutionParams> for ValidatedRunPushResolution {
    type Error = ToolError;

    fn try_from(params: FabroRunPushResolutionParams) -> Result<Self, Self::Error> {
        let trimmed_run = params.run.trim();
        if trimmed_run.is_empty() {
            return Err(ToolError::message(
                "run must not be blank; pass the target run id whose run branch the resolution \
                 lands on",
            ));
        }
        let run = trimmed_run.parse().map_err(|error| {
            ToolError::message(format!("`{trimmed_run}` is not a run id: {error}"))
        })?;
        let commit = params.commit.trim().to_string();
        if commit.is_empty() {
            return Err(ToolError::message(
                "commit must not be blank; pass the resolution commit's sha as committed on the \
                 run branch in this checkout",
            ));
        }
        Ok(Self { run, commit })
    }
}

/// The push result as the pass reads it.
#[derive(Debug, serde::Serialize)]
pub struct RunPushResolutionResult {
    /// The run whose branch now carries the resolution.
    pub run:        String,
    /// True when the branch head on the forge is the resolution commit
    /// (this call pushed it, or an earlier call already did — the door
    /// is idempotent).
    pub pushed:     bool,
    /// The branch the door pushed — always the run's own
    /// `fabro/run/<id>`.
    pub run_branch: String,
    /// The resolution commit the branch now names.
    pub head_sha:   String,
}

/// Push the resolution through the backend (the tool twin of
/// `POST /runs/{id}/resolution`).
pub async fn run_push_resolution(
    backend: Arc<dyn FabroToolBackend>,
    params: ValidatedRunPushResolution,
) -> ToolResult<RunPushResolutionResult> {
    let pushed = backend
        .push_run_resolution(&params.run, &params.commit)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))?;
    Ok(pushed)
}

/// The one-line rendering: what landed, where.
pub fn run_push_resolution_text(result: &RunPushResolutionResult) -> String {
    format!(
        "run {} resolution {} on {} — pushed",
        result.run, result.head_sha, result.run_branch
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_run_refuses_teaching_the_target() {
        let error = ValidatedRunPushResolution::try_from(FabroRunPushResolutionParams {
            run:    "   ".to_string(),
            commit: "abc123".to_string(),
        })
        .expect_err("a blank run must fail");
        assert!(
            error.to_string().contains("target run id"),
            "the refusal teaches the target: {error}"
        );
    }

    #[test]
    fn a_blank_commit_refuses_teaching_the_sha() {
        let error = ValidatedRunPushResolution::try_from(FabroRunPushResolutionParams {
            run:    "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string(),
            commit: "  ".to_string(),
        })
        .expect_err("a blank commit must fail");
        assert!(
            error.to_string().contains("resolution commit"),
            "the refusal teaches the commit: {error}"
        );
    }

    #[test]
    fn a_non_run_id_refuses_with_the_parse_error() {
        let error = ValidatedRunPushResolution::try_from(FabroRunPushResolutionParams {
            run:    "nope".to_string(),
            commit: "abc123".to_string(),
        })
        .expect_err("a non-run id must fail");
        assert!(
            error.to_string().contains("not a run id"),
            "the refusal names the shape: {error}"
        );
    }

    #[test]
    fn a_valid_pair_trims_both_values() {
        let validated = ValidatedRunPushResolution::try_from(FabroRunPushResolutionParams {
            run:    " 01ARZ3NDEKTSV4RRFFQ69G5FAV ".to_string(),
            commit: " abc123 ".to_string(),
        })
        .expect("a valid pair passes");
        assert_eq!(validated.commit, "abc123");
    }

    #[test]
    fn the_text_renderer_names_run_sha_and_branch() {
        let text = run_push_resolution_text(&RunPushResolutionResult {
            run:        "01M5844J1FDD".to_string(),
            pushed:     true,
            run_branch: "fabro/run/01M5844J1FDD".to_string(),
            head_sha:   "f32050d6f".to_string(),
        });
        assert_eq!(
            text,
            "run 01M5844J1FDD resolution f32050d6f on fabro/run/01M5844J1FDD — pushed"
        );
    }
}
