//! A Fabro run executed by Petri, in the server process.
//!
//! Until the worker's HTTP run store lands, a Petri run executes where the
//! server is: the runtime is assembled the same way the create handler
//! assembled it for `Runtime::check`, the run's records go to
//! [`SqliteRunStore`] under the Fabro run id as the run key, the admitted
//! graphs are loaded from the blob store, and `execution::host::run_configured`
//! runs the root invocation to its end. The outcome is then derived from
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
//! No stage or agent event is projected into Fabro's tables here; the
//! caller appends only the run lifecycle events Fabro's read side needs to
//! finish the run. The projection over Petri's records is the read-side
//! item that follows.

use std::path::PathBuf;
use std::sync::Arc;

use fabro_store::BlobStore;
use fabro_types::{PetriAdmission, SandboxProviderKind};
use petri_execution::host::{self, HostError, HostRun};
use petri_execution::inspect::{self, InspectError, RunInspection};
use petri_execution::{Access, CancelReason, InterviewDispatcher, RECEIPT_FILE, RunKey, RunStore};
use petri_runtime::executor::Retention;
use petri_runtime::{RunOptions, SandboxBackend};
use tokio::fs;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::admission::{self, AdmissionError};
use crate::interviewer::Unattended;
use crate::run_store::SqliteRunStore;
use crate::runtime::RuntimeSpec;

/// One run to execute.
pub struct RunRequest {
    /// The Fabro run id, which becomes Petri's run key: the run's identity
    /// in the store and the label on every sandbox of the run.
    pub run_id:    String,
    /// Where the run's workspaces, step output and blobs live.
    pub run_dir:   PathBuf,
    /// What the create handler admitted.
    pub admission: PetriAdmission,
    /// The blob store the admitted graphs are read from.
    pub blobs:     Arc<BlobStore>,
    /// The run's durable record.
    pub store:     Arc<SqliteRunStore>,
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
    #[error("the admitted graphs could not be loaded")]
    Admission(#[from] AdmissionError),
    #[error("the run's record could not be opened")]
    Open(#[source] petri_store::StoreError),
    #[error("the run's record could not be inspected")]
    Inspect(#[source] InspectError),
    #[error("the run ended without recording a status; the record says: {}", .0.join("; "))]
    Unfinished(Vec<String>),
}

/// Execute the run to its end and report what the record says.
pub async fn run(request: RunRequest) -> Result<RunOutcome, RunError> {
    let backend = backend(&request.provider)?;
    let (graph, children) = admission::load(&request.blobs, &request.admission).await?;
    let key = RunKey::new(request.run_id.as_str());
    let mut options = RunOptions::new(&request.run_dir);
    options.run_key = Some(key.clone());
    options.retention = Retention::Always;
    options.sandbox.backend = backend;
    let store: Arc<dyn RunStore> = request.store.clone();
    let runtime = request.runtime.runtime(true).store(store).options(options);

    let dispatcher = InterviewDispatcher::new(Arc::new(Unattended));
    let host_run = HostRun::new(graph)
        .with_children(children)
        .observe(Arc::new(dispatcher.clone()));
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
    info!(run_id = %request.run_id, backend = %backend, "Starting Petri run");
    let result = Box::pin(host::run_configured(&runtime, host_run, with_handle)).await;
    if let Some(task) = cancel_task {
        task.abort();
    }
    let receipt = dispatcher.shutdown().await;
    write_receipt(&request.run_dir, &receipt).await;
    match &result {
        Ok(report) => debug!(status = %report.status, "Petri run ended"),
        Err(error) => warn!(error = %error, "Petri run ended with a host error"),
    }
    let inspection = inspect(&request.store, &key).await?;
    outcome(inspection, result.err())
}

/// What the run's record says, read through a handle that holds no lease:
/// the same derivation [`run`] ends with, for a caller that only holds the
/// store, such as a test checking a finished run.
pub async fn outcome_of(store: &SqliteRunStore, run_id: &str) -> Result<RunOutcome, RunError> {
    let inspection = inspect(store, &RunKey::new(run_id)).await?;
    outcome(inspection, None)
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

/// Read the run back through a handle that holds no lease.
async fn inspect(store: &SqliteRunStore, key: &RunKey) -> Result<RunInspection, RunError> {
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
