//! A Fabro run executed by Petri: the one assembly the run's worker process
//! and the server share.
//!
//! The worker is where a Petri run executes, as a legacy run does: it
//! reaches the run's record through [`HttpRunStore`](crate::HttpRunStore)
//! with its token, and everything else here is the same as in the server.
//! The server itself executes a run only under its test override, over
//! [`SqliteRunStore`](crate::SqliteRunStore) in its own process. Both build
//! the runtime the same way the create handler built it for
//! `Runtime::check`, name the Fabro run id as the run key, and hand the run
//! to `execution::host`: [`Execution::Start`] runs the admitted graphs
//! through `run_configured`; [`Execution::Resume`] continues the run from
//! its records through `resume_configured`, with the same observers a start
//! installs, as the host's docs require. The outcome is then derived from
//! `inspect_run` over a read handle of the same store, so what the caller
//! reports is what the durable record says.
//!
//! What the standalone runner's defaults give the run: Petri's local hook
//! service for `[[run.hooks]]`, no `ExecutionHooks` of Fabro's own, the
//! [`Unattended`] interviewer that fails any question, no host tools, and
//! `Retention::Always` for every workspace, Fabro's default. Cancellation
//! rides the caller's token: when it fires, the root invocation is cancelled
//! politely and Petri records why.
//!
//! A resume here is Petri's own: the run continues from its records, and
//! sandbox leases are reconciled by label. Full recovery, where the
//! workspace a resumed stage sees is restored to the snapshot its durable
//! state names, is the integration plan's F3.5 and lands after this.
//!
//! No stage or agent event is projected into Fabro's tables here; the
//! caller appends only the run lifecycle events Fabro's read side needs to
//! finish the run, from the [`Conclusion`] this module derives. The
//! projection over Petri's records is the read-side item that follows.

use std::path::PathBuf;
use std::sync::Arc;

use fabro_types::{FailureReason, SandboxProviderKind};
use petri_execution::host::{self, HostError, HostRun};
use petri_execution::inspect::{self, InspectError, RunInspection};
use petri_execution::{
    Access, CancelReason, InterviewDispatcher, InvocationId, RECEIPT_FILE, RunKey, RunStore,
};
use petri_runtime::executor::Retention;
use petri_runtime::{RunOptions, SandboxBackend};
use tokio::fs;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::admission::AdmittedGraphs;
use crate::interviewer::Unattended;
use crate::runtime::RuntimeSpec;

/// How the run is entered: fresh, from the admitted graphs, or continued
/// from its records.
pub enum Execution {
    /// Run the admitted graphs from the start; the run must not exist in
    /// the store yet.
    Start(AdmittedGraphs),
    /// Continue the run from its records; the run must exist in the store
    /// with its root invocation declared.
    Resume,
}

/// One run to execute.
pub struct RunRequest {
    /// The Fabro run id, which becomes Petri's run key: the run's identity
    /// in the store and the label on every sandbox of the run.
    pub run_id:    String,
    /// Where the run's workspaces, step output and blobs live.
    pub run_dir:   PathBuf,
    pub execution: Execution,
    /// The run's durable record: the worker's HTTP store, or the server's
    /// SQLite store under the test override.
    pub store:     Arc<dyn RunStore>,
    pub runtime:   RuntimeSpec,
    /// The sandbox provider Fabro resolved for the run's environment.
    pub provider:  SandboxProviderKind,
    /// Fires to cancel the run.
    pub cancel:    CancellationToken,
}

/// The recorded status of a finished run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunStatus {
    Success,
    Failed,
    Cancelled,
}

/// What the durable record says about the run once it ended.
#[derive(Clone, Debug)]
pub struct RunOutcome {
    pub status:     RunStatus,
    /// The root invocation's failure message, when it failed.
    pub failure:    Option<String>,
    /// Whether the record is whole: the run recorded its finish and every
    /// log replays byte for byte.
    pub complete:   bool,
    /// Every reason `complete` is false.
    pub incomplete: Vec<String>,
}

/// Why the run could not be executed or its outcome read.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("the run's sandbox provider `{provider}` is not one Petri serves")]
    UnsupportedProvider { provider: SandboxProviderKind },
    #[error("the run's record could not be opened")]
    Open(#[source] petri_store::StoreError),
    #[error("the run's record could not be read")]
    Read(#[source] HostError),
    #[error("the run's record has no root invocation, so there is nothing to resume")]
    NothingToResume,
    #[error("the run's record could not be inspected")]
    Inspect(#[source] InspectError),
    #[error("the run ended without recording a status; the record says: {}", .0.join("; "))]
    Unfinished(Vec<String>),
}

/// How Fabro reports the run: what its read side records as the run's
/// terminal event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Conclusion {
    /// The record says the run succeeded and is whole.
    Succeeded,
    /// Anything else: the record says the run failed or was cancelled, the
    /// record is incomplete, or the run could not be executed at all.
    Failed {
        reason:  FailureReason,
        message: String,
    },
}

/// Execute the run to its end and report what the record says.
pub async fn run(request: RunRequest) -> Result<RunOutcome, RunError> {
    let backend = backend(&request.provider)?;
    let key = RunKey::new(request.run_id.as_str());
    let mut options = RunOptions::new(&request.run_dir);
    options.run_key = Some(key.clone());
    options.retention = Retention::Always;
    options.sandbox.backend = backend;
    let runtime = request
        .runtime
        .runtime(true)
        .store(Arc::clone(&request.store))
        .options(options);

    let dispatcher = InterviewDispatcher::new(Arc::new(Unattended));
    let cancel = request.cancel.clone();
    let mut cancel_task = None;
    let with_handle = |handle: petri_execution::CoordinatorHandle, secrets| {
        dispatcher.wire(handle.clone(), secrets);
        cancel_task = Some(tokio::spawn(async move {
            cancel.cancelled().await;
            info!("cancelling the Petri run");
            handle.cancel_root_for(CancelReason::Control);
        }));
    };
    let result = match request.execution {
        Execution::Start(graphs) => {
            info!(run_id = %request.run_id, backend = %backend, "Starting Petri run");
            let host_run = HostRun::new(graphs.graph)
                .with_children(graphs.children)
                .observe(Arc::new(dispatcher.clone()));
            Box::pin(host::run_configured(&runtime, host_run, with_handle)).await
        }
        Execution::Resume => {
            check_resumable(request.store.as_ref(), &key).await?;
            info!(run_id = %request.run_id, backend = %backend, "Resuming Petri run");
            Box::pin(host::resume_configured(
                &runtime,
                Vec::new(),
                vec![Arc::new(dispatcher.clone())],
                with_handle,
            ))
            .await
        }
    };
    if let Some(task) = cancel_task {
        task.abort();
    }
    let receipt = dispatcher.shutdown().await;
    write_receipt(&request.run_dir, &receipt).await;
    match &result {
        Ok(report) => debug!(status = %report.status, "Petri run ended"),
        Err(error) => warn!(error = %error, "Petri run ended with a host error"),
    }
    let inspection = inspect(request.store.as_ref(), &key).await?;
    outcome(inspection, result.err())
}

/// What the run's record says, read through a handle that holds no lease:
/// the same derivation [`run`] ends with, for a caller that only holds the
/// store, such as a test checking a finished run.
pub async fn outcome_of(store: &dyn RunStore, run_id: &str) -> Result<RunOutcome, RunError> {
    let inspection = inspect(store, &RunKey::new(run_id)).await?;
    outcome(inspection, None)
}

/// How Fabro reports what [`run`] returned. A cancelled run is a failure
/// with the cancelled reason, as the legacy executor reports one; every
/// other shortfall is a workflow error whose message says what the record,
/// or the host, said.
#[must_use]
pub fn conclusion(result: &Result<RunOutcome, RunError>) -> Conclusion {
    match result {
        Ok(RunOutcome {
            status: RunStatus::Success,
            complete: true,
            ..
        }) => Conclusion::Succeeded,
        Ok(outcome) => {
            let reason = match outcome.status {
                RunStatus::Cancelled => FailureReason::Cancelled,
                RunStatus::Success | RunStatus::Failed => FailureReason::WorkflowError,
            };
            Conclusion::Failed {
                reason,
                message: failure_message(outcome),
            }
        }
        Err(error) => Conclusion::Failed {
            reason:  FailureReason::WorkflowError,
            message: error_chain(error),
        },
    }
}

/// The failure of a run whose record says it did not succeed.
fn failure_message(outcome: &RunOutcome) -> String {
    let mut message = match (&outcome.status, &outcome.failure) {
        (RunStatus::Cancelled, _) => "the run was cancelled".to_string(),
        (_, Some(failure)) => failure.clone(),
        (RunStatus::Failed, None) => "the run failed".to_string(),
        (RunStatus::Success, None) => "the run's record is incomplete".to_string(),
    };
    if !outcome.complete {
        message.push_str(" (record incomplete: ");
        message.push_str(&outcome.incomplete.join("; "));
        message.push(')');
    }
    message
}

/// The error and every cause under it, as one line.
fn error_chain(error: &RunError) -> String {
    let mut parts = vec![error.to_string()];
    let mut cause = std::error::Error::source(error);
    while let Some(next) = cause {
        parts.push(next.to_string());
        cause = next.source();
    }
    parts.join(": ")
}

/// The sandbox backend for Fabro's provider kind.
fn backend(provider: &SandboxProviderKind) -> Result<SandboxBackend, RunError> {
    if *provider == SandboxProviderKind::LOCAL {
        Ok(SandboxBackend::Host)
    } else if *provider == SandboxProviderKind::DOCKER {
        Ok(SandboxBackend::Docker)
    } else if *provider == SandboxProviderKind::DAYTONA {
        Ok(SandboxBackend::Daytona)
    } else {
        Err(RunError::UnsupportedProvider {
            provider: provider.clone(),
        })
    }
}

/// Refuse a resume the host would not survive: `resume_configured` indexes
/// the root invocation of the stored state, so a record with none (the run
/// was created in the store and nothing more) is refused here with a named
/// error instead.
async fn check_resumable(store: &dyn RunStore, key: &RunKey) -> Result<(), RunError> {
    let logs = store
        .open(key, Access::Read)
        .await
        .map_err(RunError::Open)?;
    let state = host::stored_state(&*logs).await.map_err(RunError::Read)?;
    if state.invocations.contains_key(&InvocationId::ROOT) {
        Ok(())
    } else {
        Err(RunError::NothingToResume)
    }
}

/// Read the run back through a handle that holds no lease.
async fn inspect(store: &dyn RunStore, key: &RunKey) -> Result<RunInspection, RunError> {
    let logs = store
        .open(key, Access::Read)
        .await
        .map_err(RunError::Open)?;
    inspect::inspect_run(&*logs)
        .await
        .map_err(RunError::Inspect)
}

/// The outcome the record supports. A run whose record has no status is
/// unfinished: the host error, when there is one, says why.
fn outcome(
    inspection: RunInspection,
    host_error: Option<HostError>,
) -> Result<RunOutcome, RunError> {
    let status = match inspection.status.as_deref() {
        Some("success") => RunStatus::Success,
        Some("failed") => RunStatus::Failed,
        Some("cancelled") => RunStatus::Cancelled,
        _ => {
            let mut reasons = inspection.incomplete.clone();
            if let Some(error) = host_error {
                reasons.push(error.to_string());
            }
            return Err(RunError::Unfinished(reasons));
        }
    };
    let failure = inspection
        .invocations
        .iter()
        .find(|invocation| invocation.invocation == inspection.root.invocation)
        .and_then(|root| root.result.as_ref())
        .and_then(|result| result.failure.as_ref())
        .map(|failure| failure.message.clone());
    Ok(RunOutcome {
        status,
        failure,
        complete: inspection.complete,
        incomplete: inspection.incomplete,
    })
}

/// The interview receipt beside the run, as the standalone runner writes
/// it. A receipt that cannot be written is logged: the run's record does
/// not depend on it.
async fn write_receipt(run_dir: &std::path::Path, receipt: &petri_execution::InterviewReceipt) {
    let path = run_dir.join(RECEIPT_FILE);
    let bytes = match serde_json::to_vec_pretty(receipt) {
        Ok(bytes) => bytes,
        Err(error) => {
            warn!(error = %error, "could not encode the interview receipt");
            return;
        }
    };
    if let Err(error) = fs::create_dir_all(run_dir).await {
        warn!(path = %run_dir.display(), error = %error, "could not create the run directory");
        return;
    }
    if let Err(error) = fs::write(&path, bytes).await {
        warn!(path = %path.display(), error = %error, "could not write the interview receipt");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome_with(status: RunStatus, failure: Option<&str>, complete: bool) -> RunOutcome {
        RunOutcome {
            status,
            failure: failure.map(ToOwned::to_owned),
            complete,
            incomplete: if complete {
                Vec::new()
            } else {
                vec!["execution 0 did not finish".to_string()]
            },
        }
    }

    #[test]
    fn a_whole_successful_record_concludes_succeeded() {
        assert_eq!(
            conclusion(&Ok(outcome_with(RunStatus::Success, None, true))),
            Conclusion::Succeeded
        );
    }

    #[test]
    fn a_cancelled_record_concludes_cancelled() {
        assert_eq!(
            conclusion(&Ok(outcome_with(RunStatus::Cancelled, None, true))),
            Conclusion::Failed {
                reason:  FailureReason::Cancelled,
                message: "the run was cancelled".to_string(),
            }
        );
    }

    #[test]
    fn a_failed_record_carries_the_root_failure_and_the_incomplete_reasons() {
        assert_eq!(
            conclusion(&Ok(outcome_with(
                RunStatus::Failed,
                Some("step `say` failed"),
                false
            ))),
            Conclusion::Failed {
                reason:  FailureReason::WorkflowError,
                message: "step `say` failed (record incomplete: execution 0 did not finish)"
                    .to_string(),
            }
        );
    }

    #[test]
    fn a_host_error_concludes_with_its_chain() {
        let error = RunError::Unfinished(vec!["no status".to_string()]);
        assert_eq!(conclusion(&Err(error)), Conclusion::Failed {
            reason:  FailureReason::WorkflowError,
            message: "the run ended without recording a status; the record says: no status"
                .to_string(),
        });
    }
}
