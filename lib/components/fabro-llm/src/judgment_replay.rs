//! The ADR-0022 wave-2 evaluation replay harness.
//!
//! Before any judgment threshold goes live (ADR-0022 point 9), this module
//! re-asks the S1 `verdict_pre_screen` question over review cycles recorded
//! in the stage journals and reports agreement against the reviewers'
//! actual verdicts. It is deliberately split in two halves:
//!
//! - a deterministic, synchronous extraction half ([`extract_cycles`],
//!   [`load_corpus`]): recover review cycles from `.fabro/journal/*.jsonl` with
//!   ground truth derived from the run's own stage sequence, and
//! - a fail-open replay half ([`replay`]): one judgment call per cycle through
//!   [`JudgmentClient`], degraded entries instead of failures, so a manual
//!   replay run never aborts on one bad cycle.
//!
//! Ground truth derivation: a reviewer visit whose run later revisits the
//! implementer (or hits a gate bounce) is a recorded `changes_requested`;
//! a reviewer visit with no later implementer activity in the same run is
//! a recorded `approved`. The stage journals carry the reviewer's journal
//! payload (painpoints/observations) and the preceding implementer's —
//! that is the evidence [`ReviewCycle::state`] hands the judgment model.
//!
//! This module never reads credentials: the API key arrives as a parameter
//! (the example binary takes it from env), and the quality gate only ever
//! exercises it against a scripted twin (twin_openai pattern) — live
//! OpenRouter runs are manual, outside the gate.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use serde_json::{Value, json};
use strum::{Display, EnumString};

use crate::judgment::{AnswerValue, JudgmentClient, JudgmentResponse, Question};

/// The recorded reviewer verdict a replayed cycle is compared against.
///
/// The journal's reviewer stage does not write a structured verdict field;
/// the ground truth is the run's own stage sequence (see the module docs).
#[derive(Clone, Copy, Debug, Display, Eq, PartialEq, EnumString, Serialize)]
#[strum(serialize_all = "snake_case")]
pub enum Verdict {
    Approved,
    ChangesRequested,
}

/// The S1 verdict pre-screen question, the same wording the judgment
/// shadow hook asks (`.fabro/scripts/judgment-shadow.nu`): one `choice`
/// question with the two reviewer outcomes plus the insufficient-evidence
/// escape hatch.
#[must_use]
pub fn s1_questions() -> BTreeMap<String, Question> {
    BTreeMap::from([("verdict_pre_screen".to_string(), Question::Choice {
        instructions: "Adjudicate the reviewed change as a whole: does \
                           the evidence support the verdict the reviewer \
                           reached?"
            .to_string(),
        criteria:     BTreeMap::from([
            (
                "approved".to_string(),
                "The change satisfies its spec; approve.".to_string(),
            ),
            (
                "changes_requested".to_string(),
                "The change has gaps; changes are requested.".to_string(),
            ),
            (
                "verification_blocked".to_string(),
                "The evidence is insufficient to judge.".to_string(),
            ),
        ]),
    })])
}

/// One review cycle recovered from a run journal: the run, the reviewer
/// visit, the recorded ground truth, and the state the judgment model is
/// re-asked over.
#[derive(Clone, Debug, PartialEq)]
pub struct ReviewCycle {
    pub run_id:         String,
    pub reviewer_visit: u32,
    pub ground_truth:   Verdict,
    pub state:          Value,
}

/// Reading or walking a journal corpus failed. Malformed journal lines are
/// NOT errors — extraction skips them — only directory-level I/O fails.
#[derive(Debug, thiserror::Error)]
#[error("reading journal corpus at {path}: {source}")]
pub struct CorpusError {
    path:   std::path::PathBuf,
    #[source]
    source: std::io::Error,
}

/// Extracts review cycles from already-read journal files.
///
/// `files` pairs a file stem (the run id) with its raw JSONL content.
/// Selection is deterministic: files sort by name, cycles within a file by
/// record order, and `limit` (when set) takes the first N cycles. Each
/// reviewer visit yields one cycle; its ground truth is `changes_requested`
/// when the same run records a later implementer or gatebounce visit,
/// `approved` otherwise.
///
/// Malformed lines are skipped (fail-open extraction, same posture as the
/// replay half).
#[must_use]
pub fn extract_cycles(files: &[(String, String)], limit: Option<usize>) -> Vec<ReviewCycle> {
    let mut sorted: Vec<&(String, String)> = files.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    let mut cycles = Vec::new();
    for (run_id, content) in sorted {
        let records: Vec<Value> = content
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        cycles.extend(run_cycles(run_id, &records));
    }
    if let Some(limit) = limit {
        cycles.truncate(limit);
    }
    cycles
}

/// Loads a journal corpus from `dir` (one `<run_id>.jsonl` per run) and
/// extracts up to `limit` cycles from it. Same semantics as
/// [`extract_cycles`]; only the file walking differs.
///
/// # Errors
/// [`CorpusError`] when the directory cannot be read.
#[expect(
    clippy::disallowed_methods,
    reason = "offline corpus extraction is a deliberate one-shot synchronous tool path, not a Tokio service"
)]
pub fn load_corpus(dir: &Path, limit: Option<usize>) -> Result<Vec<ReviewCycle>, CorpusError> {
    let entries = std::fs::read_dir(dir).map_err(|source| CorpusError {
        path: dir.to_path_buf(),
        source,
    })?;
    let mut files = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(source) => {
                return Err(CorpusError {
                    path: dir.to_path_buf(),
                    source,
                });
            }
        };
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        let run_id = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_string();
        let content =
            std::fs::read_to_string(&path).map_err(|source| CorpusError { path, source })?;
        files.push((run_id, content));
    }
    Ok(extract_cycles(&files, limit))
}

/// The cycles of one run, in record order: one per reviewer visit, each
/// with the run's later implementer activity as ground truth and the
/// journal evidence around the visit as state.
fn run_cycles(run_id: &str, records: &[Value]) -> Vec<ReviewCycle> {
    let node = |record: &Value| {
        record
            .get("node")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let later_revision = |from: usize| {
        records[from + 1..]
            .iter()
            .any(|record| matches!(node(record).as_str(), "implementer" | "gatebounce"))
    };

    let mut cycles = Vec::new();
    for (index, record) in records.iter().enumerate() {
        if node(record) != "reviewer" {
            continue;
        }
        let visit = record
            .get("visit")
            .and_then(Value::as_u64)
            .and_then(|visit| u32::try_from(visit).ok())
            .unwrap_or_default();
        let ground_truth = if later_revision(index) {
            Verdict::ChangesRequested
        } else {
            Verdict::Approved
        };
        let state = cycle_state(run_id, visit, records, index);
        cycles.push(ReviewCycle {
            run_id: run_id.to_string(),
            reviewer_visit: visit,
            ground_truth,
            state,
        });
    }
    cycles
}

/// The evidence state for one reviewer visit: the last implementer visit's
/// observations before it, plus the reviewer's own journal payload. This is
/// what the judgment model re-adjudicates.
fn cycle_state(run_id: &str, visit: u32, records: &[Value], reviewer_index: usize) -> Value {
    /// The string items of one journal payload key (`painpoints` /
    /// `observations`), empty when the key is absent.
    fn observations<'a>(record: &'a Value, key: &str) -> Vec<&'a str> {
        record
            .get("data")
            .and_then(|data| data.get(key))
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
            .unwrap_or_default()
    }
    let last_implementer = records[..reviewer_index].iter().rev().find(|record| {
        record
            .get("node")
            .and_then(Value::as_str)
            .is_some_and(|node| node == "implementer")
    });
    let reviewer = &records[reviewer_index];
    json!({
        "run_id": run_id,
        "reviewer_visit": visit,
        "implementer_observations": last_implementer
            .map(|record| observations(record, "observations"))
            .unwrap_or_default(),
        "reviewer_painpoints": observations(reviewer, "painpoints"),
        "reviewer_observations": observations(reviewer, "observations"),
    })
}

/// The replay outcome of one cycle: the model's answer when the call
/// succeeded, a degraded reason when it did not.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CycleOutcome {
    pub run_id:       String,
    pub ground_truth: Verdict,
    /// The `verdict_pre_screen` answer id, or `None` on a degraded cycle.
    pub answer:       Option<String>,
    pub confidence:   Option<f64>,
    pub cost_usd:     Option<f64>,
    /// Set on degraded cycles; `None` means the call succeeded.
    pub degraded:     Option<String>,
}

impl CycleOutcome {
    /// A degraded outcome carrying the fail-open reason.
    fn degraded(run_id: &str, ground_truth: Verdict, reason: String) -> Self {
        Self {
            run_id: run_id.to_string(),
            ground_truth,
            answer: None,
            confidence: None,
            cost_usd: None,
            degraded: Some(reason),
        }
    }

    /// Whether the replayed answer matches the recorded verdict. Degraded
    /// cycles and `verification_blocked` answers never agree.
    #[must_use]
    pub fn agrees(&self) -> bool {
        match (&self.answer, self.degraded.is_none()) {
            (Some(answer), true) => {
                matches!(
                    (answer.as_str(), self.ground_truth),
                    ("approved", Verdict::Approved)
                        | ("changes_requested", Verdict::ChangesRequested)
                )
            }
            _ => false,
        }
    }
}

/// Re-asks the S1 question over every cycle, fail-open per cycle.
///
/// A missing key degrades every cycle (`no_key`) instead of failing; a
/// per-cycle call error becomes that cycle's `degraded` reason. The
/// endpoint/model pair comes from the client — the twin tests script it,
/// manual runs pass [`JudgmentEndpoint::default`] (version-pinned model,
/// ADR-0022).
pub async fn replay(
    client: &JudgmentClient,
    api_key: Option<&str>,
    cycles: &[ReviewCycle],
) -> Vec<CycleOutcome> {
    let Some(api_key) = api_key else {
        return cycles
            .iter()
            .map(|cycle| {
                CycleOutcome::degraded(&cycle.run_id, cycle.ground_truth, "no_key".to_string())
            })
            .collect();
    };
    let questions = s1_questions();
    let mut outcomes = Vec::with_capacity(cycles.len());
    for cycle in cycles {
        let outcome = match client
            .judge(api_key, cycle.state.clone(), questions.clone())
            .await
        {
            Ok(response) => answered_outcome(cycle, &response),
            Err(error) => CycleOutcome::degraded(
                &cycle.run_id,
                cycle.ground_truth,
                error.as_llm_error().to_string(),
            ),
        };
        outcomes.push(outcome);
    }
    outcomes
}

/// Maps a successful judgment response onto a cycle outcome.
fn answered_outcome(cycle: &ReviewCycle, response: &JudgmentResponse) -> CycleOutcome {
    let answer = response
        .answers
        .get("verdict_pre_screen")
        .and_then(|answer| match &answer.answer {
            AnswerValue::Choice(choice) => Some(choice.clone()),
            AnswerValue::Score(_) | AnswerValue::Noul(_) => None,
        });
    CycleOutcome {
        run_id: cycle.run_id.clone(),
        ground_truth: cycle.ground_truth,
        confidence: response
            .answers
            .get("verdict_pre_screen")
            .and_then(|answer| answer.confidence),
        cost_usd: response.usage.as_ref().and_then(|usage| usage.cost),
        answer,
        degraded: None,
    }
}

/// The ADR-0022 autonomy floor: retry autonomy requires observed agreement
/// of at least this rate before any threshold goes live.
pub const RETRY_AUTONOMY_FLOOR: f64 = 0.85;

/// The confidence grid thresholds are derived over, in ascending order.
const CONFIDENCE_GRID: [f64; 11] = [
    0.50, 0.55, 0.60, 0.65, 0.70, 0.75, 0.80, 0.85, 0.90, 0.95, 1.00,
];

/// The smallest grid step whose bucket needs at least this many judged
/// cycles before it can propose a threshold.
const THRESHOLD_MIN_BUCKET: usize = 5;

/// The aggregated report one replay run produces: agreement, confidence
/// distribution, cost, and the thresholds derived from that data.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReplayReport {
    pub cycles: usize,
    pub judged: usize,
    pub degraded: usize,
    pub agreed: usize,
    pub agreement_rate: Option<f64>,
    /// Judged cycles per confidence bucket, keyed `"<lo>-<hi>"` (plus
    /// `"<0.50"` below the grid).
    pub confidence_distribution: BTreeMap<String, usize>,
    /// Recorded ground-truth verdicts across ALL cycles (judged and
    /// degraded) — the corpus an extract-only run can still describe.
    pub ground_truth_distribution: BTreeMap<String, usize>,
    pub total_cost_usd: f64,
    pub cost_per_cycle: Option<f64>,
    pub retry_autonomy_floor: f64,
    /// Whether the overall agreement rate meets the autonomy floor.
    pub autonomy_meets_floor: Option<bool>,
    /// The smallest confidence threshold at or above which agreement
    /// reaches the floor (needs a full bucket; `None` otherwise). Derived
    /// from this data, not vendor docs.
    pub advisory_surfacing_threshold: Option<f64>,
}

impl ReplayReport {
    /// Aggregates replay outcomes into the report. Pure and synchronous:
    /// the twin tests assert its arithmetic directly.
    #[must_use]
    pub fn from_outcomes(outcomes: &[CycleOutcome]) -> Self {
        let judged: Vec<&CycleOutcome> = outcomes
            .iter()
            .filter(|outcome| outcome.degraded.is_none())
            .collect();
        let agreed = judged.iter().filter(|outcome| outcome.agrees()).count();
        let agreement_rate = (!judged.is_empty()).then(|| agreed as f64 / judged.len() as f64);
        let total_cost_usd = outcomes.iter().filter_map(|outcome| outcome.cost_usd).sum();
        let cost_per_cycle = (!judged.is_empty()).then(|| total_cost_usd / judged.len() as f64);

        let mut confidence_distribution = BTreeMap::new();
        for outcome in &judged {
            let Some(confidence) = outcome.confidence else {
                continue;
            };
            *confidence_distribution
                .entry(confidence_bucket(confidence))
                .or_insert(0) += 1;
        }
        let mut ground_truth_distribution = BTreeMap::new();
        for ground_truth in outcomes.iter().map(|outcome| outcome.ground_truth) {
            *ground_truth_distribution
                .entry(ground_truth.to_string())
                .or_insert(0) += 1;
        }
        let total_cost_usd = if total_cost_usd == 0.0 {
            0.0
        } else {
            total_cost_usd
        };

        Self {
            cycles: outcomes.len(),
            judged: judged.len(),
            degraded: outcomes.len() - judged.len(),
            agreed,
            agreement_rate,
            confidence_distribution,
            ground_truth_distribution,
            total_cost_usd,
            cost_per_cycle,
            retry_autonomy_floor: RETRY_AUTONOMY_FLOOR,
            autonomy_meets_floor: agreement_rate.map(|rate| rate >= RETRY_AUTONOMY_FLOOR),
            advisory_surfacing_threshold: derive_surfacing_threshold(&judged),
        }
    }
}

/// The bucket label a confidence lands in.
fn confidence_bucket(confidence: f64) -> String {
    let mut previous = CONFIDENCE_GRID[0];
    if confidence < previous {
        return "<0.50".to_string();
    }
    for &step in &CONFIDENCE_GRID[1..] {
        if confidence < step {
            return format!("{previous:.2}-{step:.2}");
        }
        previous = step;
    }
    "1.00".to_string()
}

/// The smallest grid threshold where the judged cycles at or above it
/// agree at the autonomy floor, with a full-enough bucket behind it.
fn derive_surfacing_threshold(judged: &[&CycleOutcome]) -> Option<f64> {
    for &threshold in &CONFIDENCE_GRID {
        let bucket: Vec<&&CycleOutcome> = judged
            .iter()
            .filter(|outcome| outcome.confidence.is_some_and(|c| c >= threshold))
            .collect();
        if bucket.len() < THRESHOLD_MIN_BUCKET {
            continue;
        }
        let agreed = bucket.iter().filter(|outcome| outcome.agrees()).count();
        if agreed as f64 / bucket.len() as f64 >= RETRY_AUTONOMY_FLOOR {
            return Some(threshold);
        }
    }
    None
}

#[cfg(test)]
use lithos_llm::middleware::RetryPolicy;

/// A no-retry policy for twin tests: the scripted double answers on the
/// first attempt.
#[cfg(test)]
fn fast_policy() -> RetryPolicy {
    use std::time::Duration;

    RetryPolicy::exponential()
        .max_attempts(1)
        .initial_delay(Duration::from_millis(1))
        .max_delay(Duration::from_millis(2))
}

#[cfg(test)]
mod tests {
    use fabro_test::test_http_client;
    use httpmock::MockServer;
    use serde_json::json;

    use super::*;
    use crate::judgment::{JudgmentClient, JudgmentEndpoint};

    fn journal_line(node: &str, visit: u64, observations: &[&str]) -> String {
        json!({
            "$schema": "fabro-journal-v1",
            "run_id": "r1",
            "node": node,
            "visit": visit,
            "status": "succeeded",
            "data": {"painpoints": [], "observations": observations},
        })
        .to_string()
    }

    fn cycle(run_id: &str, ground_truth: Verdict) -> ReviewCycle {
        ReviewCycle {
            run_id: run_id.to_string(),
            reviewer_visit: 1,
            ground_truth,
            state: json!({"run_id": run_id}),
        }
    }

    #[test]
    fn extracts_approved_when_no_later_implementer_visit() {
        let files = [(
            "run-a".to_string(),
            [
                journal_line("start", 1, &[]),
                journal_line("implementer", 1, &["built it"]),
                journal_line("reviewer", 1, &["spec satisfied"]),
            ]
            .join("\n"),
        )];
        let cycles = extract_cycles(&files, None);
        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0].ground_truth, Verdict::Approved);
        assert_eq!(cycles[0].run_id, "run-a");
        assert_eq!(cycles[0].state["implementer_observations"][0], "built it");
        assert_eq!(
            cycles[0].state["reviewer_observations"][0],
            "spec satisfied"
        );
    }

    #[test]
    fn extracts_changes_requested_when_implementer_revisited() {
        let files = [(
            "run-b".to_string(),
            [
                journal_line("implementer", 1, &[]),
                journal_line("reviewer", 1, &["gaps remain"]),
                journal_line("gatebounce", 1, &[]),
                journal_line("implementer", 2, &["fixed"]),
                journal_line("reviewer", 2, &["now fine"]),
            ]
            .join("\n"),
        )];
        let cycles = extract_cycles(&files, None);
        assert_eq!(cycles.len(), 2);
        assert_eq!(cycles[0].ground_truth, Verdict::ChangesRequested);
        assert_eq!(cycles[0].reviewer_visit, 1);
        assert_eq!(cycles[1].ground_truth, Verdict::Approved);
        assert_eq!(cycles[1].reviewer_visit, 2);
    }

    #[test]
    fn selection_is_deterministic_and_limited() {
        let file = |id: &str| {
            (
                id.to_string(),
                [journal_line("reviewer", 1, &[])].join("\n"),
            )
        };
        let mut files = vec![file("run-z"), file("run-a")];
        files.reverse();
        let cycles = extract_cycles(&files, Some(2));
        assert_eq!(
            cycles.iter().map(|c| c.run_id.as_str()).collect::<Vec<_>>(),
            ["run-a", "run-z"],
            "files sort by name regardless of input order"
        );
        assert_eq!(extract_cycles(&files, Some(1)).len(), 1);
    }

    #[test]
    fn malformed_lines_and_foreign_nodes_are_skipped() {
        let files = [(
            "run-c".to_string(),
            [
                "not json".to_string(),
                journal_line("planner", 1, &[]),
                journal_line("reviewer", 1, &[]),
            ]
            .join("\n"),
        )];
        let cycles = extract_cycles(&files, None);
        assert_eq!(cycles.len(), 1, "only the reviewer visit yields a cycle");
    }

    #[test]
    fn report_aggregates_agreement_confidence_and_cost() {
        let outcome =
            |answer: Option<&str>, confidence: Option<f64>, cost: Option<f64>| CycleOutcome {
                run_id: "r".to_string(),
                ground_truth: Verdict::Approved,
                answer: answer.map(str::to_string),
                confidence,
                cost_usd: cost,
                degraded: None,
            };
        let outcomes = [
            outcome(Some("approved"), Some(0.92), Some(0.001)),
            outcome(Some("approved"), Some(0.61), Some(0.001)),
            outcome(Some("changes_requested"), Some(0.88), Some(0.001)),
            CycleOutcome {
                run_id:       "r".to_string(),
                ground_truth: Verdict::ChangesRequested,
                answer:       None,
                confidence:   None,
                cost_usd:     None,
                degraded:     Some("no_key".to_string()),
            },
        ];
        let report = ReplayReport::from_outcomes(&outcomes);
        assert_eq!(report.cycles, 4);
        assert_eq!(report.judged, 3);
        assert_eq!(report.degraded, 1);
        assert_eq!(report.agreed, 2);
        assert!(
            (report.agreement_rate.unwrap_or(f64::NAN) - 2.0 / 3.0).abs() < 1e-9,
            "two of three judged cycles agree"
        );
        assert!((report.total_cost_usd - 0.003).abs() < 1e-9);
        assert_eq!(report.cost_per_cycle, Some(0.001));
        assert_eq!(report.confidence_distribution.get("0.60-0.65"), Some(&1));
        assert_eq!(report.confidence_distribution.get("0.85-0.90"), Some(&1));
        assert_eq!(report.confidence_distribution.get("0.90-0.95"), Some(&1));
        assert_eq!(report.autonomy_meets_floor, Some(false));
        assert_eq!(
            report.advisory_surfacing_threshold, None,
            "no bucket holds the minimum cycle count"
        );
        assert_eq!(report.ground_truth_distribution.get("approved"), Some(&3));
        assert_eq!(
            report.ground_truth_distribution.get("changes_requested"),
            Some(&1)
        );
    }

    #[test]
    fn threshold_derives_from_the_smallest_meeting_bucket() {
        let outcome = |agrees: bool, confidence: f64| CycleOutcome {
            run_id:       "r".to_string(),
            ground_truth: if agrees {
                Verdict::Approved
            } else {
                Verdict::ChangesRequested
            },
            // Every cycle answers `approved`; a disagreeing cycle is one
            // the reviewer actually recorded as changes_requested.
            answer:       Some("approved".to_string()),
            confidence:   Some(confidence),
            cost_usd:     None,
            degraded:     None,
        };
        let outcomes: Vec<CycleOutcome> = (0..5)
            .map(|i| outcome(i < 3, 0.60)) // 3/5 = 0.6 below the floor
            .chain((0..5).map(|i| outcome(i < 5, 0.90))) // 5/5 at/above 0.90
            .collect();
        let report = ReplayReport::from_outcomes(&outcomes);
        assert_eq!(report.agreement_rate, Some(0.8));
        assert_eq!(report.autonomy_meets_floor, Some(false));
        // 0.65 is the smallest grid step whose bucket (the five cycles at
        // 0.90 confidence) agrees at the floor.
        assert_eq!(report.advisory_surfacing_threshold, Some(0.65));
    }

    #[test]
    fn s1_question_carries_the_shadow_hook_options() {
        let Question::Choice {
            instructions,
            criteria,
        } = &s1_questions()["verdict_pre_screen"]
        else {
            panic!("verdict_pre_screen is a choice question");
        };
        assert!(!instructions.is_empty());
        assert_eq!(criteria.len(), 3);
        assert!(criteria.contains_key("approved"));
        assert!(criteria.contains_key("changes_requested"));
        assert!(criteria.contains_key("verification_blocked"));
    }

    /// Twin (scripted double, no live OpenRouter, no real credentials):
    /// the replay driver answers each cycle and the report agrees.
    #[tokio::test]
    async fn replay_answers_cycles_through_the_twin() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST)
                    .path("/systemone")
                    .body_includes("\"verdict_pre_screen\":{\"type\":\"choice\"");
                then.status(200).json_body(json!({
                    "answers": {
                        "verdict_pre_screen": {
                            "type": "choice",
                            "choice": "approved",
                            "confidence": 0.9
                        }
                    },
                    "usage": {"cost": 0.00002}
                }));
            })
            .await;
        let client = JudgmentClient::new(
            test_http_client(),
            JudgmentEndpoint {
                base_url: server.url("/systemone"),
                model:    "jev-latest".to_string(),
            },
            fast_policy(),
            None,
        );
        let cycles = vec![
            cycle("run-a", Verdict::Approved),
            cycle("run-b", Verdict::ChangesRequested),
        ];
        let outcomes = replay(&client, Some("test-key"), &cycles).await;
        assert_eq!(outcomes.len(), 2);
        assert_eq!(outcomes[0].answer.as_deref(), Some("approved"));
        assert!(outcomes[0].agrees());
        assert!(!outcomes[1].agrees(), "approved answer vs recorded changes");
        let report = ReplayReport::from_outcomes(&outcomes);
        assert_eq!(report.agreement_rate, Some(0.5));
        assert_eq!(report.cost_per_cycle, Some(0.00002));
    }

    /// Fail-open: a missing key degrades every cycle instead of failing.
    #[tokio::test]
    async fn replay_without_key_degrades_every_cycle() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|_when, then| {
                then.status(200).json_body(json!({
                    "answers": {
                        "verdict_pre_screen": {"answer": "approved", "confidence": 0.9}
                    }
                }));
            })
            .await;
        let client = JudgmentClient::new(
            test_http_client(),
            JudgmentEndpoint {
                base_url: server.url("/systemone"),
                model:    "jev-latest".to_string(),
            },
            fast_policy(),
            None,
        );
        let outcomes = replay(&client, None, &[cycle("run-a", Verdict::Approved)]).await;
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].degraded.as_deref(), Some("no_key"));
        assert!(!outcomes[0].agrees());
    }

    /// Fail-open: an endpoint that always 500s degrades each cycle with the
    /// provider error, and the report still aggregates.
    #[tokio::test]
    async fn replay_degrades_each_cycle_on_provider_errors() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|_when, then| {
                then.status(500).body("upstream unavailable");
            })
            .await;
        let endpoint = JudgmentEndpoint {
            base_url: server.url("/systemone"),
            model:    "jev-latest".to_string(),
        };
        let client = JudgmentClient::new(test_http_client(), endpoint, fast_policy(), None);
        let outcomes = replay(&client, Some("test-key"), &[cycle(
            "run-a",
            Verdict::Approved,
        )])
        .await;
        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].degraded.is_some());
        let report = ReplayReport::from_outcomes(&outcomes);
        assert_eq!(report.judged, 0);
        assert_eq!(report.agreement_rate, None);
    }
}
