//! Petri's coordinator events folded into the view: the run's start, its
//! invocations and executions, pause and unpause, its finish, and the
//! release of its sandbox (VIEWS.md "Run", "Sandbox").

use chrono::{DateTime, Utc};
use fabro_types::{
    Conclusion, FailureCategory, FailureDetail, FailureReason, RunControlAction, RunFailure,
    RunSandbox, RunStatus, RunTiming, StageOutcome, StartRecord, SuccessReason, usage_rollup,
};
use petri_execution::CoordinatorEvent;
use petri_execution::events::RunEvent;

use super::{FiringKey, InvocationRef, RunView, apply_status, settle_control};

impl RunView {
    pub(super) fn fold_coordinator(
        &mut self,
        record: &CoordinatorEvent,
        event: &RunEvent,
        at: DateTime<Utc>,
    ) {
        match record {
            CoordinatorEvent::RunStarted {
                root, forked_from, ..
            } => {
                self.state.root = Some(root.raw());
                self.state.started_at = Some(event.recorded_at);
                if let Some(projection) = self.projection.as_mut() {
                    // A fork's declaration names its source; a parse failure
                    // means the source was not a Fabro run, which the
                    // projection cannot show.
                    projection.forked_from = forked_from.as_ref().and_then(|origin| {
                        Some(fabro_types::ForkOrigin {
                            source_run_id: origin.source.as_str().parse().ok()?,
                            execution:     origin.position.execution.raw(),
                            firing:        origin.position.firing.raw(),
                            rerun_last:    origin.rerun_last,
                        })
                    });
                    apply_status(projection, RunStatus::Running, at);
                    projection.start = Some(StartRecord {
                        start_time: at,
                        run_branch: self.state.run_branch.clone(),
                        base_sha:   self.state.base_sha.clone(),
                    });
                    // The scope's sandbox is acquired next; `scope.acquired`
                    // or `scope.failed` settles it.
                    if let Some(sandbox) = projection.sandbox.take() {
                        projection.sandbox = Some(RunSandbox::initializing(sandbox.plan().clone()));
                    }
                }
            }
            CoordinatorEvent::InvocationDeclared { invocation, .. } => {
                let mut info = InvocationRef::default();
                if let Some(parent) = &event.context.parent {
                    info.parent = Some((parent.execution.raw(), parent.firing.raw()));
                    if let Some((fork_firing, index)) = branch_slot(&parent.slot) {
                        let group = self
                            .state
                            .stages
                            .get(&FiringKey::new(parent.execution.raw(), fork_firing))
                            .map(|stage| stage.stage_id.clone());
                        if let Some(group) = group {
                            info.branch = Some((group, index));
                        }
                    }
                }
                self.state.invocations.insert(invocation.raw(), info);
            }
            CoordinatorEvent::ExecutionDeclared {
                execution,
                invocation,
                ..
            } => {
                self.state
                    .executions
                    .insert(execution.raw(), invocation.raw());
            }
            CoordinatorEvent::InvocationFinished { invocation, result } => {
                let info = self.state.invocations.entry(invocation.raw()).or_default();
                info.failure = result
                    .failure
                    .as_ref()
                    .map(|failure| failure.message.clone());
                info.output = Some(result.output.clone());
            }
            CoordinatorEvent::RunPaused => {
                if let Some(projection) = self.projection.as_mut() {
                    apply_status(projection, projection.status.paused(), at);
                    settle_control(projection, RunControlAction::Pause);
                }
            }
            CoordinatorEvent::RunUnpaused => {
                if let Some(projection) = self.projection.as_mut() {
                    apply_status(projection, projection.status.unpaused(), at);
                    settle_control(projection, RunControlAction::Unpause);
                }
            }
            CoordinatorEvent::RunFinished { status } => {
                self.state.finished = Some(status.to_string());
                self.conclude(status.to_string().as_str(), at);
            }
            // ── Sandbox: the retention outcome (VIEWS.md "Sandbox") ─────────
            // The instance stays on `Run.sandbox`: it names what ran, and
            // `retained` says whether it still exists.
            CoordinatorEvent::ScopeReleased {
                invocation,
                retained,
                ..
            } => {
                if Some(invocation.raw()) == self.state.root {
                    self.state.sandbox_retained = Some(*retained);
                    if let Some(sandbox) = self
                        .projection
                        .as_mut()
                        .and_then(|projection| projection.sandbox.as_mut())
                    {
                        sandbox.set_retained(*retained);
                    }
                }
            }
            CoordinatorEvent::GraphRegistered { .. }
            | CoordinatorEvent::ExecutionFinished { .. }
            | CoordinatorEvent::InvocationCancelRequested { .. }
            | CoordinatorEvent::RunNoteRecorded { .. } => {}
        }
    }

    /// The run's conclusion, from its recorded finish and what the stages
    /// summed to.
    fn conclude(&mut self, status: &str, at: DateTime<Utc>) {
        use fabro_llm::LONG_RATE_LIMIT_WINDOW;
        use fabro_llm::gateway::{RateLimitWindow, reset_window};
        // Fork seam read (fabro-6655): snapshot the publish state before the
        // mutable projection borrow below.
        let pull_request_creation = self
            .projection
            .as_ref()
            .and_then(|projection| projection.pull_request_creation.clone());
        let Some(projection) = self.projection.as_mut() else {
            return;
        };
        let root = self
            .state
            .root
            .and_then(|root| self.state.invocations.get(&root));
        let failure_message = root.and_then(|root| root.failure.clone());
        let mut run_status = finished_status(status);
        // Fork seam (fabro-6655, fabro-67e5): a failed publish
        // downgrades a green conclusion to PublishBlocked, keeping
        // the run green while naming why delivery is incomplete.
        let publish_blocked_failure = if matches!(run_status, RunStatus::Succeeded { .. })
            && super::fork_taxonomy::publish_creation_failed(pull_request_creation.as_ref())
        {
            let error = pull_request_creation
                .as_ref()
                .and_then(|creation| creation.error.clone())
                .unwrap_or_else(|| "unknown error".to_string());
            run_status = RunStatus::Succeeded {
                reason: SuccessReason::PublishBlocked,
            };
            Some(super::fork_taxonomy::publish_blocked_failure(&error, at))
        } else {
            None
        };
        // Fork seam (fabro-b00c, user decision 2026-09-25 — engine-side,
        // refined 2026-09-26 option a): a green conclusion cannot stand
        // over a leg failure a CATCH-ALL (unconditional) edge consumed —
        // that is the fake-green the rule exists for. A failure an
        // EXPLICIT conditional edge routed onward is control flow, the
        // petri TerminalNode rule's own line, and stays green; a
        // retried-then-succeeded leg never counts, because the leg's
        // FINAL outcome is what the stages hold. PublishBlocked above set
        // its own success reason, and the x.kind exit edges below are an
        // explicit graph decision that consumes the failure deliberately.
        // A parent whose child died must never read as a clean success.
        let catch_all_failure = catch_all_leg_failure(projection, failure_message.as_deref());
        if super::fork_taxonomy::refuses_green_conclusion(&run_status, catch_all_failure.as_deref())
        {
            tracing::warn!(
                conclusion_downgraded = true,
                message = %failure_message.as_deref().unwrap_or(""),
                "recorded leg failure refuses the green conclusion (fabro-b00c)"
            );
            run_status = RunStatus::Failed {
                reason: FailureReason::WorkflowError,
            };
        }

        let (outcome, failure) = match run_status {
            RunStatus::Failed {
                reason: reason @ FailureReason::Cancelled,
            } => (
                StageOutcome::Failed {
                    retry_requested: false,
                },
                Some(RunFailure {
                    reason,
                    detail: FailureDetail::new(
                        failure_message
                            .clone()
                            .unwrap_or_else(|| "the run was cancelled".to_string()),
                        FailureCategory::Canceled,
                    ),
                }),
            ),
            RunStatus::Failed { reason } => (
                StageOutcome::Failed {
                    retry_requested: false,
                },
                Some(RunFailure {
                    reason,
                    detail: FailureDetail::new(
                        failure_message
                            .clone()
                            .unwrap_or_else(|| "the run failed".to_string()),
                        FailureCategory::Deterministic,
                    ),
                }),
            ),
            _ => (StageOutcome::Succeeded, None),
        };
        let failure = failure.or(publish_blocked_failure);
        // Fork seam (fabro-2e7b, ADR-0021 rev 2 Option C): a failure whose
        // message announces a provider usage-window reset (long window or
        // naive-ETA) parks the run resumable instead of failing it — the
        // pre-fire provider gate owns its recovery through rewind.
        if let (RunStatus::Failed { .. }, Some(failure)) = (&run_status, &failure) {
            let parks = reset_window(&failure.detail.message, std::time::SystemTime::now())
                .is_some_and(|window| {
                    matches!(window, RateLimitWindow::UnknownEta)
                        || matches!(
                            window,
                            RateLimitWindow::Reopens(wait)
                                if wait > LONG_RATE_LIMIT_WINDOW
                        )
                });
            if parks {
                run_status = RunStatus::Blocked {
                    blocked_reason: fabro_types::BlockedReason::QuotaRateLimit,
                };
            }
        }

        // Fork seam (fabro-288d, ADR-0010 rev Option A): the run's DOT
        // source carries the fork's `x.kind` exit edges; a boundary
        // failure upgrades to `Succeeded { Boundary }`, deadlock/soft
        // success-shapes downgrade to their park reasons.
        {
            let graph_source = projection.spec.graph_source.as_deref().unwrap_or_default();
            let exit_kinds = super::fork_exit_kinds::ExitKinds::parse(graph_source);
            // The kind lookup needs the stage that ROUTED to exit, not
            // the exit stage itself: `node@visit` display ids never match
            // the DOT edge names, and the exit node runs after the router
            // (fabro-51ad: a looping deadlock exit read as workflow_error
            // because `flaky@3`/`exit@1` never matched `flaky -> exit`).
            let last_stage = self
                .state
                .stages
                .iter()
                .rev()
                .map(|(_, stage)| stage.stage_id.node_id().to_string())
                .find(|node| node != "exit");
            if let Some(overridden) = exit_kinds.classify(status, last_stage.as_deref(), "exit") {
                run_status = overridden;
            }
        }
        apply_status(projection, run_status, at);
        projection.pending_control = None;
        projection.pending_interviews.clear();
        let rollup = usage_rollup::usage_rollup_from_projection(projection);
        let (stages, total_retries) = rollup.conclusion_stages(projection);
        let wall_time_ms = self.state.started_at.map_or(0, |started| {
            u64::try_from(at.timestamp_millis())
                .unwrap_or(0)
                .saturating_sub(started)
        });
        let timing = RunTiming::new(
            wall_time_ms,
            rollup.timing.inference_time_ms,
            rollup.timing.tool_time_ms,
        );
        let last_checkpoint = projection.checkpoints.last();
        projection.conclusion = Some(Conclusion {
            timestamp: at,
            status: outcome,
            timing,
            failure,
            final_git_commit_sha: last_checkpoint
                .and_then(|checkpoint| checkpoint.checkpoint.git_commit_sha.clone()),
            stages,
            usage: rollup.usage_if_present(),
            total_retries,
            diff: self
                .state
                .run_diff
                .clone()
                .or_else(|| last_checkpoint.map(|checkpoint| checkpoint.diff.clone()))
                .unwrap_or_default(),
        });
    }
}

/// The status Fabro gives a run at Petri's finish, by the status the finish
/// records (`success`, `cancelled`, or a failure).
pub(super) fn finished_status(status: &str) -> RunStatus {
    match status {
        "success" => RunStatus::Succeeded {
            reason: SuccessReason::Completed,
        },
        "cancelled" => RunStatus::Failed {
            reason: FailureReason::Cancelled,
        },
        _ => RunStatus::Failed {
            reason: FailureReason::WorkflowError,
        },
    }
}

/// The fork firing and branch index a branch child's call slot names:
/// `branch:<fork>@<firing>:<index>:<target>`.
fn branch_slot(slot: &str) -> Option<(u64, u32)> {
    let rest = slot.strip_prefix("branch:")?;
    let mut parts = rest.splitn(3, ':');
    let fork = parts.next()?;
    let index = parts.next()?.parse::<u32>().ok()?;
    let firing = fork.rsplit_once('@')?.1.parse::<u64>().ok()?;
    Some((firing, index))
}

/// The first leg failure a catch-all (unconditional) edge consumed, as
/// the message the downgrade names: `None` when every failed leg rode an
/// explicit conditional route — or when no leg failed at all (a
/// retried-then-succeeded leg's final outcome is success, so it never
/// appears here). The failed leg's consumer is the next stage in the
/// projection's execution order, or `exit` when the leg is last.
fn catch_all_leg_failure(
    projection: &fabro_types::RunProjection,
    recorded: Option<&str>,
) -> Option<String> {
    // Stages in EXECUTION order: `StageId` orders lexicographically, the
    // run's own order lives in each stage's first event sequence.
    let mut stages: Vec<(u32, String, Option<String>)> = projection
        .iter_stages()
        .map(|(stage_id, stage)| {
            (
                stage.first_event_seq.get(),
                stage_id.node_id().to_string(),
                stage.completion.as_ref().and_then(|completion| {
                    completion.outcome.is_failure().then(|| {
                        completion
                            .failure_reason
                            .clone()
                            .or_else(|| recorded.map(str::to_string))
                            .unwrap_or_default()
                    })
                }),
            )
        })
        .collect();
    stages.sort_by_key(|(seq, _, _)| *seq);
    let Some((index, _)) = stages
        .iter()
        .enumerate()
        .find(|(_, (_, _, failure))| failure.is_some())
    else {
        // No failed leg: a recorded message without a failed stage is a
        // superseded attempt's echo — retries are invisible.
        return None;
    };
    let (_, node, message) = &stages[index];
    let consumer = stages
        .get(index + 1)
        .map_or_else(|| "exit".to_string(), |(_, next, _)| next.clone());
    let edges = super::edge_conditions::EdgeConditions::parse(
        projection.spec.graph_source.as_deref().unwrap_or_default(),
    );
    if edges.is_conditional(node, &consumer) {
        return None;
    }
    message.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_branch_slot_names_the_fork_firing_and_the_index() {
        assert_eq!(branch_slot("branch:fan@7:2:review"), Some((7, 2)));
        assert_eq!(branch_slot("branch:fan@7:x:review"), None);
        assert_eq!(branch_slot("child:0"), None);
    }
}
