//! Runs on Petri: what the server does at create time and at execution when
//! a run's engine is Petri.
//!
//! At create, [`admit`] hands the workflow version's bundle, the run's inputs
//! and the launch to Petri's `Runtime::check` through `fabro_petri::check`,
//! maps Petri's diagnostics onto Fabro's, and stores the admitted graphs in
//! the blob store so the run executes and resumes from what was admitted.
//! Petri compiled, linted and pinned models; the legacy compile is skipped.
//!
//! At execution, a Petri run takes the same path a legacy run does: the
//! scheduler launches `fabro run __run-worker` with the worker's token, and
//! the worker executes the run through `fabro_petri::engine` over the HTTP
//! run store, appending the run lifecycle events Fabro's read side needs
//! (`run.starting`, `run.running`, then `run.completed` or `run.failed`).
//! The server keeps the worker's lease for as long as the worker lives
//! (`crate::petri_runs`). Under the test override that replaces the handler
//! registry, [`execute`] runs the same engine in the server process over the
//! run store in the server's database, so the scenario tests need no
//! worker binary; its questions go to an in-process control interviewer
//! the answer endpoint reaches directly, its secrets come from a snapshot
//! of the server's vault, and its blobs go to the server's blob store. No
//! stage or agent event is projected either way, which is the read-side
//! item that follows.
//!
//! After a server restart, [`reconcile_on_startup`] hands a Petri run the
//! previous server left in flight back to a worker in resume mode, once the
//! recovery protocol (`fabro_petri::recovery`) has brought every live
//! workspace to the snapshot its durable state names, or reports the run
//! failed when it cannot.

use std::sync::Arc;
use std::time::Instant;

use fabro_config::{Home, SettingsLayer, Storage};
use fabro_interview::ControlInterviewer;
use fabro_petri::controls::RunControls;
use fabro_petri::engine::{self, Conclusion, Execution, RunRequest};
use fabro_petri::hooks::HooksSpec;
use fabro_petri::interview::{Approval, FabroInterviewer};
use fabro_petri::petri::StoreError;
use fabro_petri::platform_records::SqlitePlatformRecords;
use fabro_petri::recovery::{self, Recovery, RecoveryRequest};
use fabro_petri::runtime::{self, RuntimeSpec};
use fabro_petri::secrets::VaultSecrets;
use fabro_petri::{SqliteRunStore, admission};
use fabro_store::platform_records::{RunLifecycleKind, RunLifecycleRecord};
use fabro_types::settings::run::{ApprovalMode, RunMode};
use fabro_types::{PetriAdmission, RunId, RunRunnableSource, RunTarget};
use fabro_util::error as error_util;
use fabro_workflow::Error as WorkflowError;
use fabro_workflow::run_status::{FailureReason, RunStatus, SuccessReason};
use lithos_llm::catalog::ProviderId;
use tokio::task;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use super::{
    AppState, RunAnswerTransport, RunExecutionMode, clear_live_run_state, run_records,
    stream_follower,
};
use crate::petri_check;
use crate::petri_runs::PetriRuns;
use crate::run_compiler::{PreparedRun, RunCompilerError};

/// The runtime Petri gets, at create and at execution: the server's run
/// defaults as the settings layer, the model client over the server's
/// catalog and credentials for the eligible providers, and the run mode.
pub(crate) fn runtime_spec(
    state: &AppState,
    eligible: &[ProviderId],
    dry_run: bool,
) -> RuntimeSpec {
    let settings_toml = settings_layer_toml(state);
    let catalog = state.catalog();
    let model_client = match runtime::model_client(
        (*catalog).clone(),
        Arc::clone(&state.llm_source),
        state.http_client.clone(),
        eligible,
    ) {
        Ok(client) => client,
        Err(err) => {
            warn!(error = %err, "Petri model client unavailable; LLM nodes stay unpinned");
            None
        }
    };
    RuntimeSpec {
        settings_toml,
        model_client,
        dry_run,
        fabro_home: Some(Home::from_env().root().to_path_buf()),
        // The in-process test path has no worker client to bind the run
        // tools to; like the legacy in-process path, it runs without them.
        run_tools: None,
    }
}

/// The server's `[run]` defaults, as the text of the operator settings
/// layer the Fabro frontend reads below `.fabro/project.toml` and
/// `workflow.toml`.
fn settings_layer_toml(state: &AppState) -> Option<String> {
    let layer = SettingsLayer {
        version: Some(1),
        run: Some((*state.manifest_run_defaults()).clone()),
        ..SettingsLayer::default()
    };
    match toml::to_string(&layer) {
        Ok(text) => Some(text),
        Err(err) => {
            warn!(error = %err, "server run defaults do not serialize; Petri gets no settings layer");
            None
        }
    }
}

/// Petri compiles the run: check the bundle, map the diagnostics, and
/// persist the admitted graphs. A refusal is the same validation error the
/// legacy compiler raised, carrying Petri's diagnostics.
pub(crate) async fn admit(
    state: &AppState,
    prepared: &PreparedRun,
    eligible: &[ProviderId],
) -> Result<PetriAdmission, RunCompilerError> {
    let settings = prepared.settings();
    let repository = match prepared.target() {
        Some(RunTarget::Folder { path }) => Some(path.into()),
        Some(RunTarget::Git(_) | RunTarget::None {}) | None => None,
    };
    let launch = petri_check::launch(&state.catalog(), settings, eligible, repository);
    let dry_run = settings.run.execution.mode == RunMode::DryRun;
    let request = petri_check::check_request(
        prepared.workflow_bundle(),
        prepared.entrypoint(),
        settings,
        prepared.vars(),
        launch,
        runtime_spec(state, eligible, dry_run),
        false,
    )
    .map_err(RunCompilerError::Workflow)?;
    let has_ready_provider = !eligible.is_empty();
    let checked = task::spawn_blocking(move || petri_check::check(&request, has_ready_provider))
        .await
        .map_err(|source| {
            RunCompilerError::Workflow(WorkflowError::engine_with_source(
                "Petri check task failed",
                source,
            ))
        })?
        .map_err(RunCompilerError::Workflow)?;
    if checked.has_errors() {
        return Err(RunCompilerError::Workflow(
            WorkflowError::ValidationFailed {
                diagnostics: checked.diagnostics,
            },
        ));
    }
    for warning in &checked.diagnostics {
        info!(code = %warning.rule, message = %warning.message, "Petri warned at admission");
    }
    let admitted = checked.admitted.ok_or_else(|| {
        RunCompilerError::Workflow(WorkflowError::engine(
            "Petri's check admitted no graph and raised no error",
        ))
    })?;
    admission::persist(&state.store_ref().blobs(), &admitted)
        .await
        .map_err(|err| {
            RunCompilerError::Workflow(WorkflowError::engine_with_source(
                "the admitted graphs could not be stored",
                err,
            ))
        })
}

/// Execute a Petri run in the server process, under the test override:
/// runnable → starting → running → succeeded or failed, with the lifecycle
/// events Fabro's read side needs. Outside tests a Petri run executes in
/// its worker process, launched as a legacy run's worker is.
pub(crate) async fn execute(state: Arc<AppState>, run_id: RunId) {
    let (run_dir, cancel, mode) = {
        let mut runs = state.runs.lock().expect("runs lock poisoned");
        let managed_run = match runs.get_mut(&run_id) {
            Some(run) if run.status == RunStatus::Runnable => run,
            _ => return,
        };
        let Some(run_dir) = managed_run.run_dir.clone() else {
            return;
        };
        let cancel = CancellationToken::new();
        managed_run.status = RunStatus::Starting;
        managed_run.cancel_token = Some(cancel.clone());
        (run_dir, cancel, managed_run.execution_mode)
    };

    stream_follower::follow_run(&state, run_id).await;
    let run_state = match run_records::projection(&state, run_id).await {
        Ok(Some(run_state)) => run_state,
        Ok(None) => {
            error!(run_id = %run_id, "Run not found at launch");
            finish(
                &state,
                run_id,
                RunStatus::Failed {
                    reason: FailureReason::WorkflowError,
                },
                Some("Run not found at launch".to_string()),
            );
            return;
        }
        Err(err) => {
            error!(run_id = %run_id, error = %err, "Failed to load run state");
            finish(
                &state,
                run_id,
                RunStatus::Failed {
                    reason: FailureReason::WorkflowError,
                },
                Some(format!("Failed to load run state: {err}")),
            );
            return;
        }
    };
    let admission = run_state.spec.admission.clone();
    let server_settings = state.server_settings();
    if super::reject_run_if_sandbox_provider_disabled(
        &state,
        &server_settings,
        run_id,
        &run_state.spec.settings.run,
    )
    .await
    {
        return;
    }
    let execution = match mode {
        RunExecutionMode::Start => {
            match admission::load(&state.store_ref().blobs(), &admission).await {
                Ok(graphs) => Execution::Start(graphs),
                Err(err) => {
                    let message = error_util::collect_chain(&err).join(": ");
                    fail_before_execution(&state, run_id, &message).await;
                    return;
                }
            }
        }
        RunExecutionMode::Resume => Execution::Resume,
    };
    // The run's secrets: a snapshot of the server's vault, as a worker
    // takes one at launch.
    let vault = match state.stores.vault.snapshot().await {
        Ok(snapshot) => snapshot.into_vault(),
        Err(err) => {
            let message = error_util::collect_chain(&err).join(": ");
            fail_before_execution(
                &state,
                run_id,
                &format!("the vault could not be read for the run: {message}"),
            )
            .await;
            return;
        }
    };
    let started = Instant::now();
    for record in [
        run_records::transition(RunLifecycleKind::Starting, RunStatus::Starting),
        run_records::transition(RunLifecycleKind::Running, RunStatus::Running),
    ] {
        if let Err(err) = run_records::lifecycle(&state, run_id, record).await {
            error!(run_id = %run_id, error = %err, "Failed to persist run lifecycle record");
            finish(
                &state,
                run_id,
                RunStatus::Failed {
                    reason: FailureReason::WorkflowError,
                },
                Some(format!("Failed to persist run lifecycle record: {err}")),
            );
            return;
        }
    }
    // The answer endpoint reaches this interviewer directly. The lifecycle
    // records above already moved the live status to Running; a run that
    // ended meanwhile (cancelled while starting) takes no transport.
    let interviewer = Arc::new(ControlInterviewer::new());
    // The steer and interrupt endpoints reach these controls in place.
    let controls = RunControls::new();
    {
        let mut runs = state.runs.lock().expect("runs lock poisoned");
        if let Some(managed_run) = runs
            .get_mut(&run_id)
            .filter(|managed_run| !managed_run.status.is_terminal())
        {
            managed_run.answer_transport = Some(RunAnswerTransport::InProcess {
                interviewer: Arc::clone(&interviewer),
                controls:    controls.clone(),
            });
        }
    }
    let approval = if run_state.spec.settings.run.execution.approval == ApprovalMode::Auto {
        Approval::Auto
    } else {
        Approval::Prompt
    };
    let petri_interviewer = FabroInterviewer::new(interviewer, approval);
    let observers = vec![petri_interviewer.observer()];
    let (_, eligible) = state.resolve_llm_client_with_ready_ids().await;
    let dry_run = run_state.spec.settings.run.execution.mode == RunMode::DryRun;
    let hooks = HooksSpec::for_run(
        Arc::new(SqlitePlatformRecords::new(Arc::clone(
            &state.stores.run_summaries,
        ))),
        &run_state.spec.settings.run,
    );
    let request = RunRequest {
        run_id: run_id.to_string(),
        run_dir: run_dir.join("petri"),
        execution,
        store: state
            .petri_projector
            .observe_store(Arc::new(SqliteRunStore::new(state.db_pool.clone()))),
        runtime: runtime_spec(&state, &eligible, dry_run),
        provider: run_state.spec.settings.run.environment.provider.clone(),
        cancel,
        // The in-process test path drives no pause: the server's transport
        // for it names the worker. A steer or an interrupt is answered in
        // place.
        controls,
        interviewer: Arc::new(petri_interviewer),
        observers,
        secrets: Some(Arc::new(VaultSecrets::from_vault(&vault))),
        blobs: Some(state.store_ref().blobs()),
        hooks: Some(hooks),
    };
    let result = Box::pin(engine::run(request)).await;
    info!(
        run_id = %run_id,
        elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        "Petri run ended"
    );
    let (status, error, record) = match engine::conclusion(&result) {
        Conclusion::Succeeded => {
            info!(run_id = %run_id, "Petri run completed");
            (
                RunStatus::Succeeded {
                    reason: SuccessReason::Completed,
                },
                None,
                run_records::succeeded(SuccessReason::Completed),
            )
        }
        Conclusion::Failed { reason, message } => {
            info!(run_id = %run_id, error = %message, "Petri run did not succeed");
            failed(reason, message)
        }
    };
    if let Err(err) = run_records::lifecycle(&state, run_id, record).await {
        error!(run_id = %run_id, error = %err, "Failed to persist run outcome");
    }
    // The view trails the terminal record; the aggregate reads the settled
    // projection, as the worker path reads the final state at worker exit.
    state.petri_projector.settle(run_id).await;
    match state.load_run_projection(&run_id).await {
        Ok(final_state) => super::accumulate_concluded_run_usage(&state, &final_state),
        Err(err) => {
            warn!(run_id = %run_id, error = ?err, "the run's final state could not be read for the usage aggregate");
        }
    }
    finish(&state, run_id, status, error);
}

/// Bring a Petri run the server left in flight back to its worker after a
/// restart: the run continues from its records, as Petri's own resume does,
/// on workspaces that match them.
///
/// The lease the previous worker held is released from outside, which
/// fences that worker should it still be alive. Then the recovery protocol
/// reads the run's durable execution state: a run with a failed checkpoint
/// is reported failed here and never resumed; otherwise every live
/// workspace on this host is verified against, reset to, or restored from
/// the snapshot its last durable finish names, and a finish with no
/// snapshot fails the run rather than resume it on stale files. The run is
/// then asked to start again as a resume (`run.start_requested` with
/// `resume`, then `run.runnable`, the same pair the API's resume appends),
/// and a managed run is registered for the scheduler in resume mode when
/// Petri's store holds the run, else in start mode: a worker that died
/// before it created the run's record left nothing to continue from, so the
/// run starts from its admitted graphs.
pub(crate) async fn reconcile_on_startup(
    state: &Arc<AppState>,
    run_id: RunId,
    run_state: &fabro_store::RunProjection,
) -> anyhow::Result<()> {
    let key = PetriRuns::key(&run_id);
    let held = match state.petri_runs.release_for_restart(run_id).await {
        Ok(()) => true,
        Err(StoreError::NotFound { .. }) => false,
        Err(err) => {
            return Err(anyhow::Error::new(err).context("releasing the Petri run's lease"));
        }
    };
    let run_dir = Storage::new(state.server_storage_dir())
        .run_scratch(&run_id)
        .root()
        .to_path_buf();
    let mode = if held {
        let request = RecoveryRequest::for_run(
            run_id,
            run_dir.join("petri"),
            Arc::new(SqliteRunStore::new(state.db_pool.clone())),
            Arc::new(SqlitePlatformRecords::new(Arc::clone(
                &state.stores.run_summaries,
            ))),
            &run_state.spec.settings.run,
        );
        match recovery::recover(request)
            .await
            .map_err(|err| anyhow::Error::new(err).context("recovering the Petri run"))?
        {
            Recovery::Start => RunExecutionMode::Start,
            Recovery::Resume { workspaces } => {
                info!(
                    run_id = %run_id,
                    workspaces = workspaces.len(),
                    "Petri run's workspaces match its durable state"
                );
                RunExecutionMode::Resume
            }
            Recovery::Failed { reason } => {
                warn!(
                    run_id = %run_id,
                    petri_key = %key,
                    error = %reason,
                    "Petri run left in flight by the previous server cannot resume; reporting it failed"
                );
                let (_, _, record) = failed(FailureReason::WorkflowError, reason);
                run_records::lifecycle(state, run_id, record).await?;
                return Ok(());
            }
        }
    } else {
        RunExecutionMode::Start
    };
    info!(
        run_id = %run_id,
        petri_key = %key,
        mode = super::worker_mode_arg(mode),
        "Petri run left in flight by the previous server; relaunching its worker"
    );
    let mut start_requested = RunLifecycleRecord::new(RunLifecycleKind::StartRequested);
    start_requested.source = Some("resume".to_string());
    let mut runnable = run_records::transition(RunLifecycleKind::Runnable, RunStatus::Runnable);
    runnable.source = Some(<&'static str>::from(RunRunnableSource::StartRequested).to_string());
    for record in [start_requested, runnable] {
        run_records::lifecycle(state, run_id, record).await?;
    }
    let mut runs = state.runs.lock().expect("runs lock poisoned");
    runs.insert(
        run_id,
        super::managed_run(
            run_state.spec.graph_source.clone().unwrap_or_default(),
            RunStatus::Runnable,
            run_id.created_at(),
            run_dir,
            mode,
        ),
    );
    Ok(())
}

/// The failed status, its message, and the `failed` lifecycle record for it.
fn failed(
    reason: FailureReason,
    message: String,
) -> (RunStatus, Option<String>, RunLifecycleRecord) {
    let detail = match reason {
        FailureReason::Cancelled => WorkflowError::Cancelled.to_string(),
        _ => message.clone(),
    };
    (
        RunStatus::Failed { reason },
        Some(message),
        run_records::failed(reason, detail),
    )
}

/// Record a failure that happened before Petri ran, then finish the run.
async fn fail_before_execution(state: &Arc<AppState>, run_id: RunId, message: &str) {
    error!(run_id = %run_id, error = message, "Petri run cannot start");
    let (status, error, record) = failed(FailureReason::WorkflowError, message.to_string());
    if let Err(err) = run_records::lifecycle(state, run_id, record).await {
        error!(run_id = %run_id, error = %err, "Failed to persist run failure status");
    }
    finish(state, run_id, status, error);
}

/// Settle the managed run and release its scheduler slot.
fn finish(state: &Arc<AppState>, run_id: RunId, status: RunStatus, error: Option<String>) {
    let mut runs = state.runs.lock().expect("runs lock poisoned");
    if let Some(managed_run) = runs.get_mut(&run_id) {
        managed_run.status = status;
        managed_run.error = error;
        clear_live_run_state(managed_run);
    }
    drop(runs);
    state.scheduler_notify.notify_one();
}
