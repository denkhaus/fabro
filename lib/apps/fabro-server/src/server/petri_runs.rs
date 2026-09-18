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

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use fabro_config::{Home, SettingsLayer, Storage};
use fabro_interview::ControlInterviewer;
use fabro_llm::selection;
use fabro_petri::check::{self, Bundle, CheckError, CheckRequest, Diagnostic, Launch};
use fabro_petri::controls::RunControls;
use fabro_petri::engine::{self, Conclusion, Execution, RunRequest};
use fabro_petri::hooks::HooksSpec;
use fabro_petri::interview::{Approval, DatabaseQuestions, FabroInterviewer};
use fabro_petri::petri::StoreError;
use fabro_petri::platform_records::SqlitePlatformRecords;
use fabro_petri::recovery::{self, Recovery, RecoveryRequest};
use fabro_petri::runtime::{self, RuntimeSpec};
use fabro_petri::secrets::VaultSecrets;
use fabro_petri::{SqliteRunStore, admission};
use fabro_types::settings::run::{ApprovalMode, RunMode};
use fabro_types::{
    Engine, PetriAdmission, RunId, RunRunnableSource, RunTarget, RunTiming, ServerSettings,
    StageOutcome,
};
use fabro_util::error as error_util;
use fabro_validate::{Diagnostic as FabroDiagnostic, Severity};
use fabro_workflow::Error as WorkflowError;
use fabro_workflow::event::Emitter;
use fabro_workflow::run_status::{FailureReason, RunStatus, SuccessReason};
use lithos_llm::catalog::ProviderId;
use tokio::task;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use super::{AppState, RunAnswerTransport, RunExecutionMode, clear_live_run_state, workflow_event};
use crate::petri_runs::PetriRuns;
use crate::run_compiler::{PreparedRun, RunCompilerError};

/// The engine a run gets: the one its workflow version names, else the
/// server's default.
pub(crate) fn engine_for(
    settings: &fabro_types::WorkflowSettings,
    server: &ServerSettings,
) -> Engine {
    settings
        .workflow
        .engine
        .unwrap_or(server.server.execution.engine)
}

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
/// legacy compiler raises, carrying Petri's diagnostics.
pub(crate) async fn admit(
    state: &AppState,
    prepared: &PreparedRun,
    eligible: &[ProviderId],
) -> Result<PetriAdmission, RunCompilerError> {
    let settings = prepared.settings();
    let mut files = BTreeMap::new();
    for workflow in prepared.workflow_bundle().workflows().values() {
        for (path, text) in &workflow.files {
            files.insert(path.to_string(), text.clone());
        }
        files.insert(workflow.path.to_string(), workflow.source.clone());
        if let Some(config) = &workflow.config {
            files.insert(config.path.to_string(), config.source.clone());
        }
    }
    let mut inputs = BTreeMap::new();
    for (name, value) in &settings.run.inputs {
        let value = serde_json::to_value(value).map_err(|err| {
            RunCompilerError::Workflow(WorkflowError::engine_with_source(
                format!("run input `{name}` does not encode as JSON"),
                err,
            ))
        })?;
        inputs.insert(name.clone(), value);
    }
    // The launch: Fabro's resolved model and provider. When the settings
    // name neither, the default offering of the eligible providers, as the
    // legacy compiler picked it, is bound as the launch model alone: a node
    // that names no model runs on it, and a node that names a model the
    // catalog lacks stays unqualified, so Petri's admission refuses it.
    let catalog = state.catalog();
    let model = settings.run.model.name.clone().or_else(|| {
        if settings.run.model.provider.is_some() {
            return None;
        }
        let eligible = eligible.iter().cloned().collect::<HashSet<_>>();
        selection::select_default(&catalog, &eligible)
            .ok()
            .map(|offering| offering.model.id().to_string())
    });
    let provider = settings.run.model.provider.clone();
    let repository = match prepared.target() {
        Some(RunTarget::Folder { path }) => Some(path.into()),
        Some(RunTarget::Git(_) | RunTarget::None {}) | None => None,
    };
    let dry_run = settings.run.execution.mode == RunMode::DryRun;
    let request = CheckRequest {
        bundle: Bundle {
            files,
            entrypoint: prepared.entrypoint().to_string(),
            project_toml: None,
        },
        inputs,
        launch: Launch {
            model,
            provider,
            repository,
        },
        runtime: runtime_spec(state, eligible, dry_run),
    };
    let admitted = task::spawn_blocking(move || check::check(&request))
        .await
        .map_err(|source| {
            RunCompilerError::Workflow(WorkflowError::engine_with_source(
                "Petri check task failed",
                source,
            ))
        })?
        .map_err(|err| match err {
            CheckError::Rejected(diagnostics) => {
                RunCompilerError::Workflow(WorkflowError::ValidationFailed {
                    diagnostics: diagnostics.iter().map(fabro_diagnostic).collect(),
                })
            }
            other => RunCompilerError::Workflow(WorkflowError::engine_with_source(
                "Petri could not check the workflow",
                other,
            )),
        })?;
    for warning in &admitted.warnings {
        info!(code = %warning.code, message = %warning.message, "Petri warned at admission");
    }
    admission::persist(&state.store_ref().blobs(), &admitted)
        .await
        .map_err(|err| {
            RunCompilerError::Workflow(WorkflowError::engine_with_source(
                "the admitted graphs could not be stored",
                err,
            ))
        })
}

/// Petri's diagnostic in Fabro's shape: the code is the rule, the hint is
/// the fix, the bundle-relative file and position are the source location.
fn fabro_diagnostic(diagnostic: &Diagnostic) -> FabroDiagnostic {
    FabroDiagnostic {
        rule: diagnostic.code.clone(),
        severity: if diagnostic.is_error() {
            Severity::Error
        } else {
            Severity::Warning
        },
        message: diagnostic.message.clone(),
        fix: diagnostic.hint.clone(),
        source_path: Some(diagnostic.file.clone()),
        line: diagnostic.line,
        column: diagnostic.column,
        ..FabroDiagnostic::default()
    }
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

    let run_store = match state.stores.runs.open_run(&run_id).await {
        Ok(run_store) => run_store,
        Err(err) => {
            error!(run_id = %run_id, error = %err, "Failed to open run store");
            finish(
                &state,
                run_id,
                RunStatus::Failed {
                    reason: FailureReason::WorkflowError,
                },
                Some(format!("Failed to open run store: {err}")),
            );
            return;
        }
    };
    tokio::spawn(super::forward_run_events_to_global(
        Arc::clone(&state),
        run_id,
        run_store.subscribe(),
    ));
    let run_state = match run_store.state().await {
        Ok(run_state) => run_state,
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
    let Some(admission) = run_state.spec.engine.petri().cloned() else {
        fail_before_execution(&state, &run_store, run_id, "the run has no Petri admission").await;
        return;
    };
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
                    fail_before_execution(&state, &run_store, run_id, &message).await;
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
                &run_store,
                run_id,
                &format!("the vault could not be read for the run: {message}"),
            )
            .await;
            return;
        }
    };
    let started = Instant::now();
    for event in [
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
    ] {
        if let Err(err) = workflow_event::append_event(&run_store, &run_id, &event).await {
            error!(run_id = %run_id, error = %err, "Failed to persist run lifecycle event");
            finish(
                &state,
                run_id,
                RunStatus::Failed {
                    reason: FailureReason::WorkflowError,
                },
                Some(format!("Failed to persist run lifecycle event: {err}")),
            );
            return;
        }
    }
    // The answer endpoint reaches this interviewer directly, as it does
    // for a legacy run in this process.
    let interviewer = Arc::new(ControlInterviewer::new());
    let steering_hub = Arc::new(fabro_workflow::SteeringHub::new(Arc::new(Emitter::new(
        run_id,
    ))));
    {
        let mut runs = state.runs.lock().expect("runs lock poisoned");
        if let Some(managed_run) = runs.get_mut(&run_id) {
            if managed_run.status == RunStatus::Starting {
                managed_run.status = RunStatus::Running;
                managed_run.answer_transport = Some(RunAnswerTransport::InProcess {
                    interviewer: Arc::clone(&interviewer),
                    steering_hub,
                });
            }
        }
    }
    let approval = if run_state.spec.settings.run.execution.approval == ApprovalMode::Auto {
        Approval::Auto
    } else {
        Approval::Prompt
    };
    let questions = Arc::new(DatabaseQuestions::new(run_store.clone(), run_id));
    let petri_interviewer = FabroInterviewer::new(interviewer, questions, approval);
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
        // The in-process test path drives no pause or steer: the server's
        // transports for those name the worker.
        controls: RunControls::new(),
        interviewer: Arc::new(petri_interviewer),
        observers,
        secrets: Some(Arc::new(VaultSecrets::from_vault(&vault))),
        blobs: Some(state.store_ref().blobs()),
        hooks: Some(hooks),
    };
    let result = Box::pin(engine::run(request)).await;
    let timing = RunTiming {
        wall_time_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        ..RunTiming::default()
    };
    let (status, error, event) = match engine::conclusion(&result) {
        Conclusion::Succeeded => {
            info!(run_id = %run_id, "Petri run completed");
            (
                RunStatus::Succeeded {
                    reason: SuccessReason::Completed,
                },
                None,
                workflow_event::Event::WorkflowRunCompleted {
                    timing,
                    artifact_count: 0,
                    status: StageOutcome::Succeeded.to_string(),
                    reason: SuccessReason::Completed,
                    final_git_commit_sha: None,
                    final_patch: None,
                    diff_summary: None,
                    usage: None,
                },
            )
        }
        Conclusion::Failed { reason, message } => {
            info!(run_id = %run_id, error = %message, "Petri run did not succeed");
            failed(reason, message, timing)
        }
    };
    if let Err(err) = workflow_event::append_event(&run_store, &run_id, &event).await {
        error!(run_id = %run_id, error = %err, "Failed to persist run outcome");
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
    run_store: &fabro_store::RunDatabase,
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
                let (_, _, event) =
                    failed(FailureReason::WorkflowError, reason, RunTiming::default());
                workflow_event::append_event(run_store, &run_id, &event).await?;
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
    for event in [
        workflow_event::Event::RunStartRequested {
            resume: true,
            actor:  None,
        },
        workflow_event::Event::RunRunnable {
            source: RunRunnableSource::StartRequested,
            actor:  None,
        },
    ] {
        workflow_event::append_event(run_store, &run_id, &event).await?;
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

/// The failed status, its message, and the `run.failed` event for it.
fn failed(
    reason: FailureReason,
    message: String,
    timing: RunTiming,
) -> (RunStatus, Option<String>, workflow_event::Event) {
    let error = match reason {
        FailureReason::Cancelled => WorkflowError::Cancelled,
        _ => WorkflowError::engine(message.clone()),
    };
    (
        RunStatus::Failed { reason },
        Some(message),
        workflow_event::Event::workflow_run_failed_from_error(
            &error, timing, reason, None, None, None, None,
        ),
    )
}

/// Record a failure that happened before Petri ran, then finish the run.
async fn fail_before_execution(
    state: &Arc<AppState>,
    run_store: &fabro_store::RunDatabase,
    run_id: RunId,
    message: &str,
) {
    error!(run_id = %run_id, error = message, "Petri run cannot start");
    let (status, error, event) = failed(
        FailureReason::WorkflowError,
        message.to_string(),
        RunTiming::default(),
    );
    if let Err(err) = workflow_event::append_event(run_store, &run_id, &event).await {
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
