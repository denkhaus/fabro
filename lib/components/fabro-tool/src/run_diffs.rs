//! `fabro_run_diffs` (ADR-0051): the reviewer's discovery door over the
//! run's diff records. The engine computes each stage checkpoint's diff
//! when it commits — the `checkpoint` platform record carries the
//! `diff_summary` and the patch as a blob — and the run finish writes the
//! whole-run patch as the `run.diff` record. Until now a reader only saw
//! the truncated inline captures the context demoted; this tool lists the
//! inventory — one entry per checkpoint plus the run-level diff — and the
//! reviewer pages a patch through `fabro_blob` with the entry's blob
//! reference.
//!
//! Read-only and keyed to the calling run: the dispatcher passes the
//! session's own run id, the same worker-identity pattern as
//! `fabro_blob`, and the tool takes no arguments. The node label rides
//! the run's checkpoint timeline; a checkpoint the projection does not
//! label reports `null`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;

use fabro_api::types::{PetriPlatformRecord, RunTimelineResponse, TimelineEntryResponse};
use fabro_store::platform_records::{CheckpointRecord, PlatformRecord, RunDiffRecord};
use fabro_types::{DiffSummary, RunId, format_blob_ref};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;

use super::common::{FabroToolBackend, ToolError, ToolResult};

/// The tool's optional run selector: the inventory defaults to the
/// calling run (the dispatcher's ambient anchor, the same
/// worker-identity pattern as `fabro_blob`); a runless session has no
/// anchor and MUST pass `run_id`. The braced-shape comment on the old
/// argument-less struct is retired — the arguments stay an object on
/// the wire either way.
#[derive(Debug, serde::Deserialize, JsonSchema)]
pub struct FabroRunDiffsParams {
    /// The run whose diff inventory to read — a run id. Defaults to the
    /// calling run; required on a runless session (no anchor).
    pub run_id: Option<String>,
}

/// What one diff entry is: a stage checkpoint's diff, or the run-level
/// diff the run's pull request carries.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    strum::Display,
    serde::Serialize,
    serde::Deserialize,
    JsonSchema,
)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum RunDiffEntryKind {
    /// One stage checkpoint's diff from its parent commit.
    Checkpoint,
    /// The run-level diff: the run branch's head against its base.
    Run,
}

/// One entry of the run's diff inventory. A checkpoint entry names the
/// stage (node, position) and the commit it made; the run entry names the
/// base and head the whole-run diff is measured between. Serialize only:
/// the payload rides the tool result, and the schema is the params' job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunDiffEntry {
    /// Whether this is a stage checkpoint's diff or the run-level diff.
    pub kind:           RunDiffEntryKind,
    /// The graph node whose stage made the commit. `null` on the run-level
    /// entry and on a checkpoint the projection does not label.
    pub node:           Option<String>,
    /// The Petri execution the checkpoint belongs to, with `firing`.
    /// `null` on the run-level entry.
    pub execution:      Option<u64>,
    /// The Petri firing the checkpoint belongs to, with `execution`.
    /// `null` on the run-level entry.
    pub firing:         Option<u64>,
    /// Files changed, additions and deletions; `null` when the diff was
    /// not computed (the record still names the commit).
    pub diff_summary:   Option<DiffSummary>,
    /// The patch as a blob reference (`blob://sha256/<hex>`): page the
    /// content with `fabro_blob`. `null` when the diff is empty or was
    /// not computed.
    pub patch_blob:     Option<String>,
    /// The commit the checkpoint made. `null` on the run-level entry.
    pub git_commit_sha: Option<String>,
    /// The commit the run branch started from (run entry only).
    pub base_sha:       Option<String>,
    /// The run branch's head commit (run entry only).
    pub head_sha:       Option<String>,
}

/// The read: the run it was read for and its diff records in `seq` order,
/// checkpoints first and the run-level diff last.
#[derive(Debug, Serialize)]
pub struct RunDiffsResult {
    /// The run whose diff records these are.
    pub run_id: String,
    /// The diff inventory: one entry per checkpoint, then the run-level
    /// diff entry when the run recorded one.
    pub diffs:  Vec<RunDiffEntry>,
}

/// Read the calling run's diff inventory through the backend.
pub async fn run_diffs(
    backend: Arc<dyn FabroToolBackend>,
    run_id: RunId,
) -> ToolResult<RunDiffsResult> {
    let diffs = backend
        .list_run_diffs(&run_id)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))?;
    Ok(RunDiffsResult {
        run_id: run_id.to_string(),
        diffs,
    })
}

/// Assemble the inventory from the wire records and the checkpoint
/// timeline: checkpoints in record order, labelled through the timeline's
/// `checkpoint_seq` join, then the run-level diff entry. The ClientBackend
/// fetches the three reads; this is the pure half, so the join and the
/// record decoding are testable without a server.
#[must_use]
pub fn run_diff_entries(
    checkpoints: Vec<PetriPlatformRecord>,
    run_diffs: Vec<PetriPlatformRecord>,
    timeline: &RunTimelineResponse,
) -> Vec<RunDiffEntry> {
    let labels: BTreeMap<u64, &TimelineEntryResponse> = timeline
        .entries
        .iter()
        .map(|entry| (entry.checkpoint_seq, entry))
        .collect();
    let mut entries = Vec::new();
    for record in checkpoints {
        let Some(entry) = checkpoint_entry(&record, labels.get(&record.seq).copied()) else {
            continue;
        };
        entries.push(entry);
    }
    for record in run_diffs {
        let Ok(diff) = serde_json::from_value::<RunDiffRecord>(Value::Object(record.record)) else {
            continue;
        };
        entries.push(RunDiffEntry {
            kind:           RunDiffEntryKind::Run,
            node:           None,
            execution:      None,
            firing:         None,
            diff_summary:   diff.diff_summary,
            patch_blob:     diff.patch_blob.as_ref().map(format_blob_ref),
            git_commit_sha: None,
            base_sha:       diff.base_sha,
            head_sha:       diff.head_sha,
        });
    }
    entries
}

/// One checkpoint wire record as an inventory entry: the diff record
/// decoded beside its timeline label. A record that is not a checkpoint
/// (or does not decode) yields `None` and is skipped — the endpoint was
/// asked for checkpoints only, so this guards a skewed server answer, not
/// honest data.
#[must_use]
fn checkpoint_entry(
    record: &PetriPlatformRecord,
    label: Option<&TimelineEntryResponse>,
) -> Option<RunDiffEntry> {
    let Ok(PlatformRecord::Checkpoint(checkpoint)) =
        serde_json::from_value::<PlatformRecord>(Value::Object(record.record.clone()))
    else {
        return None;
    };
    let CheckpointRecord {
        execution,
        firing,
        git_commit_sha,
        diff_summary,
        patch_blob,
        ..
    } = checkpoint;
    Some(RunDiffEntry {
        kind: RunDiffEntryKind::Checkpoint,
        node: label
            .map(|label| label.node_name.clone())
            .filter(|node| !node.is_empty()),
        execution: Some(execution),
        firing: Some(firing),
        diff_summary,
        patch_blob: patch_blob.as_ref().map(format_blob_ref),
        git_commit_sha,
        base_sha: None,
        head_sha: None,
    })
}

/// The compact rendering: count and run, then one line per entry with its
/// kind, stage identity and diff head — the full entry rides the JSON
/// payload, and the blob reference stays whole so the model can hand it
/// straight to `fabro_blob`.
#[must_use]
pub fn run_diffs_text(result: &RunDiffsResult) -> String {
    if result.diffs.is_empty() {
        return format!(
            "no diff records on run {} — no checkpoint carried a diff and the run-level diff \
             is not recorded yet",
            result.run_id
        );
    }
    let mut listed = format!(
        "{} diff record(s) on run {}:",
        result.diffs.len(),
        result.run_id
    );
    for diff in &result.diffs {
        let stage = match (diff.kind, diff.node.as_deref(), diff.execution, diff.firing) {
            (RunDiffEntryKind::Run, ..) => "run".to_string(),
            (_, node, Some(execution), Some(firing)) => match node {
                Some(node) => format!("checkpoint {node} (execution {execution}, firing {firing})"),
                None => format!("checkpoint (execution {execution}, firing {firing})"),
            },
            (_, node, ..) => match node {
                Some(node) => format!("checkpoint {node}"),
                None => "checkpoint".to_string(),
            },
        };
        let summary = diff.diff_summary.map_or_else(
            || "no summary".to_string(),
            |summary| {
                format!(
                    "{} file(s), +{}/-{}",
                    summary.files_changed, summary.additions, summary.deletions
                )
            },
        );
        let patch = diff
            .patch_blob
            .as_deref()
            .unwrap_or("no patch blob (empty diff)");
        let _ = write!(listed, "\n{stage}: {summary} — {patch}");
    }
    listed
}

#[cfg(test)]
mod tests {
    use fabro_types::BlobHash;

    use super::*;

    /// The record JSON as the wire carries it: an object, matching the
    /// generated `PetriPlatformRecord.record` map.
    fn record_json(record: &PlatformRecord) -> serde_json::Map<String, Value> {
        match serde_json::to_value(record).expect("platform record serializes") {
            Value::Object(map) => map,
            other => panic!("platform records encode as objects: {other}"),
        }
    }

    fn checkpoint_record(
        seq: u64,
        execution: u64,
        firing: u64,
        patch: Option<BlobHash>,
    ) -> PetriPlatformRecord {
        let record = PlatformRecord::Checkpoint(CheckpointRecord {
            execution,
            firing,
            attempt: Some(1),
            workspace: None,
            git_commit_sha: Some(format!("commit{seq}")),
            diff_summary: Some(DiffSummary {
                files_changed: 3,
                additions:     12,
                deletions:     4,
            }),
            patch_blob: patch,
            operation: None,
        });
        PetriPlatformRecord {
            seq,
            recorded_at: 1_758_067_200_123,
            record: record_json(&record),
            execution: Some(execution),
            firing: Some(firing),
        }
    }

    fn run_diff_record(seq: u64) -> PetriPlatformRecord {
        let record = PlatformRecord::RunDiff(RunDiffRecord {
            base_sha:     Some("base".to_string()),
            head_sha:     Some("head".to_string()),
            diff_summary: Some(DiffSummary {
                files_changed: 9,
                additions:     40,
                deletions:     11,
            }),
            patch_blob:   Some(BlobHash::new(b"whole run")),
        });
        PetriPlatformRecord {
            seq,
            recorded_at: 1_758_067_200_456,
            record: record_json(&record),
            execution: None,
            firing: None,
        }
    }

    fn timeline_entry(seq: u64, node_name: &str) -> TimelineEntryResponse {
        TimelineEntryResponse {
            ordinal:        1,
            checkpoint_seq: seq,
            execution:      1,
            firing:         2,
            attempt:        1,
            stage:          None,
            node_name:      node_name.to_string(),
            visit:          1,
            workspace:      None,
            run_commit_sha: None,
            diff_summary:   None,
        }
    }

    #[test]
    fn the_inventory_labels_checkpoints_and_appends_the_run_diff() {
        let patch = BlobHash::new(b"stage patch");
        let entries = run_diff_entries(
            vec![
                checkpoint_record(2, 1, 2, Some(patch)),
                checkpoint_record(5, 1, 3, None),
            ],
            vec![run_diff_record(9)],
            &RunTimelineResponse {
                entries:     vec![timeline_entry(2, "implement"), timeline_entry(5, "")],
                forked_from: None,
            },
        );
        assert_eq!(entries.len(), 3, "two checkpoints then the run diff");
        assert_eq!(entries[0].kind, RunDiffEntryKind::Checkpoint);
        assert_eq!(entries[0].node.as_deref(), Some("implement"));
        assert_eq!(entries[0].execution, Some(1));
        assert_eq!(entries[0].firing, Some(2));
        assert_eq!(entries[0].git_commit_sha.as_deref(), Some("commit2"));
        assert_eq!(
            entries[0].patch_blob.as_deref(),
            Some(format_blob_ref(&patch).as_str()),
            "the patch rides as the blob reference fabro_blob parses"
        );
        assert_eq!(
            entries[0].base_sha, None,
            "checkpoint entries carry no run shas"
        );

        // An unlabeled checkpoint reports a null node, not an empty name.
        assert_eq!(entries[1].node, None);
        assert_eq!(entries[1].patch_blob, None, "an empty diff names no patch");

        assert_eq!(entries[2].kind, RunDiffEntryKind::Run);
        assert_eq!(entries[2].node, None);
        assert_eq!(entries[2].execution, None);
        assert_eq!(entries[2].base_sha.as_deref(), Some("base"));
        assert_eq!(entries[2].head_sha.as_deref(), Some("head"));
        assert!(entries[2].patch_blob.is_some());
    }

    #[test]
    fn a_non_checkpoint_record_in_the_checkpoint_answer_is_skipped() {
        let entries =
            run_diff_entries(vec![run_diff_record(4)], Vec::new(), &RunTimelineResponse {
                entries:     Vec::new(),
                forked_from: None,
            });
        assert!(
            entries.is_empty(),
            "a skewed answer must not become a bogus entry: {entries:?}"
        );
    }

    #[test]
    fn the_renderer_names_stage_summaries_and_whole_blob_references() {
        let patch = BlobHash::new(b"stage patch");
        let entries = run_diff_entries(
            vec![checkpoint_record(2, 1, 2, Some(patch))],
            vec![run_diff_record(9)],
            &RunTimelineResponse {
                entries:     vec![timeline_entry(2, "implement")],
                forked_from: None,
            },
        );
        let result = RunDiffsResult {
            run_id: "01M5844J1FDD00000000000000".to_string(),
            diffs:  entries,
        };
        let listed = run_diffs_text(&result);
        let lines: Vec<&str> = listed.lines().collect();
        assert_eq!(
            lines[0], "2 diff record(s) on run 01M5844J1FDD00000000000000:",
            "the head names count and run: {listed}"
        );
        assert!(
            lines[1].starts_with(
                "checkpoint implement (execution 1, firing 2): 3 file(s), +12/-4 — blob://sha256/"
            ),
            "the checkpoint line names stage, summary and the whole blob reference: {listed}"
        );
        assert!(
            lines[2].starts_with("run: 9 file(s), +40/-11 — blob://sha256/"),
            "the run line names the whole-run summary and its patch: {listed}"
        );

        let empty = run_diffs_text(&RunDiffsResult {
            run_id: "01M5844J1FDD00000000000000".to_string(),
            diffs:  Vec::new(),
        });
        assert!(
            empty.contains("no diff records"),
            "the empty arm says the run has nothing to page: {empty}"
        );
    }
}
