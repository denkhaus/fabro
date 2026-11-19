//! The terminal cleanup hook (work#1478).
//!
//! When a run turns terminal and its sandbox is still reachable, the
//! server executes the run's workflow-declared cleanup command in the
//! sandbox — through the exec facet, the same `bash -c` shape the engine
//! runs hook `script=` lines with — BEFORE the container is released. A
//! workflow declares its command as `[workflow] terminal_cleanup` in its
//! workflow.toml; the declaration rides the dense settings bundle on the
//! run spec, so the cleanup semantics are pinned with the workflow
//! version that declared them. Workflows without a declaration get no
//! cleanup: a silent no-op.
//!
//! Deleting files the run created reclaims their writable-layer disk at
//! once (overlayfs upper-layer semantics), so a finished run's build
//! artifacts stop costing the retention window. The reap itself is
//! untouched: cleanup runs at the terminal transition, not in
//! `spawn_run_retention_supervisor`'s reap path.
//!
//! Failure semantics: every cleanup error is a `tracing::warn` — never a
//! terminal-status change (the run is already terminal; a failed cleanup
//! must not corrupt the record) and never a retry debt. A missing
//! declaration, a missing script, or an unreachable sandbox stays quiet
//! or warns once and is done for the run's lifetime in this process.
//!
//! The supervisor follows [`super::run_publish`]'s shape: projector
//! signals wake it, a committed-projection precheck drops live runs'
//! steady signal stream, and one bounded worker executes per terminal
//! run. A signal storm that outruns the broadcast channel (`Lagged`) or
//! a server restart can skip a run's cleanup — the 24h retention reap
//! remains the backstop, so a skipped run only keeps today's footprint,
//! never a new leak.

use std::collections::HashSet;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use fabro_store::RunProjection;
use fabro_types::{RunId, RunSandbox};
use sandbox_driver::{Exec, ExecSpec};
use tokio::sync::Semaphore;
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;
use tracing::{Instrument as _, debug, info, info_span, warn};

use super::{AppState, run_records};
use crate::sandbox_access;

/// How long the declared command may run before the provider stops it:
/// generous for a big `target/` tree, bounded so a wedged command cannot
/// hold the run's sandbox gate forever.
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(300);

/// Cleanups in flight at once. Each is one sandbox exec against the run's
/// daemon, so the bound guards the daemon, not CPU.
const MAX_CONCURRENT_CLEANUPS: usize = 4;

/// Characters of the script's stdout report (or a failure's last stderr
/// line) a log line carries.
const SUMMARY_MAX_CHARS: usize = 200;

/// Runs this server process has already handled: the once-only mark that
/// keeps a terminal run's later signals from re-running its cleanup. The
/// set never shrinks; entries are one `RunId` each and die with the
/// process.
type HandledRuns = StdMutex<HashSet<RunId>>;

/// Whether `run_id` is newly this process's to handle: the once-only
/// gate. A run already handled — cleaned, or proven to owe nothing — is
/// never visited again.
fn mark_handled(handled: &HandledRuns, run_id: RunId) -> bool {
    let mut handled = handled
        .lock()
        .expect("terminal cleanup handled-set lock poisoned");
    handled.insert(run_id)
}

/// The declared cleanup command of `run_state`'s workflow; `None` when
/// the workflow declares none (the permanent, silent no-op).
fn declared_terminal_cleanup(run_state: &RunProjection) -> Option<String> {
    let command = run_state.spec.settings.workflow.terminal_cleanup.as_ref()?;
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

/// The first stdout line, trimmed and bounded: the cleanup script's
/// one-line report (bytes freed where `df` measured it).
fn stdout_summary(stdout: &[u8]) -> String {
    let text = String::from_utf8_lossy(stdout);
    let line: String = text.lines().next().unwrap_or_default().trim().to_string();
    line.chars().take(SUMMARY_MAX_CHARS).collect()
}

/// The last non-empty stderr line, trimmed and bounded: the reason a
/// failed cleanup command named for the log.
fn stderr_reason(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let reason: String = text
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .to_string();
    reason.chars().take(SUMMARY_MAX_CHARS).collect()
}

/// The terminal cleanup for one run: resolve the declaration and the
/// run's sandbox record, reconnect the sandbox, execute the command
/// through the exec facet, and stop the sandbox again. Every failure is
/// a warning; nothing here touches the run's records.
async fn cleanup_run_sandbox(state: &AppState, run_id: RunId) {
    let run_state = match run_records::projection(state, run_id).await {
        Ok(Some(run_state)) => run_state,
        Ok(None) => return,
        Err(error) => {
            warn!(
                run_id = %run_id,
                error = %error,
                "could not read the run for its terminal cleanup; skipped"
            );
            return;
        }
    };
    if !run_state.is_terminal() {
        // The precheck acted on a committed projection; this full read is
        // the worker's own view. A run that is not terminal here is left
        // for its next signal.
        return;
    }
    let Some(command) = declared_terminal_cleanup(&run_state) else {
        return;
    };
    let Some(record) = run_state
        .sandbox
        .clone()
        .and_then(RunSandbox::into_instance)
    else {
        debug!(
            run_id = %run_id,
            "terminal cleanup skipped: the run has no sandbox record"
        );
        return;
    };
    let working_directory = record.runtime.working_directory.clone();
    let access = match state.provider_access().await {
        Ok(access) => access,
        Err(error) => {
            warn!(
                run_id = %run_id,
                error = ?error,
                "terminal cleanup skipped: provider access unavailable"
            );
            return;
        }
    };
    // The run's serialization gate: the cleanup window is an access-time
    // sandbox window, serialized against every inspection guard's
    // deferred stop on the same run (fabro-2c17).
    let gate = sandbox_access::run_sandbox_gate(&run_id);
    let _held = gate.clone().lock_owned().await;
    let sandbox = match sandbox_access::attach_running_run_sandbox(&access, &record, run_id).await {
        Ok(sandbox) => sandbox,
        Err(error) => {
            warn!(
                run_id = %run_id,
                error = %error,
                "terminal cleanup skipped: the run's sandbox is not reachable"
            );
            return;
        }
    };
    // A run that just turned terminal: the reactivated sandbox is stopped
    // again when the inspection ends — on `finish` or, for every early
    // exit, from `Drop` (fabro-afab).
    let inspection = sandbox_access::InspectionSandbox::terminal(Arc::clone(&sandbox), Some(gate));
    let spec = ExecSpec::bash(command)
        .timeout(CLEANUP_TIMEOUT)
        .working_dir(working_directory);
    match sandbox.exec().run(&spec).await {
        Ok(result) if result.success() => {
            let summary = stdout_summary(&result.stdout);
            if summary.is_empty() {
                info!(run_id = %run_id, "terminal cleanup executed");
            } else {
                info!(run_id = %run_id, summary = %summary, "terminal cleanup executed");
            }
        }
        Ok(result) => {
            warn!(
                run_id = %run_id,
                termination = ?result.termination,
                exit_code = ?result.exit_code,
                reason = %stderr_reason(&result.stderr),
                "terminal cleanup command failed; the run's terminal status is unchanged"
            );
        }
        Err(error) => {
            warn!(
                run_id = %run_id,
                error = %error,
                "terminal cleanup exec failed; the run's terminal status is unchanged"
            );
        }
    }
    inspection.finish().await;
}

/// One supervisor turn: clean one terminal run, bounded by the shared
/// permit pool so a burst of terminal runs cannot stampede the daemon.
async fn supervised_cleanup(state: Arc<AppState>, permits: Arc<Semaphore>, run_id: RunId) {
    let Ok(_permit) = permits.acquire_owned().await else {
        return;
    };
    cleanup_run_sandbox(&state, run_id).await;
}

async fn run_terminal_cleanup_supervisor(state: Arc<AppState>) {
    let shutdown = state.shutdown_token();
    let mut signals = state.petri_projector.subscribe();
    // A dropped projector sender makes recv() Ready(Err(Closed)) on EVERY
    // poll — an unguarded select arm then spins the loop without a single
    // await (fabro-629b, as run_publish parks its arm).
    let mut signals_open = true;
    let handled: HandledRuns = StdMutex::new(HashSet::new());
    let permits = Arc::new(Semaphore::new(MAX_CONCURRENT_CLEANUPS));
    loop {
        let run_id = tokio::select! {
            () = shutdown.cancelled() => break,
            signaled = signals.recv(), if signals_open => match signaled {
                Ok(run_id) => run_id,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => {
                    signals_open = false;
                    continue;
                }
            },
        };
        // The cheap committed-projection precheck: only terminal runs are
        // candidates. Live runs' steady signal stream is dropped here,
        // and a store error just waits for the run's next signal.
        let terminal = match state
            .stores
            .run_summaries
            .load_petri_projection(&run_id)
            .await
        {
            Ok(Some(committed)) => committed.is_terminal(),
            Ok(None) | Err(_) => false,
        };
        if !terminal || !mark_handled(&handled, run_id) {
            continue;
        }
        let state = Arc::clone(&state);
        let permits = Arc::clone(&permits);
        tokio::spawn(
            async move { supervised_cleanup(state, permits, run_id).await }
                .instrument(info_span!("run_terminal_cleanup", run_id = %run_id)),
        );
    }
}

/// Spawns the terminal cleanup supervisor: the projector-signal watcher
/// that executes workflow-declared cleanup commands in terminal runs'
/// still-reachable sandboxes (work#1478).
pub(crate) fn spawn_run_terminal_cleanup_supervisor(state: Arc<AppState>) -> JoinHandle<()> {
    tokio::spawn(
        run_terminal_cleanup_supervisor(state)
            .instrument(info_span!("run_terminal_cleanup_supervisor")),
    )
}
