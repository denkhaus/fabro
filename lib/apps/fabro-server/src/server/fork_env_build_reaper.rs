//! Fork-only env-build holder reaper (work#1480, 2026-11-19).
//!
//! The environment image build's holder sandbox (image_build.rs) is
//! machinery with an in-process cleanup contract: "the holder never
//! outlives the attempt". That contract is NOT crash-safe — when the
//! executor process itself dies (OOM kill, SIGKILL), the stop/delete
//! dies with it and the holder container keeps running on the daemon
//! for hours (the 2026-11-19 intervention found cooks from the previous
//! night's failed runs still racing the next wave). This reaper is the
//! crash-safe half: every [`ENV_BUILD_REAPER_INTERVAL`] the server
//! lists the daemon for containers carrying the `petri.role=env-build`
//! label (already distinct from run workspaces — proven by work#1425's
//! relief recipe), attributes each holder to its owning run THROUGH
//! THE CONTAINER NAME (`petri-<run id>-l<lease>-env-build`, the only
//! run link the holder carries), and stops+deletes exactly the holders
//! whose owning run is terminal or absent. The concurrent-race half
//! of the incident is PR #608's cross-process dedup; this arm only
//! shrinks the zombie window.
//!
//! Safety posture: FAIL-CLOSED for protection. A holder whose name
//! cannot be attributed to a run id, or whose run lookup errors, is
//! NEVER touched — only a holder whose owning run is PROVEN terminal
//! (`Succeeded`/`Failed`/`Dead`) or PROVEN absent (no run row at all,
//! e.g. swept by the failed-run cleanup) is reaped. Failure posture
//! for the pass itself is FAIL-OPEN like the retention supervisor: a
//! failing pass logs at WARN and retries on the next tick; the reaper
//! is hygiene, never a reason to disturb the server.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use chrono::Utc;
use fabro_types::{RunId, RunStatus, SandboxProviderKind};
use sandbox_driver::{
    Error as DriverError, SandboxFilter, SandboxId, SandboxProvider, SandboxState, SandboxStatus,
};
use tokio::task::JoinHandle;
use tokio::time;
use tracing::{Instrument as _, debug, info, info_span, warn};

use crate::sandbox_access::{ProviderAccess, connect_provider};

/// The label Petri's image build stamps on every holder container,
/// marking it build machinery rather than a run workspace.
pub(crate) const ENV_BUILD_ROLE_LABEL: &str = "petri.role";
pub(crate) const ENV_BUILD_ROLE_VALUE: &str = "env-build";

/// The holder name's shape: `petri-<run id>-l<lease>-env-build` —
/// `RunIdentity::container_name` (prefix, lease segment) plus the
/// build's `-env-build` suffix (executor-sandbox/src/lib.rs).
const NAME_PREFIX: &str = "petri-";
const LEASE_SEPARATOR: &str = "-l";
const NAME_SUFFIX: &str = "-env-build";

/// Reaper cadence: same class as the retention supervisor's removal
/// reaper — a zombie holder burns a CPU slice on the host for every
/// minute it survives, so the tick must be minutes, not hours. A
/// seeded orphan is reaped within one tick.
const ENV_BUILD_REAPER_INTERVAL: Duration = Duration::from_mins(5);

/// What one reaper pass found and did. Per-holder failures are counted,
/// never propagated: the pass is fail-open.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct EnvBuildReapStats {
    /// Holders the daemon listing returned.
    pub scanned:       usize,
    /// Holders stopped and deleted this pass.
    pub reaped:        usize,
    /// Holders of runs that are alive (non-terminal) — never touched.
    pub live_skipped:  usize,
    /// Holders whose name carries no attributable run id — never
    /// touched (the run workspaces share the prefix but not the
    /// suffix; anything unattributed stays).
    pub unattributed:  usize,
    /// Holders whose run lookup errored — never touched (fail-closed).
    pub lookup_failed: usize,
    /// Holders whose stop/delete errored — retried on the next tick.
    pub reap_failed:   usize,
}

/// The owning run a holder container name carries, or `None` when the
/// name is not a holder's (`petri-<run id>-l<lease>-env-build`). Pure
/// over the name; `None` always means "leave it alone".
///
/// A run id this build did not mint by store key (the no-coordinator
/// `for_run_dir` hex ids) does not parse as a [`RunId`] — such a holder
/// is unattributed and never touched; the attribution errs toward
/// never-reap.
pub(crate) fn holder_owner_run(name: &str) -> Option<RunId> {
    let core = name.strip_prefix(NAME_PREFIX)?.strip_suffix(NAME_SUFFIX)?;
    let (run, _lease) = core.rsplit_once(LEASE_SEPARATOR)?;
    run.parse().ok()
}

/// Whether a holder of a run in this lifecycle state may be reaped.
/// `None` (no run row — the run was swept or never recorded here) and
/// the unambiguous terminal classes are reapable; everything else —
/// active, parked, blocked, human-needed, mid-removal — is protected.
/// The same terminal set the sandbox-reap arm's candidate filter uses.
pub(crate) fn holder_reapable(status: Option<RunStatus>) -> bool {
    match status {
        None | Some(RunStatus::Succeeded { .. } | RunStatus::Failed { .. } | RunStatus::Dead) => {
            true
        }
        Some(_) => false,
    }
}

/// The reaper's decision core, pure over the daemon listing: attribute
/// every holder status, resolve its owning run's lifecycle, and reap
/// (through the caller's executor) exactly the terminal-or-absent ones.
/// Generic over the two async seams so the fork-only pins can drive it
/// with synthetic statuses and a scripted provider — no daemon needed.
pub(crate) async fn reap_env_build_holders<L, LFut, R, RFut>(
    statuses: &[SandboxStatus],
    run_status: L,
    reap: R,
) -> EnvBuildReapStats
where
    L: Fn(RunId) -> LFut,
    LFut: Future<Output = anyhow::Result<Option<RunStatus>>>,
    R: Fn(SandboxId) -> RFut,
    RFut: Future<Output = anyhow::Result<()>>,
{
    let mut stats = EnvBuildReapStats {
        scanned: statuses.len(),
        ..EnvBuildReapStats::default()
    };
    for status in statuses {
        if status.state == SandboxState::Deleted {
            // Already gone; the daemon lists it at most briefly.
            continue;
        }
        let name = status.name.as_deref().unwrap_or_default();
        let Some(run_id) = holder_owner_run(name) else {
            stats.unattributed += 1;
            debug!(
                name,
                "env-build reaper: holder carries no run id — leaving it"
            );
            continue;
        };
        match run_status(run_id).await {
            Err(error) => {
                stats.lookup_failed += 1;
                warn!(
                    %run_id,
                    ?error,
                    "env-build reaper: run lookup failed — leaving the holder"
                );
            }
            Ok(Some(lifecycle)) if !holder_reapable(Some(lifecycle)) => {
                stats.live_skipped += 1;
                debug!(%run_id, ?lifecycle, "env-build reaper: run is live — leaving the holder");
            }
            Ok(_) => match reap(status.id.clone()).await {
                Ok(()) => {
                    stats.reaped += 1;
                    info!(%run_id, holder = %status.id, "env-build reaper: reaped an orphaned holder");
                }
                Err(error) => {
                    stats.reap_failed += 1;
                    warn!(%run_id, holder = %status.id, ?error, "env-build reaper: reaping failed");
                }
            },
        }
    }
    stats
}

/// Stops and deletes one holder through the provider. Attaching a
/// holder that is already gone is the expected idempotency case —
/// `Ok(())`, not an error. A `stop` failure still tries the delete:
/// the holder is machinery, and a stopped-but-undeleted container is
/// the outcome we are reaping our way out of.
pub(super) async fn reap_provider_holder(
    provider: &dyn SandboxProvider,
    id: &SandboxId,
) -> anyhow::Result<()> {
    let holder = match provider.attach(id, None).await {
        Ok(holder) => holder,
        Err(DriverError::NotFound { .. }) => return Ok(()),
        Err(error) => {
            return Err(anyhow::Error::new(error)
                .context(format!("attaching the env-build holder '{id}' failed")));
        }
    };
    if let Err(problem) = holder.stop().await {
        warn!(holder = %id, %problem, "env-build reaper: stopping the holder failed");
    }
    holder.delete().await.map_err(|error| {
        anyhow::Error::new(error).context(format!("deleting the env-build holder '{id}' failed"))
    })
}

/// One production pass: reach the daemon the run containers live on,
/// list it for env-build holders, resolve owning runs from the run
/// summaries, reap the orphaned ones.
async fn env_build_reap_pass(state: &super::AppState) -> anyhow::Result<EnvBuildReapStats> {
    let access: ProviderAccess = state
        .provider_access()
        .await
        .map_err(|error| anyhow::anyhow!("loading provider access for the reaper: {error:?}"))?;
    let provider = connect_provider(&SandboxProviderKind::DOCKER, &access)
        .await
        .map_err(|error| {
            anyhow::anyhow!("connecting the docker provider for the reaper: {error}")
        })?;

    let mut filter = SandboxFilter::default();
    filter.labels.insert(
        ENV_BUILD_ROLE_LABEL.to_owned(),
        ENV_BUILD_ROLE_VALUE.to_owned(),
    );
    let statuses = provider
        .list(&filter)
        .await
        .context("listing env-build holders on the docker provider failed")?;

    let now = Utc::now();
    let summaries = Arc::clone(&state.stores.run_summaries);
    let stats = reap_env_build_holders(
        &statuses,
        |run_id| {
            let summaries = Arc::clone(&summaries);
            async move {
                let run = summaries.get(&run_id, now).await.map_err(|error| {
                    anyhow::anyhow!("reading the run summary failed: {error:?}")
                })?;
                Ok(run.map(|run| run.lifecycle.status))
            }
        },
        |id| {
            let provider = Arc::clone(&provider);
            async move { reap_provider_holder(provider.as_ref(), &id).await }
        },
    )
    .await;

    if stats.reaped > 0 || stats.reap_failed > 0 {
        info!(
            reaped = stats.reaped,
            live_skipped = stats.live_skipped,
            unattributed = stats.unattributed,
            lookup_failed = stats.lookup_failed,
            reap_failed = stats.reap_failed,
            "env-build reaper pass complete"
        );
    } else {
        debug!(
            live_skipped = stats.live_skipped,
            "env-build reaper pass complete"
        );
    }
    Ok(stats)
}

/// Fail-open tick wrapper, the retention supervisor's posture.
async fn env_build_reap_tick(state: &super::AppState) {
    if let Err(error) = env_build_reap_pass(state).await {
        warn!(?error, "env-build reaper pass failed");
    }
}

/// Fire-and-forget like the sibling supervisors in `serve.rs`: the loop
/// owns its shutdown through `AppState::shutdown_token()`. The first
/// tick fires immediately (an interval's first tick is due at once) —
/// a server restart after a crash night is exactly when the zombie
/// backlog must be cleared, and live runs' holders are protected by
/// the non-terminal rule, not by a startup delay.
pub(crate) fn spawn_env_build_reaper(state: Arc<super::AppState>) -> JoinHandle<()> {
    tokio::spawn(
        async move {
            let shutdown = state.shutdown_token();
            info!(
                interval_secs = ENV_BUILD_REAPER_INTERVAL.as_secs(),
                "env-build reaper started"
            );
            let mut tick = time::interval(ENV_BUILD_REAPER_INTERVAL);
            loop {
                tokio::select! {
                    () = shutdown.cancelled() => break,
                    _ = tick.tick() => {
                        env_build_reap_tick(&state).await;
                    }
                }
            }
        }
        .instrument(info_span!("env_build_reaper")),
    )
}
