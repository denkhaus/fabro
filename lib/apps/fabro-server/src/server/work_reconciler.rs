//! The work reconciler (ADR-0032): heals claim ledger, tracker projection
//! and outbox after crashes and outages.
//!
//! One periodic pass per project, all directions:
//!
//! - **Outbox drain**: due `claim.projected` / `claim.cleared` jobs are applied
//!   against the tracker and completed, or failed with backoff.
//! - **DB -> tracker**: live claims get their run-id label (idempotent);
//!   terminal claims get it cleared and the label object deleted.
//! - **tracker-independent orphans**: a claim whose run record is gone releases
//!   after the grace window; a claim whose run is terminal moves to
//!   `awaiting_merge` (PR open) or releases (no PR) — run status is the truth,
//!   wall-clock age is not (no 6-hour timer).
//! - **Label GC**: run-id labels with no issue are deleted.
//!
//! The DB stays the authority: every tracker write here is idempotent, and a
//! failed pass retries on the next tick.

use std::sync::Arc;

use chrono::{Duration, Utc};
use fabro_work::{ClaimState, TrackerProvider};
use tokio::task::JoinHandle;
use tokio::time::{MissedTickBehavior, interval};

use crate::server::AppState;

const PASS_INTERVAL: Duration = Duration::seconds(30);
const ORPHAN_GRACE: Duration = Duration::minutes(30);
const OUTBOX_BATCH: u32 = 20;

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ReconcileReport {
    pub projects:         usize,
    pub outbox_applied:   usize,
    pub outbox_failed:    usize,
    pub labels_set:       usize,
    pub labels_cleared:   usize,
    pub orphans_released: usize,
    pub terminal_moved:   usize,
    pub labels_gc:        u64,
}

pub(crate) fn spawn(state: Arc<AppState>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = interval(PASS_INTERVAL.to_std().expect("pass interval is valid"));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if state.is_shutting_down() {
                break;
            }
            let report = run_pass(&state).await;
            tracing::debug!(?report, "work reconciler pass");
        }
    })
}

pub(crate) async fn run_pass(state: &AppState) -> ReconcileReport {
    let mut report = ReconcileReport::default();
    let Ok(projects) = state.work_store().projects().await else {
        tracing::warn!("work reconciler: project read failed");
        return report;
    };
    report.projects = projects.len();
    for project in projects {
        let tracker = match state.work_tracker_for_project(&project).await {
            Ok(tracker) => tracker,
            Err(error) => {
                tracing::warn!(
                    project = %project.project_id,
                    error = %error,
                    "work reconciler: tracker unavailable"
                );
                continue;
            }
        };

        report.outbox_applied += drain_outbox(state, &project, tracker.as_ref()).await;

        match state.work_store().active_claims(&project.project_id).await {
            Ok(claims) => {
                for claim in claims {
                    match claim.state {
                        ClaimState::Granted | ClaimState::Active => {
                            if let Err(error) = tracker
                                .project_claim(&project, &claim.item_key, &claim.run_id.to_string())
                                .await
                            {
                                tracing::warn!(
                                    claim_id = %claim.claim_id,
                                    error = %error,
                                    "work reconciler: claim projection retry failed"
                                );
                            } else {
                                report.labels_set += 1;
                            }
                            reconcile_run_state(state, &project, &claim, &mut report).await;
                        }
                        ClaimState::AwaitingMerge => {
                            // The label is removed at run end; the PR carries
                            // the pending state.
                            if let Err(error) = tracker
                                .clear_claim(&project, &claim.item_key, &claim.run_id.to_string())
                                .await
                            {
                                tracing::warn!(
                                    claim_id = %claim.claim_id,
                                    error = %error,
                                    "work reconciler: claim clear retry failed"
                                );
                            } else {
                                report.labels_cleared += 1;
                            }
                        }
                        ClaimState::Landed | ClaimState::Released => {}
                    }
                }
            }
            Err(error) => {
                tracing::warn!(
                    project = %project.project_id,
                    error = %error,
                    "work reconciler: claim read failed"
                );
            }
        }

        match tracker.gc_claim_labels(&project).await {
            Ok(removed) => report.labels_gc += removed,
            Err(error) => tracing::warn!(
                project = %project.project_id,
                error = %error,
                "work reconciler: label gc failed"
            ),
        }
    }
    report
}

/// Apply due outbox jobs; failures reschedule with linear backoff.
async fn drain_outbox(
    state: &AppState,
    project: &fabro_work::WorkProject,
    tracker: &dyn TrackerProvider,
) -> usize {
    let mut applied = 0;
    let Ok(due) = state.work_store().due(Utc::now(), OUTBOX_BATCH).await else {
        return applied;
    };
    for entry in due {
        let item_key = entry
            .payload
            .get("item_key")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        let run_id = entry
            .payload
            .get("run_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        let outcome = match entry.kind.as_str() {
            "claim.projected" if !item_key.is_empty() && !run_id.is_empty() => {
                tracker.project_claim(project, &item_key, &run_id).await
            }
            "claim.cleared" if !item_key.is_empty() && !run_id.is_empty() => {
                tracker.clear_claim(project, &item_key, &run_id).await
            }
            other => {
                tracing::warn!(
                    kind = other,
                    "work reconciler: unknown outbox kind; completing"
                );
                Ok(())
            }
        };
        match outcome {
            Ok(()) => {
                if state.work_store().complete(&entry.id).await.is_ok() {
                    applied += 1;
                }
            }
            Err(error) => {
                let retry_at = Utc::now() + Duration::seconds((entry.attempts + 1) * 60);
                if let Err(fail_error) = state
                    .work_store()
                    .fail(&entry.id, &error.to_string(), retry_at)
                    .await
                {
                    tracing::warn!(error = %fail_error, "work reconciler: outbox fail bookkeeping");
                }
            }
        }
    }
    applied
}

/// Run-status truth for a live claim: terminal runs move to
/// `awaiting_merge` (PR) or release (no PR); a missing run releases after
/// the grace window.
async fn reconcile_run_state(
    state: &AppState,
    project: &fabro_work::WorkProject,
    claim: &fabro_work::WorkClaim,
    report: &mut ReconcileReport,
) {
    let Some(projection) = super::run_records::projection(state, claim.run_id)
        .await
        .ok()
        .flatten()
    else {
        // A run the store cannot serve: release after the grace window.
        if claim.updated_at < Utc::now() - ORPHAN_GRACE
            && state
                .work_store()
                .release(&claim.claim_id, "orphan: run record gone")
                .await
                .is_ok()
        {
            report.orphans_released += 1;
        }
        return;
    };
    if !projection.status.is_terminal() {
        return;
    }
    let pr_url = projection
        .pull_request
        .as_ref()
        .map(fabro_types::PullRequestLink::html_url);
    let _ = project;
    if let Some(pr_url) = pr_url {
        if state
            .work_store()
            .await_merge(&claim.claim_id, &pr_url)
            .await
            .is_ok()
        {
            report.terminal_moved += 1;
        }
    } else if state
        .work_store()
        .release(&claim.claim_id, "run terminal without a pull request")
        .await
        .is_ok()
    {
        report.orphans_released += 1;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use chrono::{Duration, Utc};
    use fabro_work::{WorkItem, WorkItemDraft, WorkItemFilter, WorkProject};
    use tokio::sync::Mutex;

    use super::*;
    use crate::server::AppState;
    use crate::test_support::TestAppStateBuilder;

    #[derive(Default)]
    struct ScriptedTracker {
        claims:  Mutex<Vec<(String, String)>>,
        cleared: Mutex<Vec<(String, String)>>,
        closed:  Mutex<Vec<String>>,
    }

    #[async_trait]
    impl fabro_work::TrackerProvider for ScriptedTracker {
        async fn list(
            &self,
            _project: &WorkProject,
            _filter: WorkItemFilter,
        ) -> anyhow::Result<Vec<WorkItem>> {
            Ok(Vec::new())
        }

        async fn get(
            &self,
            _project: &WorkProject,
            _item_key: &str,
        ) -> anyhow::Result<Option<WorkItem>> {
            Ok(None)
        }

        async fn create(
            &self,
            _project: &WorkProject,
            _draft: WorkItemDraft,
        ) -> anyhow::Result<WorkItem> {
            anyhow::bail!("not needed")
        }

        async fn update(
            &self,
            _project: &WorkProject,
            _item_key: &str,
            _title: Option<&str>,
            _body: Option<&str>,
        ) -> anyhow::Result<WorkItem> {
            anyhow::bail!("not needed")
        }

        async fn close(
            &self,
            _project: &WorkProject,
            item_key: &str,
            _reason: &str,
        ) -> anyhow::Result<()> {
            self.closed.lock().await.push(item_key.to_string());
            Ok(())
        }

        async fn comment(
            &self,
            _project: &WorkProject,
            _item_key: &str,
            _body: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn project_claim(
            &self,
            _project: &WorkProject,
            item_key: &str,
            run_id: &str,
        ) -> anyhow::Result<()> {
            self.claims
                .lock()
                .await
                .push((item_key.to_string(), run_id.to_string()));
            Ok(())
        }

        async fn clear_claim(
            &self,
            _project: &WorkProject,
            item_key: &str,
            run_id: &str,
        ) -> anyhow::Result<()> {
            self.cleared
                .lock()
                .await
                .push((item_key.to_string(), run_id.to_string()));
            Ok(())
        }
    }

    async fn state_with(tracker: Arc<ScriptedTracker>) -> (Arc<AppState>, fabro_work::WorkStore) {
        let state = TestAppStateBuilder::new()
            .work_tracker(tracker as Arc<dyn fabro_work::TrackerProvider>)
            .build();
        let store = fabro_work::WorkStore::new(state.db_pool.clone());
        store
            .upsert_project(&fabro_work::WorkProject {
                project_id:               "fabro".to_string(),
                display_name:             "Fabro".to_string(),
                tracker_provider:         "github".to_string(),
                tracker_repo:             "denkhaus/work".to_string(),
                target_repo:              None,
                project_label:            "project:fabro".to_string(),
                lane_labels:              [("loop".to_string(), "lane:loop".to_string())]
                    .into_iter()
                    .collect(),
                claim_label_prefix:       "fabro:run:".to_string(),
                claim_comment_enabled:    false,
                backpressure_open_pr_cap: 0,
            })
            .await
            .expect("project upserts");
        (state, store)
    }

    #[tokio::test]
    async fn a_pass_drains_the_outbox_and_projects_live_claims() {
        let tracker = Arc::new(ScriptedTracker::default());
        let (state, store) = state_with(Arc::clone(&tracker)).await;
        let run_id = fabro_types::RunId::new();
        let granted = store
            .grant(&fabro_work::GrantRequest {
                project_id: "fabro".to_string(),
                item_key: "denkhaus/work#7".to_string(),
                run_id,
                lane: "loop".to_string(),
            })
            .await
            .expect("grant");
        let fabro_work::GrantOutcome::Granted(claim) = granted else {
            panic!("grant wins");
        };
        // Simulate the failed projection: the outbox holds the retry.
        let due_before = store
            .due(Utc::now() + Duration::seconds(1), 10)
            .await
            .unwrap();
        assert_eq!(due_before.len(), 1);

        let report = run_pass(&state).await;
        assert_eq!(report.outbox_applied, 1);
        assert!(
            store
                .due(Utc::now() + Duration::minutes(10), 10)
                .await
                .unwrap()
                .is_empty()
        );

        let claims = tracker.claims.lock().await;
        assert!(
            claims
                .iter()
                .any(|(item, run)| item == "denkhaus/work#7" && run == &claim.run_id.to_string())
        );
        // The DB-first claim survives with its projection healed.
        assert_eq!(
            store.claim(&claim.claim_id).await.unwrap().unwrap().state,
            fabro_work::ClaimState::Granted
        );
    }

    #[tokio::test]
    async fn a_missing_run_releases_its_claim_only_after_the_grace_window() {
        let tracker = Arc::new(ScriptedTracker::default());
        let (state, store) = state_with(Arc::clone(&tracker)).await;
        let run_id = fabro_types::RunId::new();
        let granted = store
            .grant(&fabro_work::GrantRequest {
                project_id: "fabro".to_string(),
                item_key: "denkhaus/work#8".to_string(),
                run_id,
                lane: "loop".to_string(),
            })
            .await
            .expect("grant");
        let fabro_work::GrantOutcome::Granted(claim) = granted else {
            panic!("grant wins");
        };

        // Fresh claim: the pass must NOT release (grace window).
        let report = run_pass(&state).await;
        assert_eq!(report.orphans_released, 0);
        assert_eq!(
            store.claim(&claim.claim_id).await.unwrap().unwrap().state,
            fabro_work::ClaimState::Granted
        );

        // Age it past the grace window: the pass releases the orphan.
        let old = (Utc::now() - Duration::minutes(31)).timestamp_millis();
        sqlx::query("UPDATE work_claims SET updated_at_ms = ? WHERE claim_id = ?")
            .bind(old)
            .bind(&claim.claim_id)
            .execute(&state.db_pool)
            .await
            .expect("age the claim");

        let report = run_pass(&state).await;
        assert_eq!(report.orphans_released, 1);
        let released = store.claim(&claim.claim_id).await.unwrap().unwrap();
        assert_eq!(released.state, fabro_work::ClaimState::Released);
        assert_eq!(
            released.release_reason.as_deref(),
            Some("orphan: run record gone")
        );
    }
}
