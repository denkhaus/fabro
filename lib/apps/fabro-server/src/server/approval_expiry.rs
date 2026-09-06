//! Approval TTL backstop (fabro-54f0).
//!
//! A run parked in `pending(approval_required)` waits for a human decision
//! indefinitely — surviving restarts, blocking tracker serialization, and
//! inviting duplicate spawns. This supervisor expires overdue approvals:
//! after the run's effective approval window the run fails with
//! `approval_timeout`, freeing the slot and surfacing the cleanup in every
//! run list. The window is `settings.run.execution.approval_timeout_secs`
//! with a 24h fallback; see [`RunExecutionSettings`].

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use fabro_store::RunProjection;
use fabro_types::RunId;
use fabro_types::settings::run::RunExecutionSettings;
use fabro_types::status::{FailureReason, PendingReason, RunStatus, RunStatusKind};
use fabro_workflow::Error as WorkflowError;
use fabro_workflow::event::{Event, append_event_if};
use tokio::task::JoinHandle;
use tokio::time::{MissedTickBehavior, interval};

use crate::server::AppState;

/// Supervisor pass cadence. Approval expiry is a minutes-scale backstop,
/// not a latency-sensitive path.
const EXPIRY_PASS_INTERVAL: Duration = Duration::from_mins(1);

pub(crate) fn spawn_approval_expiry_supervisor(state: Arc<AppState>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = interval(EXPIRY_PASS_INTERVAL);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            if state.is_shutting_down() {
                break;
            }
            if let Err(err) = expire_pending_approvals(state.as_ref(), Utc::now()).await {
                tracing::warn!(error = ?err, "approval expiry pass failed");
            }
        }
    })
}

/// Fail every pending-approval run whose approval window has lapsed.
/// Returns the number of runs expired. Both appended events re-check the
/// pending state, so a concurrent approve or deny wins the race and the
/// expiry is dropped instead of corrupting the other outcome.
pub(crate) async fn expire_pending_approvals(
    state: &AppState,
    now: DateTime<Utc>,
) -> anyhow::Result<usize> {
    let pending = state
        .stores
        .run_summaries
        .list_by_statuses(&[RunStatusKind::Pending], now)
        .await?;
    let mut expired = 0usize;
    for summary in pending {
        if !matches!(summary.lifecycle.status, RunStatus::Pending {
            reason: PendingReason::ApprovalRequired,
        }) {
            continue;
        }
        let id = summary.id;
        let settings = match run_execution_settings(state, &id).await {
            Ok(settings) => settings,
            Err(err) => {
                tracing::warn!(run_id = ?id, error = ?err, "loading run settings for approval expiry failed");
                continue;
            }
        };
        let window =
            chrono::Duration::seconds(settings.effective_approval_timeout_secs().cast_signed());
        // Measured from creation: a lower bound of pending-since, so the
        // expiry never fires early for runs started long after creation.
        if now < summary.timestamps.created_at + window {
            continue;
        }
        match expire_run(state, &id, settings.effective_approval_timeout_secs(), now).await {
            Ok(()) => expired += 1,
            Err(err) => {
                tracing::warn!(
                    run_id = ?id,
                    error = ?err,
                    "expiring overdue approval failed"
                );
            }
        }
    }
    Ok(expired)
}

async fn run_execution_settings(
    state: &AppState,
    id: &RunId,
) -> anyhow::Result<RunExecutionSettings> {
    let projection = state
        .stores
        .runs
        .load_run_projection(id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("run projection vanished while expiring approval"))?;
    Ok(projection.spec.settings.run.execution.clone())
}

async fn expire_run(
    state: &AppState,
    id: &RunId,
    timeout_secs: u64,
    _now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let run_store = state.stores.runs.open_run(id).await?;
    let still_pending = |projection: &RunProjection| {
        matches!(projection.status, RunStatus::Pending {
            reason: PendingReason::ApprovalRequired,
        })
    };
    let message = format!("approval window expired after {timeout_secs}s");
    append_event_if(
        &run_store,
        id,
        &Event::RunDenied {
            reason: Some(message.clone()),
            actor:  None,
        },
        still_pending,
    )
    .await?;
    append_event_if(
        &run_store,
        id,
        &Event::workflow_run_failed_from_error(
            &WorkflowError::engine(message),
            fabro_types::RunTiming::default(),
            FailureReason::ApprovalTimeout,
            None,
            None,
            None,
            None,
        ),
        still_pending,
    )
    .await?;
    tracing::info!(run_id = ?id, timeout_secs, "expired pending approval");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;
    use std::sync::Arc;

    use chrono::Utc;
    use fabro_types::status::{FailureReason, PendingReason, RunStatus};
    use fabro_types::{Graph, RunId, WorkflowSettings, test_support};
    use fabro_workflow::event as workflow_event;
    use fabro_workflow::run_status::SuccessReason;

    use super::expire_pending_approvals;
    use crate::test_support::{
        default_test_server_settings, test_app_state_with_store, test_store_bundle,
    };

    fn pending_state() -> (Arc<crate::server::AppState>, Arc<fabro_store::Database>) {
        let (store, artifact_store) = test_store_bundle();
        let state = test_app_state_with_store(
            default_test_server_settings(),
            fabro_config::RunLayer::default(),
            5,
            Arc::clone(&store),
            artifact_store,
        );
        (state, store)
    }

    async fn append_pending_run(
        store: &fabro_store::Database,
        run_id: &RunId,
        approval_timeout_secs: Option<u64>,
    ) {
        let run_store = store.create_run(run_id).await.expect("create run store");
        let mut settings = WorkflowSettings::default();
        settings.run.execution.approval_timeout_secs =
            approval_timeout_secs.and_then(NonZeroU64::new);
        workflow_event::append_event(&run_store, run_id, &workflow_event::Event::RunCreated {
            run_id:              *run_id,
            title:               None,
            settings:            serde_json::to_value(&settings)
                .expect("workflow settings should serialize"),
            graph:               serde_json::to_value(Graph::new("test"))
                .expect("graph should serialize"),
            workflow_source:     None,
            labels:              std::collections::BTreeMap::default(),
            source_directory:    None,
            workflow_slug:       None,
            workflow_version_id: None,
            target:              None,
            automation:          None,
            provenance:          test_support::test_run_provenance(),
            manifest_blob:       None,
            spec_blob:           None,
            git:                 None,
            fork_source_ref:     None,
            retried_from:        None,
            parent_id:           None,
            web_url:             None,
        })
        .await
        .expect("append RunCreated");
        workflow_event::append_event(
            &run_store,
            run_id,
            &workflow_event::Event::RunStartRequested {
                resume: false,
                actor:  None,
            },
        )
        .await
        .expect("append RunStartRequested");
        workflow_event::append_event(&run_store, run_id, &workflow_event::Event::RunPending {
            reason: PendingReason::ApprovalRequired,
            actor:  None,
        })
        .await
        .expect("append RunPending");
    }

    async fn append_completed_run(store: &fabro_store::Database, run_id: &RunId) {
        append_pending_run(store, run_id, None).await;
        let run_store = store.open_run(run_id).await.expect("open run store");
        workflow_event::append_event(&run_store, run_id, &workflow_event::Event::RunApproved {
            actor: None,
        })
        .await
        .expect("append RunApproved");
        workflow_event::append_event(&run_store, run_id, &workflow_event::Event::RunRunnable {
            source: fabro_types::RunRunnableSource::Approved,
            actor:  None,
        })
        .await
        .expect("append RunRunnable");
        workflow_event::append_event(&run_store, run_id, &workflow_event::Event::RunStarting)
            .await
            .expect("append RunStarting");
        workflow_event::append_event(
            &run_store,
            run_id,
            &workflow_event::Event::WorkflowRunStarted {
                name:         "test".to_string(),
                run_id:       *run_id,
                base_branch:  Some("main".to_string()),
                base_sha:     Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
                run_branch:   Some("fabro/run/test".to_string()),
                worktree_dir: None,
                goal:         Some("Test approval expiry".to_string()),
            },
        )
        .await
        .expect("append WorkflowRunStarted");
        workflow_event::append_event(&run_store, run_id, &workflow_event::Event::RunRunning)
            .await
            .expect("append RunRunning");
        workflow_event::append_event(
            &run_store,
            run_id,
            &workflow_event::Event::WorkflowRunCompleted {
                timing:               fabro_types::RunTiming::wall_only(1),
                artifact_count:       0,
                status:               "succeeded".to_string(),
                reason:               SuccessReason::Completed,
                failure:              None,
                total_usd_micros:     None,
                final_git_commit_sha: None,
                final_patch:          Some(String::new()),
                diff_summary:         None,
                billing:              None,
            },
        )
        .await
        .expect("append WorkflowRunCompleted");
    }

    async fn run_status(state: &crate::server::AppState, run_id: &RunId) -> RunStatus {
        state
            .stores
            .run_summaries
            .get(run_id, Utc::now())
            .await
            .expect("load summary")
            .expect("run exists")
            .lifecycle
            .status
    }

    #[tokio::test]
    async fn expires_pending_approval_past_its_window() {
        let (state, store) = pending_state();
        let run_id = RunId::new();
        append_pending_run(&store, &run_id, Some(1)).await;

        // Advance the clock past the 1s window; the scan window still covers
        // the freshly created run.
        let later = Utc::now() + chrono::Duration::hours(2);
        let expired = expire_pending_approvals(&state, later)
            .await
            .expect("expiry pass");
        assert_eq!(expired, 1);
        assert_eq!(run_status(&state, &run_id).await, RunStatus::Failed {
            reason: FailureReason::ApprovalTimeout,
        });
    }

    #[tokio::test]
    async fn keeps_pending_approval_inside_its_window() {
        let (state, store) = pending_state();
        let run_id = RunId::new();
        // 24h fallback applies when the run sets no explicit window.
        append_pending_run(&store, &run_id, None).await;

        let expired = expire_pending_approvals(&state, Utc::now() + chrono::Duration::hours(1))
            .await
            .expect("expiry pass");
        assert_eq!(expired, 0);
        assert_eq!(run_status(&state, &run_id).await, RunStatus::Pending {
            reason: PendingReason::ApprovalRequired,
        });
    }

    #[tokio::test]
    async fn ignores_runs_that_are_not_pending_approval() {
        let (state, store) = pending_state();
        let done = RunId::new();
        append_completed_run(&store, &done).await;

        let expired = expire_pending_approvals(&state, Utc::now() + chrono::Duration::hours(30))
            .await
            .expect("expiry pass");
        assert_eq!(expired, 0);
        assert!(matches!(
            run_status(&state, &done).await,
            RunStatus::Succeeded { .. }
        ));
    }
}
