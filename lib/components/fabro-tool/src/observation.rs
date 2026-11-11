//! The stage observation recorder (`fabro_observation_record`,
//! ADR-0038).
//!
//! Every agent stage can record what it observed while working — the
//! durable painpoints and notes that later close the loop from a run
//! back into the tracker. The genus is the OBSERVATION; a painpoint (a
//! friction worth acting on) and a note (context worth keeping) are its
//! two categories. The tool is available to every stage of every run —
//! conductor, revisor, and claimless runs included — because the record
//! carries its run context natively and needs no anchor convention.
//!
//! The tool call is also the PRESENCE CONTRACT (user decision,
//! ADR-0038): a node that declares `x.observations="required"` must call
//! this tool at least once; recording a note that says "nothing durable
//! to record" is the explicit empty entry. The engine validates the
//! presence at stage completion, beside the output schema.
//!
//! Only a workflow stage session can record: the entry belongs to its
//! stage (`execution`, `firing`), and surfaces without a stage (Ask
//! Fabro, `fabro exec`) are refused with a teaching error instead of
//! writing an unattributed record.

use std::fmt::Write as _;
use std::sync::Arc;

use fabro_api::types::RunObservation;
use fabro_store::platform_records::{MAX_OBSERVATION_TEXT_CHARS, StagePosition};
use fabro_types::ObservationCategory;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::common::{FabroToolBackend, ToolError, ToolResult};

/// How long one observation entry may be. Entries are one durable
/// thought, not transcripts; the refusal names the limit and the actual
/// length so one trim is enough (the denkhaus/work#1075 ergonomics).
/// The cap itself lives at the record type (`fabro-store`) — the raw
/// append endpoint enforces the same bound (rust-security F2).
pub const MAX_OBSERVATION_CHARS: usize = MAX_OBSERVATION_TEXT_CHARS;

/// The category as the tool's schema offers it to the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FabroObservationCategory {
    /// A friction worth acting on: something that slowed this stage down
    /// or will slow the next one.
    Painpoint,
    /// Context worth keeping without an action demand — including the
    /// explicit "nothing durable to record" empty entry.
    Note,
}

impl From<FabroObservationCategory> for ObservationCategory {
    fn from(category: FabroObservationCategory) -> Self {
        match category {
            FabroObservationCategory::Painpoint => Self::Painpoint,
            FabroObservationCategory::Note => Self::Note,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FabroObservationRecordParams {
    /// Whether this entry is a painpoint (friction worth acting on) or a
    /// note (context worth keeping).
    pub category: FabroObservationCategory,
    /// The entry as the stage observed it: one durable thought, trimmed
    /// and non-blank. When there is nothing durable, record a note that
    /// says so — the call itself is the presence contract.
    pub text:     String,
}

/// A validated observation entry: its category and a normalized text.
#[derive(Debug)]
pub struct ValidatedObservation {
    pub category: ObservationCategory,
    pub text:     String,
}

impl TryFrom<FabroObservationRecordParams> for ValidatedObservation {
    type Error = ToolError;

    fn try_from(params: FabroObservationRecordParams) -> Result<Self, Self::Error> {
        let text = params.text.trim();
        if text.is_empty() {
            return Err(ToolError::message(
                "observation text must not be blank; record a note saying \"nothing durable to \
                 record\" when there is nothing — the call itself satisfies the presence contract",
            ));
        }
        let char_count = text.chars().count();
        if char_count > MAX_OBSERVATION_CHARS {
            return Err(ToolError::message(format!(
                "observation text exceeds the {MAX_OBSERVATION_CHARS}-character limit: \
                 {char_count} characters"
            )));
        }
        Ok(Self {
            category: params.category.into(),
            text:     text.to_string(),
        })
    }
}

#[derive(Debug, Serialize)]
pub struct ObservationRecordResult {
    /// The run the observation was recorded for.
    pub run_id:   String,
    /// The record's per-run `seq`: the entry's identity, what the
    /// work-item link seam references.
    pub seq:      u64,
    /// The category as stored.
    pub category: ObservationCategory,
}

/// Record one observation for `run_id` from the stage `position`
/// identifies. The backend appends the `observation.recorded` platform
/// record and returns its `seq`.
pub async fn record_observation(
    backend: Arc<dyn FabroToolBackend>,
    run_id: fabro_types::RunId,
    position: StagePosition,
    validated: ValidatedObservation,
) -> ToolResult<ObservationRecordResult> {
    let seq = backend
        .record_observation(&run_id, position, validated.category, &validated.text)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))?;
    Ok(ObservationRecordResult {
        run_id: run_id.to_string(),
        seq,
        category: validated.category,
    })
}

pub fn record_observation_text(result: &ObservationRecordResult) -> String {
    format!("{} recorded at seq {}", result.category, result.seq)
}

/// `fabro_observation_list` (ADR-0039 stage D part 2): the read twin of
/// `fabro observation list` — a run's facts, so a packet's summary can
/// be verified against the records themselves.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FabroObservationListParams {
    /// The run whose observation records to read: a FULL run id (the
    /// CLI facade's own validation). A foreign run's facts are readable
    /// by design — the adjudicator verifies other runs' packets.
    pub run: String,
}

/// The observation read: the run it was read for and its records in
/// `seq` order.
#[derive(Debug, Serialize)]
pub struct ObservationListResult {
    /// The run whose facts these are.
    pub run_id:       String,
    /// The records in the server's `seq` order.
    pub observations: Vec<RunObservation>,
}

/// The target parse: a FULL run id, the rule `fabro observation list`
/// enforces on its `--run-id` — door symmetry means the tool twin
/// refuses the same argv the facade refuses.
fn parse_target_run(run: &str) -> ToolResult<fabro_types::RunId> {
    let trimmed = run.trim();
    trimmed
        .parse()
        .map_err(|err| ToolError::message(format!("{trimmed} is not a run id: {err}")))
}

/// Read one run's observation records through the backend. The target
/// must be a run id — a selector would resolve to a run the caller may
/// not have meant, and the queue's packets name ids anyway.
pub async fn list_observations(
    backend: Arc<dyn FabroToolBackend>,
    params: FabroObservationListParams,
) -> ToolResult<ObservationListResult> {
    let run_id = parse_target_run(&params.run)?;
    let observations = backend
        .list_run_observations(&run_id)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))?;
    Ok(ObservationListResult {
        run_id: run_id.to_string(),
        observations,
    })
}

/// The list rendering: count and run, then one line per record with its
/// seq, category and fact head — the full text rides the JSON payload.
pub fn list_observations_text(result: &ObservationListResult) -> String {
    if result.observations.is_empty() {
        return format!(
            "no observation records on run {} — nothing its stages recorded",
            result.run_id
        );
    }
    let mut listed = format!(
        "{} observation record(s) on run {}:",
        result.observations.len(),
        result.run_id
    );
    for observation in &result.observations {
        let head = super::common::first_line_capped(&observation.text, 120);
        let _ = write!(
            listed,
            "\nseq {} ({}): {head}",
            observation.seq, observation.category
        );
    }
    listed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(category: FabroObservationCategory, text: &str) -> FabroObservationRecordParams {
        FabroObservationRecordParams {
            category,
            text: text.to_string(),
        }
    }

    #[test]
    fn a_painpoint_entry_validates_and_trims() {
        let validated = ValidatedObservation::try_from(params(
            FabroObservationCategory::Painpoint,
            "  the \
             gate refused every push because a run PR stayed open  ",
        ))
        .expect("a normal painpoint validates");
        assert_eq!(validated.category, ObservationCategory::Painpoint);
        assert_eq!(
            validated.text,
            "the gate refused every push because a run PR stayed open"
        );
    }

    #[test]
    fn a_note_entry_keeps_its_category() {
        let validated = ValidatedObservation::try_from(params(
            FabroObservationCategory::Note,
            "nothing \
             durable to record",
        ))
        .expect("the explicit empty entry validates");
        assert_eq!(validated.category, ObservationCategory::Note);
    }

    #[test]
    fn a_blank_entry_is_refused_with_the_empty_entry_teaching() {
        let error = ValidatedObservation::try_from(params(FabroObservationCategory::Note, "  "))
            .expect_err("a blank entry must fail");
        let message = error.to_string();
        assert!(
            message.contains("nothing durable"),
            "the refusal teaches the explicit empty entry: {message}"
        );
    }

    #[test]
    fn an_over_long_entry_is_refused_with_limit_and_length() {
        let long = "x".repeat(2147);
        let error = ValidatedObservation::try_from(params(FabroObservationCategory::Note, &long))
            .expect_err("an over-long entry must fail");
        let message = error.to_string();
        assert!(
            message.contains("2000-character limit"),
            "the refusal names the limit: {message}"
        );
        assert!(
            message.contains("2147 characters"),
            "the refusal names the actual length: {message}"
        );
    }

    #[test]
    fn the_text_renderer_names_category_and_seq() {
        let result = ObservationRecordResult {
            run_id:   "01M55K4RBQWR3D98VSR9X1P552".to_string(),
            seq:      7,
            category: ObservationCategory::Painpoint,
        };
        assert_eq!(
            record_observation_text(&result),
            "painpoint recorded at seq 7"
        );
    }

    #[test]
    fn the_list_read_refuses_anything_but_a_run_id() {
        let refused =
            parse_target_run("  not-a-run-id  ").expect_err("a selector must not pass as a run id");
        assert!(
            refused.to_string().contains("is not a run id"),
            "the refusal names the CLI facade's own rule: {refused}"
        );

        let accepted = parse_target_run("  01M5844J1FDD00000000000000  ").expect("a run id parses");
        assert_eq!(accepted.to_string(), "01M5844J1FDD00000000000000");
    }

    #[test]
    fn the_list_renderer_names_count_run_and_fact_heads() {
        let listed = list_observations_text(&ObservationListResult {
            run_id:       "01M5844J1FDD".to_string(),
            observations: vec![RunObservation {
                seq:               1,
                category:          ObservationCategory::Painpoint,
                text:              "gate.refused_push\nevery push failed".to_string(),
                execution:         None,
                firing:            None,
                stage:             None,
                linked_work_items: Vec::new(),
                recorded_at:       None,
            }],
        });
        assert_eq!(
            listed.lines().next(),
            Some("1 observation record(s) on run 01M5844J1FDD:"),
            "the head names count and run: {listed}"
        );
        assert!(
            listed.contains("seq 1 (painpoint): gate.refused_push"),
            "one line per record with its seq, category and fact head: {listed}"
        );

        let empty = list_observations_text(&ObservationListResult {
            run_id:       "01M5844J1FDD".to_string(),
            observations: Vec::new(),
        });
        assert!(
            empty.contains("nothing its stages recorded"),
            "the empty arm says the run recorded nothing: {empty}"
        );
    }
}
