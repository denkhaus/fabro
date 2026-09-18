//! Runs on Petri: what the server does at create time and at execution when
//! a run's engine is Petri.
//!
//! At create, [`admit`] hands the workflow version's bundle, the run's inputs
//! and the launch to Petri's `Runtime::check` through `fabro_petri::check`,
//! maps Petri's diagnostics onto Fabro's, and stores the admitted graphs in
//! the blob store so the run executes and resumes from what was admitted.
//! Petri compiled, linted and pinned models; the legacy compile is skipped.
//!
//! At execution, [`execute`] runs the admitted graph through
//! `fabro_petri::engine` in the server process, over the run store in the
//! server's database, until the worker's HTTP run store lands. Only the run
//! lifecycle events Fabro's read side needs are appended (`run.starting`,
//! `run.running`, then `run.completed` or `run.failed`); no stage or agent
//! event is projected, which is the read-side item that follows.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use fabro_config::SettingsLayer;
use fabro_llm::selection;
use fabro_petri::check::{self, Bundle, CheckError, CheckRequest, Diagnostic, Launch};
use fabro_petri::engine::{self, RunOutcome, RunRequest};
use fabro_petri::runtime::{self, RuntimeSpec};
use fabro_petri::{SqliteRunStore, admission};
use fabro_types::settings::run::RunMode;
use fabro_types::{
    Engine, PetriAdmission, RunId, RunTarget, RunTiming, ServerSettings, StageOutcome,
};
use fabro_util::error as error_util;
use fabro_validate::{Diagnostic as FabroDiagnostic, Severity};
use fabro_workflow::Error as WorkflowError;
use fabro_workflow::run_status::{FailureReason, RunStatus, SuccessReason};
use lithos_llm::catalog::ProviderId;
use tokio::task;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use super::{AppState, clear_live_run_state, workflow_event};
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
        fabro_home: None,
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

/// Execute a Petri run in the server process: runnable → starting → running
/// → succeeded or failed, with the lifecycle events Fabro's read side needs.
pub(crate) async fn execute(state: Arc<AppState>, run_id: RunId) {
    let (run_dir, cancel) = {
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
        (run_dir, cancel)
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
    {
        let mut runs = state.runs.lock().expect("runs lock poisoned");
        if let Some(managed_run) = runs.get_mut(&run_id) {
            if managed_run.status == RunStatus::Starting {
                managed_run.status = RunStatus::Running;
            }
        }
    }
    let (_, eligible) = state.resolve_llm_client_with_ready_ids().await;
    let dry_run = run_state.spec.settings.run.execution.mode == RunMode::DryRun;
    let request = RunRequest {
        run_id: run_id.to_string(),
        run_dir: run_dir.join("petri"),
        admission,
        blobs: state.store_ref().blobs(),
        store: Arc::new(SqliteRunStore::new(state.db_pool.clone())),
        runtime: runtime_spec(&state, &eligible, dry_run),
        provider: run_state.spec.settings.run.environment.provider.clone(),
        cancel,
    };
    let outcome = Box::pin(engine::run(request)).await;
    let timing = RunTiming {
        wall_time_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        ..RunTiming::default()
    };
    let (status, error, event) = match outcome {
        Ok(RunOutcome {
            status: engine::RunStatus::Success,
            complete: true,
            ..
        }) => {
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
        Ok(outcome) => {
            let reason = match outcome.status {
                engine::RunStatus::Cancelled => FailureReason::Cancelled,
                engine::RunStatus::Success | engine::RunStatus::Failed => {
                    FailureReason::WorkflowError
                }
            };
            let message = failure_message(&outcome);
            info!(run_id = %run_id, error = %message, "Petri run did not succeed");
            failed(reason, message, timing)
        }
        Err(err) => {
            let message = error_util::collect_chain(&err).join(": ");
            error!(run_id = %run_id, error = %message, "Petri run failed");
            failed(FailureReason::WorkflowError, message, timing)
        }
    };
    if let Err(err) = workflow_event::append_event(&run_store, &run_id, &event).await {
        error!(run_id = %run_id, error = %err, "Failed to persist run outcome");
    }
    finish(&state, run_id, status, error);
}

/// The failure of a run whose record says it did not succeed.
fn failure_message(outcome: &RunOutcome) -> String {
    let mut message = match (&outcome.status, &outcome.failure) {
        (engine::RunStatus::Cancelled, _) => "the run was cancelled".to_string(),
        (_, Some(failure)) => failure.clone(),
        (engine::RunStatus::Failed, None) => "the run failed".to_string(),
        (engine::RunStatus::Success, None) => "the run's record is incomplete".to_string(),
    };
    if !outcome.complete {
        message.push_str(" (record incomplete: ");
        message.push_str(&outcome.incomplete.join("; "));
        message.push(')');
    }
    message
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
