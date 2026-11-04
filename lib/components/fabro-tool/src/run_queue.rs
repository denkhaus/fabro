//! The adjudication queue reader (`fabro_run_queue`, ADR-0039 stage D
//! part 2, denkhaus/work#1107).
//!
//! Door symmetry (decision 12): the same read `fabro run queue` serves
//! on the CLI facade, over the SAME endpoint with the SAME
//! authorization — this module is the agent-tool half. The adjudicator's
//! agent pass reads the queue to re-verify a packet's summary before it
//! verdicts; the fast-path command stage reads the same door through the
//! CLI. Entries are rendered as a packet-shaped summary (run, state,
//! verdict, claim's work item, pull request) instead of the full run
//! summary: the queue is a worklist, not a run dump.

use std::fmt::Write as _;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::common::{FabroToolBackend, ToolError, ToolResult};
use crate::run_mark::FabroOrchestrationField;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FabroRunQueueParams {
    /// Which orchestration field's queue to read: `merge` (the
    /// adjudicator's pass-one queue) or `closable` (the merger's).
    pub field:    FabroOrchestrationField,
    /// Only runs of this workflow slug, e.g. `develop`. Optional — the
    /// adjudicator reads the develop line only.
    pub workflow: Option<String>,
    /// List exactly the runs settled on this verdict instead of the
    /// unsettled anti-join. Optional (stage E). `enabled` or `refused`;
    /// `field=closable` + `verdict=enabled` is the closing pass's
    /// worklist — runs the merger merged, whose work items may now
    /// close. `pending` and `parked` are refused: they ARE the default
    /// queue.
    pub verdict:  Option<FabroSettledVerdict>,
}

/// The settled-verdict filter (stage E): the two verdicts that free a
/// run from the anti-join. Pending and parked are refused at validation
/// — they name the default queue, not a filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, JsonSchema, Deserialize, strum::EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum FabroSettledVerdict {
    Enabled,
    Refused,
}

impl From<FabroSettledVerdict> for fabro_types::OrchestrationVerdict {
    fn from(value: FabroSettledVerdict) -> Self {
        match value {
            FabroSettledVerdict::Enabled => Self::Enabled,
            FabroSettledVerdict::Refused => Self::Refused,
        }
    }
}

/// A validated queue read: the parsed field, a trimmed workflow filter
/// (absent means no filter; a PRESENT-but-blank value is refused — the
/// CLI facade's own rule, door symmetry: both twins refuse the same
/// argv), and the optional settled-verdict filter.
#[derive(Debug, PartialEq, Eq)]
pub struct ValidatedRunQueue {
    pub field:    FabroOrchestrationField,
    pub workflow: Option<String>,
    pub verdict:  Option<FabroSettledVerdict>,
}

impl TryFrom<FabroRunQueueParams> for ValidatedRunQueue {
    type Error = ToolError;

    fn try_from(params: FabroRunQueueParams) -> Result<Self, Self::Error> {
        let workflow = match params.workflow.as_deref() {
            Some(value) => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    return Err(ToolError::message(
                        "workflow must not be blank; omit it to read every workflow's queue",
                    ));
                }
                Some(trimmed.to_string())
            }
            None => None,
        };
        Ok(Self {
            field: params.field,
            workflow,
            verdict: params.verdict,
        })
    }
}

/// One queue entry as the pass reads it: the run's identity beside the
/// field's verdict and the claim's work item — a packet's shape, not
/// the full run summary.
#[derive(Debug, Serialize)]
pub struct RunQueueSummary {
    /// The queued run's id.
    pub run:          String,
    /// The run's workflow slug, when it names one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow:     Option<String>,
    /// The run's lifecycle status.
    pub status:       String,
    /// The run's title.
    pub title:        String,
    /// The run's latest verdict on the queried field: `pending` (never
    /// adjudicated) or `parked` (an open operator escalation).
    pub verdict:      String,
    /// The work item of the run's latest claim, when it claimed one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_item:    Option<String>,
    /// The project of the run's latest claim, when it claimed one —
    /// scoped doors (the closure radius) read with THIS id (work#1113).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project:      Option<String>,
    /// The run's pull request URL, when it carries one — the trivial
    /// split's evidence (`enabled` with a PR, `refused` without).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<String>,
}

/// The queue read: the field it was read on and its entries in the
/// server's order.
#[derive(Debug, Serialize)]
pub struct RunQueueResult {
    /// The field the queue was read on.
    pub field:   FabroOrchestrationField,
    /// The queued runs, server order.
    pub entries: Vec<RunQueueSummary>,
}

/// Read the adjudication queue through the backend (the tool twin of
/// `fabro run queue`).
pub async fn run_queue(
    backend: Arc<dyn FabroToolBackend>,
    params: ValidatedRunQueue,
) -> ToolResult<RunQueueResult> {
    let entries = backend
        .run_queue_settled(
            params.field.into(),
            params.workflow.as_deref(),
            params.verdict.map(fabro_types::OrchestrationVerdict::from),
        )
        .await
        .map_err(|err| ToolError::from_anyhow(&err))?;
    Ok(RunQueueResult {
        field:   params.field,
        entries: entries
            .into_iter()
            .map(|entry| RunQueueSummary {
                run:          entry.run.id.to_string(),
                workflow:     entry.run.workflow.slug,
                status:       entry.run.lifecycle.status.to_string(),
                title:        entry.run.title,
                verdict:      entry.verdict.to_string(),
                work_item:    entry.work_item,
                project:      entry.project,
                pull_request: entry
                    .run
                    .pull_request
                    .as_ref()
                    .map(fabro_types::PullRequestLink::html_url),
            })
            .collect(),
    })
}

/// The list rendering: count, field, then one mechanical line per
/// entry — the shape a judgment pass scans before it verdicts.
pub fn run_queue_text(result: &RunQueueResult) -> String {
    if result.entries.is_empty() {
        return format!(
            "the {} queue is empty — every terminal run carries a settled verdict",
            result.field
        );
    }
    let mut listed = format!(
        "{} run(s) queued on {}:",
        result.entries.len(),
        result.field
    );
    for entry in &result.entries {
        let _ = write!(
            listed,
            "\n{} [{}] workflow={} verdict={} work_item={} pr={} — {}",
            entry.run,
            entry.status,
            entry.workflow.as_deref().unwrap_or("-"),
            entry.verdict,
            entry.work_item.as_deref().unwrap_or("none"),
            entry.pull_request.as_deref().unwrap_or("none"),
            entry.title,
        );
    }
    listed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(field: FabroOrchestrationField, workflow: Option<&str>) -> FabroRunQueueParams {
        FabroRunQueueParams {
            field,
            workflow: workflow.map(str::to_string),
            verdict: None,
        }
    }

    fn summary(run: &str, verdict: &str, work_item: Option<&str>) -> RunQueueSummary {
        RunQueueSummary {
            run:          run.to_string(),
            workflow:     Some("develop".to_string()),
            status:       "succeeded".to_string(),
            title:        "queued run".to_string(),
            verdict:      verdict.to_string(),
            work_item:    work_item.map(str::to_string),
            project:      work_item.map(|_| "fabro".to_string()),
            pull_request: None,
        }
    }

    #[test]
    fn the_field_parses_and_a_blank_workflow_filter_is_refused() {
        let validated = ValidatedRunQueue::try_from(params(
            FabroOrchestrationField::Merge,
            Some("  develop  "),
        ))
        .expect("a normal queue read validates");
        assert_eq!(validated.field, FabroOrchestrationField::Merge);
        assert_eq!(validated.workflow.as_deref(), Some("develop"));

        let error =
            ValidatedRunQueue::try_from(params(FabroOrchestrationField::Closable, Some("   ")))
                .expect_err("a blank filter must fail");
        assert!(
            error.to_string().contains("omit it to read every workflow"),
            "the refusal teaches the omission: {error}"
        );
    }

    #[test]
    fn the_text_renderer_lists_entries_and_the_empty_arm() {
        let listed = run_queue_text(&RunQueueResult {
            field:   FabroOrchestrationField::Merge,
            entries: vec![
                summary("01M5844J1FDD", "pending", Some("denkhaus/work#7")),
                summary("01M58AHT2W08", "parked", None),
            ],
        });
        assert_eq!(
            listed.lines().next(),
            Some("2 run(s) queued on merge:"),
            "the head names count and field: {listed}"
        );
        assert!(
            listed.contains(
                "01M5844J1FDD [succeeded] workflow=develop verdict=pending \
                 work_item=denkhaus/work#7 pr=none — queued run"
            ),
            "the line carries the packet's facts: {listed}"
        );
        assert!(
            listed.contains("verdict=parked work_item=none"),
            "a run without a claim renders its absence: {listed}"
        );

        let empty = run_queue_text(&RunQueueResult {
            field:   FabroOrchestrationField::Merge,
            entries: Vec::new(),
        });
        assert!(
            empty.contains("every terminal run carries a settled verdict"),
            "the empty arm is natural completion: {empty}"
        );
    }
}
