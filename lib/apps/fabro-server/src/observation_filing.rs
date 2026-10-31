//! Observation-derived work-item filing — the run-end hook (ADR-0038
//! stage 4).
//!
//! When a run reaches a terminal state and recorded at least one
//! painpoint observation, ONE distillation call (a fixed
//! settings-declared model, the run-title posture) turns the fresh
//! painpoint records into work-item candidates, and a mechanical filer
//! files them with dedupe at every layer: an observation already linked
//! files nothing (identity dedupe), an open item covering the same
//! concrete change gets LINKED instead of duplicated (change dedupe),
//! and the tracker is re-queried immediately before every create (the
//! closeout-race freshness guard). The filing rides the work outbox:
//! the terminal path only enqueues, failures retry with the
//! reconciler's backoff, and the idempotency key makes double terminal
//! records (a worker `succeeded` plus a server `dead`) harmless.
//!
//! Filing is setting-gated (`[server.observations] filing_enabled`,
//! default OFF): development and bench stacks record observations but
//! never file — only an operator's `settings.toml` turns the hook on.
//! The hook's own friction rides the same channel: a persistently
//! failing entry records one positionless painpoint on its run, and the
//! retry re-reads and files it like any other painpoint.
//!
//! The retired revisor workflow's filing discipline — the actionability
//! filter, consolidation, the tracker search before create, the
//! filing-time freshness re-query — is ported into the distillation
//! prompt and this filer; the revisor retired in the same change (its
//! input channel, the journal painpoint array, no longer exists).

use std::time::Duration;

use anyhow::Context as _;
use fabro_llm::{Client, Request};
use fabro_store::platform_records::{
    MAX_OBSERVATION_TEXT_CHARS, ObservationRecordedRecord, PlatformRecord, PlatformRecordKind,
    RunLifecycleKind,
};
use fabro_template::{TemplateContext, TemplateError};
use fabro_types::settings::server::ServerObservationsSettings;
use fabro_types::{ObservationCategory, RunId, RunProjection, RunTarget};
use fabro_work::{TrackerProvider, WorkItem, WorkItemDraft, WorkItemFilter};
use lithos_llm::catalog::ProviderId;
use serde_json::Value as JsonValue;

use crate::server::{AppState, run_records};

/// The outbox kind the terminal path enqueues and the reconciler drain
/// consumes. Unknown kinds are completed with a warning by the drain,
/// so this arm and the kind string must travel together.
pub(crate) const OUTBOX_KIND: &str = "observation.filing";

/// The output budget and deadline for one distillation call. The
/// structured small default (zai glm-4.7) is ALWAYS-REASONING and
/// reasoning tokens come out of the same budget — the floor mirrors the
/// run-title lesson (fabro-d5b1: a starving cap empties every call).
const FILING_MAX_OUTPUT_TOKENS: u32 = 2_048;
const FILING_MIN_REASONING_SAFE_TOKENS: u32 = 512;
const _: () = assert!(FILING_MAX_OUTPUT_TOKENS >= FILING_MIN_REASONING_SAFE_TOKENS);
const FILING_TIMEOUT: Duration = Duration::from_mins(1);

const FILING_PROMPT_NAME: &str = "observation_filing.md.j2";
const FILING_PROMPT_TEMPLATE: &str = include_str!("prompts/observation_filing.md.j2");

/// Payload firing keys that match no real stage: a server-origin
/// observation stays stream-readable (the kind query below finds it)
/// but out of the stage view, like an artifact for an unknown firing.
pub(crate) const POSITIONLESS_EXECUTION: u64 = u64::MAX;
pub(crate) const POSITIONLESS_FIRING: u64 = u64::MAX;

/// How many tracker hits the dedupe layers inspect per query.
const DEDUPE_QUERY_LIMIT: usize = 20;

/// The gate every terminal lifecycle append passes through (the worker
/// HTTP path and server-origin records alike): when filing is enabled,
/// one durable outbox enqueue names the run — nothing else happens on
/// the terminal path. Idempotent: a second terminal record for the same
/// run enqueues nothing new.
pub(crate) async fn enqueue_filing_if_terminal(
    state: &AppState,
    run_id: RunId,
    record: &PlatformRecord,
) {
    let PlatformRecord::RunLifecycle(record) = record else {
        return;
    };
    let terminal = matches!(
        record.transition,
        RunLifecycleKind::Succeeded | RunLifecycleKind::Failed | RunLifecycleKind::Dead
    );
    if !terminal {
        return;
    }
    if !state.server_settings().server.observations.filing_enabled {
        return;
    }
    let key = format!("observation-filing:{run_id}");
    let payload = serde_json::json!({ "run_id": run_id.to_string() });
    if let Err(error) = state.work_store().enqueue(OUTBOX_KIND, payload, &key).await {
        tracing::warn!(
            run_id = %run_id,
            error = %error,
            "observation filing enqueue failed"
        );
    }
}

/// What one drain of a filing entry produced.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FilingOutcome {
    /// The entry completes: nothing to file (each reason is a named,
    /// legitimate no-op, not an error).
    Nothing { reason: &'static str },
    /// The entry completes: candidates were filed and/or existing items
    /// were linked.
    Filed {
        items:           usize,
        linked_existing: usize,
    },
}

/// One filing pass for a terminal run: the reconciler's consumer arm.
/// `Ok` completes the outbox entry; `Err` fails it with the drain's
/// backoff, and persistent failures grow a friction record on the run
/// (see [`record_friction_once`]).
pub(crate) async fn file_run_observations(
    state: &AppState,
    run_id: &RunId,
) -> anyhow::Result<FilingOutcome> {
    // The run's project: the spec's git target names the repository, and
    // the work project whose target_repo matches owns the tracker. A run
    // nothing tracks (a bench repo, a folder target) is a no-op.
    let Some(projection) = run_records::projection(state, *run_id)
        .await
        .context("reading the run projection")?
    else {
        return Ok(FilingOutcome::Nothing {
            reason: "run record gone",
        });
    };
    let Some(target_repo) = git_target_repo(&projection) else {
        return Ok(FilingOutcome::Nothing {
            reason: "run has no git target",
        });
    };
    let projects = state
        .work_store()
        .projects()
        .await
        .context("listing work projects")?;
    let Some(project) = projects
        .iter()
        .find(|project| project.target_repo.as_deref() == Some(target_repo))
    else {
        return Ok(FilingOutcome::Nothing {
            reason: "no work project targets the run's repository",
        });
    };

    // The run's painpoint observations — notes never file.
    let records = state
        .stores
        .run_summaries
        .platform_records()
        .read_kind(run_id, PlatformRecordKind::ObservationRecorded)
        .await
        .context("reading observation records")?;
    let painpoints: Vec<(u64, String)> = records
        .iter()
        .filter_map(|stored| match &stored.record {
            PlatformRecord::ObservationRecorded(record)
                if record.category == ObservationCategory::Painpoint =>
            {
                Some((stored.seq, record.text.clone()))
            }
            _ => None,
        })
        .collect();
    if painpoints.is_empty() {
        return Ok(FilingOutcome::Nothing {
            reason: "no painpoint observations",
        });
    }

    // Identity dedupe: an observation already linked files nothing — the
    // link is the record of a filed painpoint, whatever item it names.
    let pairs: Vec<(RunId, u64)> = painpoints.iter().map(|(seq, _)| (*run_id, *seq)).collect();
    let links = state
        .work_store()
        .linked_items_for_observations(&project.project_id, &pairs)
        .await
        .context("reading observation links")?;
    let fresh: Vec<(u64, String)> = painpoints
        .into_iter()
        .filter(|(seq, _)| !links.iter().any(|link| link.seq == *seq))
        .collect();
    if fresh.is_empty() {
        return Ok(FilingOutcome::Nothing {
            reason: "every painpoint already linked",
        });
    }

    // One distillation call over the fresh painpoints.
    let inputs = FilingInputs {
        run_id:           *run_id,
        workflow:         projection.spec.graph.name.clone(),
        goal:             projection.spec.graph.goal().to_string(),
        target:           git_target_repo(&projection).unwrap_or_default().to_string(),
        workflow_version: projection
            .spec
            .workflow_version_id
            .map_or_else(|| "absent".to_string(), |id| id.to_string()),
        target_sha:       projection
            .spec
            .target
            .as_ref()
            .and_then(git_target_sha)
            .unwrap_or_else(|| "absent".to_string()),
    };
    let candidates = distill(state, &inputs, &fresh).await?;
    if candidates.is_empty() {
        return Ok(FilingOutcome::Nothing {
            reason: "distillation produced no candidates",
        });
    }

    let tracker = state
        .work_tracker_for_project(project)
        .await
        .context("resolving the project's work tracker")?;
    let mut items = 0usize;
    let mut linked_existing = 0usize;
    for candidate in candidates {
        match file_candidate(state, tracker.as_ref(), project, &inputs, &candidate).await? {
            CandidateOutcome::Filed => items += 1,
            CandidateOutcome::LinkedExisting => linked_existing += 1,
            CandidateOutcome::Dropped => {}
        }
    }
    Ok(FilingOutcome::Filed {
        items,
        linked_existing,
    })
}

#[derive(Debug, PartialEq, Eq)]
enum CandidateOutcome {
    Filed,
    LinkedExisting,
    Dropped,
}

/// One candidate through the dedupe layers: freshness (items citing
/// THIS run), change-dedupe (an open item covering the same concrete
/// change gets linked; a closed one means the change already landed),
/// then create-and-link.
async fn file_candidate(
    state: &AppState,
    tracker: &dyn TrackerProvider,
    project: &fabro_work::WorkProject,
    inputs: &FilingInputs,
    candidate: &Candidate,
) -> anyhow::Result<CandidateOutcome> {
    let run_id = inputs.run_id;
    // Freshness: the tracker as it is NOW, queried by the run id — the
    // run's own closeout can have filed items in the window since the
    // distillation snapshot.
    let citing = tracker
        .list(project, WorkItemFilter {
            query: Some(run_id.to_string()),
            limit: DEDUPE_QUERY_LIMIT,
            ..WorkItemFilter::default()
        })
        .await
        .context("freshness query failed")?;
    // Change-dedupe: the candidate's title is the focused theme query.
    let found = tracker
        .list(project, WorkItemFilter {
            query: Some(candidate.title.clone()),
            limit: DEDUPE_QUERY_LIMIT,
            ..WorkItemFilter::default()
        })
        .await
        .context("change-dedupe query failed")?;
    if let Some(covering) = covering_item(citing.iter().chain(found.iter()), candidate) {
        if covering.state != "open" {
            // A closed item covering the change: the change is already
            // implemented, the candidate has no filing claim.
            tracing::debug!(
                item = %covering.item_key,
                "filing drop: a closed item already covers the change"
            );
            return Ok(CandidateOutcome::Dropped);
        }
        for seq in &candidate.source_seqs {
            state
                .work_store()
                .link_observation(&project.project_id, &covering.item_key, &run_id, *seq)
                .await
                .context("linking the covering item to its observations")?;
        }
        tracing::info!(
            item = %covering.item_key,
            run_id = %run_id,
            "observation filing linked an existing item"
        );
        return Ok(CandidateOutcome::LinkedExisting);
    }

    let created = tracker
        .create(project, WorkItemDraft {
            title:  candidate.title.clone(),
            body:   candidate_body(
                &inputs.run_id,
                &inputs.workflow_version,
                &inputs.target_sha,
                candidate,
            ),
            labels: draft_labels(project, candidate),
        })
        .await
        .context("creating the work item")?;
    for seq in &candidate.source_seqs {
        state
            .work_store()
            .link_observation(&project.project_id, &created.item_key, &run_id, *seq)
            .await
            .context("linking the created item to its observations")?;
    }
    tracing::info!(
        item = %created.item_key,
        run_id = %run_id,
        "observation filing created a work item"
    );
    Ok(CandidateOutcome::Filed)
}

/// One distilled candidate: what/where/effect plus the observation seqs
/// it carries (the link's identity).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub(crate) title:       String,
    pub(crate) body:        String,
    pub(crate) priority:    u8,
    pub(crate) source_seqs: Vec<u64>,
    pub(crate) needs_user:  bool,
}

/// The plain facts one distillation call needs — the projection's
/// run-identifying fields, unpacked once.
struct FilingInputs {
    run_id:           RunId,
    workflow:         String,
    goal:             String,
    target:           String,
    workflow_version: String,
    target_sha:       String,
}

/// The distillation: one structured LLM call over the fresh painpoints.
/// Any failure is an outbox retry — a checked-in template that will not
/// render is a bug this surfaces loudly, not a silent degrade.
async fn distill(
    state: &AppState,
    inputs: &FilingInputs,
    fresh: &[(u64, String)],
) -> anyhow::Result<Vec<Candidate>> {
    let (client, provider_id, model_id) = resolve_distill_model(state).await?;
    let prompt = build_prompt(inputs, fresh)?;
    let request = Request::builder()
        .model(format!("{provider_id}/{model_id}"))
        .user(prompt)
        .max_output_tokens(FILING_MAX_OUTPUT_TOKENS)
        .timeout(FILING_TIMEOUT)
        .build()
        .context("building the distillation request")?;
    let completion = client
        .complete_object(request, "observation_filing", response_schema())
        .await
        .context("the distillation call failed")?;
    Ok(parse_candidates(&completion.object, fresh))
}

/// The filing model: the settings-declared `provider:model`, else the
/// catalog's structured small default (the run-title posture).
async fn resolve_distill_model(state: &AppState) -> anyhow::Result<(Client, ProviderId, String)> {
    let (resolved, ready) = state.resolve_llm_client_with_ready_ids().await;
    let resolved = resolved.context("resolving the LLM client")?;
    let settings: ServerObservationsSettings = state.server_settings().server.observations.clone();
    let catalog = state.catalog();
    if let Some(declared) = settings.filing_model.as_deref() {
        let (provider, model) = declared
            .split_once(':')
            .with_context(|| format!("filing_model must be provider:model, got {declared:?}"))?;
        let provider_id = ProviderId::new(provider);
        anyhow::ensure!(
            ready.contains(&provider_id),
            "filing model provider {provider:?} has no ready credentials"
        );
        catalog
            .model(provider, model)
            .with_context(|| format!("filing model {declared:?} is not in the catalog"))?;
        return Ok((resolved.client, provider_id, model.to_string()));
    }
    let small = catalog
        .small_default_for(&ready)
        .context("no structured small default is ready for the filing distillation")?;
    Ok((
        resolved.client,
        small.provider.id().clone(),
        small.model.id().to_string(),
    ))
}

fn build_prompt(inputs: &FilingInputs, fresh: &[(u64, String)]) -> Result<String, TemplateError> {
    let observations = fresh
        .iter()
        .map(|(seq, text)| format!("- [{seq}] {text}"))
        .collect::<Vec<_>>()
        .join("\n");
    let entries = [
        ("run_id".to_string(), inputs.run_id.to_string()),
        ("workflow".to_string(), inputs.workflow.clone()),
        ("goal".to_string(), inputs.goal.clone()),
        ("target".to_string(), inputs.target.clone()),
        ("observations".to_string(), observations),
    ];
    let template_inputs = entries
        .into_iter()
        .map(|(key, value)| (key, toml::Value::String(value)))
        .collect();
    let ctx = TemplateContext::new().with_inputs(template_inputs);
    fabro_template::render_named(FILING_PROMPT_NAME, FILING_PROMPT_TEMPLATE, &ctx)
}

/// Parse and harden the distillation output against the fresh seqs:
/// unknown source seqs drop (never link what this pass did not distill),
/// malformed entries drop, an empty result is valid.
pub(crate) fn parse_candidates(object: &JsonValue, fresh: &[(u64, String)]) -> Vec<Candidate> {
    let Some(raw) = object.get("candidates").and_then(JsonValue::as_array) else {
        return Vec::new();
    };
    raw.iter()
        .filter_map(|entry| {
            let title = entry.get("title")?.as_str()?.trim().to_string();
            let body = entry.get("body")?.as_str()?.trim().to_string();
            if title.is_empty() || body.is_empty() {
                return None;
            }
            let priority = match entry.get("priority").and_then(JsonValue::as_u64) {
                Some(1) => 1,
                _ => 2,
            };
            let needs_user = entry
                .get("needs_user")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false);
            let source_seqs: Vec<u64> = entry
                .get("source_seqs")?
                .as_array()?
                .iter()
                .filter_map(JsonValue::as_u64)
                .filter(|seq| fresh.iter().any(|(fresh_seq, _)| fresh_seq == seq))
                .collect();
            if source_seqs.is_empty() {
                return None;
            }
            Some(Candidate {
                title,
                body,
                priority,
                source_seqs,
                needs_user,
            })
        })
        .collect()
}

fn response_schema() -> JsonValue {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["candidates"],
        "properties": {
            "candidates": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["title", "body", "priority", "source_seqs"],
                    "properties": {
                        "title": {
                            "type": "string",
                            "description": "Short imperative, English"
                        },
                        "body": {
                            "type": "string",
                            "description": "What/where/effect, grounded in the observations"
                        },
                        "priority": { "type": "integer", "enum": [1, 2] },
                        "source_seqs": {
                            "type": "array",
                            "items": { "type": "integer" },
                            "description": "The observation seqs this candidate distills"
                        },
                        "needs_user": { "type": "boolean" }
                    }
                }
            }
        }
    })
}

fn candidate_body(
    run_id: &RunId,
    workflow_version: &str,
    target_sha: &str,
    candidate: &Candidate,
) -> String {
    use std::fmt::Write as _;
    let mut body = candidate.body.clone();
    body.push_str("\n\nPriority: ");
    body.push_str(match candidate.priority {
        1 => "1",
        _ => "2",
    });
    let seqs = candidate
        .source_seqs
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let _ = write!(body, "\nObservations: run {run_id} seqs {seqs}");
    if candidate.needs_user {
        body.push_str("\nCapability-affecting: implementation awaits explicit user approval.");
    }
    let _ = write!(
        body,
        "\nBasis: run {run_id}, workflow version {workflow_version}, commit {target_sha}"
    );
    body
}

fn draft_labels(project: &fabro_work::WorkProject, candidate: &Candidate) -> Vec<String> {
    let mut labels = vec![project.project_label.clone(), "revision".to_string()];
    if candidate.needs_user {
        labels.push("needs-user".to_string());
    }
    labels
}

fn git_target_repo(projection: &RunProjection) -> Option<&str> {
    match projection.spec.target.as_ref()? {
        RunTarget::Git(git) => Some(&git.repo),
        _ => None,
    }
}

fn git_target_sha(target: &RunTarget) -> Option<String> {
    match target {
        RunTarget::Git(git) => git.sha.clone(),
        _ => None,
    }
}

/// Token words too common to carry meaning for the covering check.
const COVER_STOPWORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "of", "in", "to", "for", "on", "with", "is", "are", "be",
    "when", "that", "this", "it", "its", "into", "from", "by", "at", "as",
];

/// Normalize a title to its significant tokens: lowercase alphanumeric
/// words minus stopwords — the covering check's comparison basis.
fn significant_tokens(title: &str) -> Vec<String> {
    title
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token: &&str| token.len() > 2 && !COVER_STOPWORDS.contains(token))
        .map(ToString::to_string)
        .collect()
}

/// The conservative covering predicate — over-linking pollutes closure
/// radii, so an item covers only on exact normalized-title equality or
/// when EVERY significant candidate token (three or more) appears in
/// its title or body. First covering item wins (deterministic order:
/// the caller chains freshness hits before theme hits).
pub(crate) fn covering_item<'a>(
    mut hits: impl Iterator<Item = &'a WorkItem>,
    candidate: &Candidate,
) -> Option<&'a WorkItem> {
    let candidate_tokens = significant_tokens(&candidate.title);
    hits.find(|item| {
        let item_title_tokens = significant_tokens(&item.title);
        if item_title_tokens == candidate_tokens {
            return true;
        }
        candidate_tokens.len() >= 3 && {
            let hay = format!("{} {}", item.title, item.body).to_lowercase();
            candidate_tokens
                .iter()
                .all(|token| hay.contains(token.as_str()))
        }
    })
}

/// Record the hook's own friction as a positionless painpoint on the
/// run, once per outbox entry: the next retry re-reads it with the
/// other painpoints and the machine files its own failure like any
/// demand. Best-effort — a failure here only logs.
pub(crate) async fn record_friction_once(
    state: &AppState,
    run_id: RunId,
    entry_id: &str,
    error: &str,
) {
    let marker = format!("run-end observation filing failed (outbox {entry_id})");
    let existing = state
        .stores
        .run_summaries
        .platform_records()
        .read_kind(&run_id, PlatformRecordKind::ObservationRecorded)
        .await;
    if let Ok(records) = existing {
        let already = records.iter().any(|stored| match &stored.record {
            PlatformRecord::ObservationRecorded(record) => record.text.starts_with(&marker),
            _ => false,
        });
        if already {
            return;
        }
    }
    let mut text = format!("{marker}: {error}. Fix the filing pipeline; the outbox entry retries.");
    let cap = MAX_OBSERVATION_TEXT_CHARS.saturating_sub(1);
    if text.chars().count() > cap {
        text = text.chars().take(cap).collect();
    }
    let record = PlatformRecord::ObservationRecorded(ObservationRecordedRecord {
        category: ObservationCategory::Painpoint,
        text,
        execution: POSITIONLESS_EXECUTION,
        firing: POSITIONLESS_FIRING,
    });
    if let Err(append_error) = run_records::append(state, run_id, record).await {
        tracing::warn!(
            run_id = %run_id,
            error = %append_error,
            "recording the filing friction failed"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use chrono::{Duration, Utc};
    use fabro_store::platform_records::{PlatformRecord, RunLifecycleKind, RunLifecycleRecord};
    use fabro_types::{ObservationCategory, RunStatus, SuccessReason};
    use fabro_work::{WorkItem, WorkItemDraft, WorkItemFilter, WorkProject};
    use tokio::sync::Mutex;

    use super::*;
    use crate::test_support::{TestAppStateBuilder, default_test_server_settings};

    fn inputs() -> FilingInputs {
        FilingInputs {
            run_id:           RunId::new(),
            workflow:         "Develop".to_string(),
            goal:             "Work the queue".to_string(),
            target:           "denkhaus/fabro".to_string(),
            workflow_version: "abc123".to_string(),
            target_sha:       "deadbeef".to_string(),
        }
    }

    fn candidate(title: &str, seqs: &[u64]) -> Candidate {
        Candidate {
            title:       title.to_string(),
            body:        "The concrete change and its effect.".to_string(),
            priority:    2,
            source_seqs: seqs.to_vec(),
            needs_user:  false,
        }
    }

    #[test]
    fn prompt_renders_every_variable_and_carries_the_observations() {
        let inputs = inputs();
        let fresh = vec![
            (7, "first painpoint".to_string()),
            (9, "second painpoint".to_string()),
        ];
        let prompt = build_prompt(&inputs, &fresh).unwrap();
        assert!(prompt.contains(&inputs.run_id.to_string()));
        assert!(prompt.contains("Develop"));
        assert!(prompt.contains("denkhaus/fabro"));
        assert!(prompt.contains("- [7] first painpoint"));
        assert!(prompt.contains("- [9] second painpoint"));
        assert!(!prompt.contains("{{"));
    }

    #[test]
    fn parse_candidates_keeps_valid_entries_and_hardens_defaults() {
        let fresh = vec![(3, "a".to_string()), (5, "b".to_string())];
        let object = serde_json::json!({
            "candidates": [
                {
                    "title": "Fix the scanner slice arithmetic",
                    "body": "Carry the absolute cursor beside the relative offset.",
                    "priority": 1,
                    "source_seqs": [3, 5]
                },
                {
                    "title": "Defaults applied",
                    "body": "No priority, no needs_user flag.",
                    "source_seqs": [3]
                }
            ]
        });
        let parsed = parse_candidates(&object, &fresh);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].priority, 1);
        assert!(!parsed[0].needs_user);
        assert_eq!(
            parsed[1].priority, 2,
            "an unknown priority hardens to normal"
        );
    }

    #[test]
    fn parse_candidates_drops_entries_without_fresh_source_seqs() {
        let fresh = vec![(3, "a".to_string())];
        let object = serde_json::json!({
            "candidates": [
                { "title": "Unknown seq", "body": "b", "priority": 2, "source_seqs": [99] },
                { "title": "No seqs", "body": "b", "priority": 2, "source_seqs": [] },
                { "title": "", "body": "blank title", "priority": 2, "source_seqs": [3] },
                { "title": "Valid", "body": "b", "priority": 2, "source_seqs": [3] }
            ]
        });
        let parsed = parse_candidates(&object, &fresh);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].title, "Valid");
    }

    #[test]
    fn parse_candidates_accepts_an_empty_result() {
        let parsed = parse_candidates(&serde_json::json!({"candidates": []}), &[]);
        assert!(parsed.is_empty());
        let missing = parse_candidates(&serde_json::json!({}), &[]);
        assert!(missing.is_empty());
    }

    #[test]
    fn covering_item_matches_exact_normalized_titles() {
        let item = work_item(
            "denkhaus/work#1",
            "Fix the Scanner Slice Arithmetic",
            "open",
            "",
        );
        let hits = [item];
        let found = covering_item(
            hits.iter(),
            &candidate("fix the scanner slice arithmetic", &[1]),
        );
        assert!(found.is_some(), "normalized equality covers");
    }

    #[test]
    fn covering_item_requires_every_significant_token() {
        let item = work_item(
            "denkhaus/work#1",
            "Fix the scanner offset drift",
            "open",
            "The relative index walks a suffix; slice arithmetic must carry \
             the absolute cursor beside it.",
        );
        let near = [work_item("denkhaus/work#2", "Unrelated", "open", "scanner")];
        // Every significant token present in title+body: covers.
        let hits = [item];
        assert!(
            covering_item(
                hits.iter(),
                &candidate("Fix scanner slice arithmetic drift", &[1])
            )
            .is_some()
        );
        // Only one of four tokens present: does not cover.
        assert!(
            covering_item(
                near.iter(),
                &candidate("Fix scanner slice arithmetic drift", &[1])
            )
            .is_none()
        );
    }

    #[test]
    fn covering_item_needs_three_significant_tokens_for_subset_matches() {
        let item = work_item(
            "denkhaus/work#1",
            "Widget",
            "open",
            "body mentions calibrate",
        );
        let hits = [item];
        // Two significant tokens: below the subset floor, equality is the
        // only path — no cover.
        assert!(covering_item(hits.iter(), &candidate("Calibrate widget", &[1])).is_none());
    }

    fn work_item(key: &str, title: &str, state: &str, body: &str) -> WorkItem {
        WorkItem {
            item_key:   key.to_string(),
            title:      title.to_string(),
            body:       body.to_string(),
            state:      state.to_string(),
            labels:     Vec::new(),
            priority:   2,
            url:        String::new(),
            updated_at: None,
        }
    }

    #[test]
    fn candidate_body_carries_priority_provenance_and_basis() {
        let inputs = inputs();
        let body = candidate_body(
            &inputs.run_id,
            &inputs.workflow_version,
            &inputs.target_sha,
            &Candidate {
                needs_user: true,
                ..candidate("T", &[4, 8])
            },
        );
        assert!(body.contains("Priority: 2"));
        assert!(
            body.contains("Capability-affecting: implementation awaits explicit user approval.")
        );
        assert!(body.contains(&format!("Observations: run {} seqs 4, 8", inputs.run_id)));
        assert!(body.contains("Basis: run"));
        assert!(body.contains(&inputs.workflow_version));
        assert!(body.contains(&inputs.target_sha));
    }

    #[test]
    fn draft_labels_stamp_project_revision_and_needs_user() {
        let project = WorkProject {
            project_id:               "fabro".to_string(),
            display_name:             "Fabro".to_string(),
            tracker_provider:         "github".to_string(),
            tracker_repo:             "denkhaus/work".to_string(),
            target_repo:              Some("denkhaus/fabro".to_string()),
            project_label:            "project:fabro".to_string(),
            lane_labels:              std::collections::BTreeMap::default(),
            claim_label_prefix:       "fabro:run:".to_string(),
            claim_comment_enabled:    false,
            backpressure_open_pr_cap: 0,
        };
        assert_eq!(draft_labels(&project, &candidate("T", &[1])), vec![
            "project:fabro".to_string(),
            "revision".to_string()
        ]);
        assert_eq!(
            draft_labels(&project, &Candidate {
                needs_user: true,
                ..candidate("T", &[1])
            }),
            vec![
                "project:fabro".to_string(),
                "revision".to_string(),
                "needs-user".to_string()
            ]
        );
    }

    fn terminal_record() -> PlatformRecord {
        PlatformRecord::RunLifecycle(
            RunLifecycleRecord::new(RunLifecycleKind::Succeeded).with_status(
                RunStatus::Succeeded {
                    reason: SuccessReason::Completed,
                },
            ),
        )
    }

    #[tokio::test]
    async fn enqueue_gates_on_the_setting_and_dedupes_double_fires() {
        let state = TestAppStateBuilder::new().build();
        let store = fabro_work::WorkStore::new(state.db_pool.clone());
        let run_id = RunId::new();

        // Filing disabled (the test default): nothing enqueues.
        enqueue_filing_if_terminal(&state, run_id, &terminal_record()).await;
        assert!(
            store
                .due(Utc::now() + Duration::seconds(1), 10)
                .await
                .unwrap()
                .is_empty()
        );

        // Enabled: exactly one durable entry, idempotent on a second
        // terminal record (worker succeeded plus server dead).
        let mut settings = default_test_server_settings();
        settings.server.observations.filing_enabled = true;
        let state = TestAppStateBuilder::new()
            .runtime_settings(settings, fabro_config::RunLayer::default())
            .build();
        let store = fabro_work::WorkStore::new(state.db_pool.clone());
        enqueue_filing_if_terminal(&state, run_id, &terminal_record()).await;
        enqueue_filing_if_terminal(&state, run_id, &terminal_record()).await;
        let due = store
            .due(Utc::now() + Duration::seconds(1), 10)
            .await
            .unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].kind, OUTBOX_KIND);
        assert_eq!(
            due[0].idempotency_key,
            format!("observation-filing:{run_id}")
        );
        assert_eq!(
            due[0].payload.get("run_id").and_then(|v| v.as_str()),
            Some(run_id.to_string().as_str())
        );
    }

    #[tokio::test]
    async fn enqueue_ignores_non_terminal_records() {
        let mut settings = default_test_server_settings();
        settings.server.observations.filing_enabled = true;
        let state = TestAppStateBuilder::new()
            .runtime_settings(settings, fabro_config::RunLayer::default())
            .build();
        let store = fabro_work::WorkStore::new(state.db_pool.clone());
        let run_id = RunId::new();
        let running = PlatformRecord::RunLifecycle(
            RunLifecycleRecord::new(RunLifecycleKind::Running).with_status(RunStatus::Running),
        );
        enqueue_filing_if_terminal(&state, run_id, &running).await;
        assert!(
            store
                .due(Utc::now() + Duration::seconds(1), 10)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn friction_records_once_per_entry_and_stays_positionless() {
        let state = TestAppStateBuilder::new().build();
        let run_id = RunId::new();
        record_friction_once(&state, run_id, "entry-1", "the distillation call failed").await;
        record_friction_once(&state, run_id, "entry-1", "it failed again").await;
        let records = state
            .stores
            .run_summaries
            .platform_records()
            .read_kind(&run_id, PlatformRecordKind::ObservationRecorded)
            .await
            .unwrap();
        assert_eq!(records.len(), 1, "the once-guard blocks a second record");
        match &records[0].record {
            PlatformRecord::ObservationRecorded(record) => {
                assert_eq!(record.category, ObservationCategory::Painpoint);
                assert!(
                    record
                        .text
                        .starts_with("run-end observation filing failed (outbox entry-1)")
                );
                assert_eq!(record.execution, POSITIONLESS_EXECUTION);
                assert_eq!(record.firing, POSITIONLESS_FIRING);
            }
            other => panic!("expected an observation record, got {other:?}"),
        }
    }

    /// A scripted tracker for the filer semantics: `list` answers from a
    /// canned hit table keyed by query prefix, `create` records drafts.
    #[derive(Default)]
    struct FilingTracker {
        hits:    Mutex<Vec<WorkItem>>,
        created: Mutex<Vec<WorkItemDraft>>,
    }

    #[async_trait]
    impl fabro_work::TrackerProvider for FilingTracker {
        async fn list(
            &self,
            _project: &WorkProject,
            filter: WorkItemFilter,
        ) -> anyhow::Result<Vec<WorkItem>> {
            let query = filter.query.unwrap_or_default();
            let hits = self.hits.lock().await;
            Ok(hits
                .iter()
                .filter(|item| {
                    query.is_empty()
                        || item.title.to_lowercase().contains(&query.to_lowercase())
                        || item.body.to_lowercase().contains(&query.to_lowercase())
                })
                .cloned()
                .collect())
        }

        async fn get(
            &self,
            _project: &WorkProject,
            _item_key: &str,
        ) -> anyhow::Result<Option<WorkItem>> {
            Ok(None)
        }

        async fn create(
            &self,
            _project: &WorkProject,
            draft: WorkItemDraft,
        ) -> anyhow::Result<WorkItem> {
            let mut created = work_item("denkhaus/work#900", &draft.title, "open", &draft.body);
            created.labels = draft.labels.clone();
            self.created.lock().await.push(draft);
            Ok(created)
        }

        async fn update(
            &self,
            _project: &WorkProject,
            _item_key: &str,
            _title: Option<&str>,
            _body: Option<&str>,
        ) -> anyhow::Result<WorkItem> {
            anyhow::bail!("not needed")
        }

        async fn close(
            &self,
            _project: &WorkProject,
            _item_key: &str,
            _reason: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn comment(
            &self,
            _project: &WorkProject,
            _item_key: &str,
            _body: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn project_claim(
            &self,
            _project: &WorkProject,
            _item_key: &str,
            _run_id: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn clear_claim(
            &self,
            _project: &WorkProject,
            _item_key: &str,
            _run_id: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }

    async fn filing_state_with(
        tracker: Arc<FilingTracker>,
    ) -> (Arc<AppState>, fabro_work::WorkStore, WorkProject) {
        let state = TestAppStateBuilder::new()
            .work_tracker(tracker as Arc<dyn fabro_work::TrackerProvider>)
            .build();
        let store = fabro_work::WorkStore::new(state.db_pool.clone());
        let project = WorkProject {
            project_id:               "fabro".to_string(),
            display_name:             "Fabro".to_string(),
            tracker_provider:         "github".to_string(),
            tracker_repo:             "denkhaus/work".to_string(),
            target_repo:              Some("denkhaus/fabro".to_string()),
            project_label:            "project:fabro".to_string(),
            lane_labels:              std::collections::BTreeMap::default(),
            claim_label_prefix:       "fabro:run:".to_string(),
            claim_comment_enabled:    false,
            backpressure_open_pr_cap: 0,
        };
        store
            .upsert_project(&project)
            .await
            .expect("project upserts");
        (state, store, project)
    }

    #[tokio::test]
    async fn file_candidate_creates_and_links_when_nothing_covers() {
        let tracker = Arc::new(FilingTracker::default());
        let (state, store, project) = filing_state_with(Arc::clone(&tracker)).await;
        let inputs = inputs();
        let outcome = file_candidate(
            &state,
            tracker.as_ref(),
            &project,
            &inputs,
            &candidate("Calibrate the widget drift meter", &[11]),
        )
        .await
        .unwrap();
        assert_eq!(outcome, CandidateOutcome::Filed);
        let drafts = tracker.created.lock().await;
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].labels, vec!["project:fabro", "revision"]);
        let links = store
            .linked_items_for_observations("fabro", &[(inputs.run_id, 11)])
            .await
            .unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].item_key, "denkhaus/work#900");
    }

    #[tokio::test]
    async fn file_candidate_links_an_open_covering_item_instead_of_filing() {
        let tracker = Arc::new(FilingTracker::default());
        tracker.hits.lock().await.push(work_item(
            "denkhaus/work#7",
            "Calibrate the widget drift meter",
            "open",
            "",
        ));
        let (state, store, project) = filing_state_with(Arc::clone(&tracker)).await;
        let inputs = inputs();
        let outcome = file_candidate(
            &state,
            tracker.as_ref(),
            &project,
            &inputs,
            &candidate("calibrate the widget drift meter", &[12]),
        )
        .await
        .unwrap();
        assert_eq!(outcome, CandidateOutcome::LinkedExisting);
        assert!(
            tracker.created.lock().await.is_empty(),
            "no duplicate filed"
        );
        let links = store
            .linked_items_for_observations("fabro", &[(inputs.run_id, 12)])
            .await
            .unwrap();
        assert_eq!(links[0].item_key, "denkhaus/work#7");
    }

    #[tokio::test]
    async fn file_candidate_drops_when_a_closed_item_covers() {
        let tracker = Arc::new(FilingTracker::default());
        tracker.hits.lock().await.push(work_item(
            "denkhaus/work#8",
            "Calibrate the widget drift meter",
            "closed",
            "",
        ));
        let (state, _store, project) = filing_state_with(Arc::clone(&tracker)).await;
        let outcome = file_candidate(
            &state,
            tracker.as_ref(),
            &project,
            &inputs(),
            &candidate("calibrate the widget drift meter", &[13]),
        )
        .await
        .unwrap();
        assert_eq!(outcome, CandidateOutcome::Dropped);
        assert!(tracker.created.lock().await.is_empty());
    }

    #[tokio::test]
    async fn file_candidate_treats_a_run_citing_item_as_covering() {
        // The freshness layer: an item whose BODY cites this run covers
        // even though its title shares nothing with the candidate.
        let tracker = Arc::new(FilingTracker::default());
        let inputs = inputs();
        tracker.hits.lock().await.push(work_item(
            "denkhaus/work#9",
            "Entirely different title words",
            "open",
            &format!("Fix the drift; observed in run {}", inputs.run_id),
        ));
        let (state, store, project) = filing_state_with(Arc::clone(&tracker)).await;
        // The freshness query (by run id) finds it; the theme query does
        // not — covering_item over the CHAIN must still link it because
        // the candidate's tokens all appear in its title+body union.
        let outcome = file_candidate(&state, tracker.as_ref(), &project, &inputs, &Candidate {
            title:       "entirely different title words".to_string(),
            body:        String::new(),
            priority:    2,
            source_seqs: vec![14],
            needs_user:  false,
        })
        .await
        .unwrap();
        assert_eq!(outcome, CandidateOutcome::LinkedExisting);
        let links = store
            .linked_items_for_observations("fabro", &[(inputs.run_id, 14)])
            .await
            .unwrap();
        assert_eq!(links[0].item_key, "denkhaus/work#9");
    }
}
