//! The platform's publish of a finished run's branch (fabro-ac40).
//!
//! The petri rework dropped the legacy engine's run-branch push: a
//! successful run's work stays in the server's checkpoint (the snapshot
//! repositories and the `run.diff` blob), and the standalone runner does
//! no Git operations by design. This supervisor closes the gap from the
//! platform side, durably:
//!
//! - a run that reached terminal success with `[run.run_branch] push` gets its
//!   EXACT final commit (the `run.diff` head) pushed from the run's snapshot
//!   repository to the origin under its run branch (`fabro/run/<id>`),
//!   authenticated with the server's GitHub credentials — the same push the
//!   manual bridge performed, now in-process and idempotent (a retry of an
//!   already-pushed branch is "everything up-to-date");
//! - the outcome is a `run.branch_published` platform record: published,
//!   skipped (with a machine-readable reason), or terminally failed after
//!   bounded retries. Any outcome settles the debt, so the candidate set stays
//!   bounded;
//! - when the run's `[run.pull_request] enabled` is set, a successful publish
//!   requests the pull request itself, exactly as `POST
//!   /runs/{id}/pull_request` does: a `pull_request.requested` record the pull
//!   request supervisor picks up.
//!
//! Candidates come from a store scan (succeeded runs with a repository,
//! no outcome record, finished after a boot-time cutoff that keeps
//! pre-feature backlogs untouched) plus the projector's signals, so a
//! run finishing while the server lives is published without waiting
//! for the scan interval, and a run that finished during a restart is
//! picked up by the scan's lookback.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use fabro_config::Storage;
use fabro_store::RunProjection;
use fabro_store::platform_records::{
    PlatformRecord, PlatformRecordKind, PullRequestRequestedRecord, RunBranchPublishOutcome,
    RunBranchPublishedRecord,
};
use fabro_types::{PullRequestCreationId, RunId};
use tokio::process::Command as TokioCommand;
use tokio::sync::broadcast::error::RecvError;
use tokio::task::{self, JoinHandle, JoinSet};
use tokio::{fs, time};
use tracing::{Instrument as _, info, info_span, warn};

use super::{AppState, run_records};

/// How often the scan re-reads the store for publish candidates.
const PUBLISH_SCAN_INTERVAL: Duration = Duration::from_secs(30);
/// How far back a booting server adopts finished runs: a run that
/// completed within this window before startup is still published, older
/// backlogs are deliberately left alone (fabro-ac40 landed with them).
const PUBLISH_BACKLOG_LOOKBACK: Duration = Duration::from_hours(2);
/// How many runs one scan hands to workers at most.
const PUBLISH_SCAN_LIMIT: u32 = 128;
/// Publishes waiting for a worker at most; the store scan re-offers what
/// does not fit.
const PUBLISH_QUEUE_CAPACITY: usize = MAX_CONCURRENT_PUBLISHES * 8;
/// Publishes in flight at once; the work is a local git push, so the
/// bound guards token mints and GitHub pressure, not CPU.
const MAX_CONCURRENT_PUBLISHES: usize = 2;
/// Push attempts before a publish is recorded terminally failed —
/// replication lag and transient git failures get their chance, matching
/// the pull request creation budget.
const PUBLISH_ATTEMPTS: u32 = 3;
const PUBLISH_RETRY_DELAY: Duration = Duration::from_secs(2);
/// Stop retrying a run after this many worker attempts that could not
/// even record a durable outcome (store errors).
const MAX_WORKER_FAILURES_PER_RUN: u32 = 3;

/// Push permission the minted installation token requests.
fn push_permissions() -> serde_json::Value {
    serde_json::json!({ "contents": "write" })
}

/// The publish queue: the run ids waiting for a worker, deduplicated
/// and bounded.
pub(super) struct RunPublishQueue {
    capacity: usize,
    ordered:  BTreeSet<RunId>,
}

impl RunPublishQueue {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            ordered: BTreeSet::new(),
        }
    }

    fn push(&mut self, run_id: RunId) -> bool {
        if self.ordered.len() >= self.capacity || !self.ordered.insert(run_id) {
            return false;
        }
        true
    }

    fn pop(&mut self) -> Option<RunId> {
        self.ordered.pop_first()
    }
}

impl Default for RunPublishQueue {
    fn default() -> Self {
        Self::new(PUBLISH_QUEUE_CAPACITY)
    }
}

impl AppState {
    pub(super) fn enqueue_run_publish(&self, run_id: RunId) -> bool {
        self.run_publish_queue
            .lock()
            .expect("run publish queue lock poisoned")
            .push(run_id)
    }

    fn pop_run_publish(&self) -> Option<RunId> {
        self.run_publish_queue
            .lock()
            .expect("run publish queue lock poisoned")
            .pop()
    }
}

/// The snapshots directory of a run: its scratch root's `petri`
/// subdirectory, where the checkpoint hooks publish every workspace's
/// bare snapshot repository.
fn snapshots_dir(state: &AppState, run_id: &RunId) -> PathBuf {
    Storage::new(state.server_storage_dir())
        .run_scratch(run_id)
        .root()
        .join("petri")
        .join("snapshots")
}

/// The snapshot repository under `dir` that holds `head_sha`, if any:
/// each candidate is probed with `git cat-file -e`.
async fn find_snapshot_repository(dir: &Path, head_sha: &str) -> Option<PathBuf> {
    let mut entries = fs::read_dir(dir).await.ok()?;
    let mut repositories = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().is_some_and(|extension| extension == "git") && path.is_dir() {
            repositories.push(path);
        }
    }
    for repository in repositories {
        let output = TokioCommand::new("git")
            .arg("--git-dir")
            .arg(&repository)
            .args(["cat-file", "-e", &format!("{head_sha}^{{commit}}")])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
        if matches!(output, Ok(status) if status.success()) {
            return Some(repository);
        }
    }
    None
}

/// Push `head_sha` from the snapshot repository to the origin under the
/// run branch, authenticated through the credential helper so the token
/// never appears in argv, the URL, or rendered errors.
async fn push_snapshot_commit(
    repository: &Path,
    origin_url: &str,
    head_sha: &str,
    run_branch: &str,
    token: &str,
) -> Result<(), String> {
    let refspec = format!("{head_sha}:refs/heads/{run_branch}");
    let mut command = TokioCommand::new("git");
    fabro_github::apply_probe_git_env(&mut command, token);
    let output = command
        .arg("--git-dir")
        .arg(repository)
        .args(["push", origin_url, &refspec])
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|err| format!("spawning git push failed: {err}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "git push failed (exit {}): {}",
        output.status.code().unwrap_or(-1),
        stderr.trim()
    ))
}

/// Why a push token could not be minted: the integration is not
/// configured at all (a skip), or minting failed (a fault).
enum TokenMint {
    Unavailable,
    Failed(String),
}

/// The GitHub token a push authenticates with: the server's credentials
/// resolve a bearer token scoped to the repository with contents write.
/// Returns the normalized https origin beside it.
async fn mint_push_token(
    state: &AppState,
    origin_url: &str,
) -> Result<(String, String), TokenMint> {
    let settings = state.server_settings();
    let creds = state
        .github_credentials(&settings.server.integrations.github)
        .await
        .map_err(|err| TokenMint::Failed(err.to_string()))?
        .ok_or(TokenMint::Unavailable)?;
    let https_url = fabro_github::ssh_url_to_https(origin_url);
    let (owner, repo) = fabro_github::parse_github_owner_repo(&https_url)
        .map_err(|err| TokenMint::Failed(format!("{err:#}")))?;
    let client = state
        .http_client()
        .map_err(|err| TokenMint::Failed(err.to_string()))?;
    let token = creds
        .resolve_bearer_token(
            &client,
            &owner,
            &repo,
            state.github_api_base_url.as_str(),
            push_permissions(),
        )
        .await
        .map_err(|err| TokenMint::Failed(format!("minting a push token failed: {err:#}")))?;
    Ok((https_url, token))
}

/// What the publish owes for a run, decided from its projection.
#[derive(Debug)]
enum PublishDecision<'a> {
    /// The run has not reached terminal success yet: nothing is decided
    /// and nothing is recorded — the scan revisits it when it has.
    Wait,
    /// Push the final commit from the snapshot repository.
    Push {
        origin_url: &'a str,
        run_branch: &'a str,
        head_sha:   &'a str,
    },
    /// Nothing to do; `reason` names why, machine-readably.
    Skip { reason: &'static str },
}

fn decide_publish(run_state: &RunProjection) -> PublishDecision<'_> {
    let Some(conclusion) = run_state.conclusion.as_ref() else {
        return PublishDecision::Wait;
    };
    if !conclusion.status.is_successful() {
        // A run that finished any other way owes nothing; one still
        // running is not decided here at all.
        return if run_state.status.is_terminal() {
            PublishDecision::Skip {
                reason: "run_not_successful",
            }
        } else {
            PublishDecision::Wait
        };
    }
    let settings = &run_state.spec.settings.run;
    if !settings.run_branch.push {
        return PublishDecision::Skip {
            reason: "run_branch_push_disabled",
        };
    }
    let Some(origin_url) = run_state.spec.repo_origin_url() else {
        return PublishDecision::Skip {
            reason: "no_repo_origin",
        };
    };
    let Some(run_branch) = run_state
        .start
        .as_ref()
        .and_then(|start| start.run_branch.as_deref())
    else {
        return PublishDecision::Skip {
            reason: "no_run_branch",
        };
    };
    let Some(head_sha) = conclusion
        .final_git_commit_sha
        .as_deref()
        .filter(|sha| !sha.trim().is_empty())
    else {
        return PublishDecision::Skip {
            reason: "no_final_commit",
        };
    };
    let diff_empty = conclusion
        .diff
        .patch
        .as_deref()
        .is_none_or(|patch| patch.trim().is_empty());
    if diff_empty {
        return PublishDecision::Skip {
            reason: "empty_diff",
        };
    }
    PublishDecision::Push {
        origin_url,
        run_branch,
        head_sha,
    }
}

/// Whether the committed projection shows the run finished successfully:
/// the precondition for doing any publish work at all.
fn terminally_green(run_state: &RunProjection) -> bool {
    run_state
        .conclusion
        .as_ref()
        .is_some_and(|conclusion| conclusion.status.is_successful())
}

/// Whether the run already carries a publish outcome record.
async fn publish_settled(state: &AppState, run_id: &RunId) -> anyhow::Result<bool> {
    Ok(!state
        .stores
        .run_summaries
        .platform_records()
        .read_kind(run_id, PlatformRecordKind::RunBranchPublished)
        .await?
        .is_empty())
}

/// Publish one run's branch, or record why that is not owed. The outer
/// `Err` is an infrastructure failure the supervisor may retry; the
/// outcome record makes the publish itself durable either way.
pub(super) async fn process_run_publish(state: &AppState, run_id: RunId) -> anyhow::Result<()> {
    // The cheap precheck reads the committed projection without waiting
    // on the projector: a run that is not terminally green yet — the
    // common case for a live run's signals — is left entirely alone, so
    // the worker never spins on a settle or writes an early outcome.
    match state
        .stores
        .run_summaries
        .load_petri_projection(&run_id)
        .await
    {
        Ok(Some(committed)) if terminally_green(&committed) => {}
        Ok(_) => return Ok(()),
        Err(err) => return Err(err.into()),
    }
    let _publish_guard = state.run_publish_locks.lock(run_id).await;
    if publish_settled(state, &run_id).await? {
        return Ok(());
    }
    let Some(run_state) = run_records::projection(state, run_id).await? else {
        return Ok(());
    };
    // The projection load waited for the projector; a concurrent worker
    // may have settled the run in between.
    if publish_settled(state, &run_id).await? {
        return Ok(());
    }
    let recorded_branch = || {
        run_state
            .start
            .as_ref()
            .and_then(|start| start.run_branch.clone())
            .unwrap_or_default()
    };
    let recorded_sha = || {
        run_state
            .conclusion
            .as_ref()
            .and_then(|conclusion| conclusion.final_git_commit_sha.clone())
            .unwrap_or_default()
    };
    let (run_branch, head_sha, outcome) = match decide_publish(&run_state) {
        PublishDecision::Wait => return Ok(()),
        PublishDecision::Skip { reason } => {
            info!(run_id = %run_id, reason, "Run branch publish skipped");
            (
                recorded_branch(),
                recorded_sha(),
                RunBranchPublishOutcome::Skipped {
                    reason: reason.to_string(),
                },
            )
        }
        PublishDecision::Push {
            origin_url,
            run_branch,
            head_sha,
        } => {
            let outcome =
                push_with_attempts(state, &run_id, origin_url, run_branch, head_sha).await;
            (run_branch.to_string(), head_sha.to_string(), outcome)
        }
    };
    let published = matches!(outcome, RunBranchPublishOutcome::Published);
    run_records::append(
        state,
        run_id,
        PlatformRecord::RunBranchPublished(RunBranchPublishedRecord {
            run_branch,
            head_sha,
            outcome,
        }),
    )
    .await?;
    if published {
        request_pull_request_if_enabled(state, &run_id).await;
    }
    Ok(())
}

/// The bounded push: attempts with a short backoff, the last error in
/// the terminal failure.
async fn push_with_attempts(
    state: &AppState,
    run_id: &RunId,
    origin_url: &str,
    run_branch: &str,
    head_sha: &str,
) -> RunBranchPublishOutcome {
    let Some(repository) = find_snapshot_repository(&snapshots_dir(state, run_id), head_sha).await
    else {
        return RunBranchPublishOutcome::Failed {
            error: format!(
                "no snapshot repository under the run's scratch holds commit {head_sha}"
            ),
        };
    };
    let (https_origin, token) = match mint_push_token(state, origin_url).await {
        Ok(minted) => minted,
        Err(TokenMint::Unavailable) => {
            return RunBranchPublishOutcome::Skipped {
                reason: "github_credentials_unavailable".to_string(),
            };
        }
        Err(TokenMint::Failed(error)) => return RunBranchPublishOutcome::Failed { error },
    };
    for attempt in 1..PUBLISH_ATTEMPTS {
        match push_snapshot_commit(&repository, &https_origin, head_sha, run_branch, &token).await {
            Ok(()) => return published(run_id, run_branch, head_sha),
            Err(error) => {
                warn!(
                    run_id = %run_id,
                    attempt,
                    error = %error,
                    "Run branch push failed; retrying after a short backoff"
                );
                time::sleep(PUBLISH_RETRY_DELAY).await;
            }
        }
    }
    match push_snapshot_commit(&repository, &https_origin, head_sha, run_branch, &token).await {
        Ok(()) => published(run_id, run_branch, head_sha),
        Err(error) => RunBranchPublishOutcome::Failed { error },
    }
}

fn published(run_id: &RunId, run_branch: &str, head_sha: &str) -> RunBranchPublishOutcome {
    info!(
        run_id = %run_id,
        run_branch,
        head_sha,
        "Run branch published: the final commit pushed to the origin"
    );
    RunBranchPublishOutcome::Published
}

/// Request the run's pull request the settings ask for, after a
/// successful publish: the same `pull_request.requested` record the
/// `POST /runs/{id}/pull_request` handler appends, handed to the pull
/// request supervisor.
async fn request_pull_request_if_enabled(state: &AppState, run_id: &RunId) {
    let run_state = match run_records::projection(state, *run_id).await {
        Ok(Some(run_state)) => run_state,
        Ok(None) => return,
        Err(err) => {
            warn!(run_id = %run_id, error = %err, "could not read the run to request its pull request");
            return;
        }
    };
    if !run_state
        .spec
        .settings
        .run
        .pull_request
        .as_ref()
        .is_some_and(|settings| settings.enabled)
    {
        return;
    }
    // The create lock keeps the endpoint and this request from racing on
    // the same append; under it, the projection is the latest word.
    let _create_guard = state.pull_request_create_locks.lock(*run_id).await;
    let Ok(Some(run_state)) = run_records::projection(state, *run_id).await else {
        return;
    };
    if run_state.pull_request.is_some()
        || run_state
            .pull_request_creation
            .as_ref()
            .is_some_and(fabro_types::PullRequestCreation::is_pending)
    {
        return;
    }
    let Some(model) = publish_pr_model(state, &run_state).await else {
        warn!(
            run_id = %run_id,
            "pull request enabled but no model is available for its generation; skipping the automatic request"
        );
        return;
    };
    let creation_id = PullRequestCreationId::new();
    if let Err(err) = run_records::append(
        state,
        *run_id,
        PlatformRecord::PullRequestRequested(PullRequestRequestedRecord {
            creation_id,
            model,
            force: false,
        }),
    )
    .await
    {
        warn!(run_id = %run_id, error = %err, "could not append the pull request request");
        return;
    }
    let Ok(Some(run_state)) = run_records::projection(state, *run_id).await else {
        return;
    };
    if let Some(creation) = run_state
        .pull_request_creation
        .as_ref()
        .filter(|creation| creation.id == creation_id && creation.is_pending())
    {
        info!(run_id = %run_id, "Publish requested the run's pull request");
        state.enqueue_pull_request_creation(*run_id, creation.requested_at);
        state.notify_pull_request_scheduler();
    }
}

/// The model a publish-requested pull request generates with: the
/// configured `[run.pull_request] model`, else the catalog default.
async fn publish_pr_model(state: &AppState, run_state: &RunProjection) -> Option<String> {
    if let Some(model) = run_state
        .spec
        .settings
        .run
        .pull_request
        .as_ref()
        .and_then(|settings| settings.model.clone())
    {
        return Some(model);
    }
    let catalog = state.catalog();
    let configured = state.ready_llm_provider_ids().await;
    catalog
        .default_offering_for(&configured)
        .map(|entry| entry.model.id().to_string())
}

/// The scan: candidates are succeeded repository runs without an
/// outcome record, finished after the cutoff. `queued` mirrors the
/// AppState queue so a signal does not double-enqueue between scans.
async fn recover_publish_candidates(
    state: &AppState,
    cutoff_ms: i64,
    active: &HashMap<task::Id, RunId>,
    failures: &HashMap<RunId, u32>,
    queued: &mut HashSet<RunId>,
) -> anyhow::Result<()> {
    let candidates = state
        .stores
        .run_summaries
        .list_run_publish_candidate_run_ids(cutoff_ms, PUBLISH_SCAN_LIMIT)
        .await?;
    let mut enqueued = 0;
    for run_id in candidates {
        if queued.contains(&run_id) || !can_dispatch(&run_id, active, failures) {
            continue;
        }
        if !state.enqueue_run_publish(run_id) {
            break;
        }
        queued.insert(run_id);
        enqueued += 1;
    }
    if enqueued > 0 {
        tracing::debug!(enqueued, "Recovered run branch publish candidates");
    }
    Ok(())
}

/// Whether the supervisor may hand `run_id` to a worker right now: not
/// already being processed, and not past the store-failure retry cap.
fn can_dispatch(
    run_id: &RunId,
    active: &HashMap<task::Id, RunId>,
    failures: &HashMap<RunId, u32>,
) -> bool {
    !active.values().any(|active_id| active_id == run_id)
        && failures.get(run_id).copied().unwrap_or(0) < MAX_WORKER_FAILURES_PER_RUN
}

/// Whether a projector signal is worth a publish worker: the run's
/// committed projection already shows terminal success. Live runs' steady
/// stream of signals is dropped here.
async fn signal_is_publishable(state: &AppState, run_id: &RunId) -> bool {
    matches!(
        state.stores.run_summaries.load_petri_projection(run_id).await,
        Ok(Some(ref committed)) if terminally_green(committed)
    )
}

/// The boot cutoff: finished runs older than the lookback stay
/// unpublished on purpose.
fn boot_cutoff_ms(started_at: chrono::DateTime<chrono::Utc>) -> i64 {
    (started_at - chrono::Duration::from_std(PUBLISH_BACKLOG_LOOKBACK).unwrap_or_default())
        .timestamp_millis()
}

async fn run_publish_supervisor(state: Arc<AppState>) {
    let shutdown = state.shutdown_token();
    let mut workers = JoinSet::new();
    let mut active: HashMap<task::Id, RunId> = HashMap::new();
    let mut failures: HashMap<RunId, u32> = HashMap::new();
    let mut queued: HashSet<RunId> = HashSet::new();
    let mut scan_requested = true;
    let cutoff_ms = boot_cutoff_ms(chrono::Utc::now());
    let mut scan_interval = time::interval(PUBLISH_SCAN_INTERVAL);
    scan_interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);

    // The projector signals after every committed pass; a run that
    // finishes while the server lives is published without waiting for
    // the scan.
    let mut signals = state.petri_projector.subscribe();
    // A dropped projector sender makes recv() Ready(Err(Closed)) on EVERY
    // poll — an unguarded select arm then spins the loop without a single
    // await (fabro-629b: 100% single-core, zero syscalls, runtime
    // starvation). The guard parks the arm once the channel is gone; the
    // scheduler notify and the scan interval keep the loop alive.
    let mut signals_open = true;

    loop {
        if scan_requested {
            if let Err(error) =
                recover_publish_candidates(&state, cutoff_ms, &active, &failures, &mut queued).await
            {
                warn!(%error, "Failed to scan for run branch publishes");
            }
            scan_requested = false;
        }

        while active.len() < MAX_CONCURRENT_PUBLISHES {
            let Some(run_id) = state.pop_run_publish() else {
                break;
            };
            queued.remove(&run_id);
            if !can_dispatch(&run_id, &active, &failures) {
                continue;
            }
            let state = Arc::clone(&state);
            let handle = workers.spawn(
                async move { process_run_publish(&state, run_id).await }
                    .instrument(info_span!("run_publish", run_id = %run_id)),
            );
            active.insert(handle.id(), run_id);
        }

        if shutdown.is_cancelled() {
            break;
        }

        tokio::select! {
            () = shutdown.cancelled() => break,
            () = state.run_publish_scheduler_notified() => {},
            signaled = signals.recv(), if signals_open => {
                match signaled {
                    Ok(run_id) => {
                        if signal_is_publishable(&state, &run_id).await
                            && !queued.contains(&run_id)
                            && state.enqueue_run_publish(run_id)
                        {
                            queued.insert(run_id);
                        }
                    }
                    Err(RecvError::Lagged(_)) => {
                        scan_requested = true;
                    }
                    Err(RecvError::Closed) => signals_open = false,
                }
            },
            _ = scan_interval.tick() => scan_requested = true,
            // join_next_with_id is Ready(None) FOREVER once the JoinSet is
            // empty — an unguarded arm spins the whole loop without a
            // single await (fabro-629b, live-proven by instrumented rig:
            // 584M iterations, active=0). Park it while no worker runs.
            joined = workers.join_next_with_id(), if !active.is_empty() => {
                match joined {
                    Some(Ok((task_id, result))) => {
                        let run_id = active.remove(&task_id);
                        match (run_id, result) {
                            (Some(run_id), Ok(())) => {
                                failures.remove(&run_id);
                            }
                            (Some(run_id), Err(err)) => {
                                *failures.entry(run_id).or_default() += 1;
                                warn!(run_id = %run_id, error = %err, "Run branch publish worker failed");
                            }
                            (None, result) => {
                                warn!(?result, "Run branch publish worker finished without a tracked run id");
                            }
                        }
                    }
                    Some(Err(err)) => {
                        if let Some(run_id) = active.remove(&err.id()) {
                            *failures.entry(run_id).or_default() += 1;
                            warn!(run_id = %run_id, error = %err, "Run branch publish worker stopped unexpectedly");
                        } else {
                            warn!(error = %err, "Run branch publish worker stopped unexpectedly");
                        }
                    }
                    None => {},
                }
            }
        }
    }

    while let Some(joined) = workers.join_next().await {
        if let Err(err) = joined {
            warn!(error = %err, "Run branch publish worker stopped during shutdown");
        }
    }
}

pub(crate) fn spawn_run_publish_supervisor(state: Arc<AppState>) -> JoinHandle<()> {
    tokio::spawn(run_publish_supervisor(state).instrument(info_span!("run_publish_supervisor")))
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use fabro_types::test_support as types_support;

    use super::*;

    #[test]
    fn boot_cutoff_subtracts_the_lookback() {
        let now = chrono::Utc::now();
        let cutoff = boot_cutoff_ms(now);
        let expected = (now - chrono::Duration::from_std(PUBLISH_BACKLOG_LOOKBACK).unwrap())
            .timestamp_millis();
        assert_eq!(cutoff, expected);
    }

    #[test]
    fn publish_queue_deduplicates_and_bounds() {
        let first = RunId::new();
        let second = RunId::new();
        let (first, second) = if first < second {
            (first, second)
        } else {
            (second, first)
        };
        let overflow = RunId::new();
        let mut queue = RunPublishQueue::new(2);

        assert!(queue.push(first));
        assert!(!queue.push(first));
        assert!(queue.push(second));
        assert!(!queue.push(overflow));
        assert_eq!(queue.pop(), Some(first));
        assert_eq!(queue.pop(), Some(second));
        assert_eq!(queue.pop(), None);

        assert!(queue.push(overflow));
        assert_eq!(queue.pop(), Some(overflow));
    }

    /// `git` in a directory, its stdout. The fixture builder is
    /// synchronous on purpose: it arranges repositories before the
    /// async code under test runs.
    #[expect(
        clippy::disallowed_methods,
        reason = "test fixture arrangement; the code under test uses tokio"
    )]
    fn git(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@example.test")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@example.test")
            .output()
            .expect("git runs");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    /// A bare snapshot-style repository holding one commit with a file,
    /// beside an empty sibling the discovery must skip. Synchronous
    /// fixture arrangement, like [`git`].
    #[expect(
        clippy::disallowed_methods,
        reason = "test fixture arrangement; the code under test uses tokio"
    )]
    fn snapshot_fixture(dir: &Path) -> std::path::PathBuf {
        let snapshots = dir.join("snapshots");
        std::fs::create_dir_all(&snapshots).unwrap();
        git(&snapshots, &["init", "--bare", "-q", "holding.git"]);
        git(&snapshots, &["init", "--bare", "-q", "empty.git"]);
        let work = dir.join("work");
        std::fs::create_dir_all(&work).unwrap();
        git(&work, &["init", "-q", "-b", "main"]);
        std::fs::write(work.join("answer.txt"), "42").expect("fixture writes");
        git(&work, &["add", "."]);
        git(&work, &["commit", "-q", "-m", "the final commit"]);
        let holding = snapshots.join("holding.git");
        git(&work, &[
            "push",
            "-q",
            holding.to_string_lossy().as_ref(),
            "main",
        ]);
        holding
    }

    #[tokio::test]
    async fn find_snapshot_repository_finds_the_one_holding_the_commit() {
        let dir = tempfile::tempdir().unwrap();
        let holding = snapshot_fixture(dir.path());
        let work = dir.path().join("work");
        let head = git(&work, &["rev-parse", "HEAD"]);

        let found = find_snapshot_repository(&dir.path().join("snapshots"), &head).await;
        assert_eq!(found.as_deref(), Some(holding.as_path()));
        assert!(
            find_snapshot_repository(&dir.path().join("snapshots"), "deadbeef")
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn push_snapshot_commit_pushes_the_final_commit_as_the_run_branch() {
        let dir = tempfile::tempdir().unwrap();
        let holding = snapshot_fixture(dir.path());
        let work = dir.path().join("work");
        let head = git(&work, &["rev-parse", "HEAD"]);
        let origin = dir.path().join("origin.git");
        git(dir.path(), &[
            "init",
            "--bare",
            "-q",
            &origin.to_string_lossy(),
        ]);

        push_snapshot_commit(
            &holding,
            &origin.to_string_lossy(),
            &head,
            "fabro/run/01TEST",
            "a-test-token",
        )
        .await
        .expect("the snapshot push succeeds");

        let pushed = git(&origin, &["rev-parse", "refs/heads/fabro/run/01TEST"]);
        assert_eq!(pushed, head);

        // The push is idempotent: a retry of the same refspec is
        // everything-up-to-date, not a failure.
        push_snapshot_commit(
            &holding,
            &origin.to_string_lossy(),
            &head,
            "fabro/run/01TEST",
            "a-test-token",
        )
        .await
        .expect("the retry succeeds");
    }

    fn projection_for_decide(push: bool) -> RunProjection {
        let mut spec = types_support::test_run_spec();
        spec.settings.run.run_branch.push = push;
        spec.git = Some(fabro_types::GitContext {
            origin_url: "https://github.com/acme/widgets.git".to_string(),
            branch:     "main".to_string(),
            sha:        None,
            dirty:      fabro_types::DirtyStatus::Clean,
        });
        let mut projection = RunProjection::new("decide".to_string(), spec, chrono::Utc::now());
        projection.status = fabro_types::RunStatus::Succeeded {
            reason: fabro_types::SuccessReason::Completed,
        };
        projection.start = Some(fabro_types::StartRecord {
            start_time: chrono::Utc::now(),
            run_branch: Some("fabro/run/01TEST".to_string()),
            base_sha:   Some("base".to_string()),
        });
        projection.conclusion = Some(fabro_types::Conclusion {
            timestamp:            chrono::Utc::now(),
            status:               fabro_types::StageOutcome::Succeeded,
            timing:               fabro_types::RunTiming::wall_only(1_000),
            failure:              None,
            final_git_commit_sha: Some("abc123".to_string()),
            stages:               Vec::new(),
            usage:                None,
            total_retries:        0,
            diff:                 fabro_types::RunDiff {
                patch:   Some("diff --git a/x b/x".to_string()),
                summary: None,
            },
        });
        projection
    }

    #[test]
    fn decide_publish_pushes_a_green_repository_run() {
        let projection = projection_for_decide(true);
        match decide_publish(&projection) {
            PublishDecision::Push {
                origin_url,
                run_branch,
                head_sha,
            } => {
                assert_eq!(origin_url, "https://github.com/acme/widgets.git");
                assert_eq!(run_branch, "fabro/run/01TEST");
                assert_eq!(head_sha, "abc123");
            }
            other => panic!("expected a push, got {other:?}"),
        }
    }

    #[test]
    fn decide_publish_skips_when_push_is_disabled() {
        let projection = projection_for_decide(false);
        match decide_publish(&projection) {
            PublishDecision::Skip { reason } => assert_eq!(reason, "run_branch_push_disabled"),
            other => panic!("expected a skip, got {other:?}"),
        }
    }

    #[test]
    fn decide_publish_waits_on_a_run_without_a_conclusion() {
        let mut projection = projection_for_decide(true);
        projection.conclusion = None;
        match decide_publish(&projection) {
            PublishDecision::Wait => {}
            other => panic!("expected a wait, got {other:?}"),
        }
    }

    #[test]
    fn decide_publish_skips_an_empty_diff() {
        let mut projection = projection_for_decide(true);
        projection.conclusion.as_mut().unwrap().diff.patch = None;
        match decide_publish(&projection) {
            PublishDecision::Skip { reason } => assert_eq!(reason, "empty_diff"),
            other => panic!("expected a skip, got {other:?}"),
        }
    }
}
