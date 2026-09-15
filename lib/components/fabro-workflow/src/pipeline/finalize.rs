use std::sync::Arc;

use fabro_hooks::{HookContext, HookEvent};
use fabro_types::{DiffSummary, EventBody, RunFailure, RunProjection};
use lithos_llm::types::Usage;

use super::fork_terminal_taxonomy::{
    apply_boundary_upgrade, apply_soft_exit_downgrade, publish_failure_from_error,
    soft_stop_failure_reason,
};
use super::types::{Concluded, Executed, FinalizeOptions, Finalized, PublishOutcome, Published};
use crate::context::keys;
use crate::error::{Error, run_failure_from_error, run_failure_from_outcome_failure};
use crate::event::{Event, RunNoticeCode, RunNoticeLevel};
use crate::outcome::{FailureDetail, Outcome, StageOutcome};
use crate::records::Conclusion;
use crate::run_options::RunOptions;
use crate::run_status::{FailureReason, RunStatus, SuccessReason};
use crate::runtime_store::RunStoreHandle;
use crate::sandbox_git::{git_diff_with_timeout, list_diff_numstat, summarize_diff_numstat};
use crate::services::RunServices;
use crate::usage_rollup;

pub fn classify_engine_result(
    engine_result: &Result<Outcome, Error>,
) -> (StageOutcome, Option<RunFailure>, RunStatus) {
    match engine_result {
        Ok(outcome) => {
            let status = outcome.status;
            let failure_reason = outcome
                .failure
                .as_ref()
                .map_or(FailureReason::WorkflowError, soft_stop_failure_reason);
            let failure = outcome
                .failure
                .as_ref()
                .map(|failure| run_failure_from_outcome_failure(failure, failure_reason));
            let run_status = match status {
                StageOutcome::Succeeded | StageOutcome::Skipped => RunStatus::Succeeded {
                    reason: SuccessReason::Completed,
                },
                StageOutcome::PartiallySucceeded => RunStatus::Succeeded {
                    reason: SuccessReason::PartialSuccess,
                },
                StageOutcome::Failed { .. } => RunStatus::Failed {
                    reason: failure_reason,
                },
            };
            (status, failure, run_status)
        }
        Err(err) => {
            let reason = err.failure_reason();
            (
                StageOutcome::Failed {
                    retry_requested: false,
                },
                Some(run_failure_from_error(err, reason)),
                RunStatus::Failed { reason },
            )
        }
    }
}

pub(crate) async fn build_conclusion_from_store(
    run_store: &RunStoreHandle,
    status: StageOutcome,
    failure: Option<RunFailure>,
    run_wall_time_ms: u64,
    final_git_commit_sha: Option<String>,
) -> Conclusion {
    let projection = run_store.state().await.ok();
    build_conclusion_from_projection(
        projection.as_ref(),
        status,
        failure,
        run_wall_time_ms,
        final_git_commit_sha,
    )
}

fn build_conclusion_from_projection(
    projection: Option<&RunProjection>,
    status: StageOutcome,
    failure: Option<RunFailure>,
    run_wall_time_ms: u64,
    final_git_commit_sha: Option<String>,
) -> Conclusion {
    let rollup = projection
        .map(usage_rollup::usage_rollup_from_projection)
        .unwrap_or_default();
    let (stages, total_retries) = projection
        .map(|projection| rollup.conclusion_stages(projection))
        .unwrap_or_default();
    Conclusion {
        timestamp: chrono::Utc::now(),
        status,
        timing: rollup.timing.with_wall_time(run_wall_time_ms),
        failure,
        final_git_commit_sha,
        stages,
        usage: rollup.usage_if_present(),
        total_retries,
        diff: fabro_types::RunDiff::default(),
        exit_kind: String::new(),
    }
}

/// Failed and cancelled runs use a shorter diff timeout so a corrupted
/// workspace cannot stall consumers waiting on the terminal event.
async fn compute_final_patch(
    run_options: &RunOptions,
    services: &RunServices,
    status: StageOutcome,
) -> (Option<String>, Option<DiffSummary>) {
    let Some(base_sha) = run_options.git.as_ref().and_then(|g| g.base_sha.clone()) else {
        return (None, None);
    };
    let timeout_ms = match status {
        StageOutcome::Succeeded | StageOutcome::PartiallySucceeded => 30_000,
        _ => 10_000,
    };
    let to_sha = "HEAD";
    let (patch_result, numstat_result) = tokio::join!(
        git_diff_with_timeout(&services.sandbox, &base_sha, timeout_ms),
        list_diff_numstat(&services.sandbox, &base_sha, to_sha),
    );
    let final_patch = match patch_result {
        Ok(patch) if !patch.is_empty() => Some(patch),
        Ok(_) => None,
        Err(err) => {
            services.emitter.notice(
                RunNoticeLevel::Warn,
                RunNoticeCode::GitDiffFailed,
                format!("final diff failed: {err}"),
            );
            None
        }
    };
    let diff_summary = match numstat_result {
        Ok(numstat) => Some(summarize_diff_numstat(&numstat)),
        Err(err) => {
            services.emitter.notice(
                RunNoticeLevel::Warn,
                RunNoticeCode::GitDiffFailed,
                format!("final diff stats failed: {err}"),
            );
            None
        }
    };
    (final_patch, diff_summary)
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn usage_from_projection(projection: &RunProjection) -> Option<Usage> {
    usage_rollup::usage_rollup_from_projection(projection).usage_if_present()
}

pub(crate) fn build_terminal_event(
    outcome: &Result<Outcome, Error>,
    timing: fabro_types::RunTiming,
    artifact_count: usize,
    final_git_commit_sha: Option<String>,
    final_patch: Option<String>,
    diff_summary: Option<DiffSummary>,
    usage: Option<Usage>,
    exit_kind: Option<&str>,
    publish_failure: Option<RunFailure>,
    boundary_failure: Option<RunFailure>,
) -> Event {
    let outcome_status = outcome.as_ref().map_or(
        StageOutcome::Failed {
            retry_requested: false,
        },
        |o| o.status,
    );

    if outcome_status == StageOutcome::Succeeded
        || outcome_status == StageOutcome::PartiallySucceeded
    {
        return Event::WorkflowRunCompleted {
            timing,
            artifact_count,
            status: outcome_status.to_string(),
            reason: match (exit_kind, outcome_status, publish_failure.as_ref()) {
                // A declared park point outranks everything: work is
                // preserved and the loop re-enters next run (fabro-08b4).
                (Some("boundary"), _, _) => SuccessReason::Boundary,
                // Publish-blocked outranks the partial-success marker: the
                // actionable state is "work done, delivery blocked", and the
                // execution detail stays visible through the status string.
                (_, _, Some(_)) => SuccessReason::PublishBlocked,
                (_, StageOutcome::PartiallySucceeded, None) => SuccessReason::PartialSuccess,
                _ => SuccessReason::Completed,
            },
            failure: boundary_failure.or(publish_failure),
            final_git_commit_sha,
            final_patch,
            diff_summary,
            usage,
        };
    }

    // Exit-kind classification (fabro-b907): a soft-stop exit attribute
    // reclassifies the failure so notifications/UI can distinguish
    // deadlock-for-human and re-enterable soft stops from hard errors.
    // Only applies when the graph routed through a kind-bearing exit; the
    // ordinary error path is untouched.
    let exit_reason = match exit_kind {
        Some("deadlock") => Some(FailureReason::Deadlock),
        Some("soft") => Some(FailureReason::SoftStop),
        _ => None,
    };
    let failure = match outcome {
        Err(err) => {
            if let Some(reason) = exit_reason {
                run_failure_from_error(err, reason)
            } else {
                run_failure_from_error(err, err.failure_reason())
            }
        }
        Ok(outcome) => {
            let fallback_reason = exit_reason.unwrap_or_else(|| {
                outcome
                    .failure
                    .as_ref()
                    .map_or(FailureReason::WorkflowError, soft_stop_failure_reason)
            });
            if let Some(failure) = outcome.failure.as_ref() {
                run_failure_from_outcome_failure(failure, fallback_reason)
            } else {
                let fallback = Error::engine("run failed");
                run_failure_from_error(&fallback, fallback_reason)
            }
        }
    };
    Event::WorkflowRunFailed {
        failure,
        timing,
        final_git_commit_sha,
        final_patch,
        diff_summary,
        usage,
    }
}

async fn stop_sandbox_on_terminal(
    services: &RunServices,
    run_id: &fabro_types::RunId,
    workflow_name: &str,
    stop_on_terminal: bool,
) -> fabro_sandbox::Result<()> {
    let hook_ctx = HookContext::new(
        HookEvent::SandboxCleanup,
        *run_id,
        workflow_name.to_string(),
    );
    let _ = services.run_hooks(&hook_ctx).await;
    if stop_on_terminal {
        services.sandbox.stop().await?;
    }
    Ok(())
}

/// CONCLUDE phase: collect the execution result, final commit, and diff.
///
/// # Errors
///
/// Returns `Error` if the run state needed to build the conclusion cannot be
/// collected.
pub async fn conclude(executed: Executed, options: &FinalizeOptions) -> Result<Concluded, Error> {
    let Executed {
        graph,
        outcome,
        run_options,
        wall_time_ms,
        final_context,
        engine,
        model: _,
    } = executed;
    let services = Arc::clone(&engine.run);

    let (final_status, failure_reason, _run_status) = classify_engine_result(&outcome);

    let events = services.run_store.list_events().await.unwrap_or_default();
    let artifact_count = events
        .iter()
        .filter(|envelope| matches!(envelope.event.body, EventBody::ArtifactCaptured(_)))
        .count();
    let projection = services.run_store.state().await.ok();
    let mut conclusion = build_conclusion_from_projection(
        projection.as_ref(),
        final_status,
        failure_reason,
        wall_time_ms,
        options.last_git_sha.clone(),
    );

    // Exit-kind (fabro-b907): carried in the conclusion for the terminal
    // event; empty string = natural exit (no kind attribute).
    conclusion.exit_kind = final_context
        .get(keys::INTERNAL_EXIT_KIND)
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();

    let (final_patch, diff_summary) =
        compute_final_patch(&run_options, &services, final_status).await;
    conclusion.diff = fabro_types::RunDiff {
        patch:   final_patch,
        summary: diff_summary,
    };

    Ok(Concluded {
        outcome,
        conclusion,
        artifact_count,
        graph,
        run_options,
        services,
    })
}

/// FINALIZE phase: persist the final conclusion, emit the terminal event, and
/// clean up the sandbox.
///
/// This runs after PUBLISH so a required push or pull-request failure becomes
/// the terminal run result.
///
/// # Errors
///
/// Returns `Error` if persisting terminal state fails.
pub async fn finalize(published: Published, options: &FinalizeOptions) -> Result<Finalized, Error> {
    let Published {
        execution_outcome,
        publish_outcome,
        publish_error,
        mut conclusion,
        artifact_count,
        run_options,
        services,
    } = published;

    let PublishOutcome {
        pushed_branch,
        pr_url,
    } = publish_outcome;
    // An execution failure outranks a publish failure: publish only runs
    // after a successful execution, so the two are never both set. A publish
    // failure on a green execution no longer fails the run (fabro-67e5): the
    // work is done and checkpointed, only the outward delivery is blocked.
    // The outcome stays successful and the publish failure travels as a
    // two-part terminal state — status `Succeeded` with a `PublishBlocked`
    // reason plus a `PublishFailed` failure detail for remediation.
    let (outcome, publish_failure) = match (execution_outcome, publish_error) {
        (Err(error), _) => (Err(error), None),
        (Ok(outcome), Some(error)) => (
            Ok(outcome),
            Some(publish_failure_from_error(&error, pushed_branch.as_deref())),
        ),
        (Ok(outcome), None) => (Ok(outcome), None),
    };

    // Boundary upgrade (fabro-08b4): a graph-declared park point
    // (exit edge kind="boundary") turns a stage-failure terminal into a
    // success-shaped state — work is preserved, the goal is not terminal,
    // the next run re-enters autonomously.
    let (outcome, boundary_failure) =
        apply_boundary_upgrade(outcome, conclusion.exit_kind.as_str());

    // Soft-exit downgrade (fabro-18a5): a green exit stage reached through
    // a kind="deadlock"/"soft" edge is a failed terminal, not a success.
    let (outcome, soft_exit_failure) =
        apply_soft_exit_downgrade(outcome, conclusion.exit_kind.as_str());

    let (final_status, failure, _run_status) = classify_engine_result(&outcome);
    conclusion.status = final_status;
    // A soft-exit, publish, or boundary failure is the actionable detail; an
    // execution-level failure detail (partial success) stays visible
    // through the stage summaries and the outcome status string.
    conclusion.failure = soft_exit_failure
        .clone()
        .or(boundary_failure.clone())
        .or(publish_failure.clone())
        .or(failure);

    let exit_kind = (!conclusion.exit_kind.is_empty()).then_some(conclusion.exit_kind.as_str());
    let terminal_event = build_terminal_event(
        &outcome,
        conclusion.timing,
        artifact_count,
        conclusion.final_git_commit_sha.clone(),
        conclusion.diff.patch.clone(),
        conclusion.diff.summary,
        conclusion.usage,
        exit_kind,
        publish_failure.clone(),
        boundary_failure.clone(),
    );
    services.emitter.emit(&terminal_event);

    if options.preserve_sandbox {
        let info = services.sandbox.sandbox_info();
        let message = if info.is_empty() {
            "sandbox preserved".to_string()
        } else {
            format!("sandbox preserved: {info}")
        };
        services.emitter.notice(
            RunNoticeLevel::Info,
            RunNoticeCode::SandboxPreserved,
            message,
        );
    }
    if let Err(e) = stop_sandbox_on_terminal(
        &services,
        &options.run_id,
        &options.workflow_name,
        options.stop_on_terminal,
    )
    .await
    {
        tracing::warn!(error = %fabro_sandbox::display_for_log(&e), "Sandbox stop failed");
        let exec_output_tail = fabro_sandbox::default_redacted_output_tail(&e);
        services.emitter.notice_with_tail(
            RunNoticeLevel::Warn,
            RunNoticeCode::SandboxCleanupFailed,
            format!("sandbox stop failed: {}", e.display_with_causes()),
            exec_output_tail,
        );
    }

    Ok(Finalized {
        run_id: run_options.run_id,
        outcome,
        conclusion,
        pushed_branch,
        pr_url,
        publish_failure,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use anyhow::Result;
    use fabro_auth::test_support as auth_test_support;
    use fabro_graphviz::graph::Graph;
    use fabro_sandbox::test_support::MockSandbox;
    use fabro_store::{Database, RunDatabase, RunProjection};
    use fabro_types::{
        EventBody, RunEvent, RunId, RunSpec, StageCompletion, WorkflowSettings, first_event_seq,
        fixtures, test_support,
    };
    use object_store::memory::InMemory;

    use super::*;
    use crate::context::Context;
    use crate::error::ErrorStage;
    use crate::event::{Emitter, StoreProgressLogger, append_event};
    use crate::records::Checkpoint;
    use crate::run_options::{GitCheckpointOptions, RunOptions};
    use crate::runtime_store::RunStoreHandle;
    use crate::sandbox_git_runtime::SandboxGitRuntime;
    use crate::services::EngineServices;

    fn test_run_id() -> RunId {
        fixtures::RUN_1
    }

    fn test_run_options(run_dir: &std::path::Path) -> RunOptions {
        RunOptions {
            settings:         WorkflowSettings::default(),
            run_dir:          run_dir.to_path_buf(),
            cancel_token:     tokio_util::sync::CancellationToken::new(),
            run_id:           test_run_id(),
            labels:           HashMap::new(),
            workflow_slug:    None,
            github_app:       None,
            pre_run_git:      None,
            fork_source_ref:  None,
            base_branch:      None,
            display_base_sha: None,
            git_identity:     None,
            git:              None,
        }
    }

    fn test_executed(
        graph: Graph,
        outcome: Result<Outcome, Error>,
        run_options: RunOptions,
        wall_time_ms: u64,
        services: Arc<RunServices>,
    ) -> Executed {
        let mut engine = EngineServices::test_default();
        engine.run = services;
        Executed {
            graph,
            outcome,
            run_options,
            wall_time_ms,
            final_context: Context::new(),
            engine: Arc::new(engine),
            model: "test-model".to_string(),
        }
    }

    async fn finalize_executed(
        executed: Executed,
        options: &FinalizeOptions,
    ) -> Result<Finalized, Error> {
        let concluded = conclude(executed, options).await?;
        let published = crate::pipeline::publish(concluded, &crate::pipeline::PublishOptions {
            pr_config:           None,
            github_app:          None,
            origin_url:          None,
            pr_model:            "test-model".to_string(),
            pr_resolved_model:   "test-model".to_string(),
            pr_reasoning_effort: None,
        })
        .await;
        finalize(published, options).await
    }

    fn test_store() -> Arc<Database> {
        Arc::new(fabro_store::test_support::test_database(
            Arc::new(InMemory::new()),
            "",
            Duration::from_millis(1),
            None,
        ))
    }

    async fn seeded_run_store() -> RunDatabase {
        let run_store = test_store().create_run(&test_run_id()).await.unwrap();
        append_event(&run_store, &test_run_id(), &Event::RunCreated {
            run_id:              test_run_id(),
            title:               None,
            settings:            serde_json::to_value(WorkflowSettings::default()).unwrap(),
            graph:               serde_json::to_value(fabro_types::Graph::new("checkpoint"))
                .unwrap(),
            workflow_source:     None,
            labels:              std::collections::BTreeMap::new(),
            source_directory:    Some("/tmp/project".to_string()),
            workflow_slug:       Some("checkpoint".to_string()),
            workflow_version_id: None,
            target:              None,
            automation:          None,
            provenance:          test_support::test_run_provenance(),
            spec_blob:           None,
            git:                 None,
            fork_source_ref:     None,
            retried_from:        None,
            parent_id:           None,
            web_url:             None,
        })
        .await
        .unwrap();
        run_store
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "checkpoint tests use synchronous git commands to set up temporary repositories"
    )]
    fn init_git_repo(repo: &Path) {
        let init = std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(repo)
            .output()
            .unwrap();
        assert!(init.status.success());
        for (key, value) in [("user.name", "Test"), ("user.email", "test@test.com")] {
            let config = std::process::Command::new("git")
                .args(["config", key, value])
                .current_dir(repo)
                .output()
                .unwrap();
            assert!(config.status.success());
        }
        let commit = std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "initial"])
            .current_dir(repo)
            .output()
            .unwrap();
        assert!(commit.status.success());
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "checkpoint tests use synchronous git commands to set up temporary repositories"
    )]
    fn git_commit_all(repo: &Path, msg: &str) -> String {
        let add = std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(repo)
            .output()
            .unwrap();
        assert!(add.status.success());
        let commit = std::process::Command::new("git")
            .args(["commit", "-m", msg])
            .current_dir(repo)
            .output()
            .unwrap();
        assert!(
            commit.status.success(),
            "git commit failed: {}",
            String::from_utf8_lossy(&commit.stderr)
        );
        let rev_parse = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(repo)
            .output()
            .unwrap();
        assert!(rev_parse.status.success());
        String::from_utf8(rev_parse.stdout)
            .unwrap()
            .trim()
            .to_string()
    }

    fn record_events(emitter: &Arc<Emitter>) -> Arc<std::sync::Mutex<Vec<RunEvent>>> {
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        emitter.on_event(move |event| {
            captured.lock().unwrap().push(event.clone());
        });
        events
    }

    fn checkpoint_with(
        completed_nodes: Vec<&str>,
        node_outcomes: HashMap<String, Outcome>,
    ) -> Checkpoint {
        Checkpoint {
            timestamp: chrono::Utc::now(),
            current_node: completed_nodes
                .last()
                .copied()
                .unwrap_or("start")
                .to_string(),
            completed_nodes: completed_nodes.into_iter().map(str::to_string).collect(),
            node_retries: HashMap::new(),
            context_values: HashMap::new(),
            node_outcomes,
            next_node_id: None,
            git_commit_sha: None,
            loop_failure_signatures: HashMap::new(),
            restart_failure_signatures: HashMap::new(),
            node_visits: HashMap::new(),
        }
    }

    fn test_projection() -> RunProjection {
        RunProjection::new(
            "Test run".to_string(),
            RunSpec {
                run_id:              test_run_id(),
                settings:            WorkflowSettings::default(),
                graph:               Graph::new("test"),
                graph_source:        None,
                workflow_slug:       None,
                workflow_version_id: None,
                target:              None,
                automation:          None,
                source_directory:    None,
                labels:              HashMap::new(),
                provenance:          test_support::test_run_provenance(),
                definition_blob:     None,
                spec_blob:           None,
                git:                 None,
                fork_source_ref:     None,
            },
            chrono::Utc::now(),
        )
    }

    use crate::test_support::test_usage;

    /// fabro-67e5: publish-blocked outranks the partial-success marker in the
    /// reason, while the status string keeps the execution detail.
    #[test]
    fn partial_success_with_publish_failure_reports_publish_blocked() {
        let mut partial = Outcome::success();
        partial.status = StageOutcome::PartiallySucceeded;
        let publish_failure = publish_failure_from_error(
            &Error::publish("pull request creation failed"),
            Some("fabro/run/test"),
        );

        let event = build_terminal_event(
            &Ok(partial),
            fabro_types::RunTiming::wall_only(10),
            0,
            None,
            None,
            None,
            None,
            None,
            Some(publish_failure),
            None,
        );

        match event {
            Event::WorkflowRunCompleted {
                status,
                reason,
                failure,
                ..
            } => {
                assert_eq!(status, "partially_succeeded");
                assert_eq!(reason, SuccessReason::PublishBlocked);
                assert_eq!(
                    failure.as_ref().map(|failure| failure.reason),
                    Some(FailureReason::PublishFailed)
                );
            }
            other => panic!("expected run.completed, got {other:?}"),
        }
    }

    /// fabro-08b4: a boundary exit upgrades a terminal engine error into
    /// the success-shaped park state — the failure detail survives.
    #[test]
    fn boundary_upgrade_parks_engine_error_green() {
        let error = Error::engine("reviewer exhausted retries after 401s");
        let (outcome, boundary_failure) = apply_boundary_upgrade(Err(error.clone()), "boundary");

        let parked = outcome.expect("boundary upgrades the error to a green outcome");
        assert_eq!(parked.status, StageOutcome::Succeeded);
        assert!(
            parked.failure.is_some(),
            "the engine failure detail must survive the upgrade"
        );
        let failure = boundary_failure.expect("boundary failure detail");
        assert_eq!(
            failure.detail.message,
            "reviewer exhausted retries after 401s"
        );
    }

    /// Only the declared boundary kind upgrades; soft, deadlock, and natural
    /// exits keep the engine error terminal.
    #[test]
    fn boundary_upgrade_ignores_other_exit_kinds() {
        for kind in ["soft", "deadlock", "natural", ""] {
            let (outcome, boundary_failure) =
                apply_boundary_upgrade(Err(Error::engine("boom")), kind);
            assert!(outcome.is_err(), "kind={kind:?} must stay a failure");
            assert!(boundary_failure.is_none());
        }
        // A green outcome passes through untouched even on a boundary exit.
        let (outcome, boundary_failure) =
            apply_boundary_upgrade(Ok(Outcome::success()), "boundary");
        assert!(outcome.is_ok());
        assert!(boundary_failure.is_none());
    }

    /// fabro-18a5: a green exit stage routed through a soft exit is a
    /// failed terminal — deadlock parks for a human, soft re-enters next
    /// run — while natural and boundary exits keep green terminals green.
    #[test]
    fn soft_exit_downgrade_converts_green_terminal_to_failed() {
        for (exit_kind, expected) in [
            ("deadlock", FailureReason::Deadlock),
            ("soft", FailureReason::SoftStop),
        ] {
            let (outcome, soft_exit_failure) =
                apply_soft_exit_downgrade(Ok(Outcome::success()), exit_kind);
            let error = outcome.expect_err("a green terminal must downgrade to a failure");
            let failure = soft_exit_failure.expect("downgrade carries the failure detail");
            assert_eq!(failure.reason, expected, "kind={exit_kind}");
            assert!(
                failure.detail.message.contains("kind="),
                "the remediation must name the exit kind: {}",
                failure.detail.message
            );
            assert!(error.to_string().contains(exit_kind));
        }

        // Natural and boundary exits keep green terminals green, and an
        // already-failed terminal passes through untouched (the exit-kind
        // reclassification in build_terminal_event owns that path).
        for kind in ["natural", "boundary", ""] {
            let (outcome, soft_exit_failure) =
                apply_soft_exit_downgrade(Ok(Outcome::success()), kind);
            assert!(outcome.is_ok(), "kind={kind:?} must stay green");
            assert!(soft_exit_failure.is_none());
        }
        let (outcome, soft_exit_failure) =
            apply_soft_exit_downgrade(Err(Error::engine("boom")), "deadlock");
        assert!(outcome.is_err());
        assert!(soft_exit_failure.is_none());
    }

    /// fabro-a3d8: a long-window provider rate limit — the reset deadline
    /// hours away, carried only in the failure message prose — parks the run
    /// as a soft stop instead of a hard `workflow_error`, so the existing
    /// resume primitive (which rejects only succeeded runs) can re-enter it
    /// after the usage window reopens.
    #[test]
    fn long_window_rate_limit_failure_parks_as_soft_stop() {
        let future = chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now())
            + chrono::Duration::hours(5);
        // RFC3339 with offset: the trustworthy form (fabro-0607). The naive
        // form parks too, via the unknown-ETA classifier — covered below.
        let message = format!(
            "Usage limit reached for 5 hour. Your limit will reset at {}",
            future.to_rfc3339()
        );
        let mut outcome = Outcome::success();
        outcome.status = StageOutcome::Failed {
            retry_requested: false,
        };
        outcome.failure = Some(FailureDetail::new(
            message,
            crate::error::FailureCategory::TransientInfra,
        ));

        let (status, failure, run_status) = classify_engine_result(&Ok(outcome));

        assert!(matches!(status, StageOutcome::Failed { .. }));
        let failure = failure.expect("failed outcome carries a failure");
        assert_eq!(failure.reason, FailureReason::SoftStop);
        assert!(matches!(run_status, RunStatus::Failed {
            reason: FailureReason::SoftStop,
        }));
    }

    /// The same shape without a long reset window keeps the hard-error
    /// mapping: short 429s are ordinary transient failures.
    /// fabro-0607: a reset timestamp without a UTC offset parks as a soft
    /// stop even when its naive-UTC reading lies in the PAST — the
    /// west-of-UTC sharp edge that previously disabled the park silently.
    #[test]
    fn naive_past_reset_prose_still_parks_as_soft_stop() {
        // Wallclock 2026-01-01 05:35:16 with NO offset: as naive-UTC this
        // is long past, which used to read as "window reopened".
        let message =
            "Usage limit reached for 5 hour. Your limit will reset at 2026-01-01 05:35:16";
        let mut outcome = Outcome::success();
        outcome.status = StageOutcome::Failed {
            retry_requested: false,
        };
        outcome.failure = Some(FailureDetail::new(
            message.to_string(),
            crate::error::FailureCategory::TransientInfra,
        ));
        let (status, failure, run_status) = classify_engine_result(&Ok(outcome));
        assert!(matches!(status, StageOutcome::Failed { .. }));
        let failure = failure.expect("failed outcome carries a failure");
        assert_eq!(failure.reason, FailureReason::SoftStop);
        assert!(matches!(run_status, RunStatus::Failed {
            reason: FailureReason::SoftStop,
        }));
    }

    #[test]
    fn short_window_rate_limit_failure_stays_a_hard_error() {
        let mut outcome = Outcome::success();
        outcome.status = StageOutcome::Failed {
            retry_requested: false,
        };
        outcome.failure = Some(FailureDetail::new(
            "rate limit exceeded, retry shortly",
            crate::error::FailureCategory::TransientInfra,
        ));

        let (_, failure, run_status) = classify_engine_result(&Ok(outcome));

        assert_eq!(
            failure.expect("failed outcome carries a failure").reason,
            FailureReason::WorkflowError
        );
        assert!(matches!(run_status, RunStatus::Failed {
            reason: FailureReason::WorkflowError,
        }));
    }

    /// fabro-18a5 end to end: a green exit stage reached through a
    /// kind="deadlock" edge must terminate failed(Deadlock) with no publish
    /// side effects — the live incident opened a pull request for code whose
    /// gates never passed and left the run unresumable.
    #[tokio::test]
    async fn deadlock_exit_with_green_outcome_fails_instead_of_publishing() {
        let repo_dir = tempfile::tempdir().unwrap();
        let emitter = Arc::new(Emitter::new(test_run_id()));
        let events = record_events(&emitter);
        let services = test_services(
            RunStoreHandle::local(seeded_run_store().await),
            emitter,
            MockSandbox::linux().sandbox(),
        );
        let mut run_options = test_run_options(repo_dir.path());
        run_options.git = Some(GitCheckpointOptions {
            base_sha:   None,
            run_branch: Some("fabro/run/deadlock".to_string()),
        });

        // The gate guard fired: the last stage succeeded (the red verdict is
        // stage output, not a stage failure) and the graph routed to the
        // exit node through a kind="deadlock" edge.
        let final_context = Context::new();
        final_context.set(keys::INTERNAL_EXIT_KIND, serde_json::json!("deadlock"));
        let mut engine = EngineServices::test_default();
        engine.run = services;
        let executed = Executed {
            graph: Graph::new("test"),
            outcome: Ok(Outcome::success()),
            run_options,
            wall_time_ms: 5,
            final_context,
            engine: Arc::new(engine),
            model: "test-model".to_string(),
        };

        let options = FinalizeOptions {
            run_dir:          repo_dir.path().to_path_buf(),
            run_id:           test_run_id(),
            workflow_name:    "test".to_string(),
            preserve_sandbox: false,
            stop_on_terminal: true,
            last_git_sha:     Some("final-sha".to_string()),
        };
        let concluded = conclude(executed, &options).await.unwrap();
        let published = crate::pipeline::publish(concluded, &crate::pipeline::PublishOptions {
            pr_config:           None,
            github_app:          None,
            origin_url:          None,
            pr_model:            "test-model".to_string(),
            pr_resolved_model:   "test-model".to_string(),
            pr_reasoning_effort: None,
        })
        .await;

        assert_eq!(
            published.publish_outcome,
            PublishOutcome::default(),
            "publish must not run for a soft-exit terminal"
        );
        assert!(published.publish_error.is_none());

        let finalized = finalize(published, &options).await.unwrap();
        assert!(
            finalized.outcome.is_err(),
            "the deadlock exit must downgrade the green terminal"
        );
        assert_eq!(finalized.pushed_branch, None);
        assert_eq!(
            finalized.conclusion.failure.as_ref().map(|f| f.reason),
            Some(FailureReason::Deadlock)
        );

        let events = events.lock().unwrap();
        let names = events.iter().map(RunEvent::event_name).collect::<Vec<_>>();
        assert_eq!(
            names,
            vec!["run.failed"],
            "the terminal event must be run.failed, not run.completed"
        );
    }

    /// The terminal event on a boundary exit reads
    /// succeeded(boundary) with the parked failure attached.
    #[test]
    fn boundary_exit_kind_reports_boundary_reason() {
        // Post-upgrade shape: finalize() already parked the engine error
        // into a green outcome with the failure detail attached.
        let mut parked = Outcome::success();
        parked.failure = Some(fabro_types::FailureDetail::new(
            "reviewer gave up mid-run",
            fabro_types::FailureCategory::Deterministic,
        ));
        let parked_failure = RunFailure {
            reason: FailureReason::WorkflowError,
            detail: fabro_types::FailureDetail::new(
                "reviewer gave up mid-run",
                fabro_types::FailureCategory::Deterministic,
            ),
        };
        let event = build_terminal_event(
            &Ok(parked),
            fabro_types::RunTiming::wall_only(10),
            0,
            None,
            None,
            None,
            None,
            Some("boundary"),
            None,
            Some(parked_failure),
        );
        match event {
            Event::WorkflowRunCompleted {
                reason, failure, ..
            } => {
                assert_eq!(reason, SuccessReason::Boundary);
                assert!(failure.is_some_and(|failure| failure.detail.message.contains("reviewer")));
            }
            other => panic!("expected run.completed, got {other:?}"),
        }
    }

    #[test]
    fn publish_error_builds_publish_failed_terminal_event() {
        let event = build_terminal_event(
            &Err(Error::publish("GitHub rejected pull request creation")),
            fabro_types::RunTiming::wall_only(10),
            0,
            Some("final-sha".to_string()),
            Some("diff".to_string()),
            None,
            None,
            None,
            None,
            None,
        );

        match event {
            Event::WorkflowRunFailed { failure, .. } => {
                assert_eq!(failure.reason, FailureReason::PublishFailed);
            }
            other => panic!("expected run failure, got {other:?}"),
        }
    }

    #[test]
    fn deadlock_exit_kind_reclassifies_failure_reason() {
        // fabro-b907: a soft-stop exit attribute maps to FailureReason::Deadlock
        // so notifications distinguish needs-a-human from hard errors.
        let failed = Outcome::fail("review deadlock: 3 cycles");
        let event = build_terminal_event(
            &Ok(failed),
            fabro_types::RunTiming::wall_only(10),
            0,
            None,
            None,
            None,
            None,
            Some("deadlock"),
            None,
            None,
        );
        match event {
            Event::WorkflowRunFailed { failure, .. } => {
                assert_eq!(failure.reason, FailureReason::Deadlock);
            }
            other => panic!("expected run failure, got {other:?}"),
        }
    }

    #[test]
    fn soft_exit_kind_reclassifies_failure_reason() {
        let failed = Outcome::fail("evidence capture failed twice");
        let event = build_terminal_event(
            &Ok(failed),
            fabro_types::RunTiming::wall_only(10),
            0,
            None,
            None,
            None,
            None,
            Some("soft"),
            None,
            None,
        );
        match event {
            Event::WorkflowRunFailed { failure, .. } => {
                assert_eq!(failure.reason, FailureReason::SoftStop);
            }
            other => panic!("expected run failure, got {other:?}"),
        }
    }

    #[test]
    fn natural_exit_kind_keeps_error_reason() {
        let failed = Outcome::fail("planner errored");
        let event = build_terminal_event(
            &Ok(failed),
            fabro_types::RunTiming::wall_only(10),
            0,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        match event {
            Event::WorkflowRunFailed { failure, .. } => {
                assert_eq!(failure.reason, FailureReason::WorkflowError);
            }
            other => panic!("expected run failure, got {other:?}"),
        }
    }

    #[test]
    fn conclusion_stage_order_follows_projection_first_event_order() {
        let mut projection = test_projection();
        projection.stage_entry("zebra", 1, first_event_seq(1));
        projection.stage_entry("apple", 1, first_event_seq(2));
        let checkpoint = checkpoint_with(
            vec!["apple", "zebra"],
            HashMap::from([
                ("apple".to_string(), Outcome::success()),
                ("zebra".to_string(), Outcome::success()),
            ]),
        );

        projection.checkpoints.push(fabro_types::CheckpointRecord {
            seq: 10,
            checkpoint,
            diff: fabro_types::RunDiff::default(),
        });
        let conclusion = build_conclusion_from_projection(
            Some(&projection),
            StageOutcome::Succeeded,
            None,
            10,
            None,
        );

        let stage_ids = conclusion
            .stages
            .iter()
            .map(|stage| stage.stage_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(stage_ids, vec!["zebra", "apple"]);
    }

    #[test]
    fn conclusion_includes_skipped_stage_from_projection_checkpoint_fallback() {
        let mut projection = test_projection();
        projection.stage_entry("skipped", 1, first_event_seq(4));
        projection.stage_entry("finished", 1, first_event_seq(5));
        let checkpoint = checkpoint_with(
            vec!["finished"],
            HashMap::from([
                ("finished".to_string(), Outcome::success()),
                (
                    "skipped".to_string(),
                    Outcome::skipped("condition was false"),
                ),
            ]),
        );

        projection.checkpoints.push(fabro_types::CheckpointRecord {
            seq: 10,
            checkpoint,
            diff: fabro_types::RunDiff::default(),
        });
        let conclusion = build_conclusion_from_projection(
            Some(&projection),
            StageOutcome::Succeeded,
            None,
            10,
            None,
        );

        let stage_ids = conclusion
            .stages
            .iter()
            .map(|stage| stage.stage_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(stage_ids, vec!["skipped", "finished"]);
    }

    #[test]
    fn conclusion_usage_sums_retry_visit_usage_from_projection() {
        let mut projection = test_projection();
        let failed_usage = test_usage("gpt-old", 100, 10);
        let success_usage = test_usage("gpt-new", 200, 20);
        let failed = projection.stage_entry("verify", 1, first_event_seq(1));
        failed.timing = Some(fabro_types::StageTiming::wall_only(1200));
        failed.usage = failed_usage.usage;
        failed.model = Some(failed_usage.model().clone());
        failed.completion = Some(StageCompletion {
            outcome:        StageOutcome::Failed {
                retry_requested: true,
            },
            notes:          None,
            failure_reason: Some("try again".to_string()),
            timestamp:      chrono::Utc::now(),
        });
        let succeeded = projection.stage_entry("verify", 2, first_event_seq(2));
        succeeded.timing = Some(fabro_types::StageTiming::wall_only(800));
        succeeded.usage = success_usage.usage;
        succeeded.model = Some(success_usage.model().clone());
        succeeded.completion = Some(StageCompletion {
            outcome:        StageOutcome::Succeeded,
            notes:          None,
            failure_reason: None,
            timestamp:      chrono::Utc::now(),
        });

        let mut latest_outcome = Outcome::success();
        latest_outcome.usage = Some(success_usage);
        latest_outcome.timing = Some(fabro_types::StageTiming::wall_only(800));
        let mut checkpoint = checkpoint_with(
            vec!["verify", "verify"],
            HashMap::from([("verify".to_string(), latest_outcome)]),
        );
        checkpoint.node_retries.insert("verify".to_string(), 2);

        projection.checkpoints.push(fabro_types::CheckpointRecord {
            seq: 10,
            checkpoint,
            diff: fabro_types::RunDiff::default(),
        });
        let conclusion = build_conclusion_from_projection(
            Some(&projection),
            StageOutcome::Succeeded,
            None,
            10,
            None,
        );

        let usage = conclusion.usage.unwrap();
        assert_eq!(usage.tokens.input, 300);
        assert_eq!(usage.tokens.output, 30);
        assert_eq!(usage.cost.map(|cost| cost.usd_micros), Some(330));
        assert_eq!(conclusion.stages.len(), 1);
        assert_eq!(conclusion.stages[0].stage_id, "verify");
        assert_eq!(conclusion.stages[0].timing.wall_time_ms, 2000);
        assert_eq!(
            conclusion.stages[0].usage.cost.map(|cost| cost.usd_micros),
            Some(330)
        );
        assert_eq!(conclusion.stages[0].retries, 1);
    }

    fn test_services(
        run_store: RunStoreHandle,
        emitter: Arc<Emitter>,
        sandbox: Arc<fabro_sandbox::RunSandbox>,
    ) -> Arc<RunServices> {
        let locations = crate::services::RunLocations::for_sandbox(
            None,
            sandbox.as_ref(),
            Path::new(".").to_path_buf(),
        );
        RunServices::new(
            run_store,
            emitter,
            sandbox,
            None,
            locations,
            tokio_util::sync::CancellationToken::new(),
            lithos_llm::catalog::builtin::anthropic(),
            "claude-sonnet-4-6".to_string(),
            auth_test_support::vault_only_credential_source(),
            Arc::new(fabro_llm::test_support::test_catalog()),
            Arc::new(SandboxGitRuntime::new()),
            crate::stage_execution::StageExecutionTracker::default(),
        )
    }

    #[tokio::test]
    async fn finalize_persists_conclusion_in_projection() {
        let temp = tempfile::tempdir().unwrap();
        let run_dir = temp.path().join("run");
        std::fs::create_dir_all(&run_dir).unwrap();
        let run_store = seeded_run_store().await;
        crate::test_support::mark_run_running(&run_store, &test_run_id()).await;
        let emitter = Arc::new(Emitter::new(test_run_id()));
        let store_logger = StoreProgressLogger::new(run_store.clone());
        store_logger.register(&emitter);
        let sandbox: Arc<fabro_sandbox::RunSandbox> = Arc::new(
            fabro_sandbox::local_sandbox(std::env::current_dir().unwrap())
                .await
                .unwrap(),
        );
        let locations =
            crate::services::RunLocations::for_sandbox(None, sandbox.as_ref(), run_dir.clone());
        let services = RunServices::new(
            run_store.clone().into(),
            Arc::clone(&emitter),
            sandbox,
            None,
            locations,
            tokio_util::sync::CancellationToken::new(),
            lithos_llm::catalog::builtin::anthropic(),
            "claude-sonnet-4-6".to_string(),
            auth_test_support::vault_only_credential_source(),
            Arc::new(fabro_llm::test_support::test_catalog()),
            Arc::new(SandboxGitRuntime::new()),
            crate::stage_execution::StageExecutionTracker::default(),
        );
        let executed = test_executed(
            Graph::new("test"),
            Ok(Outcome::success()),
            test_run_options(&run_dir),
            5,
            services,
        );

        let concluded = finalize_executed(executed, &FinalizeOptions {
            run_dir:          run_dir.clone(),
            run_id:           test_run_id(),
            workflow_name:    "test".to_string(),
            preserve_sandbox: true,
            stop_on_terminal: true,
            last_git_sha:     None,
        })
        .await
        .unwrap();
        store_logger.flush().await.unwrap();

        assert_eq!(concluded.conclusion.status, StageOutcome::Succeeded);
    }

    #[tokio::test]
    async fn configured_run_branch_without_remote_is_not_reported_as_pushed() {
        let repo_dir = tempfile::tempdir().unwrap();
        let emitter = Arc::new(Emitter::new(test_run_id()));
        let events = record_events(&emitter);
        let services = test_services(
            RunStoreHandle::local(seeded_run_store().await),
            emitter,
            MockSandbox::linux().sandbox(),
        );
        let mut run_options = test_run_options(repo_dir.path());
        run_options.git = Some(GitCheckpointOptions {
            base_sha:   None,
            run_branch: Some("fabro/run/test".to_string()),
        });
        let executed = test_executed(
            Graph::new("test"),
            Ok(Outcome::success()),
            run_options,
            5,
            services,
        );
        let options = FinalizeOptions {
            run_dir:          repo_dir.path().to_path_buf(),
            run_id:           test_run_id(),
            workflow_name:    "test".to_string(),
            preserve_sandbox: false,
            stop_on_terminal: true,
            last_git_sha:     Some("final-sha".to_string()),
        };
        let concluded = conclude(executed, &options).await.unwrap();
        let published = crate::pipeline::publish(concluded, &crate::pipeline::PublishOptions {
            pr_config:           None,
            github_app:          None,
            origin_url:          None,
            pr_model:            "test-model".to_string(),
            pr_resolved_model:   "test-model".to_string(),
            pr_reasoning_effort: None,
        })
        .await;

        assert_eq!(published.publish_outcome, PublishOutcome::default());
        assert!(published.publish_error.is_none());
        let finalized = finalize(published, &options).await.unwrap();

        assert!(finalized.outcome.is_ok());
        assert_eq!(finalized.pushed_branch, None);
        let events = events.lock().unwrap();
        let names = events.iter().map(RunEvent::event_name).collect::<Vec<_>>();
        assert_eq!(names, vec!["run.completed"]);
    }

    #[tokio::test]
    async fn final_push_failure_becomes_terminal_publish_failure() {
        let repo_dir = tempfile::tempdir().unwrap();
        // The sandbox is unreachable, so the final push cannot run.
        let sandbox = MockSandbox {
            exec_error: Some("sandbox unreachable".into()),
            ..MockSandbox::linux()
        }
        .sandbox();
        let emitter = Arc::new(Emitter::new(test_run_id()));
        let events = record_events(&emitter);
        let services = test_services(
            RunStoreHandle::local(seeded_run_store().await),
            emitter,
            sandbox,
        );
        let mut run_options = test_run_options(repo_dir.path());
        run_options.git = Some(GitCheckpointOptions {
            base_sha:   None,
            run_branch: Some("fabro/run/test".to_string()),
        });
        let executed = test_executed(
            Graph::new("test"),
            Ok(Outcome::success()),
            run_options,
            5,
            services,
        );
        let options = FinalizeOptions {
            run_dir:          repo_dir.path().to_path_buf(),
            run_id:           test_run_id(),
            workflow_name:    "test".to_string(),
            preserve_sandbox: false,
            stop_on_terminal: true,
            last_git_sha:     Some("final-sha".to_string()),
        };
        let concluded = conclude(executed, &options).await.unwrap();
        let published = crate::pipeline::publish(concluded, &crate::pipeline::PublishOptions {
            pr_config:           None,
            github_app:          None,
            origin_url:          Some("https://github.com/owner/repo.git".to_string()),
            pr_model:            "test-model".to_string(),
            pr_resolved_model:   "test-model".to_string(),
            pr_reasoning_effort: None,
        })
        .await;

        assert!(matches!(
            &published.publish_error,
            Some(Error::Stage {
                stage: ErrorStage::Publish,
                ..
            })
        ));
        let finalized = finalize(published, &options).await.unwrap();

        // fabro-67e5: a green run whose push failed no longer reads as a
        // plain failure — the execution outcome stays successful and the
        // publish failure travels as the two-part terminal state.
        assert!(finalized.outcome.is_ok());
        assert_eq!(finalized.conclusion.status, StageOutcome::Succeeded);
        assert_eq!(
            finalized.publish_failure.as_ref().map(|f| f.reason),
            Some(FailureReason::PublishFailed)
        );
        assert_eq!(
            finalized
                .conclusion
                .failure
                .as_ref()
                .map(|failure| failure.reason),
            Some(FailureReason::PublishFailed)
        );
        // The branch never reached the remote, so the remediation must say
        // the work lives in checkpoints, not on the pushed branch.
        assert!(
            finalized
                .publish_failure
                .expect("publish failure present")
                .detail
                .message
                .contains("NOT pushed"),
            "remediation should name the missing push"
        );
        let events = events.lock().unwrap();
        let names = events.iter().map(RunEvent::event_name).collect::<Vec<_>>();
        // Exactly one durable git.push event per high-level push — retries
        // nest inside it as attempts, never as extra events.
        assert_eq!(names, vec!["git.push", "run.completed"]);
        match &events.first().unwrap().body {
            EventBody::GitPush(props) => {
                assert!(!props.success);
                // MockSandbox's default git_push_ref fails before any attempt
                // runs, so the nested history is empty here.
                assert!(props.attempts.is_empty());
            }
            other => panic!("expected git.push, got {other:?}"),
        }
        match &events.last().unwrap().body {
            EventBody::RunCompleted(props) => {
                assert_eq!(props.reason, SuccessReason::PublishBlocked);
                assert_eq!(
                    props.failure.as_ref().map(|failure| failure.reason),
                    Some(FailureReason::PublishFailed)
                );
            }
            other => panic!("expected run.completed, got {other:?}"),
        }
    }

    /// An empty diff means there is nothing to open a pull request for. The
    /// branch still gets pushed and the run still succeeds.
    #[tokio::test]
    async fn empty_diff_pushes_branch_without_opening_pull_request() {
        let repo_dir = tempfile::tempdir().unwrap();
        init_git_repo(repo_dir.path());
        let emitter = Arc::new(Emitter::new(test_run_id()));
        let events = record_events(&emitter);
        let services = test_services(
            RunStoreHandle::local(seeded_run_store().await),
            emitter,
            Arc::new(
                fabro_sandbox::local_sandbox(repo_dir.path().to_path_buf())
                    .await
                    .unwrap(),
            ),
        );
        let mut run_options = test_run_options(repo_dir.path());
        run_options.base_branch = Some("main".to_string());
        run_options.git = Some(GitCheckpointOptions {
            base_sha:   None,
            run_branch: Some("fabro/run/test".to_string()),
        });
        let executed = test_executed(
            Graph::new("test"),
            Ok(Outcome::success()),
            run_options,
            5,
            services,
        );
        let options = FinalizeOptions {
            run_dir:          repo_dir.path().to_path_buf(),
            run_id:           test_run_id(),
            workflow_name:    "test".to_string(),
            preserve_sandbox: false,
            stop_on_terminal: true,
            last_git_sha:     Some("final-sha".to_string()),
        };
        let mut concluded = conclude(executed, &options).await.unwrap();
        concluded.conclusion.diff.patch = None;
        let published = crate::pipeline::publish(concluded, &crate::pipeline::PublishOptions {
            pr_config:           Some(fabro_types::settings::run::PullRequestSettings {
                enabled:          true,
                draft:            true,
                auto_merge:       false,
                merge_strategy:   fabro_types::settings::run::MergeStrategy::Squash,
                model:            None,
                reasoning_effort: None,
            }),
            github_app:          None,
            origin_url:          Some("https://github.com/owner/repo.git".to_string()),
            // Distinct sentinels (regression a1e27c9bf): if this path ever
            // reaches PR creation, the content call must use the resolved
            // PR model, never the run-model fallback field.
            pr_model:            "run-model-sentinel".to_string(),
            pr_resolved_model:   "pr-model-sentinel".to_string(),
            pr_reasoning_effort: None,
        })
        .await;

        assert!(published.publish_error.is_none());
        let finalized = finalize(published, &options).await.unwrap();

        assert!(finalized.outcome.is_ok());
        assert_eq!(finalized.pushed_branch.as_deref(), Some("fabro/run/test"));
        assert_eq!(finalized.pr_url, None);
        let events = events.lock().unwrap();
        let names = events.iter().map(RunEvent::event_name).collect::<Vec<_>>();
        assert_eq!(names, vec!["git.push", "run.completed"]);
    }

    /// `base_sha` is where the run started, not what it produced. Reporting it
    /// as the final commit would both mis-state a durable field and make the
    /// remote-head check reject a branch that was pushed correctly.
    #[tokio::test]
    async fn untracked_final_commit_does_not_fall_back_to_base_sha() {
        let repo_dir = tempfile::tempdir().unwrap();
        init_git_repo(repo_dir.path());
        let emitter = Arc::new(Emitter::new(test_run_id()));
        let services = test_services(
            RunStoreHandle::local(seeded_run_store().await),
            emitter,
            Arc::new(
                fabro_sandbox::local_sandbox(repo_dir.path().to_path_buf())
                    .await
                    .unwrap(),
            ),
        );
        let mut run_options = test_run_options(repo_dir.path());
        run_options.base_branch = Some("main".to_string());
        run_options.git = Some(GitCheckpointOptions {
            base_sha:   Some("base-sha".to_string()),
            run_branch: Some("fabro/run/test".to_string()),
        });
        let executed = test_executed(
            Graph::new("test"),
            Ok(Outcome::success()),
            run_options,
            5,
            services,
        );
        let options = FinalizeOptions {
            run_dir:          repo_dir.path().to_path_buf(),
            run_id:           test_run_id(),
            workflow_name:    "test".to_string(),
            preserve_sandbox: false,
            stop_on_terminal: true,
            last_git_sha:     None,
        };
        let mut concluded = conclude(executed, &options).await.unwrap();

        assert_eq!(concluded.conclusion.final_git_commit_sha, None);

        // No pull request wanted, so publish still pushes the branch and the
        // run succeeds without needing a commit SHA at all.
        concluded.conclusion.diff.patch = None;
        let published = crate::pipeline::publish(concluded, &crate::pipeline::PublishOptions {
            pr_config:           None,
            github_app:          None,
            origin_url:          Some("https://github.com/owner/repo.git".to_string()),
            pr_model:            "test-model".to_string(),
            pr_resolved_model:   "test-model".to_string(),
            pr_reasoning_effort: None,
        })
        .await;
        let finalized = finalize(published, &options).await.unwrap();

        assert!(finalized.outcome.is_ok());
        assert_eq!(finalized.pushed_branch.as_deref(), Some("fabro/run/test"));
        assert_eq!(finalized.conclusion.final_git_commit_sha, None);
    }

    /// fabro-67e5: a pull-request failure after a successful push keeps the
    /// run green (`succeeded` + `publish_blocked`) so the dev loop does not
    /// read the finished work as broken.
    #[tokio::test]
    async fn pull_request_failure_precedes_terminal_publish_failure() {
        let repo_dir = tempfile::tempdir().unwrap();
        init_git_repo(repo_dir.path());
        let emitter = Arc::new(Emitter::new(test_run_id()));
        let events = record_events(&emitter);
        let services = test_services(
            RunStoreHandle::local(seeded_run_store().await),
            emitter,
            Arc::new(
                fabro_sandbox::local_sandbox(repo_dir.path().to_path_buf())
                    .await
                    .unwrap(),
            ),
        );
        let mut run_options = test_run_options(repo_dir.path());
        run_options.base_branch = Some("main".to_string());
        run_options.git = Some(GitCheckpointOptions {
            base_sha:   None,
            run_branch: Some("fabro/run/test".to_string()),
        });
        let executed = test_executed(
            Graph::new("test"),
            Ok(Outcome::success()),
            run_options,
            5,
            services,
        );
        let options = FinalizeOptions {
            run_dir:          repo_dir.path().to_path_buf(),
            run_id:           test_run_id(),
            workflow_name:    "test".to_string(),
            preserve_sandbox: false,
            stop_on_terminal: true,
            last_git_sha:     Some("final-sha".to_string()),
        };
        let mut concluded = conclude(executed, &options).await.unwrap();
        concluded.conclusion.diff.patch =
            Some("diff --git a/a b/a\n+published change\n".to_string());
        let published = crate::pipeline::publish(concluded, &crate::pipeline::PublishOptions {
            pr_config:           Some(fabro_types::settings::run::PullRequestSettings {
                enabled:          true,
                draft:            true,
                auto_merge:       false,
                merge_strategy:   fabro_types::settings::run::MergeStrategy::Squash,
                model:            None,
                reasoning_effort: None,
            }),
            github_app:          None,
            origin_url:          Some("https://github.com/owner/repo.git".to_string()),
            pr_model:            "test-model".to_string(),
            pr_resolved_model:   "test-model".to_string(),
            pr_reasoning_effort: None,
        })
        .await;
        let finalized = finalize(published, &options).await.unwrap();

        assert!(finalized.outcome.is_ok());
        assert_eq!(finalized.conclusion.status, StageOutcome::Succeeded);
        assert_eq!(
            finalized.publish_failure.as_ref().map(|f| f.reason),
            Some(FailureReason::PublishFailed)
        );
        // The push landed before the pull request failed, so the branch is
        // still reported — that is exactly the run where the user needs it.
        assert_eq!(finalized.pushed_branch.as_deref(), Some("fabro/run/test"));
        assert!(
            finalized
                .publish_failure
                .expect("publish failure present")
                .detail
                .message
                .contains("was pushed"),
            "remediation should name the pushed branch"
        );
        let events = events.lock().unwrap();
        let names = events.iter().map(RunEvent::event_name).collect::<Vec<_>>();
        assert_eq!(names, vec![
            "git.push",
            "pull_request.failed",
            "run.completed"
        ]);
        match &events.last().unwrap().body {
            EventBody::RunCompleted(props) => {
                assert_eq!(props.reason, SuccessReason::PublishBlocked);
                assert_eq!(
                    props.failure.as_ref().map(|failure| failure.reason),
                    Some(FailureReason::PublishFailed)
                );
            }
            other => panic!("expected run.completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn finalize_stops_sandbox_on_terminal_without_deleting() {
        let repo_dir = tempfile::tempdir().unwrap();
        let sandbox = MockSandbox::linux();
        let services = test_services(
            RunStoreHandle::local(seeded_run_store().await),
            Arc::new(Emitter::new(test_run_id())),
            sandbox.sandbox(),
        );
        let executed = test_executed(
            Graph::new("test"),
            Ok(Outcome::success()),
            test_run_options(repo_dir.path()),
            5,
            services,
        );

        finalize_executed(executed, &FinalizeOptions {
            run_dir:          repo_dir.path().to_path_buf(),
            run_id:           test_run_id(),
            workflow_name:    "test".to_string(),
            preserve_sandbox: false,
            stop_on_terminal: true,
            last_git_sha:     None,
        })
        .await
        .unwrap();

        assert_eq!(sandbox.driver().stop_count(), 1);
        assert_eq!(sandbox.driver().delete_count(), 0);
    }

    #[tokio::test]
    async fn finalize_leaves_sandbox_running_when_stop_on_terminal_is_false() {
        let repo_dir = tempfile::tempdir().unwrap();
        let sandbox = MockSandbox::linux();
        let services = test_services(
            RunStoreHandle::local(seeded_run_store().await),
            Arc::new(Emitter::new(test_run_id())),
            sandbox.sandbox(),
        );
        let executed = test_executed(
            Graph::new("test"),
            Ok(Outcome::success()),
            test_run_options(repo_dir.path()),
            5,
            services,
        );

        finalize_executed(executed, &FinalizeOptions {
            run_dir:          repo_dir.path().to_path_buf(),
            run_id:           test_run_id(),
            workflow_name:    "test".to_string(),
            preserve_sandbox: false,
            stop_on_terminal: false,
            last_git_sha:     None,
        })
        .await
        .unwrap();

        assert_eq!(sandbox.driver().stop_count(), 0);
        assert_eq!(sandbox.driver().delete_count(), 0);
    }

    #[tokio::test]
    async fn finalize_terminal_event_includes_diff_summary() {
        let repo_dir = tempfile::tempdir().unwrap();
        let repo = repo_dir.path();
        init_git_repo(repo);
        tokio::fs::write(repo.join("notes.txt"), "one\n")
            .await
            .unwrap();
        let base = git_commit_all(repo, "base");
        tokio::fs::write(repo.join("notes.txt"), "one\ntwo\nthree\n")
            .await
            .unwrap();
        let head = git_commit_all(repo, "head");

        let run_store = seeded_run_store().await;
        let emitter = Arc::new(Emitter::new(test_run_id()));
        let events = record_events(&emitter);
        let services = test_services(
            RunStoreHandle::local(run_store),
            Arc::clone(&emitter),
            Arc::new(
                fabro_sandbox::local_sandbox(repo.to_path_buf())
                    .await
                    .unwrap(),
            ),
        );
        let mut run_options = test_run_options(repo);
        run_options.git = Some(GitCheckpointOptions {
            base_sha:   Some(base),
            run_branch: None,
        });
        let executed = test_executed(
            Graph::new("test"),
            Ok(Outcome::success()),
            run_options,
            5,
            services,
        );

        finalize_executed(executed, &FinalizeOptions {
            run_dir:          repo.to_path_buf(),
            run_id:           test_run_id(),
            workflow_name:    "test".to_string(),
            preserve_sandbox: true,
            stop_on_terminal: true,
            last_git_sha:     Some(head),
        })
        .await
        .unwrap();

        let events = events.lock().unwrap();
        let run_completed = events
            .iter()
            .find(|event| event.event_name() == "run.completed")
            .expect("run.completed event");
        let properties = run_completed.properties().unwrap();
        assert_eq!(
            properties["diff_summary"],
            serde_json::json!({
                "files_changed": 1,
                "additions": 2,
                "deletions": 0
            })
        );
    }
}
