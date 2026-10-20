//! The work-item run tools (ADR-0032): claim, read, file, close, release.
//!
//! Engine-mediated by construction: the tools call the server, the server
//! owns the claim mutex and the tracker provider. Agents never hold a forge
//! credential (ADR-0019). The planner claims with `fabro_work_next`
//! (the atomic grant), reads and files items with `fabro_work_show` /
//! `fabro_work_create`, and releases a misrouted or stale claim with
//! `fabro_work_release`; `fabro_work_close` lands an item's closure.

use std::sync::Arc;

use fabro_api::types::{WorkClaim, WorkItem};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::common::{FabroToolBackend, ToolError, ToolResult};

/// `fabro_work_next`: atomically grant the next free item of a lane to a
/// run. The server picks the highest-priority unclaimed item and grants it
/// exclusively (one live claim per item and per run).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FabroWorkNextParams {
    /// The project id (the run's tracker scope).
    pub project: String,
    /// The lane to claim for, e.g. `loop`.
    pub lane:    String,
    /// The run that claims: a run id. Optional; defaults to THIS run.
    pub run:     Option<String>,
}

/// `fabro_work_show`: read one work item by its provider-stable key.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FabroWorkShowParams {
    /// The project id.
    pub project:  String,
    /// The provider-stable item key, e.g. `denkhaus/work#123`.
    #[serde(rename = "itemKey")]
    pub item_key: String,
}

/// `fabro_work_create`: file a work item in the project's tracker.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FabroWorkCreateParams {
    /// The project id.
    pub project: String,
    /// Short English title (repo rule).
    pub title:   String,
    /// The spec body.
    pub body:    String,
    /// Additional labels beyond the project label.
    #[serde(default)]
    pub labels:  Vec<String>,
}

/// `fabro_work_close`: close a work item with a reason comment.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FabroWorkCloseParams {
    /// The project id.
    pub project:  String,
    /// The provider-stable item key.
    #[serde(rename = "itemKey")]
    pub item_key: String,
    /// Why the item closes; lands as a tracker comment.
    pub reason:   String,
}

/// `fabro_work_release`: release a claim back to the queue.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FabroWorkReleaseParams {
    /// The claim id `fabro_work_next` returned.
    pub claim:  String,
    /// Why the claim is released (misroute, stale basis, planner skip).
    pub reason: String,
}

/// A validated next-claim request: the run selector is trimmed or absent
/// (the session's own run), project and lane are non-blank.
#[derive(Debug)]
pub struct ValidatedWorkNext {
    pub project: String,
    pub lane:    String,
    pub run:     Option<String>,
}

impl TryFrom<FabroWorkNextParams> for ValidatedWorkNext {
    type Error = ToolError;

    fn try_from(params: FabroWorkNextParams) -> Result<Self, Self::Error> {
        let project = non_blank(&params.project, "project")?;
        let lane = non_blank(&params.lane, "lane")?;
        let run = match params.run.as_deref() {
            Some(value) => Some(non_blank(value, "run")?),
            None => None,
        };
        Ok(Self { project, lane, run })
    }
}

fn non_blank(value: &str, field: &str) -> Result<String, ToolError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ToolError::message(format!("{field} must not be blank")));
    }
    Ok(trimmed.to_string())
}

/// The grant outcome as the tool reports it. `granted` carries the claim
/// and the item; `already_held` carries the run's live claim; `empty`
/// means the lane has no free item.
#[derive(Debug, Serialize)]
pub struct WorkNextResult {
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim:   Option<WorkClaim>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item:    Option<WorkItem>,
}

/// Resolve the target run (or the session's own when absent) and claim.
pub async fn work_next(
    backend: Arc<dyn FabroToolBackend>,
    params: ValidatedWorkNext,
) -> ToolResult<WorkNextResult> {
    let run_id = backend
        .resolve_optional_own_run(params.run.as_deref())
        .await
        .map_err(|err| ToolError::from_anyhow(&err))?;
    let response = backend
        .work_next(&params.project, &run_id, &params.lane)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))?;
    Ok(WorkNextResult {
        outcome: response.outcome.to_string(),
        claim:   response.claim,
        item:    response.item,
    })
}

pub async fn work_show(
    backend: Arc<dyn FabroToolBackend>,
    params: FabroWorkShowParams,
) -> ToolResult<WorkItem> {
    let project = non_blank(&params.project, "project")?;
    let item_key = non_blank(&params.item_key, "itemKey")?;
    backend
        .work_show(&project, &item_key)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))
}

pub async fn work_create(
    backend: Arc<dyn FabroToolBackend>,
    params: FabroWorkCreateParams,
) -> ToolResult<WorkItem> {
    let project = non_blank(&params.project, "project")?;
    let title = non_blank(&params.title, "title")?;
    let body = non_blank(&params.body, "body")?;
    backend
        .work_create(&project, &title, &body, params.labels)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))
}

pub async fn work_close(
    backend: Arc<dyn FabroToolBackend>,
    params: FabroWorkCloseParams,
) -> ToolResult<WorkItem> {
    let project = non_blank(&params.project, "project")?;
    let item_key = non_blank(&params.item_key, "itemKey")?;
    let reason = non_blank(&params.reason, "reason")?;
    backend
        .work_close(&project, &item_key, &reason)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))
}

pub async fn work_release(
    backend: Arc<dyn FabroToolBackend>,
    params: FabroWorkReleaseParams,
) -> ToolResult<WorkClaim> {
    let claim = non_blank(&params.claim, "claim")?;
    let reason = non_blank(&params.reason, "reason")?;
    backend
        .work_release(&claim, &reason)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))
}

pub fn work_next_text(result: &WorkNextResult) -> String {
    match result.outcome.as_str() {
        "granted" => {
            let item = result
                .item
                .as_ref()
                .map(|item| item.item_key.clone())
                .or_else(|| result.claim.as_ref().map(|claim| claim.item_key.clone()))
                .unwrap_or_else(|| "-".to_string());
            let claim = result
                .claim
                .as_ref()
                .map_or_else(|| "-", |claim| claim.run_id.as_str());
            format!("claimed {item} for run {claim}")
        }
        "already_held" => match &result.claim {
            Some(claim) => format!(
                "run {} already holds {} (claim {})",
                claim.run_id, claim.item_key, claim.claim_id
            ),
            None => "this run already holds an item".to_string(),
        },
        "empty" => "no free work item in this lane".to_string(),
        other => format!("work claim outcome: {other}"),
    }
}

pub fn work_show_text(result: &WorkItem) -> String {
    format!("{} — {} [{}]", result.item_key, result.title, result.state)
}

pub fn work_create_text(result: &WorkItem) -> String {
    format!("filed {} — {}", result.item_key, result.title)
}

pub fn work_close_text(result: &WorkItem) -> String {
    format!("closed {} — {}", result.item_key, result.title)
}

pub fn work_release_text(result: &WorkClaim) -> String {
    format!(
        "released claim {} on {} ({}): {}",
        result.claim_id,
        result.item_key,
        result.state,
        result.release_reason.as_deref().unwrap_or("-")
    )
}

#[cfg(test)]
mod tests {
    use fabro_api::types::WorkClaimState;

    use super::*;

    fn next_params(project: &str, lane: &str, run: Option<&str>) -> FabroWorkNextParams {
        FabroWorkNextParams {
            project: project.to_string(),
            lane:    lane.to_string(),
            run:     run.map(str::to_string),
        }
    }

    #[test]
    fn a_next_request_validates_and_trims() {
        let validated = ValidatedWorkNext::try_from(next_params(" fabro ", "loop", Some(" 01M ")))
            .expect("a normal request validates");
        assert_eq!(validated.project, "fabro");
        assert_eq!(validated.lane, "loop");
        assert_eq!(validated.run.as_deref(), Some("01M"));
    }

    #[test]
    fn omitting_the_run_targets_the_own_run() {
        let validated = ValidatedWorkNext::try_from(next_params("fabro", "loop", None))
            .expect("a normal request validates");
        assert_eq!(validated.run, None);
    }

    #[test]
    fn blank_fields_are_rejected_with_teaching_messages() {
        let error = ValidatedWorkNext::try_from(next_params("  ", "loop", None))
            .expect_err("a blank project must fail");
        assert!(error.to_string().contains("project"));
        let error = ValidatedWorkNext::try_from(next_params("fabro", "  ", None))
            .expect_err("a blank lane must fail");
        assert!(error.to_string().contains("lane"));
        let error = ValidatedWorkNext::try_from(next_params("fabro", "loop", Some("   ")))
            .expect_err("a blank run selector must fail");
        assert!(error.to_string().contains("run"));
    }

    #[test]
    fn the_text_renderers_name_the_essentials() {
        let granted = WorkNextResult {
            outcome: "granted".to_string(),
            claim:   Some(WorkClaim {
                claim_id:       uuid::Uuid::nil(),
                granted_at:     chrono::Utc::now(),
                item_key:       "denkhaus/work#1".to_string(),
                lane:           "loop".to_string(),
                pr_url:         None,
                project_id:     "fabro".to_string(),
                release_reason: None,
                released_at:    None,
                run_id:         "01M".to_string(),
                state:          WorkClaimState::Granted,
                updated_at:     chrono::Utc::now(),
            }),
            item:    None,
        };
        let text = work_next_text(&granted);
        assert!(text.contains("denkhaus/work#1"));
        assert!(text.contains("granted".to_string().as_str()) || text.contains("claimed"));

        let empty = WorkNextResult {
            outcome: "empty".to_string(),
            claim:   None,
            item:    None,
        };
        assert!(work_next_text(&empty).contains("no free work item"));
    }
}
