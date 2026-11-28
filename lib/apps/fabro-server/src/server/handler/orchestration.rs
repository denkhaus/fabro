//! The orchestration doors (ADR-0039 stage D, denkhaus/work#1107): the
//! verdict write door, the adjudication queue read, and the run
//! observations read — the three surfaces the adjudicator's fast path
//! and agent pass share with their CLI twins (`fabro run mark`,
//! `fabro run queue`, `fabro observation list`).
//!
//! The write door is FOREIGN by design: the caller's own run (from the
//! worker principal) is the WRITER, the path's run is the TARGET — a run
//! writes verdicts about OTHER runs. Which fields a run may write is the
//! declaration of its workflow definition (`[run.grants]`, ADR-0052): the
//! door refuses every write the workflow did not declare. That each field
//! has exactly one declaring workflow per project (ADR-0039 decision 6)
//! is the project's check, not the engine's.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use axum_extra::extract::Query as ExtraQuery;
use chrono::Utc;
use fabro_api::types::{
    GlobalObservation, GlobalObservationList, ObservationManageRequest,
    ObservationManageRequestAction, ObservationManageResponse, ObservationWorkItemLink,
    PaginationMeta, RunMergeResponse, RunObservation, RunObservationList,
    RunOrchestrationMarkRequest, RunOrchestrationMarkResponse, RunQueueEntry,
    RunQueueEntryAcceptance, RunQueueEntryVerdict, RunQueueList,
};
use fabro_store::platform_records::{
    ObservationRecordedRecord, ObservationTriageState, PlatformRecord, PlatformRecordKind,
    RunOrchestrationRecord, StoredPlatformRecord,
};
use fabro_types::settings::run::{MergeStrategy, RunGrantsSettings};
use fabro_types::{
    OrchestrationField, OrchestrationVerdict, Principal, PullRequestLink, PullRequestState, RunId,
};
use futures_util::future::join_all;
use tracing::info;

use super::super::{AppState, run_records};
use super::observation_triage;
use super::observation_triage::{
    LinkedItemResolution, LinkedItemResolver, LinkedItemState, RESOLVE_CONCURRENCY,
};
use crate::error::ApiError;
use crate::principal_middleware::RequiredRunManagementActor;

/// How many runs one queue read returns at most. The queue is the
/// adjudicator's worklist, not an archive walk: a gear that needs more
/// than this per pass has a backlog problem a bigger page will not fix.
const QUEUE_PAGE_LIMIT: u32 = 200;

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/runs/queue", get(list_queue))
        .route("/runs/{id}/orchestration", post(mark_orchestration))
        .route("/runs/{id}/merge", post(merge_run))
        .route(
            "/runs/{id}/resolution",
            post(super::super::resolution_push::push_run_resolution),
        )
        .route("/runs/{id}/observations", get(list_observations))
        .route("/observations", get(list_global_observations))
        .route("/observations/manage", post(manage_observation))
}

#[derive(Debug, Default, serde::Deserialize)]
struct ObservationFeedQuery {
    category: Option<String>,
    triage:   Option<String>,
    run:      Option<String>,
    q:        Option<String>,
    sort:     Option<String>,
    limit:    Option<u32>,
    page:     Option<u32>,
}

/// The global observations feed: every run's observation records in one
/// server-paged listing, newest first by default. Matching rides the
/// MATERIALIZED triage state (work#1385): the write doors keep the
/// side table current, so the triage filter — page and exact total
/// alike — is SQL (the same shape the category filter rides). The
/// RETURNED page re-resolves LIVE: links, item states and the
/// decision rows re-derive per row through the memoized resolver, a
/// filtered page drops rows whose live-derived state no longer matches,
/// and the link colors (denkhaus/work#1298) ride those live states. A
/// filtered total counts the materialized membership — a tracker-side
/// change outside the write doors stays counted until a write touches
/// the record again (the corner the API description names).
async fn list_global_observations(
    _auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    ExtraQuery(params): ExtraQuery<ObservationFeedQuery>,
) -> Result<Response, ApiError> {
    let page = params.page.unwrap_or(1).max(1) as usize;
    let page_size = params.limit.unwrap_or(25).clamp(1, 100) as usize;
    let newest_first = !matches!(params.sort.as_deref(), Some("recorded"));
    let run_id = match params.run.as_deref() {
        Some(raw) => Some(
            raw.parse::<RunId>()
                .map_err(|err| ApiError::bad_request(format!("invalid run id '{raw}': {err}")))?,
        ),
        None => None,
    };
    let category = match params.category.as_deref() {
        None => None,
        Some("painpoint") => Some(fabro_types::ObservationCategory::Painpoint),
        Some("note") => Some(fabro_types::ObservationCategory::Note),
        Some(other) => {
            return Err(ApiError::bad_request(format!(
                "unknown category '{other}' (painpoint | note)"
            )));
        }
    };
    let triage_filter = match params.triage.as_deref() {
        None => None,
        Some("untriaged") => Some(ObservationTriageState::Untriaged),
        Some("filed") => Some(ObservationTriageState::Filed),
        Some("in_progress") => Some(ObservationTriageState::InProgress),
        Some("closed") => Some(ObservationTriageState::Closed),
        Some("reviewed") => Some(ObservationTriageState::Reviewed),
        Some("filing_blocked") => Some(ObservationTriageState::FilingBlocked),
        Some(other) => {
            return Err(ApiError::bad_request(format!(
                "unknown triage state '{other}' (untriaged | filed | in_progress | closed | \
                 reviewed | filing_blocked)"
            )));
        }
    };

    let store = state.stores.run_summaries.platform_records();
    let offset = (page - 1) * page_size;
    // Page and COUNT share one WHERE: kind, run, text, the category
    // serde fragment AND the materialized triage state — the pager's
    // exact total is SQL semantics (work#1296, work#1385).
    let category_sql = category.map(|category| category.to_string());
    let read_failed = |error| {
        tracing::error!(error = ?error, "global observations read failed");
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Reading the observations feed failed.",
        )
    };
    let (entries, total) = store
        .read_kind_global(
            PlatformRecordKind::ObservationRecorded,
            newest_first,
            page_size,
            offset,
            run_id.as_ref(),
            params.q.as_deref(),
            category_sql.as_deref(),
            triage_filter,
        )
        .await
        .map_err(read_failed)?;
    // The page resolver (work#1362/#1363): the tracker half resolves
    // once per DISTINCT item for the whole page, the store halves once
    // per run — the only live resolution a request pays is its own
    // page's.
    let mut resolver = LinkedItemResolver::new(&state);
    let mut links_by_run: HashMap<RunId, Vec<fabro_work::ObservationLink>> = HashMap::new();
    let mut decisions_by_run: HashMap<RunId, HashMap<u64, (String, i64)>> = HashMap::new();
    let mut observations = Vec::with_capacity(entries.len());
    for (entry_run, stored) in &entries {
        let PlatformRecord::ObservationRecorded(record) = &stored.record else {
            continue;
        };
        let (links, states) = triage_links_of_seq(
            &state,
            &mut resolver,
            &mut links_by_run,
            entry_run,
            stored.seq,
        )
        .await;
        let decision = decision_of_seq(&state, &mut decisions_by_run, entry_run, stored.seq).await;
        let decision =
            decision.map(|(decision, decided_at_ms)| (decision.as_str(), *decided_at_ms));
        let triage = observation_triage::derive_triage(
            &states,
            decision,
            chrono::Utc::now().timestamp_millis(),
        );
        // The live overlay: a materialized-matching row whose LIVE
        // derivation has drifted (an item closed in the tracker since
        // the last write) falls out of the filtered page. An unfiltered
        // page keeps every row — its states are live by definition. The
        // total stays the materialized count — the corner is named, not
        // hidden.
        if triage_filter.is_some_and(|wanted| triage != wanted) {
            continue;
        }
        observations.push(wire_global_observation(
            entry_run, stored, record, triage, links,
        ));
    }
    Ok(Json(GlobalObservationList {
        observations,
        total: total.min(i64::MAX.unsigned_abs()).cast_signed(),
    })
    .into_response())
}

/// One stored observation record as the global feed serves it: the
/// record's body beside its derived triage state and resolved links.
fn wire_global_observation(
    entry_run: &RunId,
    stored: &StoredPlatformRecord,
    record: &ObservationRecordedRecord,
    triage: ObservationTriageState,
    links: Vec<ObservationWorkItemLink>,
) -> GlobalObservation {
    GlobalObservation {
        run_id:            entry_run.to_string(),
        workflow_name:     None,
        seq:               stored.seq.cast_signed(),
        category:          record.category,
        text:              record.text.clone(),
        recorded_at:       chrono::DateTime::from_timestamp_millis(
            stored.recorded_at.cast_signed(),
        ),
        triage_state:      observation_triage::wire_triage_state(triage),
        linked_work_items: links,
    }
}

/// A run's link rows, cached per run for the length of one feed request
/// (work#1296): a run with many records walks its ledger read a single
/// time. A store failure degrades to empty — the feed keeps serving.
async fn run_link_rows<'a>(
    state: &AppState,
    links_by_run: &'a mut HashMap<RunId, Vec<fabro_work::ObservationLink>>,
    run_id: &RunId,
) -> &'a [fabro_work::ObservationLink] {
    if !links_by_run.contains_key(run_id) {
        let links = state
            .work_store()
            .linked_items_for_run(run_id)
            .await
            .unwrap_or_default();
        links_by_run.insert(*run_id, links);
    }
    &links_by_run[run_id]
}

/// One record's resolved links: the link rows of `seq` (store half,
/// cheap) beside the tracker resolution (the expensive half), memoized
/// once per DISTINCT item for the whole request through the resolver and
/// joined concurrently — never one round trip per link row
/// (work#1362/#1363). A record without link rows resolves nothing. The
/// wire links ride beside the states the derivation reads, so the live
/// overlay derives from exactly what the wire shows.
async fn triage_links_of_seq(
    state: &AppState,
    resolver: &mut LinkedItemResolver<'_>,
    links_by_run: &mut HashMap<RunId, Vec<fabro_work::ObservationLink>>,
    run_id: &RunId,
    seq: u64,
) -> (Vec<ObservationWorkItemLink>, Vec<Option<LinkedItemState>>) {
    let run_links = run_link_rows(state, links_by_run, run_id).await;
    let seq_links: Vec<&fabro_work::ObservationLink> =
        run_links.iter().filter(|link| link.seq == seq).collect();
    if seq_links.is_empty() {
        return (Vec::new(), Vec::new());
    }
    // The not-yet-memoized distinct items of this record, resolved in
    // bounded concurrent joins (the RESOLVE_CONCURRENCY bound,
    // work#1379); the resolver's map dedupes across the whole request.
    // Chunked joins keep the result order aligned with `pending` for
    // the zip below.
    let mut pending: Vec<(String, String)> = Vec::new();
    for link in &seq_links {
        let key = (link.project_id.clone(), link.item_key.clone());
        if !resolver.resolved.contains_key(&key) && !pending.contains(&key) {
            pending.push(key);
        }
    }
    let mut resolutions = Vec::with_capacity(pending.len());
    for chunk in pending.chunks(RESOLVE_CONCURRENCY) {
        let part = join_all(chunk.iter().map(|(project_id, item_key)| {
            observation_triage::resolve_linked_item(state, project_id, item_key)
        }))
        .await;
        resolutions.extend(part);
    }
    for (key, resolved) in pending.into_iter().zip(resolutions) {
        resolver.resolved.insert(key, resolved);
    }
    let mut states = Vec::with_capacity(seq_links.len());
    let links = seq_links
        .into_iter()
        .map(|link| {
            let resolution = resolver
                .resolved
                .get(&(link.project_id.clone(), link.item_key.clone()));
            states.push(resolution.and_then(LinkedItemResolution::state));
            let (item_state, item_url, item_state_stale) = match resolution {
                Some(LinkedItemResolution::Resolved(item)) => (
                    Some(LinkedItemState::into_wire(item.state)),
                    Some(item.url.clone()),
                    false,
                ),
                // The read failed — the null state is a degradation the
                // wire must name (denkhaus/work#1364).
                Some(LinkedItemResolution::Stale) => (None, None, true),
                // The tracker answered (not-found), or the record
                // resolved nothing: honest nulls, no staleness.
                _ => (None, None, false),
            };
            ObservationWorkItemLink {
                item_key: link.item_key.clone(),
                linked_at: Some(link.linked_at),
                item_state,
                item_state_stale,
                item_url,
            }
        })
        .collect();
    (links, states)
}

/// One record's decision row (the wire decision beside its `decided_at`
/// clock — the derivation's park TTL reads both), from the run's cached
/// decision rows — one store read per run for the whole request.
async fn decision_of_seq<'a>(
    state: &AppState,
    decisions_by_run: &'a mut HashMap<RunId, HashMap<u64, (String, i64)>>,
    run_id: &RunId,
    seq: u64,
) -> Option<&'a (String, i64)> {
    if !decisions_by_run.contains_key(run_id) {
        let decisions = state
            .work_store()
            .decisions_for_run(run_id)
            .await
            .unwrap_or_default();
        let map = decisions
            .into_iter()
            .map(|decision| {
                (
                    decision.seq,
                    (decision.decision, decision.decided_at.timestamp_millis()),
                )
            })
            .collect();
        decisions_by_run.insert(*run_id, map);
    }
    decisions_by_run.get(run_id)?.get(&seq)
}

/// The triager's decision door (ADR-0044): records one triage decision
/// on one observation record, keyed to its append-only (run, seq)
/// identity. `no_filing` is the first action — the triager judged the
/// friction needs no work item, and the feed derives `reviewed` for the
/// record; a later filing overrides (link rows beat the decision row).
/// `filing_blocked` (work#1585) parks a record whose filing write
/// FAILED (rate limited, 4xx): the decision row lands alone — never a
/// link — and the record derives `filing_blocked` until the park TTL
/// passes, when the derivation re-reads it `untriaged`. Upsert: the
/// latest decision on a record wins. Authorization mirrors
/// the verdict door's chain: user principals write unconditionally, a
/// workflow run only when its workflow declares the `observation_triage`
/// grant (ADR-0052 — the door's twin surfaces are the agent tool
/// `fabro_observation_manage` and the CLI `fabro observation manage`).
async fn manage_observation(
    RequiredRunManagementActor(principal): RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Json(request): Json<ObservationManageRequest>,
) -> Result<Response, ApiError> {
    let run_id: RunId = request.run_id.parse().map_err(|err| {
        ApiError::bad_request(format!("invalid runId '{}': {err}", request.run_id))
    })?;
    let seq = u64::try_from(request.seq).map_err(|_| {
        ApiError::bad_request(format!(
            "seq must be a record's non-negative per-run seq, not {}",
            request.seq
        ))
    })?;
    let reason = request.reason.trim();
    if reason.is_empty() {
        return Err(ApiError::bad_request(
            "the decision reason must not be blank — a decision without a rationale teaches \
             nothing",
        ));
    }
    let decision = request.action.to_string();
    // `link_existing` carries the item half beside the decision: the
    // project scopes the link row, the item key names the already-filed
    // item (dedupe at the link, ADR-0038 decision 5 — never a second
    // item for a finding an item already carries).
    let link_target = match request.action {
        ObservationManageRequestAction::LinkExisting => {
            let project_id = request
                .project_id
                .clone()
                .filter(|id| !id.trim().is_empty());
            let item_key = request
                .item_key
                .clone()
                .filter(|key| !key.trim().is_empty());
            match (project_id, item_key) {
                (Some(project_id), Some(item_key)) => Some((project_id, item_key)),
                _ => {
                    return Err(ApiError::bad_request(
                        "link_existing needs the item half beside the observation half — pass \
                         projectId and itemKey",
                    ));
                }
            }
        }
        // The decision-only actions: neither writes a link. `filing_blocked`
        // (work#1585) parks a record whose filing write FAILED — the
        // decision row lands alone and the derivation owns the park TTL.
        ObservationManageRequestAction::NoFiling
        | ObservationManageRequestAction::FilingBlocked => None,
    };
    let by_run = match principal {
        Principal::User(_) => None,
        Principal::Worker { run_id: writer } => {
            if !grants_for_writer(&state, &writer).await?.observation_triage {
                return Err(ApiError::with_code(
                    StatusCode::FORBIDDEN,
                    "Only users and workflow runs whose workflow declares \
                     `observation_triage = true` under `[run.grants]` write triage decisions \
                     (ADR-0044, ADR-0052)."
                        .to_string(),
                    "triage_manage_not_allowed",
                ));
            }
            Some(writer)
        }
        _ => {
            return Err(ApiError::with_code(
                StatusCode::FORBIDDEN,
                "Only users and the triager workflow write triage decisions.",
                "triage_manage_not_allowed",
            ));
        }
    };
    // The target record must exist: the decision keys on the record's
    // append-only identity — a decision for a nonexistent seq is packet
    // drift, not a triage.
    let exists = state
        .stores
        .run_summaries
        .platform_records()
        .read_kind(&run_id, PlatformRecordKind::ObservationRecorded)
        .await
        .map_err(|err| {
            tracing::error!(error = ?err, run_id = %run_id, "observation record read failed");
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Reading the observation record failed.",
            )
        })?
        .iter()
        .any(|stored| stored.seq == seq);
    if !exists {
        return Err(ApiError::with_code(
            StatusCode::NOT_FOUND,
            format!(
                "No observation record {seq} on run {run_id} — the door keys on the record's \
                 append-only identity."
            ),
            "observation_not_found",
        ));
    }
    // The link arm: an existing item takes the finding (dedupe at the
    // link — idempotent, an already-linked pair stays one row; the
    // reason rides the decision row as the durable rationale).
    if let Some((project_id, item_key)) = link_target {
        let project = state
            .work_store()
            .project(&project_id)
            .await
            .map_err(|err| {
                tracing::error!(error = ?err, project_id = %project_id, "work project read failed");
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Reading the work project failed.",
                )
            })?
            .ok_or_else(|| {
                ApiError::with_code(
                    StatusCode::NOT_FOUND,
                    format!("work project '{project_id}' not found"),
                    "work_project_not_found",
                )
            })?;
        let outcome = state
            .work_store()
            .link_observation(&project.project_id, &item_key, &run_id, seq)
            .await
            .map_err(|err| {
                tracing::error!(error = ?err, run_id = %run_id, seq, "triage link write failed");
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Recording the triage link failed.",
                )
            })?;
        // The decision row rides the link (ADR-0044 decision 2: the
        // link is written BESIDE the decision — the rationale stays
        // durable, the upsert keeps latest-wins). The derivation
        // ignores it wherever the link exists (link rows beat the
        // decision row).
        let stored_decision = state
            .work_store()
            .decide_observation(&run_id, seq, &decision, reason, by_run.as_ref())
            .await
            .map_err(|err| {
                tracing::error!(error = ?err, run_id = %run_id, seq, "triage decision write failed");
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Recording the triage decision failed.",
                )
            })?;
        // The write door's materialization (work#1385): both halves of
        // this write landed, so the record's derived state is
        // re-derived once and stored beside the record.
        super::observation_triage::rederive_and_materialize(&state, &run_id, seq).await;
        let outcome_note = match outcome {
            fabro_work::LinkOutcome::Linked => "linked",
            fabro_work::LinkOutcome::AlreadyLinked => "already_linked",
        };
        info!(
            run_id = %run_id,
            seq,
            item_key = %item_key,
            by_run = ?by_run,
            outcome = outcome_note,
            "observation triage link recorded"
        );
        return Ok(Json(ObservationManageResponse {
            run_id:     stored_decision.run_id.to_string(),
            seq:        stored_decision.seq.cast_signed(),
            action:     stored_decision.decision,
            reason:     format!("{} ({outcome_note})", stored_decision.reason),
            decided_at: stored_decision.decided_at,
            by_run:     stored_decision.by_run.map(|run| run.to_string()),
            item_key:   Some(item_key),
        })
        .into_response());
    }
    let stored_decision = state
        .work_store()
        .decide_observation(&run_id, seq, &decision, reason, by_run.as_ref())
        .await
        .map_err(|err| {
            tracing::error!(error = ?err, run_id = %run_id, seq, "triage decision write failed");
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Recording the triage decision failed.",
            )
        })?;
    // The write door's materialization (work#1385): the decision write
    // can flip a linkless record between untriaged and reviewed.
    super::observation_triage::rederive_and_materialize(&state, &run_id, seq).await;
    info!(
        run_id = %run_id,
        seq,
        decision,
        by_run = ?stored_decision.by_run,
        "observation triage decision recorded"
    );
    Ok(Json(ObservationManageResponse {
        run_id:     stored_decision.run_id.to_string(),
        seq:        stored_decision.seq.cast_signed(),
        action:     stored_decision.decision,
        reason:     stored_decision.reason,
        decided_at: stored_decision.decided_at,
        by_run:     stored_decision.by_run.map(|run| run.to_string()),
        item_key:   None,
    })
    .into_response())
}

#[derive(Debug, serde::Deserialize)]
struct QueueQuery {
    field:    Option<OrchestrationField>,
    workflow: Option<String>,
    /// Settled-verdict filter (stage E): flips the anti-join — the queue
    /// lists exactly the runs whose latest verdict on the field IS this
    /// value. The closing pass reads `closable = enabled`.
    verdict:  Option<OrchestrationVerdict>,
    /// The other-field requirement (work#1126): `<field>=<verdict>`
    /// restricts the anti-join to runs whose latest verdict on the
    /// NAMED OTHER field IS this verdict — the merger's worklist is
    /// `field=closable` + `requires=merge=enabled`.
    requires: Option<String>,
}

/// Parse the `requires` pair (`<field>=<verdict>`) with teaching
/// refusals: exactly one `=`, both halves known enums, the OTHER field
/// (never the queue's own), and never beside a `verdict` filter (the
/// requirement belongs to the unsettled anti-join only).
fn parse_queue_requirement(
    raw: &str,
    field: OrchestrationField,
    verdict_filter_present: bool,
) -> Result<Option<(OrchestrationField, OrchestrationVerdict)>, ApiError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let raw = trimmed;
    if verdict_filter_present {
        return Err(ApiError::with_code(
            StatusCode::BAD_REQUEST,
            "`requires` belongs to the unsettled anti-join — it cannot combine with `verdict` \
             (the settled read flips the join; the requirement has no meaning there)",
            "queue_requires_conflict",
        ));
    }
    let Some((raw_field, raw_verdict)) = raw.split_once('=') else {
        return Err(ApiError::with_code(
            StatusCode::BAD_REQUEST,
            format!(
                "`{raw}` is not a field=verdict pair — the requirement reads, for example, \
                 merge=enabled"
            ),
            "queue_requires_malformed",
        ));
    };
    let other_field: OrchestrationField = serde_json::from_value(serde_json::Value::String(
        raw_field.to_string(),
    ))
    .map_err(|_| {
        ApiError::with_code(
            StatusCode::BAD_REQUEST,
            format!(
                "`{raw_field}` is not an orchestration field — the fields are merge and closable"
            ),
            "queue_requires_unknown_field",
        )
    })?;
    if other_field == field {
        return Err(ApiError::with_code(
            StatusCode::BAD_REQUEST,
            format!(
                "the requirement must name the OTHER field — `{raw_field}` is the queue's own \
                 field (a run settled on it has already left the anti-join)"
            ),
            "queue_requires_same_field",
        ));
    }
    let verdict: OrchestrationVerdict = serde_json::from_value(serde_json::Value::String(
        raw_verdict.to_string(),
    ))
    .map_err(|_| {
        ApiError::with_code(
            StatusCode::BAD_REQUEST,
            format!(
                "`{raw_verdict}` is not a verdict — the requirement accepts enabled or \
                         refused (pending is absence, parked re-enters the queue by itself)"
            ),
            "queue_requires_unknown_verdict",
        )
    })?;
    if !matches!(
        verdict,
        OrchestrationVerdict::Enabled | OrchestrationVerdict::Refused
    ) {
        return Err(ApiError::with_code(
            StatusCode::BAD_REQUEST,
            format!(
                "`{raw_verdict}` is not a settled verdict — the requirement accepts enabled or \
                 refused (pending is absence, parked re-enters the queue by itself)"
            ),
            "queue_requires_not_settled",
        ));
    }
    Ok(Some((other_field, verdict)))
}

/// The adjudication queue: terminal runs whose latest verdict on the
/// field is not settled (pending or parked), optionally narrowed to one
/// workflow's runs — the adjudicator's worklist filter. Each entry is
/// decorated with the work item of the run's latest claim (released
/// claims included): the claim ledger is the run → work-item edge
/// (ADR-0039 stage D part 2).
async fn list_queue(
    _auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Query(query): Query<QueueQuery>,
) -> Response {
    let field = query.field.unwrap_or(OrchestrationField::Merge);
    let requirement = match &query.requires {
        Some(raw) => match parse_queue_requirement(raw, field, query.verdict.is_some()) {
            Ok(requirement) => requirement,
            Err(error) => return error.into_response(),
        },
        None => None,
    };
    // The +1 probe: `has_more` must be honest — a worklist bigger than
    // one page must never read as complete (the fast path would sweep
    // only the first page and route "Queue empty" beside a full
    // backlog).
    let fetched = match query.verdict {
        // The settled-verdict read (stage E): the closing pass's
        // worklist — exactly the runs settled on the named verdict.
        Some(verdict) => {
            state
                .stores
                .run_summaries
                .verdict_queue(
                    field,
                    verdict,
                    query.workflow.as_deref(),
                    QUEUE_PAGE_LIMIT + 1,
                    Utc::now(),
                )
                .await
        }
        None => {
            state
                .stores
                .run_summaries
                .adjudication_queue(
                    field,
                    requirement,
                    query.workflow.as_deref(),
                    QUEUE_PAGE_LIMIT + 1,
                    Utc::now(),
                )
                .await
        }
    };
    let fetched = match fetched {
        Ok(runs) => runs,
        Err(err) => {
            return ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
                .into_response();
        }
    };
    let has_more = usize::try_from(QUEUE_PAGE_LIMIT).is_ok_and(|limit| fetched.len() > limit);
    let runs = if has_more {
        let limit = usize::try_from(QUEUE_PAGE_LIMIT).expect("a page limit that fits a usize");
        fetched[..limit].to_vec()
    } else {
        fetched
    };

    // The verdict map names the parked runs: the queue holds pending and
    // parked entries, and only the parked ones re-entered by escalation.
    // ONE read pair serves both the entry verdicts and the acceptance
    // folds below — the store folds all records per field, so a second
    // read would scan the same rows twice.
    let (merge_verdicts, closable_verdicts) =
        match super::super::acceptance::verdict_maps(&state).await {
            Ok(maps) => maps,
            Err(err) => {
                return ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
                    .into_response();
            }
        };
    let verdicts = match field {
        OrchestrationField::Merge => &merge_verdicts,
        OrchestrationField::Closable => &closable_verdicts,
    };
    let run_ids = runs.iter().map(|run| run.id).collect::<Vec<_>>();
    // The decoration carries the claim's item AND project (work#1113):
    // queue consumers read scoped doors (the closure radius) with the
    // claim's own project, never a hardcoded one.
    let claim_by_run = match state.stores.work.latest_claims_of_runs(&run_ids).await {
        Ok(claims) => claims
            .into_iter()
            .map(|claim| (claim.run_id, claim))
            .collect::<HashMap<RunId, fabro_work::WorkClaim>>(),
        Err(err) => {
            return ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
                .into_response();
        }
    };

    let data = state.decorate_run_summaries(runs).await;
    let mut entries = Vec::with_capacity(data.len());
    for run in data {
        let verdict = match verdicts.get(&run.id) {
            Some(OrchestrationVerdict::Parked) => RunQueueEntryVerdict::Parked,
            Some(OrchestrationVerdict::Enabled) => RunQueueEntryVerdict::Enabled,
            Some(OrchestrationVerdict::Refused) => RunQueueEntryVerdict::Refused,
            _ => RunQueueEntryVerdict::Pending,
        };
        let (work_item, project, acceptance) = match claim_by_run.get(&run.id) {
            Some(claim) => {
                // The claimed entry carries the item's acceptance fold
                // (denkhaus/work#1555): re-derived per read, never
                // stored.
                let fold = match super::super::acceptance::item_fold(
                    &state,
                    &claim.project_id,
                    &claim.item_key,
                    (&merge_verdicts, &closable_verdicts),
                )
                .await
                {
                    Ok(fold) => Some(wire_acceptance(fold)),
                    Err(err) => {
                        return ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
                            .into_response();
                    }
                };
                (
                    Some(claim.item_key.clone()),
                    Some(claim.project_id.clone()),
                    fold,
                )
            }
            None => (None, None, None),
        };
        entries.push(RunQueueEntry {
            work_item,
            project,
            run,
            verdict,
            acceptance,
        });
    }
    Json(RunQueueList {
        data: entries,
        meta: PaginationMeta {
            has_more,
            // The full count is not computed (the +1 probe carries the
            // paging truth; `total` stays absent rather than reporting
            // the page length as the worklist size).
            total: None,
        },
    })
    .into_response()
}

/// The wire acceptance state for a derived one (denkhaus/work#1555):
/// the API type stays the generated enum — no parallel DTO, the same
/// posture as the observation feed's `wire_triage_state`.
fn wire_acceptance(state: fabro_work::AcceptanceState) -> RunQueueEntryAcceptance {
    match state {
        fabro_work::AcceptanceState::NoClaimOpenItem => RunQueueEntryAcceptance::NoClaimOpenItem,
        fabro_work::AcceptanceState::NoClaimClosedItem => {
            RunQueueEntryAcceptance::NoClaimClosedItem
        }
        fabro_work::AcceptanceState::WaitingForMergeVerdict => {
            RunQueueEntryAcceptance::WaitingForMergeVerdict
        }
        fabro_work::AcceptanceState::MergeRefused => RunQueueEntryAcceptance::MergeRefused,
        fabro_work::AcceptanceState::WaitingForClosable => {
            RunQueueEntryAcceptance::WaitingForClosable
        }
        fabro_work::AcceptanceState::ClosableLanded => RunQueueEntryAcceptance::ClosableLanded,
        fabro_work::AcceptanceState::ClosedUnlanded => RunQueueEntryAcceptance::ClosedUnlanded,
        fabro_work::AcceptanceState::LandedClosed => RunQueueEntryAcceptance::LandedClosed,
    }
}

/// The verdict write door. Authorization is the worker token's chain:
/// the token names the WRITER run, the writer's workflow definition
/// declares the fields its runs may write (`[run.grants]`, ADR-0052).
/// The target run is the path.
async fn mark_orchestration(
    RequiredRunManagementActor(principal): RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(target): Path<RunId>,
    Json(request): Json<RunOrchestrationMarkRequest>,
) -> Result<Response, ApiError> {
    let Principal::Worker { run_id: writer } = principal else {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            "Only workflow runs write orchestration verdicts. Users read verdicts through the \
             run view.",
            "orchestration_not_allowed",
        ));
    };
    if writer == target {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            "A run cannot mark itself — verdicts are written about OTHER runs (ADR-0039 \
             decision 1: a run never judges itself).",
            "orchestration_self_mark",
        ));
    }
    let grants = grants_for_writer(&state, &writer).await?;
    if grants.verdict_fields.is_empty() {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            "This run's workflow declares no `verdict_fields` under `[run.grants]` — it writes \
             no verdicts (ADR-0052)."
                .to_string(),
            "orchestration_not_allowed",
        ));
    }
    if !grants.allows_verdict(request.field) {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            format!(
                "This run's workflow declares `verdict_fields = {:?}` under `[run.grants]`, \
                 not `{}`.",
                grants.verdict_fields, request.field
            ),
            "orchestration_field_not_owned",
        ));
    }

    // The target must exist; `require_projection` settles the projector
    // so the response reflects the verdict this call writes.
    run_records::require_projection(&state, target).await?;

    let record = PlatformRecord::RunOrchestration(RunOrchestrationRecord {
        field:   request.field,
        verdict: request.verdict,
        by_run:  Some(writer),
    });
    let stored = run_records::append(&state, target, record)
        .await
        .map_err(|err| {
            tracing::error!(
                error = ?err,
                target_run_id = %target,
                "orchestration verdict append failed"
            );
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Recording the orchestration verdict failed.",
            )
        })?;
    info!(
        target_run_id = %target,
        writer_run_id = %writer,
        field = %request.field,
        verdict = %request.verdict,
        seq = stored.seq,
        "orchestration verdict recorded"
    );
    Ok(Json(RunOrchestrationMarkResponse {
        run_id:  target.to_string(),
        field:   request.field,
        verdict: request.verdict,
        by_run:  writer.to_string(),
    })
    .into_response())
}

/// The grants the writer run's workflow declared (ADR-0052): read off
/// the writer's stored run spec, where the workflow definition's
/// `[run.grants]` block landed at admission. Every door that acts on
/// behalf of a run checks these — the verdict write door, the merge
/// door, the resolve push door, the triage manage door, and the
/// work-item close door. An unreadable writer is an internal error, not
/// a silent refusal: the token named a run the store cannot see.
pub(in crate::server) async fn grants_for_writer(
    state: &AppState,
    writer: &RunId,
) -> Result<RunGrantsSettings, ApiError> {
    state
        .stores
        .run_summaries
        .load_petri_projection(writer)
        .await
        .map_err(|err| {
            tracing::error!(error = ?err, writer_run_id = %writer, "writer run projection read failed");
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Reading the writer run's projection failed.",
            )
        })?
        .map(|projection| projection.spec.settings.run.grants.clone())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::FORBIDDEN,
                "The writer run named by the worker token does not exist.",
            )
        })
}

/// The merger's merge door (ADR-0039 stage E): merge the TARGET run's
/// pull request server-side — the technical integration act, the
/// supervisor's merge machinery moved into a door. The authorization
/// chain mirrors the verdict door: the worker token names the WRITER,
/// the writer's workflow must be the merger (the hardcoded allowlist),
/// and the TARGET must carry a pull request whose latest `merge`
/// verdict is `enabled` — the adjudicator's content verdict; the merger
/// never judges content. This door writes NO verdict (one door, one
/// write): the merger marks `closable` through the verdict door after
/// the merge (and its smoke) settles.
async fn merge_run(
    RequiredRunManagementActor(principal): RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(target): Path<RunId>,
) -> Result<Response, ApiError> {
    let Principal::Worker { run_id: writer } = principal else {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            "Only workflow runs merge pull requests. Users merge through GitHub.",
            "merge_not_allowed",
        ));
    };
    if writer == target {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            "A run cannot merge itself — a run integrates OTHER runs (ADR-0039 decision 1).",
            "merge_self",
        ));
    }
    if !grants_for_writer(&state, &writer).await?.merge {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            "Only workflow runs whose workflow declares `merge = true` under `[run.grants]` \
             merge pull requests (ADR-0052)."
                .to_string(),
            "merge_not_allowed",
        ));
    }
    // The target must exist and settle its projection — the PR and the
    // verdict fold read off it.
    let projection = run_records::require_projection(&state, target).await?;
    let Some(record) = projection.pull_request.clone() else {
        return Err(ApiError::with_code(
            StatusCode::CONFLICT,
            "The run carries no pull request — there is nothing to merge. The adjudicator's \
             pass one refuses PR-less runs; the merger never sees one.",
            "merge_no_pull_request",
        ));
    };
    if state
        .stores
        .run_summaries
        .platform_records()
        .latest_verdict_of_run(OrchestrationField::Merge, &target)
        .await
        .map_err(|err| {
            tracing::error!(error = ?err, target_run_id = %target, "merge verdict read failed");
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Reading the run's merge verdict failed.",
            )
        })?
        != Some(OrchestrationVerdict::Enabled)
    {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            "The adjudicator has not enabled this run's merge — the merger integrates only \
             merge-enabled runs (ADR-0039 decision 6: the content verdict precedes the \
             technical merge).",
            "merge_not_enabled",
        ));
    }
    // The acceptance backstop (denkhaus/work#1553): the claimed item's
    // named parts must be ACCOUNTED before the PR lands — every
    // unchecked task-list checkbox covered by a `part=verified:`
    // review observation or checked in the body. The correlated-failure
    // cure's mechanical half: a deliberately half-landed item cannot
    // pass the chain even when the reviewer approved and the
    // adjudicator enabled — the door refuses BEFORE landing, not after.
    let refusal_parts = super::super::acceptance::merge_refusal_parts(&state, &target)
        .await
        .map_err(|err| {
            tracing::error!(error = ?err, target_run_id = %target, "merge acceptance read failed");
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Reading the claimed item's acceptance state failed.",
            )
        })?;
    if let Some(parts) = refusal_parts {
        let listed = parts
            .iter()
            .take(3)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("; ");
        let more = parts.len().saturating_sub(3);
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            format!(
                "The claimed item carries {} unaccounted named part(s): {listed}{}. The item's \
                 task list says work remains — the reviewer's per-part verified record accounts \
                 the parts this pull request delivers, the body's checked boxes the landed ones.",
                parts.len(),
                if more > 0 {
                    format!(" (+{more} more)")
                } else {
                    String::new()
                },
            ),
            "merge_acceptance_unaccounted",
        ));
    }

    // The merge itself: server-side GitHub credentials (the supervisor's
    // machinery — squash, the line's merge shape).
    let creds = super::pull_requests::load_server_github_credentials(&state).await?;
    let github = super::pull_requests::server_github_context(&state, &creds)?;

    // Idempotence (work#1127, the grilled explicit state read): a
    // re-drive of an already-merged pull request answers 200
    // immediately — a crash between the merge and the closable mark
    // must not strand a merged run as a phantom conflict packet. The
    // probe FAILS OPEN: on a read error the merge call itself stays
    // the authority (GitHub answers 405/409 for an already-merged PR
    // only when the state read could not prove it).
    let already_merged =
        match fabro_github::get_pull_request(&github, &record.owner, &record.repo, record.number)
            .await
        {
            Ok(detail) => detail.merged,
            Err(error) => {
                tracing::warn!(
                    error = ?error,
                    target_run_id = %target,
                    pr_number = record.number,
                    "merge door: the idempotence state read failed — proceeding to the merge call"
                );
                false
            }
        };
    if already_merged {
        info!(
            target_run_id = %target,
            writer_run_id = %writer,
            pr_number = record.number,
            "run pull request already merged (merger door, idempotent re-drive)"
        );
        record_pull_request_merged(&state, target, &record).await;
        return Ok(Json(RunMergeResponse {
            run_id:       target.to_string(),
            merged:       true,
            pull_request: record.html_url(),
        })
        .into_response());
    }

    let merge = fabro_github::merge_pull_request(
        &github,
        &record.owner,
        &record.repo,
        record.number,
        MergeStrategy::Squash,
    )
    .await;
    match merge {
        Ok(()) => {
            info!(
                target_run_id = %target,
                writer_run_id = %writer,
                pr_number = record.number,
                "run pull request merged (merger door, squash)"
            );
            record_pull_request_merged(&state, target, &record).await;
            Ok(Json(RunMergeResponse {
                run_id:       target.to_string(),
                merged:       true,
                pull_request: record.html_url(),
            })
            .into_response())
        }
        Err(fabro_github::PullRequestApiError::Conflict { status, body, .. }) => {
            // The typed conflict signal (fabro #1117): the fast path
            // parks the run for the agent pass on exactly this code.
            Err(ApiError::with_code(
                StatusCode::CONFLICT,
                format!("The pull request cannot merge (status {status}): {body}"),
                "merge_conflict",
            ))
        }
        Err(error) => {
            tracing::error!(
                error = ?error,
                target_run_id = %target,
                pr_number = record.number,
                "run pull request merge failed"
            );
            Err(ApiError::with_code(
                StatusCode::BAD_GATEWAY,
                format!("The GitHub merge call failed: {error}"),
                "merge_failed",
            ))
        }
    }
}

/// Record the merge outcome on the run's stored pull request link
/// (denkhaus/work#1173): a `pull_request.merged` platform record the
/// projection folds onto `pull_request.state`, so the run list — and the
/// board's PR chip — carries the outcome. Idempotent: a link already
/// settled `merged` is not re-recorded. A record failure is logged, never
/// a failed response: the merge is a fact on GitHub either way, and the
/// merged-wait's observation is the second writer that recovers the
/// state.
async fn record_pull_request_merged(state: &AppState, target: RunId, record: &PullRequestLink) {
    if record.state == Some(PullRequestState::Merged) {
        return;
    }
    if let Err(err) = run_records::append(state, target, PlatformRecord::PullRequestMerged).await {
        tracing::error!(
            error = ?err,
            target_run_id = %target,
            pr_number = record.number,
            "recording the merged pull request state failed"
        );
    }
}

/// A run's observation records in `seq` order: the facts its stages
/// recorded, with the category that routes adjudication (a painpoint
/// forces the agent pass). Each record carries its stage attribution
/// (the projection's positioned fold — `StageProjection.observations`)
/// and its linked work items (the ADR-0038 link join), the two columns
/// the run detail view's observations table renders (denkhaus/work#1098).
async fn list_observations(
    _auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<RunId>,
) -> Result<Response, ApiError> {
    let projection = run_records::require_projection(&state, run_id).await?;
    let records = state
        .stores
        .run_summaries
        .platform_records()
        .read_kind(&run_id, PlatformRecordKind::ObservationRecorded)
        .await
        .map_err(|err| {
            tracing::error!(error = ?err, run_id = %run_id, "run observations read failed");
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Reading the run's observations failed.",
            )
        })?;
    // The positioned attribution: which stage each record folded onto.
    // A record the fold did not place (a positionless server-origin
    // record, e.g. the run-end hook's own friction notes) keeps no
    // stage and renders un-attributed.
    let stage_of_seq: HashMap<u64, String> = projection
        .iter_stages()
        .flat_map(|(stage_id, stage)| {
            stage
                .observations
                .iter()
                .map(|observation| (observation.seq, stage_id.to_string()))
        })
        .collect();
    // The link join (ADR-0038): the work items filed from or backfilled
    // onto each record. A store failure degrades to an empty column —
    // the observations themselves still list (the radius precedent).
    let mut links_of_seq: HashMap<u64, Vec<ObservationWorkItemLink>> = HashMap::new();
    let mut pairs_of_seq: HashMap<u64, Vec<(String, String)>> = HashMap::new();
    match state.work_store().linked_items_for_run(&run_id).await {
        Ok(links) => {
            for link in links {
                let pair = (link.project_id.clone(), link.item_key.clone());
                pairs_of_seq.entry(link.seq).or_default().push(pair);
                links_of_seq
                    .entry(link.seq)
                    .or_default()
                    .push(ObservationWorkItemLink {
                        item_key:         link.item_key,
                        linked_at:        Some(link.linked_at),
                        item_state:       None,
                        item_state_stale: false,
                        item_url:         None,
                    });
            }
        }
        Err(err) => {
            tracing::warn!(
                run_id = %run_id,
                error = ?err,
                "observation link read failed; the linked-item column degrades to empty"
            );
        }
    }
    // The linked items' live states and tracker URLs, resolved once per
    // distinct (project, item) through the per-request resolver
    // (work#1362): closed, open with a live claim (a run is working
    // it), or open. A FAILED read keeps nulls but marks them stale
    // (denkhaus/work#1364); a not-found keeps honest nulls.
    let mut resolver = LinkedItemResolver::new(&state);
    for (seq, pairs) in &pairs_of_seq {
        let Some(links) = links_of_seq.get_mut(seq) else {
            continue;
        };
        for (link, (project_id, item_key)) in links.iter_mut().zip(pairs) {
            match resolver.resolve(project_id, item_key).await {
                LinkedItemResolution::Resolved(item) => {
                    link.item_state = Some(LinkedItemState::into_wire(item.state));
                    link.item_url = Some(item.url);
                    link.item_state_stale = false;
                }
                LinkedItemResolution::Stale => {
                    link.item_state = None;
                    link.item_url = None;
                    link.item_state_stale = true;
                }
                LinkedItemResolution::NotFound => {
                    link.item_state = None;
                    link.item_url = None;
                    link.item_state_stale = false;
                }
            }
        }
    }

    let observations = records
        .iter()
        .filter_map(|stored| match &stored.record {
            PlatformRecord::ObservationRecorded(record) => Some(wire_observation(
                stored.seq,
                stored.recorded_at,
                record,
                stage_of_seq.get(&stored.seq).cloned(),
                links_of_seq.get(&stored.seq).cloned().unwrap_or_default(),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    Ok(Json(RunObservationList { observations }).into_response())
}

/// One stored observation record as the API serves it. The category is
/// the canonical `fabro_types::ObservationCategory` (reused through
/// `with_replacement` — no parallel DTO, no bridge).
fn wire_observation(
    seq: u64,
    recorded_at_ms: u64,
    record: &ObservationRecordedRecord,
    stage: Option<String>,
    linked_work_items: Vec<ObservationWorkItemLink>,
) -> RunObservation {
    RunObservation {
        seq: seq.cast_signed(),
        category: record.category,
        text: record.text.clone(),
        execution: i64::try_from(record.execution).ok(),
        firing: i64::try_from(record.firing).ok(),
        stage,
        linked_work_items,
        recorded_at: chrono::DateTime::from_timestamp_millis(recorded_at_ms.cast_signed()),
    }
}
