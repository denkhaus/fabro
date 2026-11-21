//! The acceptance fold's row resolver (denkhaus/work#1555): the server
//! half that reads the stores the pure fold
//! ([`fabro_work::acceptance_fold`]) consumes. The fold itself is pure
//! vocabulary over rows — this module is the one home that knows WHICH
//! rows: the item's view-store state, the item's claim history, each
//! claim run's latest `merge`/`closable` verdicts, and each claim PR's
//! observed state.
//!
//! Every reader of the fold resolves rows HERE (the reconciler's pass,
//! the queue view; the close door rides the work#1552 follow-up), so
//! the input shapes stay identical across consumers — the B1 finding's
//! "one derived reader" is this module plus the pure function, never a
//! per-surface re-derivation. Verdict maps are read once per batch
//! (`latest_verdicts_by_run`) and shared across the batch's claims; a
//! single claim resolves its own reads.

use std::collections::HashMap;

use anyhow::{Context, Result};
use fabro_types::{OrchestrationField, OrchestrationVerdict, PullRequestState, RunId};

use super::AppState;
use fabro_work::{AcceptanceState, ClaimFold, WorkClaim};

/// The per-claim run-scoped evidence a [`ClaimFold`] needs besides the
/// claim row itself. A plain-data carry between the batched verdict
/// reads and the pure fold.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ClaimVerdicts {
    pub merge:    Option<OrchestrationVerdict>,
    pub closable: Option<OrchestrationVerdict>,
}

/// The latest verdict per run for BOTH orchestration fields the fold
/// reads — the batch read (two store folds, shared by every claim of a
/// pass or a queue page). Never per-claim: `latest_verdict_of_run`
/// scans the records table per call, and a batch that called it per
/// claim would pay a full scan per row.
pub(crate) async fn verdict_maps(
    state: &AppState,
) -> Result<(HashMap<RunId, OrchestrationVerdict>, HashMap<RunId, OrchestrationVerdict>)> {
    let merge = latest_verdict_map(state, OrchestrationField::Merge).await?;
    let closable = latest_verdict_map(state, OrchestrationField::Closable).await?;
    Ok((merge, closable))
}

async fn latest_verdict_map(
    state: &AppState,
    field: OrchestrationField,
) -> Result<HashMap<RunId, OrchestrationVerdict>> {
    Ok(state
        .stores
        .run_summaries
        .platform_records()
        .latest_verdicts_by_run(field)
        .await
        .context("reading the latest verdicts by run")?
        .into_iter()
        .collect())
}

/// The claim's verdicts out of a batch's maps — the no-I/O half of the
/// resolution.
pub(crate) fn verdicts_from_maps(
    run_id: &RunId,
    merge: &HashMap<RunId, OrchestrationVerdict>,
    closable: &HashMap<RunId, OrchestrationVerdict>,
) -> ClaimVerdicts {
    ClaimVerdicts {
        merge:    merge.get(run_id).copied(),
        closable: closable.get(run_id).copied(),
    }
}

/// The PR state the platform observed for the claim's pull request,
/// read off the claim run's stored projection (no projector settle —
/// the fold re-derives every pass, it never forces one). `None` when
/// the projection is gone or the link carries no observed state.
pub(crate) async fn claim_pr_state(state: &AppState, claim: &WorkClaim) -> Option<PullRequestState> {
    let projection = state
        .stores
        .run_summaries
        .load_petri_projection(&claim.run_id)
        .await
        .ok()
        .flatten()?;
    let pr_url = claim.pr_url.as_deref();
    projection
        .pull_request
        .as_ref()
        .filter(|link| pr_url.is_none_or(|url| link.html_url() == url))
        .and_then(|link| link.state)
}

/// Assemble one claim's [`ClaimFold`] from maps plus an already-known
/// PR state (a caller that holds the run's projection reads no store).
pub(crate) fn claim_fold<'a>(
    claim: &'a WorkClaim,
    verdicts: ClaimVerdicts,
    pr_state: Option<PullRequestState>,
) -> ClaimFold<'a> {
    ClaimFold {
        claim,
        merge_verdict: verdicts.merge,
        closable_verdict: verdicts.closable,
        pr_state,
    }
}

/// The item's tracker-visible open state as projected locally: the view
/// store's row. A missing row reads OPEN — the safe side for every
/// consumer (the claim pool lists unclaimed items; a close precondition
/// must run its checks).
async fn item_open(state: &AppState, project_id: &str, item_key: &str) -> bool {
    match state.stores.work_item_views.get(project_id, item_key).await {
        Ok(Some(item)) => item.item.state == "open",
        Ok(None) => true,
        Err(error) => {
            tracing::warn!(
                project_id,
                item_key,
                error = ?error,
                "work item view read failed; the fold reads the item open"
            );
            true
        }
    }
}

/// The item-level fold, fully resolved from the stores: the claim
/// history, each claim's verdicts (from the batch maps) and PR state
/// (one projection read per claim), and the view-store item state. The
/// queue view's per-entry read; the reconciler folds per claim through
/// [`claim_fold`].
pub(crate) async fn item_fold(
    state: &AppState,
    project_id: &str,
    item_key: &str,
    verdicts: (&HashMap<RunId, OrchestrationVerdict>, &HashMap<RunId, OrchestrationVerdict>),
) -> Result<AcceptanceState> {
    let claims = state
        .work_store()
        .claims_for_item(project_id, item_key)
        .await
        .context("reading the item's claim history")?;
    let open = item_open(state, project_id, item_key).await;
    let mut folds = Vec::with_capacity(claims.len());
    for claim in &claims {
        let claim_verdicts = verdicts_from_maps(&claim.run_id, verdicts.0, verdicts.1);
        let pr_state = claim_pr_state(state, claim).await;
        folds.push(claim_fold(claim, claim_verdicts, pr_state));
    }
    Ok(fabro_work::acceptance_fold(open, &folds))
}

/// One claim's fold with a projection the caller already holds (the
/// reconciler's run-state path reads it anyway) and verdicts resolved
/// from the pass's batch maps. `projection` is the claim run's stored
/// projection, `None` when the run record is gone — the PR state is
/// then unobserved, and the reconciler's orphan grace owns that case.
pub(crate) fn claim_fold_with<'a>(
    claim: &'a WorkClaim,
    verdicts: ClaimVerdicts,
    projection: Option<&fabro_types::RunProjection>,
) -> ClaimFold<'a> {
    let pr_state = projection
        .and_then(|projection| projection.pull_request.as_ref())
        .filter(|link| {
            claim
                .pr_url
                .as_deref()
                .is_none_or(|url| link.html_url() == url)
        })
        .and_then(|link| link.state);
    claim_fold(claim, verdicts, pr_state)
}

/// The fold for one live claim, fully resolved (denkhaus/work#1555):
/// item state from the view store, verdicts from the batch maps, PR
/// state off a projection — the caller's when it holds one (the
/// reconciler's run-state path reads the settled projection anyway),
/// else the stored projection read here without a settle (the fold
/// re-derives every pass; it never forces the projector). The
/// reconciler's worklist arms read THIS state: `merge_refused` and
/// `closed_unlanded` release, everything else holds.
pub(crate) async fn live_claim_fold(
    state: &AppState,
    claim: &WorkClaim,
    verdicts: (&HashMap<RunId, OrchestrationVerdict>, &HashMap<RunId, OrchestrationVerdict>),
    projection: Option<&fabro_types::RunProjection>,
) -> Result<AcceptanceState> {
    let stored;
    let projection = match projection {
        Some(projection) => Some(projection),
        None => {
            stored = state
                .stores
                .run_summaries
                .load_petri_projection(&claim.run_id)
                .await
                .context("reading the claim run's stored projection")?;
            stored.as_deref()
        }
    };
    let open = item_open(state, &claim.project_id, &claim.item_key).await;
    let claim_verdicts = verdicts_from_maps(&claim.run_id, verdicts.0, verdicts.1);
    let fold = claim_fold_with(claim, claim_verdicts, projection);
    Ok(fabro_work::acceptance_fold(open, &[fold]))
}
