//! The run title setter (`fabro_run_set_title`, fabro-9711/ADR-0030).
//!
//! Run titles default to the workflow's generic goal title, and the
//! engine LLM-refines one at creation. The agent that defines the work —
//! the planner that claims the seed — knows what the run is actually
//! about, so it refines the title through this tool: a few sentences
//! specific to the claimed seed, never a generic workflow label (user
//! decision 2026-10-18). The write reuses the server's title-update
//! path, so every surface (board tile, runs list, run detail header,
//! Slack terminal notification) shows it without a new field.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::common::{FabroToolBackend, ToolError, ToolResult};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FabroRunSetTitleParams {
    /// The run whose title changes: a run id prefix or workflow name.
    /// Optional; defaults to THIS run — a scoped session cannot retitle a
    /// foreign run anyway.
    pub run:   Option<String>,
    /// The new title: a few sentences describing what THIS run works on —
    /// for a claimed seed, what that seed demands (not a generic workflow
    /// label). The server trims, rejects blank, and rejects titles
    /// over 100 characters.
    pub title: String,
}

/// A validated title update: a trimmed selector (or none, meaning the
/// session's own run) and a normalized title.
#[derive(Debug)]
pub struct ValidatedRunSetTitle {
    pub run:   Option<String>,
    pub title: String,
}

impl TryFrom<FabroRunSetTitleParams> for ValidatedRunSetTitle {
    type Error = ToolError;

    fn try_from(params: FabroRunSetTitleParams) -> Result<Self, Self::Error> {
        let run = match params.run.as_deref() {
            Some(value) => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    return Err(ToolError::message(
                        "run must not be blank; omit it to title this run",
                    ));
                }
                Some(trimmed.to_string())
            }
            None => None,
        };
        // Mirror the server's normalization so a bad title fails here,
        // with the tool's own message, before any request leaves.
        let title = fabro_types::normalize_explicit_run_title(&params.title)
            .map_err(|err| ToolError::message(err.to_string()))?;
        Ok(Self { run, title })
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RunSetTitleResult {
    /// The run whose title was set, as the server resolved it.
    pub run_id: String,
    /// The title now on the run (normalized).
    pub title:  String,
}

pub async fn set_run_title(
    backend: Arc<dyn FabroToolBackend>,
    params: ValidatedRunSetTitle,
) -> ToolResult<RunSetTitleResult> {
    backend
        .set_run_title(params.run.as_deref(), &params.title)
        .await
        .map(|run| RunSetTitleResult {
            run_id: run.id.to_string(),
            title:  run.title,
        })
        .map_err(|err| ToolError::from_anyhow(&err))
}

pub fn set_run_title_text(result: &RunSetTitleResult) -> String {
    format!("run {} titled: {}", result.run_id, result.title)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(run: Option<&str>, title: &str) -> FabroRunSetTitleParams {
        FabroRunSetTitleParams {
            run:   run.map(str::to_string),
            title: title.to_string(),
        }
    }

    #[test]
    fn a_seed_specific_title_passes_and_trims_the_selector() {
        let validated = ValidatedRunSetTitle::try_from(params(
            Some("  01M55  "),
            "seed fabro-9711: meaningful run titles for claimed seeds",
        ))
        .expect("a normal title validates");
        assert_eq!(validated.run.as_deref(), Some("01M55"));
        assert_eq!(
            validated.title,
            "seed fabro-9711: meaningful run titles for claimed seeds"
        );
    }

    #[test]
    fn omitting_the_selector_targets_the_own_run() {
        let validated =
            ValidatedRunSetTitle::try_from(params(None, "revises loop probe: context keys"))
                .expect("a normal title validates");
        assert_eq!(validated.run, None);
    }

    #[test]
    fn a_blank_selector_is_rejected_with_a_teaching_message() {
        let error = ValidatedRunSetTitle::try_from(params(Some("   "), "any title"))
            .expect_err("a blank selector must fail");
        assert!(error.to_string().contains("omit it to title this run"));
    }

    #[test]
    fn a_blank_title_is_rejected_like_the_server_rejects_it() {
        let error = ValidatedRunSetTitle::try_from(params(None, "   "))
            .expect_err("a blank title must fail");
        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn an_over_long_title_is_rejected_like_the_server_rejects_it() {
        let long = "x".repeat(137);
        let error = ValidatedRunSetTitle::try_from(params(None, &long))
            .expect_err("an explicit title over the cap fails, mirroring the server");
        assert!(
            error.to_string().contains("100-character limit"),
            "the refusal names the cap: {error}"
        );
        assert!(
            error.to_string().contains("137 characters"),
            "the refusal names the actual length so one trim is enough: {error}"
        );
    }

    #[test]
    fn the_text_renderer_names_the_run_and_the_title() {
        let result = RunSetTitleResult {
            run_id: "01M55K4RBQWR3D98VSR9X1P552".to_string(),
            title:  "seed fabro-9711: meaningful run titles".to_string(),
        };
        let text = set_run_title_text(&result);
        assert!(text.contains("01M55K4RBQWR3D98VSR9X1P552"));
        assert!(text.contains("seed fabro-9711: meaningful run titles"));
    }
}
