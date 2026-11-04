//! The orchestration verdict marker (`fabro_run_mark`, ADR-0039 stage D,
//! denkhaus/work#1107).
//!
//! The adjudicator's and the merger's agent stages write their verdict on
//! a FOREIGN run through this tool — the tool twin of `fabro run mark`,
//! both over the same endpoint with the same authorization: the server's
//! workflow allowlist binds each gear's workflow to exactly one field
//! (`adjudicator` writes `merge`, `merger` writes `closable`). The target
//! run is ALWAYS explicit: unlike `fabro_run_set_title` there is no
//! own-run default, because a run marking itself is the conflict of
//! interest this door exists to end.

use std::sync::Arc;

use fabro_types::{OrchestrationField, OrchestrationVerdict};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::common::{FabroToolBackend, ToolError, ToolResult};

/// The wire form of [`OrchestrationField`] (schema-facing twin, the
/// `FabroObservationCategory` pattern: fabro-types carries no schemars).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Deserialize,
    Serialize,
    JsonSchema,
    strum::Display,
    strum::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum FabroOrchestrationField {
    Merge,
    Closable,
}

impl From<FabroOrchestrationField> for OrchestrationField {
    fn from(value: FabroOrchestrationField) -> Self {
        match value {
            FabroOrchestrationField::Merge => Self::Merge,
            FabroOrchestrationField::Closable => Self::Closable,
        }
    }
}

/// The wire form of [`OrchestrationVerdict`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema, strum::Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum FabroOrchestrationVerdict {
    Parked,
    Enabled,
    Refused,
}

impl From<FabroOrchestrationVerdict> for OrchestrationVerdict {
    fn from(value: FabroOrchestrationVerdict) -> Self {
        match value {
            FabroOrchestrationVerdict::Parked => Self::Parked,
            FabroOrchestrationVerdict::Enabled => Self::Enabled,
            FabroOrchestrationVerdict::Refused => Self::Refused,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FabroRunMarkParams {
    /// The TARGET run the verdict applies to: a run id. Always explicit —
    /// the adjudicator and the merger write verdicts about OTHER runs; a
    /// run marking itself is refused by the door.
    pub run:     String,
    /// Which orchestration field the verdict names: `merge` (the
    /// adjudicator's) or `closable` (the merger's). The server refuses a
    /// field the caller's workflow does not own.
    pub field:   FabroOrchestrationField,
    /// The verdict: `enabled`, `refused`, or `parked` (an open operator
    /// escalation the next pass resumes).
    pub verdict: FabroOrchestrationVerdict,
}

/// A validated verdict write: a trimmed, non-blank target selector and
/// the parsed field and verdict.
#[derive(Debug)]
pub struct ValidatedRunMark {
    pub run:     String,
    pub field:   OrchestrationField,
    pub verdict: OrchestrationVerdict,
}

impl TryFrom<FabroRunMarkParams> for ValidatedRunMark {
    type Error = ToolError;

    fn try_from(params: FabroRunMarkParams) -> Result<Self, Self::Error> {
        let run = params.run.trim().to_string();
        if run.is_empty() {
            return Err(ToolError::message(
                "run must name the TARGET run — the verdict is about another run, so the id is \
                 always explicit",
            ));
        }
        Ok(Self {
            run,
            field: params.field.into(),
            verdict: params.verdict.into(),
        })
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RunMarkResult {
    /// The run the verdict was recorded on (the target).
    pub run_id:  String,
    /// The field the verdict names.
    pub field:   FabroOrchestrationField,
    /// The verdict as recorded.
    pub verdict: FabroOrchestrationVerdict,
    /// The gear run that wrote the verdict (this session's own run).
    pub by_run:  String,
}

pub async fn mark_run(
    backend: Arc<dyn FabroToolBackend>,
    params: ValidatedRunMark,
) -> ToolResult<RunMarkResult> {
    backend
        .mark_run_orchestration(&params.run, params.field, params.verdict)
        .await
        .map(|outcome| RunMarkResult {
            run_id:  outcome.run_id,
            field:   match outcome.field {
                OrchestrationField::Merge => FabroOrchestrationField::Merge,
                OrchestrationField::Closable => FabroOrchestrationField::Closable,
            },
            verdict: match outcome.verdict {
                OrchestrationVerdict::Parked => FabroOrchestrationVerdict::Parked,
                OrchestrationVerdict::Enabled => FabroOrchestrationVerdict::Enabled,
                OrchestrationVerdict::Refused => FabroOrchestrationVerdict::Refused,
            },
            by_run:  outcome.by_run,
        })
        .map_err(|err| ToolError::from_anyhow(&err))
}

pub fn mark_run_text(result: &RunMarkResult) -> String {
    format!(
        "run {} marked {}: {} (by run {})",
        result.run_id, result.field, result.verdict, result.by_run
    )
}

/// What a successful mark call returns, as the backend reports it.
#[derive(Debug, Clone)]
pub struct RunMarkOutcome {
    pub run_id:  String,
    pub field:   OrchestrationField,
    pub verdict: OrchestrationVerdict,
    pub by_run:  String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(
        run: &str,
        field: FabroOrchestrationField,
        verdict: FabroOrchestrationVerdict,
    ) -> FabroRunMarkParams {
        FabroRunMarkParams {
            run: run.to_string(),
            field,
            verdict,
        }
    }

    #[test]
    fn a_target_run_passes_and_trims() {
        let validated = ValidatedRunMark::try_from(params(
            "  01M5844J1FDD  ",
            FabroOrchestrationField::Merge,
            FabroOrchestrationVerdict::Enabled,
        ))
        .expect("a normal mark validates");
        assert_eq!(validated.run, "01M5844J1FDD");
        assert_eq!(validated.field, OrchestrationField::Merge);
        assert_eq!(validated.verdict, OrchestrationVerdict::Enabled);
    }

    #[test]
    fn a_blank_target_is_rejected_with_a_teaching_message() {
        let error = ValidatedRunMark::try_from(params(
            "   ",
            FabroOrchestrationField::Merge,
            FabroOrchestrationVerdict::Refused,
        ))
        .expect_err("a blank target must fail");
        assert!(
            error
                .to_string()
                .contains("the verdict is about another run"),
            "the refusal teaches WHY the target is explicit: {error}"
        );
    }

    #[test]
    fn the_text_renderer_names_target_field_verdict_and_writer() {
        let result = RunMarkResult {
            run_id:  "01M5844J1FDD".to_string(),
            field:   FabroOrchestrationField::Merge,
            verdict: FabroOrchestrationVerdict::Enabled,
            by_run:  "01M58AHT2W08".to_string(),
        };
        let text = mark_run_text(&result);
        assert!(text.contains("01M5844J1FDD"));
        assert!(text.contains("merge"));
        assert!(text.contains("enabled"));
        assert!(text.contains("01M58AHT2W08"));
    }
}
