//! A Petri run in the worker process.
//!
//! When `fabro run __run-worker` finds that its run's stored spec names
//! Petri as the engine, the run executes here instead of through the legacy
//! executor, over the same worker services: the authenticated client, the
//! control channel the server pushes cancels through, the signal handlers,
//! the vault snapshot and the CLI catalog. The engine assembly itself is
//! `fabro_petri::engine`, shared with the server's in-process test path, so
//! the run gets the same runtime, options and interviewer either way.
//!
//! The run's record is [`HttpRunStore`] over the worker's client, leased
//! for this launch: the worker mints one owner id at start, logs it, and
//! every lease the run takes over the API names it. `--mode start` loads
//! the admitted graphs through the client's blob read and runs them;
//! `--mode resume` continues the run from its records. Either way the
//! worker appends the lifecycle events Fabro's read side needs
//! (`run.starting`, `run.running`, then `run.completed` or `run.failed`)
//! through the client, as the legacy worker does.
//!
//! Of the server's controls, cancel and answers are wired: the control
//! channel's cancel and `SIGTERM`/`SIGINT` fire one token, which cancels
//! Petri's root invocation politely, and an `interview.answer` message
//! reaches the control interviewer the run's questions wait on
//! (`fabro_petri::interview`), so a human gate answered through the API
//! continues. Pause, unpause and steer are received and ignored with a
//! warning until their Petri adapters land. A control channel that is lost
//! for good cancels the run the same way, and the worker exits with that
//! loss as its error once the run has settled.
//!
//! The runtime's settings layer is left empty here: the run's graphs were
//! lowered and admitted at create time with the server's layer, and nothing
//! lowers again at execution. The model client is built from the worker's
//! catalog and vault snapshot for the providers whose credentials resolve,
//! the same eligible set the legacy worker's LLM backend uses. The same
//! vault snapshot is the run's secret provider, the run's blobs go to the
//! server's blob table through the worker's client, and the Fabro home the
//! server named on the command line is the home the skills step reads.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use fabro_auth::VaultCredentialSource;
use fabro_client::{Client, ServerTarget};
use fabro_interview::ControlInterviewer;
use fabro_llm::credentials::{CredentialProvider, readiness};
use fabro_petri::blobs::ClientBlobs;
use fabro_petri::engine::{self, Conclusion, Execution, RunRequest};
use fabro_petri::interview::{Approval, EventSinkQuestions, FabroInterviewer};
use fabro_petri::petri::OwnerId;
use fabro_petri::runtime::{self, RuntimeSpec};
use fabro_petri::secrets::VaultSecrets;
use fabro_petri::{HttpRunStore, admission};
use fabro_store::RunProjection;
use fabro_types::settings::run::{ApprovalMode, RunMode};
use fabro_types::{FailureReason, RunId, RunTiming, StageOutcome, SuccessReason};
use fabro_vault::Vault;
use fabro_workflow::Error as WorkflowError;
use fabro_workflow::event::{self as workflow_event, Emitter, Event, RunEventSink};
use fabro_workflow::run_control::RunControlState;
use fabro_workflow::runtime_store::RunStoreHandle;
use tokio::sync::RwLock as AsyncRwLock;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::runner::{self, WorkerTitlePhase};
use crate::args::RunWorkerMode;
use crate::command_context;

/// What the worker holds when it hands a run to Petri.
pub(super) struct PetriWorker<'a> {
    pub(super) run_id:       RunId,
    pub(super) target:       ServerTarget,
    pub(super) client:       Client,
    /// The legacy run store over the same client, which carries the
    /// lifecycle events to the server with its retries.
    pub(super) run_store:    RunStoreHandle,
    pub(super) run_state:    RunProjection,
    pub(super) storage_dir:  &'a Path,
    pub(super) run_dir:      PathBuf,
    pub(super) mode:         RunWorkerMode,
    /// The Fabro home the server named; `None` falls back to Petri's own
    /// lookup of the worker's environment.
    pub(super) fabro_home:   Option<PathBuf>,
    pub(super) worker_token: &'a str,
}

/// Execute the run to its end. `Ok` when the record says it succeeded;
/// the failure otherwise, after the terminal event is appended, so the
/// worker exits as the legacy worker does for a failed run.
pub(super) async fn execute(worker: PetriWorker<'_>) -> Result<()> {
    let run_id = worker.run_id;
    let Some(admission) = worker.run_state.spec.engine.petri().cloned() else {
        bail!("run {run_id} names Petri as its engine but carries no admission");
    };
    let owner = OwnerId::mint();
    info!(
        run_id = %run_id,
        owner = %owner,
        mode = ?worker.mode,
        "Petri worker starting; every lease of this run names this launch"
    );
    let store = Arc::new(HttpRunStore::for_worker(
        worker.client.clone_for_reuse(),
        owner,
    ));

    let cancel_token = CancellationToken::new();
    let run_control = RunControlState::new();
    runner::install_signal_handlers(Arc::clone(&run_control), cancel_token.clone())?;
    let interviewer = Arc::new(ControlInterviewer::new());
    let emitter = Arc::new(Emitter::new(run_id));
    let steering_hub = Arc::new(fabro_workflow::SteeringHub::new(Arc::clone(&emitter)));
    let mut control_manager = runner::spawn_worker_control_manager(
        worker.target.clone(),
        run_id,
        worker.worker_token.to_owned(),
        Arc::clone(&interviewer),
        cancel_token.clone(),
        steering_hub,
        run_control,
    );
    control_manager.wait_for_first_connection().await?;
    warn!(
        run_id = %run_id,
        "a Petri run answers cancel and questions only: pause, unpause and steer are not wired \
         yet and are ignored"
    );
    let sink = RunEventSink::map(
        runner::stamp_system_worker,
        RunEventSink::backend(worker.run_store.clone()),
    );
    let approval = if worker.run_state.spec.settings.run.execution.approval == ApprovalMode::Auto {
        Approval::Auto
    } else {
        Approval::Prompt
    };
    let questions = Arc::new(EventSinkQuestions::new(sink.clone(), run_id));
    let petri_interviewer = FabroInterviewer::new(interviewer, questions, approval);
    let observers = vec![petri_interviewer.observer()];

    let vault = runner::load_worker_vault(worker.storage_dir).await?;
    let secrets = VaultSecrets::from_vault(&*vault.read().await);
    let runtime = runtime_spec(&vault, &worker.run_state, worker.fabro_home.clone()).await?;
    let execution = match worker.mode {
        RunWorkerMode::Start => {
            let client = worker.client.clone_for_reuse();
            let graphs = admission::load_with(
                |blob| {
                    let client = client.clone_for_reuse();
                    async move { client.read_run_blob(&run_id, &blob).await }
                },
                &admission,
            )
            .await
            .context("loading the admitted graphs")?;
            Execution::Start(graphs)
        }
        RunWorkerMode::Resume => Execution::Resume,
    };

    let started = Instant::now();
    for event in [Event::RunStarting, Event::RunRunning] {
        workflow_event::append_event_to_sink(&sink, &run_id, &event).await?;
    }
    runner::set_worker_title(&run_id, WorkerTitlePhase::Running);

    let request = RunRequest {
        run_id: run_id.to_string(),
        run_dir: worker.run_dir.join("petri"),
        execution,
        store,
        runtime,
        provider: worker
            .run_state
            .spec
            .settings
            .run
            .environment
            .provider
            .clone(),
        cancel: cancel_token.clone(),
        interviewer: Arc::new(petri_interviewer),
        observers,
        secrets: Some(Arc::new(secrets)),
        blobs: Some(Arc::new(ClientBlobs::new(
            worker.client.clone_for_reuse(),
            run_id,
        ))),
    };
    let run = Box::pin(engine::run(request));
    tokio::pin!(run);
    let mut control_lost = None;
    let result = loop {
        tokio::select! {
            result = &mut run => break result,
            lost = control_manager.fatal_control_loss(), if control_lost.is_none() => {
                // The server can no longer reach this worker: end the run
                // politely, let Petri record why, then report the loss.
                warn!(run_id = %run_id, error = %lost, "worker control lost; cancelling the Petri run");
                control_lost = Some(lost);
                cancel_token.cancel();
            }
        }
    };
    control_manager.finish();

    let timing = RunTiming {
        wall_time_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        ..RunTiming::default()
    };
    let (event, phase, failure) = match engine::conclusion(&result) {
        Conclusion::Succeeded => {
            info!(run_id = %run_id, "Petri run completed");
            (
                Event::WorkflowRunCompleted {
                    timing,
                    artifact_count: 0,
                    status: StageOutcome::Succeeded.to_string(),
                    reason: SuccessReason::Completed,
                    final_git_commit_sha: None,
                    final_patch: None,
                    diff_summary: None,
                    usage: None,
                },
                WorkerTitlePhase::Succeeded,
                None,
            )
        }
        Conclusion::Failed { reason, message } => {
            info!(run_id = %run_id, error = %message, "Petri run did not succeed");
            let error = match reason {
                FailureReason::Cancelled => WorkflowError::Cancelled,
                _ => WorkflowError::engine(message.clone()),
            };
            let phase = if reason == FailureReason::Cancelled {
                WorkerTitlePhase::Cancelled
            } else {
                WorkerTitlePhase::Failed
            };
            (
                Event::workflow_run_failed_from_error(
                    &error, timing, reason, None, None, None, None,
                ),
                phase,
                Some(message),
            )
        }
    };
    workflow_event::append_event_to_sink(&sink, &run_id, &event).await?;
    runner::set_worker_title(&run_id, phase);
    if let Some(lost) = control_lost {
        return Err(lost);
    }
    match failure {
        None => Ok(()),
        Some(message) => Err(anyhow!("Petri run failed: {message}")),
    }
}

/// The runtime the worker hands Petri: no settings layer (nothing lowers
/// at execution), the model client over the worker's catalog and vault for
/// the providers whose credentials resolve, the run's mode, and the Fabro
/// home the server named.
async fn runtime_spec(
    vault: &Arc<AsyncRwLock<Vault>>,
    run_state: &RunProjection,
    fabro_home: Option<PathBuf>,
) -> Result<RuntimeSpec> {
    let catalog =
        command_context::load_cli_catalog().context("failed to build worker LLM catalog")?;
    let credentials: Arc<dyn CredentialProvider> =
        Arc::new(VaultCredentialSource::new(Arc::clone(vault)));
    let ready = readiness(catalog.enabled_providers(), credentials.as_ref()).await;
    for (provider, issue) in &ready.issues {
        warn!(provider = %provider, error = %issue, "model provider credentials unusable");
    }
    let model_client = match runtime::model_client(catalog, credentials, None, &ready.ready) {
        Ok(client) => client,
        Err(err) => {
            warn!(error = %err, "Petri model client unavailable; LLM nodes run without one");
            None
        }
    };
    Ok(RuntimeSpec {
        settings_toml: None,
        model_client,
        dry_run: run_state.spec.settings.run.execution.mode == RunMode::DryRun,
        fabro_home,
    })
}
