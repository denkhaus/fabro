use std::sync::Arc;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::common::{FabroToolBackend, RunSummaryResult, ToolError, ToolResult};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FabroRunsListParams {
    /// Workflow slug to enumerate. Required for inspects-scoped callers.
    pub workflow:      Option<String>,
    /// Inclusive lower bound on run creation time (RFC 3339).
    pub created_since: Option<String>,
}

#[derive(Debug)]
pub struct ValidatedRunsList {
    pub workflow:      Option<String>,
    pub created_since: Option<DateTime<Utc>>,
}

impl TryFrom<FabroRunsListParams> for ValidatedRunsList {
    type Error = ToolError;

    fn try_from(params: FabroRunsListParams) -> Result<Self, Self::Error> {
        let workflow = params.workflow.map(|workflow| {
            let trimmed = workflow.trim();
            if trimmed.is_empty() {
                return Err(ToolError::message("workflow must not be blank"));
            }
            Ok(trimmed.to_string())
        });
        let workflow = match workflow {
            Some(result) => Some(result?),
            None => None,
        };
        let created_since = params
            .created_since
            .as_deref()
            .map(DateTime::parse_from_rfc3339)
            .transpose()
            .map_err(|err| ToolError::message(format!("created_since must be RFC 3339: {err}")))?
            .map(|timestamp| timestamp.with_timezone(&Utc));
        Ok(Self {
            workflow,
            created_since,
        })
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RunsListResult {
    pub workflow: String,
    pub runs:     Vec<RunSummaryResult>,
}

/// Enumerate runs of one workflow (ADR-0011 revisor bookkeeping).
///
/// `inspects` is the caller's declared authority: the workflow must be
/// given and must appear in it. The server re-checks the scope.
pub async fn runs_list(
    backend: Arc<dyn FabroToolBackend>,
    params: ValidatedRunsList,
    inspects: &[String],
) -> ToolResult<RunsListResult> {
    if inspects.is_empty() {
        return Err(ToolError::message(
            "fabro_runs_list requires a graph that declares inspects",
        ));
    }
    let Some(workflow) = params.workflow.as_deref() else {
        return Err(ToolError::message(
            "workflow is required: list one of the workflows your graph declares in inspects",
        ));
    };
    if !inspects.iter().any(|declared| declared == workflow) {
        return Err(ToolError::message(format!(
            "workflow '{workflow}' is not in this workflow's inspects scope"
        )));
    }
    let runs = backend
        .list_runs_of_workflow(workflow, params.created_since)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))?;
    let runs = runs.iter().map(super::common::run_summary_result).collect();
    Ok(RunsListResult {
        workflow: workflow.to_string(),
        runs,
    })
}

pub fn runs_list_text(result: &RunsListResult) -> String {
    format!(
        "listed {} run(s) of workflow {}",
        result.runs.len(),
        result.workflow
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rejects_blank_and_unparsable_values() {
        let err = ValidatedRunsList::try_from(FabroRunsListParams {
            workflow:      Some("  ".to_string()),
            created_since: None,
        })
        .unwrap_err();
        assert!(err.as_str().contains("blank"));

        let err = ValidatedRunsList::try_from(FabroRunsListParams {
            workflow:      Some("develop".to_string()),
            created_since: Some("2026-08-31".to_string()),
        })
        .unwrap_err();
        assert!(err.as_str().contains("RFC 3339"));
    }

    #[test]
    fn validate_trims_workflow_and_parses_since() {
        let params = ValidatedRunsList::try_from(FabroRunsListParams {
            workflow:      Some(" develop ".to_string()),
            created_since: Some("2026-08-31T00:00:00Z".to_string()),
        })
        .unwrap();
        assert_eq!(params.workflow.as_deref(), Some("develop"));
        assert_eq!(
            params.created_since.map(|since| since.to_rfc3339()),
            Some("2026-08-31T00:00:00+00:00".to_string())
        );
    }
}
