//! Fork-only pull-request staleness supervisor (fabro-895d + fabro-94e8,
//! petri port W2-2 Phase A 2026-09-21).
//!
//! Every run-linked pull request is polled: dirty PRs get update-branch
//! (3 strikes then close), tracker-only conflicts get a closed-wins union
//! merge through the git data API, over-age PRs (>24h) retire — except
//! bookkeeping-only PRs, which PARK so their tracker state is never
//! silently dropped. Merged/closed PRs clear their counters.
//!
//! Petri adaptations (from the engine-era original):
//! - candidates come from `platform_records` (latest pull_request.* record
//!   linked), not from the deleted run event log;
//! - projections load through `run_records::projection`;
//! - closing records a `PullRequestUnlinked` platform record (the old
//!   PullRequestClosed event has no petri equivalent; unlink is the
//!   projection's "this run no longer has a PR" state).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use fabro_store::platform_records::{PlatformRecord, PullRequestLinkedRecord};
use fabro_types::RunId;
use tokio::task::JoinHandle;
use tokio::time;
use tracing::{Instrument as _, info, info_span, warn};

use super::handler::pull_requests::{load_server_github_credentials, server_github_context};
use super::pull_request_conflict::{
    ConflictResolutionClient, all_bookkeeping_paths, all_tracker_paths,
};
use super::{AppState, run_records};

const PULL_REQUEST_STALENESS_SCAN_INTERVAL: Duration = Duration::from_mins(15);
const PULL_REQUEST_GITHUB_CALL_TIMEOUT: Duration = Duration::from_secs(30);
const PULL_REQUEST_MAX_AGE: Duration = Duration::from_hours(24);
const MAX_UPDATE_BRANCH_ATTEMPTS: u32 = 3;

/// In-memory staleness bookkeeping (a server restart resets the strike
/// cap, which only delays retirement by at most three more polls).
#[derive(Default)]
pub(super) struct StalePrState {
    pub(super) update_failures: HashMap<RunId, u32>,
    pub(super) parked:          HashMap<RunId, String>,
}

pub(crate) fn spawn_pull_request_staleness_supervisor(state: Arc<AppState>) -> JoinHandle<()> {
    tokio::spawn(
        run_pull_request_staleness_supervisor(state)
            .instrument(info_span!("pull_request_staleness_supervisor")),
    )
}

async fn run_pull_request_staleness_supervisor(state: Arc<AppState>) {
    let shutdown = state.shutdown_token();
    let mut scan_interval = time::interval(PULL_REQUEST_STALENESS_SCAN_INTERVAL);
    scan_interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
    let mut counters = StalePrState::default();

    loop {
        tokio::select! {
            () = shutdown.cancelled() => break,
            _ = scan_interval.tick() => {
                if let Err(error) =
                    process_stale_pull_requests(&state, &mut counters).await
                {
                    warn!(%error, "Failed to scan stale run pull requests");
                }
            }
        }
    }
}

/// One staleness pass over every run-linked pull request. Public to the
/// crate so wire tests drive it deterministically instead of waiting for
/// the interval.
pub(super) async fn process_stale_pull_requests(
    state: &AppState,
    counters: &mut StalePrState,
) -> anyhow::Result<()> {
    let candidates = state
        .stores
        .run_summaries
        .list_linked_pull_request_run_ids()
        .await?;
    if candidates.is_empty() {
        return Ok(());
    }
    let creds = match load_server_github_credentials(state).await {
        Ok(creds) => creds,
        Err(err) => {
            warn!(error = %err.detail(), "Skipping stale pull request scan: GitHub integration unavailable");
            return Ok(());
        }
    };
    let github = server_github_context(state, &creds)
        .map_err(|err| anyhow::anyhow!("GitHub integration unavailable: {}", err.detail()))?;

    for run_id in candidates {
        // Parked PRs (fabro-895d) stay open untouched: no updates, no close.
        if counters.parked.contains_key(&run_id) {
            continue;
        }
        let projection = match run_records::projection(state, run_id).await {
            Ok(Some(projection)) => projection,
            Ok(None) => continue,
            Err(error) => {
                warn!(%run_id, %error, "Failed to load stale pull request candidate");
                continue;
            }
        };
        let Some(record) = projection.pull_request.clone() else {
            continue;
        };

        let fetch = time::timeout(
            PULL_REQUEST_GITHUB_CALL_TIMEOUT,
            fabro_github::get_pull_request(&github, &record.owner, &record.repo, record.number),
        )
        .await;
        let detail = match fetch {
            Ok(Ok(detail)) => detail,
            Ok(Err(fabro_github::PullRequestApiError::NotFound { .. })) => continue,
            Ok(Err(error)) => {
                warn!(%run_id, %error, "Failed to fetch run-linked pull request");
                continue;
            }
            Err(_) => {
                warn!(%run_id, "Run-linked pull request fetch timed out");
                continue;
            }
        };

        if detail.merged || !detail.state.eq_ignore_ascii_case("open") {
            counters.update_failures.remove(&run_id);
            counters.parked.remove(&run_id);
            continue;
        }

        // Age cap first (fabro-94e8 Q4): a run PR older than 24h retires —
        // unless every changed path is dev-loop bookkeeping, in which case
        // it parks so its tracker state is never silently dropped.
        let created_at = chrono::DateTime::parse_from_rfc3339(&detail.created_at)
            .map(|parsed| parsed.with_timezone(&chrono::Utc))
            .ok();
        match created_at {
            Some(created_at) if created_at < chrono::Utc::now() - PULL_REQUEST_MAX_AGE => {
                match over_age_disposition(state, &creds, &run_id, &record).await {
                    OverAgeDisposition::Park => {
                        park_stale_pull_request(
                            counters,
                            &run_id,
                            &record,
                            "over-age pull request only carries dev-loop bookkeeping changes",
                        );
                    }
                    OverAgeDisposition::Close => {
                        close_stale_pull_request(state, &github, &run_id, &record).await?;
                    }
                }
                counters.update_failures.remove(&run_id);
                continue;
            }
            Some(_) => {}
            None => {
                warn!(%run_id, created_at = %detail.created_at, "Cannot parse pull request created_at");
            }
        }

        // Poll set: mergeable=false AND dirty. `blocked` means required
        // checks, not a moved base — updating would not clear it (Q2).
        let dirty = detail.mergeable == Some(false)
            && detail
                .mergeable_state
                .as_deref()
                .is_some_and(|state| state.eq_ignore_ascii_case("dirty"));
        if !dirty {
            counters.update_failures.remove(&run_id);
            continue;
        }

        let failed = counters.update_failures.get(&run_id).copied().unwrap_or(0);
        if failed >= MAX_UPDATE_BRANCH_ATTEMPTS {
            close_stale_pull_request(state, &github, &run_id, &record).await?;
            counters.update_failures.remove(&run_id);
            continue;
        }

        let update = time::timeout(
            PULL_REQUEST_GITHUB_CALL_TIMEOUT,
            fabro_github::update_pull_request_branch(
                &github,
                &record.owner,
                &record.repo,
                record.number,
            ),
        )
        .await;
        match update {
            Ok(Ok(())) => {
                counters.update_failures.remove(&run_id);
                info!(%run_id, pr_number = record.number, "Update-branch requested for stale run pull request");
            }
            Ok(Err(fabro_github::PullRequestApiError::Conflict { status, .. })) => {
                // Q3: the PR stays open; the run wait observes
                // closed_unmerged/timeout and the conductor routes manual.
                match handle_update_conflict(state, &creds, &run_id, &record, &detail, status).await
                {
                    ConflictOutcome::Resolved => {
                        counters.update_failures.remove(&run_id);
                        info!(
                            %run_id,
                            pr_number = record.number,
                            "Resolved tracker JSONL update-branch conflict with a closed-wins union merge"
                        );
                    }
                    ConflictOutcome::Parked { reason } => {
                        park_stale_pull_request(counters, &run_id, &record, &reason);
                    }
                    ConflictOutcome::Struck => {
                        // A strike stays a strike only when the conflict is
                        // not confined to loop tracker JSONL (fabro-895d).
                        let attempts = counters.update_failures.entry(run_id).or_default();
                        *attempts += 1;
                        warn!(
                            %run_id,
                            pr_number = record.number,
                            status,
                            attempts = *attempts,
                            "Update-branch conflicted for run pull request"
                        );
                    }
                }
            }
            Ok(Err(fabro_github::PullRequestApiError::NotFound { .. })) => {}
            Ok(Err(error)) => {
                warn!(%run_id, %error, "Update-branch failed for run pull request");
            }
            Err(_) => {
                warn!(%run_id, "Update-branch timed out for run pull request");
            }
        }
    }
    Ok(())
}

/// What the supervisor does with an over-age run pull request (fabro-895d).
enum OverAgeDisposition {
    Park,
    Close,
}

async fn over_age_disposition(
    state: &AppState,
    creds: &fabro_github::GitHubCredentials,
    run_id: &RunId,
    record: &fabro_types::PullRequestLink,
) -> OverAgeDisposition {
    let client =
        match ConflictResolutionClient::new(state, creds, &record.owner, &record.repo).await {
            Ok(client) => client,
            Err(error) => {
                warn!(
                    %run_id,
                    pr_number = record.number,
                    %error,
                    "Cannot classify over-age pull request changes; keeping the close disposition"
                );
                return OverAgeDisposition::Close;
            }
        };
    match client.list_pull_request_files(record.number).await {
        Ok(files) if all_bookkeeping_paths(&files) => OverAgeDisposition::Park,
        Ok(_) => OverAgeDisposition::Close,
        Err(error) => {
            warn!(
                %run_id,
                pr_number = record.number,
                %error,
                "Cannot list over-age pull request files; keeping the close disposition"
            );
            OverAgeDisposition::Close
        }
    }
}

/// Outcome of one conflicted update-branch attempt (fabro-895d).
enum ConflictOutcome {
    Resolved,
    Parked { reason: String },
    Struck,
}

/// Classify and, when possible, resolve a 422 update-branch conflict.
///
/// Conflicting paths are the intersection of head-side and base-side
/// changes since the merge base (both fetched via the compare API). An
/// intersection confined to `.seeds/**`/`.mulch/**` is resolved by
/// unioning both sides of each file with closed-wins semantics and pushing
/// the merge commit; a resolution failure parks the PR.
async fn handle_update_conflict(
    state: &AppState,
    creds: &fabro_github::GitHubCredentials,
    run_id: &RunId,
    record: &fabro_types::PullRequestLink,
    detail: &fabro_types::PullRequestGithubDetail,
    status: u16,
) -> ConflictOutcome {
    let client =
        match ConflictResolutionClient::new(state, creds, &record.owner, &record.repo).await {
            Ok(client) => client,
            Err(error) => {
                warn!(
                    %run_id,
                    pr_number = record.number,
                    %error,
                    "Cannot classify update-branch conflict; counting a strike"
                );
                return ConflictOutcome::Struck;
            }
        };
    let base_ref = detail.base.ref_name.as_str();
    let head_ref = detail.head.ref_name.as_str();
    // Head side with change statuses: the run branch's own changes, reused
    // both for the conflict intersection and as the run-scoped overlay of
    // the diff-based merge tree (fabro-4ebd).
    let head_side = client
        .compare_entries(&format!("{base_ref}...{head_ref}"))
        .await;
    let base_side = client
        .compare_filenames(&format!("{head_ref}...{base_ref}"))
        .await;
    let (head_side, base_side) = match (head_side, base_side) {
        (Ok(head_side), Ok(base_side)) => (head_side, base_side),
        (Err(error), _) | (_, Err(error)) => {
            warn!(
                %run_id,
                pr_number = record.number,
                %error,
                "Cannot compare base/head sides of the update-branch conflict; counting a strike"
            );
            return ConflictOutcome::Struck;
        }
    };

    let head_set: std::collections::HashSet<&str> = head_side
        .iter()
        .map(|entry| entry.filename.as_str())
        .collect();
    let conflicting: Vec<String> = base_side
        .into_iter()
        .filter(|path| head_set.contains(path.as_str()))
        .collect();

    if !all_tracker_paths(&conflicting) {
        if conflicting.is_empty() {
            warn!(
                %run_id,
                pr_number = record.number,
                status,
                "Update-branch conflict has no overlapping changed paths to resolve; counting a strike"
            );
        }
        return ConflictOutcome::Struck;
    }

    match client
        .resolve_tracker_conflict(base_ref, head_ref, &conflicting, &head_side)
        .await
    {
        Ok(()) => ConflictOutcome::Resolved,
        Err(reason) => ConflictOutcome::Parked {
            reason: format!("tracker JSONL conflict resolution failed: {reason}"),
        },
    }
}

fn park_stale_pull_request(
    counters: &mut StalePrState,
    run_id: &RunId,
    record: &fabro_types::PullRequestLink,
    reason: &str,
) {
    warn!(
        %run_id,
        pr_number = record.number,
        reason,
        "Parking stale run pull request — it stays open; resolve manually"
    );
    counters.parked.insert(*run_id, reason.to_string());
}

async fn close_stale_pull_request(
    state: &AppState,
    github: &fabro_github::GitHubContext<'_>,
    run_id: &RunId,
    record: &fabro_types::PullRequestLink,
) -> anyhow::Result<()> {
    if let Err(error) =
        fabro_github::close_pull_request(github, &record.owner, &record.repo, record.number).await
    {
        warn!(
            %run_id,
            pr_number = record.number,
            %error,
            "Failed to close stale run pull request"
        );
        return Ok(());
    }
    info!(%run_id, pr_number = record.number, "Closed stale run pull request");
    // Petri: closing is an unlink in the projection (no Closed record kind).
    run_records::append(
        state,
        *run_id,
        PlatformRecord::PullRequestUnlinked(PullRequestLinkedRecord {
            owner:  record.owner.clone(),
            repo:   record.repo.clone(),
            number: record.number,
        }),
    )
    .await
    .map_err(|error| anyhow::anyhow!("recording pull request unlink: {error}"))?;
    Ok(())
}
