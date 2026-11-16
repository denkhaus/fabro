//! The orchestration doors (ADR-0039 stage D, denkhaus/work#1107): the
//! verdict write door, the adjudication queue read, and the run
//! observations read — the three surfaces the adjudicator's fast path
//! and agent pass share with their CLI twins (`fabro run mark`,
//! `fabro run queue`, `fabro observation list`).
//!
//! The write door is FOREIGN by design: the caller's own run (from the
//! worker principal) is the WRITER, the path's run is the TARGET — the
//! adjudicator and the merger write verdicts about OTHER runs. Which
//! workflow may write which field is a hardcoded pair (the stage-D
//! grill's carrier decision): `adjudicator` runs own `merge`, `merger`
//! runs own `closable`, and the door refuses every crossing write —
//! one writer per field (ADR-0039 decision 6), structural, not
//! conventional.

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
    GlobalObservation, GlobalObservationList, GlobalObservationTriageState,
    ObservationManageRequest, ObservationManageRequestAction, ObservationManageResponse,
    ObservationWorkItemLink, ObservationWorkItemLinkItemState, PaginationMeta, RunMergeResponse,
    RunObservation, RunObservationList, RunOrchestrationMarkRequest, RunOrchestrationMarkResponse,
    RunQueueEntry, RunQueueEntryVerdict, RunQueueList,
};
use fabro_store::platform_records::{
    ObservationRecordedRecord, PlatformRecord, PlatformRecordKind, RunOrchestrationRecord,
    StoredPlatformRecord,
};
use fabro_types::settings::run::MergeStrategy;
use fabro_types::{
    OrchestrationField, OrchestrationVerdict, Principal, PullRequestLink, PullRequestState, RunId,
};
use futures_util::future::join_all;
use tracing::info;

use super::super::{AppState, run_records};
use crate::error::ApiError;
use crate::principal_middleware::RequiredRunManagementActor;

/// The workflow allowlist (ADR-0039 stage D grill, hardcoded carrier
/// decision): the only workflow slugs whose runs may write orchestration
/// verdicts, each bound to exactly one field. The names are stable,
/// line-owned contracts — a change is a deliberate code edit, not a
/// settings knob. Shared with the work-item close door (stage E): the
/// adjudicator is the only workflow that closes work items.
pub(super) const MERGE_WRITER_WORKFLOW: &str = "adjudicator";
pub(super) const CLOSABLE_WRITER_WORKFLOW: &str = "merger";

/// The triager's workflow slug (ADR-0044): the only workflow whose runs
/// may write observation triage decisions — the manage door's allowlist,
/// the mark door's posture. The triager owns global painpoint triage;
/// user principals write through the same door unconditionally.
pub(super) const TRIAGER_WORKFLOW: &str = "triager";

/// The wire name of the `no_filing` decision — the one value the
/// derivation's linkless branch keys on; a const so the wire vocabulary
/// has one owner beside the OpenAPI enum.
const NO_FILING_DECISION: &str = "no_filing";

/// How many runs one queue read returns at most. The queue is the
/// adjudicator's worklist, not an archive walk: a gear that needs more
/// than this per pass has a backlog problem a bigger page will not fix.
const QUEUE_PAGE_LIMIT: u32 = 200;

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/runs/queue", get(list_queue))
        .route("/runs/{id}/orchestration", post(mark_orchestration))
        .route("/runs/{id}/merge", post(merge_run))
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
/// server-paged listing, newest first by default. The triage state is
/// DERIVED per row (links + live item states + the claim ledger), so a
/// triage filter pages by over-fetching and filtering — SQL cannot see
/// the derivation. The total counts the SQL filters; a triage-filtered
/// page may therefore be shorter than the pager hopes.
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
        Some("untriaged") => Some(TriageState::Untriaged),
        Some("filed") => Some(TriageState::Filed),
        Some("in_progress") => Some(TriageState::InProgress),
        Some("closed") => Some(TriageState::Closed),
        Some("reviewed") => Some(TriageState::Reviewed),
        Some(other) => {
            return Err(ApiError::bad_request(format!(
                "unknown triage state '{other}' (untriaged | filed | in_progress | closed | \
                 reviewed)"
            )));
        }
    };

    let store = state.stores.run_summaries.platform_records();
    let offset = (page - 1) * page_size;
    // The category filter rides the SQL WHERE (the serde fragment of
    // the record's category), so the page fetch and its COUNT agree
    // exactly; only the triage filter derives past SQL (work#1296).
    let category_sql = category.map(|category| category.to_string());
    let read_failed = |error| {
        tracing::error!(error = ?error, "global observations read failed");
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Reading the observations feed failed.",
        )
    };
    // The derivation's tracker half resolves once per DISTINCT item for
    // the whole request (work#1362/#1363): the walk used to resolve per
    // link row — the same item linked from N records cost N tracker
    // round trips, which is what made triage-filtered reads take a
    // minute. The resolver memoizes both the tracker read and the
    // claim-ledger read per distinct (project, item).
    let mut resolver = LinkedItemResolver::new(&state);
    let mut links_by_run: HashMap<RunId, Vec<fabro_work::ObservationLink>> = HashMap::new();
    // The decision half (ADR-0044): one store read per run, beside the
    // link rows. A store failure degrades to empty — the feed keeps
    // serving.
    let mut decisions_by_run: HashMap<RunId, HashMap<u64, String>> = HashMap::new();
    let (observations, total) = match triage_filter {
        None => {
            let (entries, total) = store
                .read_kind_global(
                    PlatformRecordKind::ObservationRecorded,
                    newest_first,
                    page_size,
                    offset,
                    run_id.as_ref(),
                    params.q.as_deref(),
                    category_sql.as_deref(),
                )
                .await
                .map_err(read_failed)?;
            let mut rows = Vec::with_capacity(entries.len());
            for (entry_run, stored) in &entries {
                let PlatformRecord::ObservationRecorded(record) = &stored.record else {
                    continue;
                };
                let links = triage_links_of_seq(
                    &state,
                    &mut resolver,
                    &mut links_by_run,
                    entry_run,
                    stored.seq,
                )
                .await;
                let decision =
                    decision_of_seq(&state, &mut decisions_by_run, entry_run, stored.seq).await;
                let triage = derive_triage(&links, decision);
                rows.push(wire_global_observation(
                    entry_run, stored, record, triage, links,
                ));
            }
            (rows, total)
        }
        Some(wanted) => {
            // EXACT total under a post-SQL filter (work#1296): walk
            // every matching record once, derive its triage, count the
            // matches, and slice the page from the walked matches —
            // replacing the over-fetch rounds whose total could only
            // grow while paging. Links are fetched once per run
            // (work#1296), item states resolve once per distinct item
            // through the per-request resolver (work#1362), the missing
            // ones join concurrently (work#1363), and the linkless
            // short-circuits need no tracker I/O at all: a record with
            // link rows can never be untriaged or reviewed, and one
            // without derives those two states from link presence and
            // the decision row alone.
            const WALK_BATCH: usize = 500;
            let mut matched: Vec<GlobalObservation> = Vec::new();
            let mut walk_offset = 0usize;
            loop {
                let (entries, _seen) = store
                    .read_kind_global(
                        PlatformRecordKind::ObservationRecorded,
                        newest_first,
                        WALK_BATCH,
                        walk_offset,
                        run_id.as_ref(),
                        params.q.as_deref(),
                        category_sql.as_deref(),
                    )
                    .await
                    .map_err(read_failed)?;
                if entries.is_empty() {
                    break;
                }
                let exhausted = entries.len() < WALK_BATCH;
                for (entry_run, stored) in &entries {
                    let PlatformRecord::ObservationRecorded(record) = &stored.record else {
                        continue;
                    };
                    let run_links = run_link_rows(&state, &mut links_by_run, entry_run).await;
                    let has_links = run_links.iter().any(|link| link.seq == stored.seq);
                    let decision =
                        decision_of_seq(&state, &mut decisions_by_run, entry_run, stored.seq).await;
                    if !has_links {
                        // Linkless records derive from the decision row
                        // alone: reviewed on a no_filing decision,
                        // untriaged without one. No tracker I/O.
                        let triage = if decision == Some(NO_FILING_DECISION) {
                            TriageState::Reviewed
                        } else {
                            TriageState::Untriaged
                        };
                        if triage == wanted {
                            matched.push(wire_global_observation(
                                entry_run,
                                stored,
                                record,
                                triage,
                                Vec::new(),
                            ));
                        }
                        continue;
                    }
                    // Link rows present: filed / in_progress / closed —
                    // the links outrank any decision (a filing
                    // overrides), and the states resolve deduped.
                    let links = triage_links_of_seq(
                        &state,
                        &mut resolver,
                        &mut links_by_run,
                        entry_run,
                        stored.seq,
                    )
                    .await;
                    let triage = derive_triage(&links, decision);
                    if triage != wanted {
                        continue;
                    }
                    matched.push(wire_global_observation(
                        entry_run, stored, record, triage, links,
                    ));
                }
                walk_offset += entries.len();
                if exhausted {
                    break;
                }
            }
            let total = matched.len() as u64;
            let observations = matched
                .into_iter()
                .skip(offset)
                .take(page_size)
                .collect::<Vec<_>>();
            (observations, total)
        }
    };
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
    triage: TriageState,
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
        triage_state:      triage.into_wire(),
        linked_work_items: links,
    }
}

/// The wire triage state and its derivation over a record's links and
/// decision row.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TriageState {
    Untriaged,
    Filed,
    InProgress,
    Closed,
    Reviewed,
}

impl TriageState {
    fn into_wire(self) -> GlobalObservationTriageState {
        match self {
            Self::Untriaged => GlobalObservationTriageState::Untriaged,
            Self::Filed => GlobalObservationTriageState::Filed,
            Self::InProgress => GlobalObservationTriageState::InProgress,
            Self::Closed => GlobalObservationTriageState::Closed,
            Self::Reviewed => GlobalObservationTriageState::Reviewed,
        }
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

/// One record's wire links with resolved live item states: the link rows
/// of `seq` (store half, cheap) beside the tracker resolution (the
/// expensive half), memoized once per DISTINCT item for the whole
/// request through the resolver and joined concurrently — never one
/// round trip per link row (work#1362/#1363). A record without link
/// rows resolves nothing: it derives `untriaged` from link presence
/// alone.
async fn triage_links_of_seq(
    state: &AppState,
    resolver: &mut LinkedItemResolver<'_>,
    links_by_run: &mut HashMap<RunId, Vec<fabro_work::ObservationLink>>,
    run_id: &RunId,
    seq: u64,
) -> Vec<ObservationWorkItemLink> {
    let run_links = run_link_rows(state, links_by_run, run_id).await;
    let seq_links: Vec<&fabro_work::ObservationLink> =
        run_links.iter().filter(|link| link.seq == seq).collect();
    if seq_links.is_empty() {
        return Vec::new();
    }
    // The not-yet-memoized distinct items of this record, resolved in
    // one concurrent join; the resolver's map dedupes across the whole
    // request.
    let mut pending: Vec<(String, String)> = Vec::new();
    for link in &seq_links {
        let key = (link.project_id.clone(), link.item_key.clone());
        if !resolver.resolved.contains_key(&key) && !pending.contains(&key) {
            pending.push(key);
        }
    }
    let resolutions = join_all(
        pending
            .iter()
            .map(|(project_id, item_key)| resolve_linked_item(state, project_id, item_key)),
    )
    .await;
    for (key, resolved) in pending.into_iter().zip(resolutions) {
        resolver.resolved.insert(key, resolved);
    }
    seq_links
        .into_iter()
        .map(|link| {
            let (item_state, item_url, item_state_stale) = match resolver
                .resolved
                .get(&(link.project_id.clone(), link.item_key.clone()))
            {
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
        .collect()
}

fn derive_triage(links: &[ObservationWorkItemLink], decision: Option<&str>) -> TriageState {
    if links.is_empty() {
        // Link rows beat the decision row (ADR-0044): a filing overrides
        // wherever both exist. Linkless, a no_filing decision reads
        // reviewed — the triager judged and no item was warranted.
        return if decision == Some(NO_FILING_DECISION) {
            TriageState::Reviewed
        } else {
            TriageState::Untriaged
        };
    }
    if links
        .iter()
        .any(|link| link.item_state == Some(ObservationWorkItemLinkItemState::InProgress))
    {
        return TriageState::InProgress;
    }
    if links
        .iter()
        .any(|link| link.item_state != Some(ObservationWorkItemLinkItemState::Closed))
    {
        return TriageState::Filed;
    }
    TriageState::Closed
}

/// One record's decision string (`no_filing` today), from the run's
/// cached decision rows — one store read per run for the whole request.
async fn decision_of_seq<'a>(
    state: &AppState,
    decisions_by_run: &'a mut HashMap<RunId, HashMap<u64, String>>,
    run_id: &RunId,
    seq: u64,
) -> Option<&'a str> {
    if !decisions_by_run.contains_key(run_id) {
        let decisions = state
            .work_store()
            .decisions_for_run(run_id)
            .await
            .unwrap_or_default();
        let map = decisions
            .into_iter()
            .map(|decision| (decision.seq, decision.decision))
            .collect();
        decisions_by_run.insert(*run_id, map);
    }
    decisions_by_run.get(run_id)?.get(&seq).map(String::as_str)
}

/// The triager's decision door (ADR-0044): records one triage decision
/// on one observation record, keyed to its append-only (run, seq)
/// identity. `no_filing` is the first action — the triager judged the
/// friction needs no work item, and the feed derives `reviewed` for the
/// record; a later filing overrides (link rows beat the decision row).
/// Upsert: the latest decision on a record wins. Authorization mirrors
/// the verdict door's chain: user principals write unconditionally, a
/// workflow run only when its workflow is the triager (the hardcoded
/// allowlist posture — the door's twin surfaces are the agent tool
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
        ObservationManageRequestAction::NoFiling => None,
    };
    let by_run = match principal {
        Principal::User(_) => None,
        Principal::Worker { run_id: writer } => {
            if !is_triage_manager_writer(&state, &writer).await? {
                return Err(ApiError::with_code(
                    StatusCode::FORBIDDEN,
                    format!(
                        "Only users and the {TRIAGER_WORKFLOW} workflow write triage \
                         decisions (ADR-0044) — this run's workflow is not on the allowlist."
                    ),
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

/// Whether the writer run's workflow is the triager (the manage door's
/// allowlist). An unreadable writer summary is an internal error, not a
/// silent refusal — the same posture as the verdict door.
async fn is_triage_manager_writer(state: &AppState, writer: &RunId) -> Result<bool, ApiError> {
    let summary = state
        .stores
        .run_summaries
        .get(writer, Utc::now())
        .await
        .map_err(|err| {
            tracing::error!(error = ?err, writer_run_id = %writer, "writer run summary read failed");
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Reading the writer run's summary failed.",
            )
        })?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::FORBIDDEN,
                "The writer run named by the worker token does not exist.",
            )
        })?;
    Ok(matches!(
        summary.workflow.slug.as_deref(),
        Some(TRIAGER_WORKFLOW)
    ))
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
    // Read once, like the store's own fold.
    let verdicts = match state
        .stores
        .run_summaries
        .platform_records()
        .latest_verdicts_by_run(field)
        .await
    {
        Ok(verdicts) => verdicts
            .into_iter()
            .collect::<HashMap<RunId, OrchestrationVerdict>>(),
        Err(err) => {
            return ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
                .into_response();
        }
    };
    let run_ids = runs.iter().map(|run| run.id).collect::<Vec<_>>();
    // The decoration carries the claim's item AND project (work#1113):
    // queue consumers read scoped doors (the closure radius) with the
    // claim's own project, never a hardcoded one.
    let claim_by_run = match state.stores.work.latest_claims_of_runs(&run_ids).await {
        Ok(claims) => claims
            .into_iter()
            .map(|claim| (claim.run_id, (claim.item_key, claim.project_id)))
            .collect::<HashMap<RunId, (String, String)>>(),
        Err(err) => {
            return ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
                .into_response();
        }
    };

    let data = state.decorate_run_summaries(runs).await;
    let entries = data
        .into_iter()
        .map(|run| {
            let verdict = match verdicts.get(&run.id) {
                Some(OrchestrationVerdict::Parked) => RunQueueEntryVerdict::Parked,
                Some(OrchestrationVerdict::Enabled) => RunQueueEntryVerdict::Enabled,
                Some(OrchestrationVerdict::Refused) => RunQueueEntryVerdict::Refused,
                _ => RunQueueEntryVerdict::Pending,
            };
            let (work_item, project) = match claim_by_run.get(&run.id) {
                Some((item_key, project_id)) => (Some(item_key.clone()), Some(project_id.clone())),
                None => (None, None),
            };
            RunQueueEntry {
                work_item,
                project,
                run,
                verdict,
            }
        })
        .collect::<Vec<_>>();
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

/// The verdict write door. Authorization is the identity chain of the
/// stage-D grill: the worker token names the WRITER run, the writer's
/// run summary names its workflow, the hardcoded pair binds the
/// workflow to the field it may write. The target run is the path.
async fn mark_orchestration(
    RequiredRunManagementActor(principal): RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(target): Path<RunId>,
    Json(request): Json<RunOrchestrationMarkRequest>,
) -> Result<Response, ApiError> {
    let Principal::Worker { run_id: writer } = principal else {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            "Only workflow runs write orchestration verdicts — the adjudicator and the merger \
             gears. Users read verdicts through the run view.",
            "orchestration_not_allowed",
        ));
    };
    if writer == target {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            "A run cannot mark itself — the adjudicator and the merger write verdicts about \
             OTHER runs (ADR-0039 decision 1: a run never judges itself).",
            "orchestration_self_mark",
        ));
    }
    let Some(owned_field) = owned_field_for_writer(&state, &writer).await? else {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            format!(
                "This run's workflow is not on the orchestration allowlist — only the \
                 {MERGE_WRITER_WORKFLOW} and {CLOSABLE_WRITER_WORKFLOW} workflows write \
                 verdicts."
            ),
            "orchestration_not_allowed",
        ));
    };
    if request.field != owned_field {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            format!(
                "One writer per field (ADR-0039 decision 6): this run's workflow writes \
                 `{owned_field}`, not `{}`.",
                request.field
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

/// The field this writer's workflow owns, `None` when the workflow is
/// not on the allowlist. An unreadable writer summary is an internal
/// error, not a silent refusal: the token named a run the store cannot
/// see.
async fn owned_field_for_writer(
    state: &AppState,
    writer: &RunId,
) -> Result<Option<OrchestrationField>, ApiError> {
    let summary = state
        .stores
        .run_summaries
        .get(writer, Utc::now())
        .await
        .map_err(|err| {
            tracing::error!(error = ?err, writer_run_id = %writer, "writer run summary read failed");
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Reading the writer run's summary failed.",
            )
        })?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::FORBIDDEN,
                "The writer run named by the worker token does not exist.",
            )
        })?;
    Ok(match summary.workflow.slug.as_deref() {
        Some(MERGE_WRITER_WORKFLOW) => Some(OrchestrationField::Merge),
        Some(CLOSABLE_WRITER_WORKFLOW) => Some(OrchestrationField::Closable),
        _ => None,
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
            "Only workflow runs merge pull requests — the merger gear. Users merge through \
             GitHub.",
            "merge_not_allowed",
        ));
    };
    if writer == target {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            "A run cannot merge itself — the merger integrates OTHER runs (ADR-0039 decision 1).",
            "merge_self",
        ));
    }
    let Some(owned_field) = owned_field_for_writer(&state, &writer).await? else {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            format!(
                "This run's workflow is not on the orchestration allowlist — only the \
                 {MERGE_WRITER_WORKFLOW} and {CLOSABLE_WRITER_WORKFLOW} workflows write \
                 verdicts or merge."
            ),
            "merge_not_allowed",
        ));
    };
    if owned_field != OrchestrationField::Closable {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            format!(
                "Only the {CLOSABLE_WRITER_WORKFLOW} merges pull requests — this run's workflow \
                 writes `{owned_field}` verdicts, it does not integrate."
            ),
            "merge_not_merger",
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

/// A linked work item's live state as the observations column shows it:
/// open, open with a live claim (a run is working it), or closed.
#[derive(Clone, Copy, Debug)]
enum LinkedItemState {
    Open,
    InProgress,
    Closed,
}

impl LinkedItemState {
    fn into_wire(self) -> ObservationWorkItemLinkItemState {
        match self {
            Self::Open => ObservationWorkItemLinkItemState::Open,
            Self::InProgress => ObservationWorkItemLinkItemState::InProgress,
            Self::Closed => ObservationWorkItemLinkItemState::Closed,
        }
    }
}

/// Resolve one linked item's state. The claim ledger outranks the
/// tracker read: a live claim means the item is open and being worked,
/// whatever the provider's indexing lag says.
///
/// The answer says WHY a state is missing (denkhaus/work#1364):
/// [`LinkedItemResolution::NotFound`] is the tracker's verdict that the
/// item is gone, [`LinkedItemResolution::Stale`] is a degradation — the
/// tracker could not be asked (project gone, credentials) or failed the
/// read (installation budget exhausted even after the client's
/// rate-limit retries). The feed surfaces stale as
/// `item_state_stale` instead of silently null.
#[derive(Clone, Debug)]
enum LinkedItemResolution {
    /// The tracker answered with the item's live state and URL.
    Resolved(ResolvedLinkedItem),

    /// The tracker answered: no such item — a verdict, not a failure.
    NotFound,

    /// The read FAILED (rate limit, transport, configuration) — the
    /// null state is a degradation, not a verdict.
    Stale,
}

/// A linked work item's live state and tracker URL, resolved together in one
/// tracker round-trip (denkhaus/work#1235: the client never builds forge
/// URLs — the work item is an abstract interface).
#[derive(Clone, Debug)]
struct ResolvedLinkedItem {
    state: LinkedItemState,
    url:   String,
}

async fn resolve_linked_item(
    state: &AppState,
    project_id: &str,
    item_key: &str,
) -> LinkedItemResolution {
    let Some(project) = state.work_store().project(project_id).await.ok().flatten() else {
        return LinkedItemResolution::Stale;
    };
    let Ok(tracker) = state.work_tracker_for_project(&project).await else {
        return LinkedItemResolution::Stale;
    };
    let item = match tracker.get(&project, item_key).await {
        Ok(Some(item)) => item,
        // The tracker answered: the item is gone — a verdict.
        Ok(None) => return LinkedItemResolution::NotFound,
        // The read failed (rate-limit budget exhausted, transport) — a
        // degradation the feed must mark, not hide (denkhaus/work#1364).
        Err(_) => return LinkedItemResolution::Stale,
    };
    let state = if matches!(
        state
            .work_store()
            .active_claim_for_item(project_id, item_key)
            .await,
        Ok(Some(_))
    ) {
        LinkedItemState::InProgress
    } else {
        match item.state.as_str() {
            "closed" => LinkedItemState::Closed,
            _ => LinkedItemState::Open,
        }
    };
    LinkedItemResolution::Resolved(ResolvedLinkedItem {
        state,
        url: item.url,
    })
}

/// The per-request batch of linked item states (denkhaus/work#1362):
/// each distinct (project, item) resolves EXACTLY ONCE per feed page or
/// walk — the tracker read and the claim-ledger read behind
/// [`resolve_linked_item`] are memoized here, so a page of records
/// linking the same item costs one round-trip, not one per record per
/// link. One resolver lives for one HTTP request and is dropped with
/// it: no cross-request state, no invalidation, no stale columns.
struct LinkedItemResolver<'a> {
    state:    &'a AppState,
    resolved: HashMap<(String, String), LinkedItemResolution>,
}

impl<'a> LinkedItemResolver<'a> {
    fn new(state: &'a AppState) -> Self {
        Self {
            state,
            resolved: HashMap::new(),
        }
    }

    async fn resolve(&mut self, project_id: &str, item_key: &str) -> LinkedItemResolution {
        let cache_key = (project_id.to_owned(), item_key.to_owned());
        if let Some(hit) = self.resolved.get(&cache_key) {
            return hit.clone();
        }
        let resolved = resolve_linked_item(self.state, project_id, item_key).await;
        self.resolved.insert(cache_key, resolved.clone());
        resolved
    }
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

#[cfg(test)]
mod link_resolution_tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use fabro_work::{TrackerProvider, WorkItem, WorkItemDraft, WorkItemFilter, WorkProject};

    use super::{LinkedItemResolution, LinkedItemState};
    use crate::server::AppState;
    use crate::test_support::TestAppStateBuilder;

    /// A tracker whose `get` answers one scripted item — the seam the
    /// observation tables' link resolution reads through
    /// (denkhaus/work#1235: state and URL resolve together, one
    /// round-trip). `get_fails` scripts the degraded read of
    /// denkhaus/work#1364: the installation budget is exhausted and the
    /// client's retries are spent.
    struct UrlScriptedTracker {
        item:      Option<WorkItem>,
        get_fails: bool,
    }

    #[async_trait]
    impl TrackerProvider for UrlScriptedTracker {
        async fn list(
            &self,
            _project: &WorkProject,
            _filter: WorkItemFilter,
        ) -> anyhow::Result<Vec<WorkItem>> {
            Ok(Vec::new())
        }

        async fn get(
            &self,
            _project: &WorkProject,
            _item_key: &str,
        ) -> anyhow::Result<Option<WorkItem>> {
            if self.get_fails {
                anyhow::bail!("API rate limit exceeded for installation ID 161242007");
            }
            Ok(self.item.clone())
        }

        async fn create(
            &self,
            _project: &WorkProject,
            _draft: WorkItemDraft,
        ) -> anyhow::Result<WorkItem> {
            anyhow::bail!("not needed")
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
            anyhow::bail!("not needed")
        }

        async fn comment(
            &self,
            _project: &WorkProject,
            _item_key: &str,
            _body: &str,
        ) -> anyhow::Result<()> {
            anyhow::bail!("not needed")
        }

        async fn project_claim(
            &self,
            _project: &WorkProject,
            _item_key: &str,
            _run_id: &str,
        ) -> anyhow::Result<()> {
            anyhow::bail!("not needed")
        }

        async fn clear_claim(
            &self,
            _project: &WorkProject,
            _item_key: &str,
            _run_id: &str,
        ) -> anyhow::Result<()> {
            anyhow::bail!("not needed")
        }
    }

    fn scripted_item(state: &str, url: &str) -> WorkItem {
        WorkItem {
            item_key:   "denkhaus/work#12".to_string(),
            title:      "Scripted".to_string(),
            body:       String::new(),
            state:      state.to_string(),
            labels:     Vec::new(),
            priority:   2,
            url:        url.to_string(),
            updated_at: None,
        }
    }

    async fn state_with(tracker: UrlScriptedTracker) -> Arc<AppState> {
        let state = TestAppStateBuilder::new()
            .work_tracker(Arc::new(tracker) as Arc<dyn TrackerProvider>)
            .build();
        let store = fabro_work::WorkStore::new(state.db_pool.clone());
        store
            .upsert_project(&WorkProject {
                project_id:               "fabro".to_string(),
                display_name:             "Fabro".to_string(),
                tracker_provider:         "github".to_string(),
                tracker_repo:             "denkhaus/work".to_string(),
                target_repo:              None,
                project_label:            "project:fabro".to_string(),
                lane_labels:              [("loop".to_string(), "lane:loop".to_string())]
                    .into_iter()
                    .collect(),
                claim_label_prefix:       "fabro:run:".to_string(),
                claim_comment_enabled:    false,
                backpressure_open_pr_cap: 0,
            })
            .await
            .expect("project upserts");
        state
    }

    #[tokio::test]
    async fn a_resolved_link_carries_its_state_and_tracker_url_together() {
        let state = state_with(UrlScriptedTracker {
            item:      Some(scripted_item(
                "open",
                "https://tracker.example/denkhaus/work/issues/12",
            )),
            get_fails: false,
        })
        .await;

        let resolved = super::resolve_linked_item(&state, "fabro", "denkhaus/work#12").await;

        let LinkedItemResolution::Resolved(resolved) = resolved else {
            panic!("the scripted item resolves: {resolved:?}");
        };
        assert!(
            matches!(resolved.state, LinkedItemState::Open),
            "the live state rides the resolution"
        );
        assert_eq!(
            resolved.url, "https://tracker.example/denkhaus/work/issues/12",
            "the tracker URL rides the SAME round-trip — clients never build forge URLs"
        );
    }

    #[tokio::test]
    async fn a_closed_item_resolves_closed_with_its_url() {
        let state = state_with(UrlScriptedTracker {
            item:      Some(scripted_item(
                "closed",
                "https://tracker.example/denkhaus/work/issues/12",
            )),
            get_fails: false,
        })
        .await;

        let resolved = super::resolve_linked_item(&state, "fabro", "denkhaus/work#12").await;

        let LinkedItemResolution::Resolved(resolved) = resolved else {
            panic!("the scripted item resolves: {resolved:?}");
        };
        assert!(matches!(resolved.state, LinkedItemState::Closed));
        assert_eq!(
            resolved.url,
            "https://tracker.example/denkhaus/work/issues/12"
        );
    }

    #[tokio::test]
    async fn an_absent_item_is_the_trackers_verdict_not_a_stale_read() {
        let state = state_with(UrlScriptedTracker {
            item:      None,
            get_fails: false,
        })
        .await;

        assert!(
            matches!(
                super::resolve_linked_item(&state, "fabro", "denkhaus/work#13").await,
                LinkedItemResolution::NotFound
            ),
            "the tracker answered — honest nulls, no staleness"
        );
    }

    #[tokio::test]
    async fn a_failed_tracker_read_is_stale_not_silently_null() {
        let state = state_with(UrlScriptedTracker {
            item:      None,
            get_fails: true,
        })
        .await;

        assert!(
            matches!(
                super::resolve_linked_item(&state, "fabro", "denkhaus/work#12").await,
                LinkedItemResolution::Stale
            ),
            "the failed read is the explicit degradation denkhaus/work#1364 names"
        );
    }
}
