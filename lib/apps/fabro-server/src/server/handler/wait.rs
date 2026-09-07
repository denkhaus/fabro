//! Long-poll wait endpoint for runs (`GET /runs/{id}/wait`, fabro-571e).
//!
//! One blocking call replaces agent-side sleep/poll loops: callers ask for
//! a terminal run state or a merged run pull request and hold the
//! connection open until the condition is met or their own deadline
//! expires. A deadline hit is a structured `timeout` result carrying the
//! current status, so callers re-wait or route without guessing.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use chrono::Utc;
use fabro_api::types::{RunWaitResult, RunWaitResultReached};
use fabro_types::RunId;
use fabro_types::status::RunStatus;
use serde::Deserialize;
use tokio::time::{sleep, timeout};

use crate::error::ApiError;
use crate::principal_middleware::RequireRunManagementTarget;
use crate::server::AppState;
use crate::server::handler::pull_requests::{self, PullRequestGithubContext};

/// Longest single wait a caller may request. Longer waits re-issue the
/// call; this keeps held connections and in-flight work bounded when a
/// client walks away without cancelling.
const MAX_TIMEOUT_MS: u64 = 3_600_000;

/// Store poll cadence while waiting. Run status transitions are also
/// broadcast on the global event bus, but a short poll keeps the wait
/// correct when broadcasts are missed (server restart, backpressure).
const STORE_POLL_INTERVAL: Duration = Duration::from_millis(1_500);

/// GitHub poll cadence for `until=merged`. Generous by design: merge
/// completion is a minutes-scale process, and the API budget is shared
/// with PR creation and supervision.
const GITHUB_POLL_INTERVAL: Duration = Duration::from_secs(10);

/// Per-call timeout for the external GitHub fetch. A hung request must
/// not outlive the caller's wait deadline; the poll loop keeps
/// retrying on timeout until the deadline expires.
const GITHUB_CALL_TIMEOUT: Duration = Duration::from_secs(15);

/// How many consecutive GitHub polls must report a blocked/dirty
/// `mergeable_state` before the wait reports `reached=blocked`. One
/// observation can be GitHub mid-recompute; two sustained observations
/// mean the merge gate (required checks or a mergeable base) is stuck
/// and re-waiting will not clear it (fabro-bde4).
const BLOCKED_POLL_THRESHOLD: u32 = 2;

/// Confirmation poll cadence after a first blocked/dirty observation.
/// Shorter than [`GITHUB_POLL_INTERVAL`]: the sustained-window check
/// needs a quick second sample, not another full poll period, before
/// the state can flip back to clean/unknown on a transient.
const BLOCKED_CONFIRMATION_POLL_INTERVAL: Duration = Duration::from_millis(1_500);

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum WaitUntil {
    Terminal,
    Merged,
}

#[derive(Deserialize, Debug)]
struct WaitRunParams {
    until:      WaitUntil,
    timeout_ms: u64,
}

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/runs/{id}/wait", get(wait_run))
}

async fn wait_run(
    RequireRunManagementTarget(id, _actor): RequireRunManagementTarget,
    State(state): State<Arc<AppState>>,
    Query(params): Query<WaitRunParams>,
) -> Response {
    // Boundary validation, not silent reshaping: the documented contract
    // (OpenAPI minimum/maximum) rejects out-of-range deadlines with 400 so
    // callers learn their input was wrong (standards review finding).
    if params.timeout_ms == 0 || params.timeout_ms > MAX_TIMEOUT_MS {
        return ApiError::bad_request(format!("timeout_ms must be between 1 and {MAX_TIMEOUT_MS}"))
            .into_response();
    }
    let timeout = Duration::from_millis(params.timeout_ms);
    let deadline = Instant::now() + timeout;

    match params.until {
        WaitUntil::Terminal => wait_until_terminal(&state, &id, deadline).await,
        WaitUntil::Merged => {
            // Precondition: a stored PR link and GitHub credentials. Failing
            // fast beats holding a doomed wait.
            match pull_requests::load_pull_request_github_context(&state, &id).await {
                Ok(ctx) => wait_until_merged(&state, &id, &ctx, deadline).await,
                Err(err) => err.into_response(),
            }
        }
    }
}

/// Poll the store until the run is terminal or the deadline expires.
async fn wait_until_terminal(state: &AppState, id: &RunId, deadline: Instant) -> Response {
    loop {
        let status = match current_status(state, id).await {
            Ok(status) => status,
            Err(response) => return response,
        };
        if status.is_terminal() {
            return wait_result(id, RunWaitResultReached::Terminal, status, None);
        }
        if let Some(status) = deadline_status(deadline, status).await {
            return wait_result(id, RunWaitResultReached::Timeout, status, None);
        }
    }
}

/// Poll the store and the GitHub pull request until the PR merges, the run
/// fails hard, the PR closes without merging, the PR's merge gate stays
/// blocked, or the deadline expires.
async fn wait_until_merged(
    state: &AppState,
    id: &RunId,
    ctx: &PullRequestGithubContext,
    deadline: Instant,
) -> Response {
    let github = match pull_requests::server_github_context(state, &ctx.creds) {
        Ok(github) => github,
        Err(err) => return err.into_response(),
    };

    let mut last_github_poll: Option<Instant> = None;
    let mut blocked_streak: u32 = 0;
    loop {
        let status = match current_status(state, id).await {
            Ok(status) => status,
            Err(response) => return response,
        };
        // A run that failed hard will not merge; surface the terminal state
        // instead of holding the connection until the deadline.
        if matches!(status, RunStatus::Failed { .. } | RunStatus::Dead) {
            return wait_result(
                id,
                RunWaitResultReached::Terminal,
                status,
                Some(ctx.record.clone()),
            );
        }
        let poll_due = last_github_poll.is_none_or(|last| {
            let interval = if blocked_streak > 0 {
                BLOCKED_CONFIRMATION_POLL_INTERVAL
            } else {
                GITHUB_POLL_INTERVAL
            };
            last.elapsed() >= interval
        });
        if poll_due {
            last_github_poll = Some(Instant::now());
            let fetch = timeout(
                GITHUB_CALL_TIMEOUT,
                fabro_github::get_pull_request(&github, &ctx.owner, &ctx.repo, ctx.number),
            )
            .await;
            match fetch {
                Ok(Ok(detail)) => {
                    if detail.merged {
                        return wait_result(
                            id,
                            RunWaitResultReached::Merged,
                            status,
                            Some(ctx.record.clone()),
                        );
                    }
                    if detail.state.eq_ignore_ascii_case("closed") {
                        return wait_result(
                            id,
                            RunWaitResultReached::ClosedUnmerged,
                            status,
                            Some(ctx.record.clone()),
                        );
                    }
                    // An open PR whose mergeable_state stays dirty/blocked
                    // across consecutive polls is a stuck merge gate (failed
                    // required checks or an unmergeable base). `None` and
                    // "unknown" mean GitHub is still computing and never
                    // count (fabro-bde4).
                    if detail.mergeable_state.as_deref().is_some_and(|state| {
                        state.eq_ignore_ascii_case("dirty") || state.eq_ignore_ascii_case("blocked")
                    }) {
                        blocked_streak = blocked_streak.saturating_add(1);
                        if blocked_streak >= BLOCKED_POLL_THRESHOLD {
                            return wait_result(
                                id,
                                RunWaitResultReached::Blocked,
                                status,
                                Some(ctx.record.clone()),
                            );
                        }
                    } else {
                        blocked_streak = 0;
                    }
                }
                Ok(Err(fabro_github::PullRequestApiError::NotFound { .. })) => {
                    // A deleted PR will never merge; report it as
                    // closed-unmerged so callers route to repair.
                    return wait_result(
                        id,
                        RunWaitResultReached::ClosedUnmerged,
                        status,
                        Some(ctx.record.clone()),
                    );
                }
                Ok(Err(err)) => {
                    // Transient GitHub failures keep the wait alive; the
                    // deadline still bounds the total attempt.
                    tracing::warn!(
                        run_id = ?id,
                        error = ?err,
                        "run wait pull request poll failed"
                    );
                }
                Err(_) => {
                    tracing::warn!(
                        run_id = ?id,
                        timeout_secs = GITHUB_CALL_TIMEOUT.as_secs(),
                        "run wait pull request poll timed out"
                    );
                }
            }
        }
        if let Some(status) = deadline_status(deadline, status).await {
            return wait_result(
                id,
                RunWaitResultReached::Timeout,
                status,
                Some(ctx.record.clone()),
            );
        }
    }
}

/// Load the run's current status. An `Err` response ends the wait: the run
/// vanished from the store or the store itself failed.
async fn current_status(state: &AppState, id: &RunId) -> Result<RunStatus, Response> {
    match state.stores.run_summaries.get(id, Utc::now()).await {
        Ok(Some(run)) => Ok(run.lifecycle.status),
        Ok(None) => Err(ApiError::not_found("Run not found.").into_response()),
        Err(err) => {
            Err(ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response())
        }
    }
}

/// Sleep one store-poll interval unless the deadline is due first.
/// Returns the deadline status when the wait expired, `None` to poll
/// again. Splitting the deadline check into the sleep keeps both loops
/// free of duplicated clock arithmetic.
async fn deadline_status(deadline: Instant, status: RunStatus) -> Option<RunStatus> {
    let now = Instant::now();
    if now >= deadline {
        return Some(status);
    }
    sleep(STORE_POLL_INTERVAL.min(deadline - now)).await;
    None
}

fn wait_result(
    id: &RunId,
    reached: RunWaitResultReached,
    status: RunStatus,
    pull_request: Option<fabro_types::PullRequestLink>,
) -> Response {
    tracing::debug!(run_id = ?id, reached = %reached, "run wait finished");
    (
        StatusCode::OK,
        Json(RunWaitResult {
            run_id: id.to_string(),
            reached,
            status,
            pull_request,
        }),
    )
        .into_response()
}
