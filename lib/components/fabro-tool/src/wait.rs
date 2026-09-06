//! `fabro_run_wait`: block until one run is terminal or its pull request
//! merged (fabro-571e).
//!
//! Division of labor with `fabro_run_gather` (ADR-0016): this tool waits on
//! ONE run for up to an hour, including pull-request merge integration, on
//! a single server-side long-poll connection. `fabro_run_gather` stays the
//! short, bounded, multi-run fan-in tool.

use std::sync::Arc;

use fabro_api::types::RunWaitResultReached;
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Value, json};

use super::common::{FabroToolBackend, ToolError, ToolResult};

/// Server-side long-poll ceiling; mirrors the API spec maximum.
pub const MAX_WAIT_TIMEOUT_MS: u64 = 3_600_000;

/// Which condition ends the wait. Mirrors the API query enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunWaitUntil {
    Terminal,
    Merged,
}

#[derive(Debug, serde::Deserialize, JsonSchema)]
pub struct FabroRunWaitParams {
    /// Run id or selector of the run to wait for.
    pub run_id:     String,
    /// Wait condition: terminal run state, or merged pull request.
    pub until:      RunWaitUntil,
    /// Deadline in milliseconds (1..=3_600_000). Expiry returns
    /// reached=timeout with the current status — re-wait to continue.
    pub timeout_ms: u64,
}

#[derive(Debug)]
pub struct ValidatedRunWait {
    pub run_id:     String,
    pub until:      RunWaitUntil,
    pub timeout_ms: u64,
}

impl TryFrom<FabroRunWaitParams> for ValidatedRunWait {
    type Error = ToolError;

    fn try_from(params: FabroRunWaitParams) -> Result<Self, Self::Error> {
        let run_id = params.run_id.trim();
        if run_id.is_empty() {
            return Err(ToolError::message("run_id is required"));
        }
        if params.timeout_ms == 0 {
            return Err(ToolError::message("timeout_ms must be at least 1"));
        }
        if params.timeout_ms > MAX_WAIT_TIMEOUT_MS {
            return Err(ToolError::message(format!(
                "timeout_ms must be no more than {MAX_WAIT_TIMEOUT_MS}; split longer waits into repeated calls"
            )));
        }
        Ok(Self {
            run_id:     run_id.to_string(),
            until:      params.until,
            timeout_ms: params.timeout_ms,
        })
    }
}

/// Which condition ended a wait. Mirrors the API `reached` field.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    JsonSchema,
    strum::Display,
    strum::EnumString,
    strum::VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum RunWaitReached {
    Terminal,
    Merged,
    ClosedUnmerged,
    Timeout,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RunWaitOutcome {
    pub run_id:       String,
    pub reached:      RunWaitReached,
    /// Run status object (kind + reason) at the time the wait ended.
    pub status:       Value,
    /// Pull-request link object, present for merged-mode results.
    pub pull_request: Option<Value>,
}

impl RunWaitReached {
    fn from_api(reached: RunWaitResultReached) -> Self {
        match reached {
            RunWaitResultReached::Terminal => Self::Terminal,
            RunWaitResultReached::Merged => Self::Merged,
            RunWaitResultReached::ClosedUnmerged => Self::ClosedUnmerged,
            RunWaitResultReached::Timeout => Self::Timeout,
        }
    }
}

pub async fn run_wait(
    backend: Arc<dyn FabroToolBackend>,
    params: ValidatedRunWait,
) -> ToolResult<RunWaitOutcome> {
    let run_id = backend
        .resolve_run(&params.run_id)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))?
        .id;
    let result = backend
        .wait_run(&run_id, params.until, params.timeout_ms)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))?;
    let pull_request = result.pull_request.as_ref().map(|link| json!(link));
    Ok(RunWaitOutcome {
        run_id: result.run_id,
        reached: RunWaitReached::from_api(result.reached),
        status: json!(result.status),
        pull_request,
    })
}

pub fn run_wait_text(result: &RunWaitOutcome) -> String {
    format!("wait for run {} returned {}", result.run_id, result.reached)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_requires_run_id() {
        let err = ValidatedRunWait::try_from(FabroRunWaitParams {
            run_id:     "   ".to_string(),
            until:      RunWaitUntil::Terminal,
            timeout_ms: 1_000,
        })
        .unwrap_err();
        assert!(err.to_string().contains("run_id is required"), "{err}");
    }

    #[test]
    fn validate_rejects_zero_timeout() {
        let err = ValidatedRunWait::try_from(FabroRunWaitParams {
            run_id:     "01M1SQYDJMHSVJYMZB3C643K7J".to_string(),
            until:      RunWaitUntil::Merged,
            timeout_ms: 0,
        })
        .unwrap_err();
        assert!(err.to_string().contains("at least 1"), "{err}");
    }

    #[test]
    fn validate_rejects_timeout_above_ceiling() {
        let err = ValidatedRunWait::try_from(FabroRunWaitParams {
            run_id:     "01M1SQYDJMHSVJYMZB3C643K7J".to_string(),
            until:      RunWaitUntil::Terminal,
            timeout_ms: MAX_WAIT_TIMEOUT_MS + 1,
        })
        .unwrap_err();
        assert!(err.to_string().contains("no more than"), "{err}");
    }

    #[test]
    fn reached_enum_uses_snake_case_wire_names() {
        assert_eq!(
            RunWaitReached::ClosedUnmerged.to_string(),
            "closed_unmerged"
        );
        assert_eq!(
            serde_json::to_value(RunWaitReached::Timeout).unwrap(),
            serde_json::json!("timeout")
        );
    }
}
