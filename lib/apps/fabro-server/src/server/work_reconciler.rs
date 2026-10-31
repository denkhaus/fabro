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
use fabro_types::RunId;
use fabro_work::{ClaimState, TrackerProvider};
use tokio::task::JoinHandle;
use tokio::time::{MissedTickBehavior, interval};

use crate::observation_filing;
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
        // The DB is the authority, not the enqueue-time payload snapshot:
        // grant payloads carry item_key, transition payloads do not (only
        // claim_id + state) — both shapes heal from the claim row, whose
        // CURRENT state names the tracker op. Fabro-ba22: the payload-only
        // dispatch dropped transition entries as "unknown" because they
        // lack item_key.
        let claim_id = entry
            .payload
            .get("claim_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        let run_id: Option<RunId> = entry
            .payload
            .get("run_id")
            .and_then(|value| value.as_str())
            .and_then(|value| value.parse().ok());
        let outcome = match entry.kind.as_str() {
            "claim.projected" | "claim.cleared" if !claim_id.is_empty() => {
                match state.work_store().claim(&claim_id).await {
                    Ok(Some(claim))
                        if matches!(claim.state, ClaimState::Granted | ClaimState::Active) =>
                    {
                        tracker
                            .project_claim(project, &claim.item_key, &claim.run_id.to_string())
                            .await
                    }
                    Ok(Some(claim)) => {
                        // awaiting_merge / landed / released: the label
                        // must not outlive the claim.
                        tracker
                            .clear_claim(project, &claim.item_key, &claim.run_id.to_string())
                            .await
                    }
                    // Claim row gone: nothing to project — label GC owns
                    // the tracker side. Complete.
                    Ok(None) => Ok(()),
                    Err(error) => Err(anyhow::anyhow!(error)),
                }
            }
            // The run-end observation filing (ADR-0038 stage 4): the arm
            // resolves the run's own project and tracker, so it ignores
            // the loop's — an entry filed for another project's run is
            // still processed exactly once, here.
            observation_filing::OUTBOX_KIND => {
                if let Some(run_id) = run_id {
                    observation_filing::file_run_observations(state, &run_id)
                        .await
                        .map(|outcome| {
                            tracing::debug!(run_id = %run_id, ?outcome, "observation filing pass");
                        })
                } else {
                    tracing::warn!(
                        id = %entry.id,
                        "observation filing entry carries no run_id; completing"
                    );
                    Ok(())
                }
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
                // The filing hook's own friction rides the same channel:
                // from the second failed attempt on, one positionless
                // painpoint names the failing pipeline on the run itself
                // (ADR-0038 stage 4) — the retry re-reads and files it.
                if entry.kind == observation_filing::OUTBOX_KIND && entry.attempts >= 1 {
                    if let Some(run_id) = run_id {
                        observation_filing::record_friction_once(
                            state,
                            run_id,
                            &entry.id,
                            &error.to_string(),
                        )
                        .await;
                    }
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
        claims:                  Mutex<Vec<(String, String)>>,
        cleared:                 Mutex<Vec<(String, String)>>,
        closed:                  Mutex<Vec<String>>,
        /// When set, the next project_claim call fails once (retry pin).
        fail_project_claim_once: std::sync::atomic::AtomicBool,
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
            if self
                .fail_project_claim_once
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                anyhow::bail!("scripted projection failure");
            }
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

    // Fabro-ba22 regression: transition enqueues carry no item_key (only
    // claim_id + run_id + state) — the payload-only dispatch used to
    // complete them as "unknown outbox kind" without any tracker call.
    // The claim row is the authority: its CURRENT state names the op.
    #[tokio::test]
    async fn transition_shaped_outbox_entries_heal_from_the_claim_row() {
        let tracker = Arc::new(ScriptedTracker::default());
        let (state, store) = state_with(Arc::clone(&tracker)).await;
        let run_id = fabro_types::RunId::new();
        let granted = store
            .grant(&fabro_work::GrantRequest {
                project_id: "fabro".to_string(),
                item_key: "denkhaus/work#9".to_string(),
                run_id,
                lane: "loop".to_string(),
            })
            .await
            .expect("grant");
        let fabro_work::GrantOutcome::Granted(claim) = granted else {
            panic!("grant wins");
        };
        // The grant entry drains first (payload carries item_key).
        drain_outbox(
            &state,
            &store.projects().await.unwrap()[0],
            tracker.as_ref(),
        )
        .await;
        tracker.claims.lock().await.clear();

        // Activate: the transition enqueues claim.projected WITHOUT
        // item_key — the shape the old dispatch dropped as unknown.
        store.activate(&claim.claim_id).await.expect("activate");
        let due = store
            .due(Utc::now() + Duration::seconds(1), 10)
            .await
            .unwrap();
        assert_eq!(due.len(), 1);
        assert!(
            due[0].payload.get("item_key").is_none(),
            "transition payload must not need item_key: {:?}",
            due[0].payload
        );

        let applied = drain_outbox(
            &state,
            &store.projects().await.unwrap()[0],
            tracker.as_ref(),
        )
        .await;
        assert_eq!(applied, 1);
        assert!(
            store
                .due(Utc::now() + Duration::minutes(10), 10)
                .await
                .unwrap()
                .is_empty(),
            "entry completed"
        );
        let claims = tracker.claims.lock().await;
        assert!(
            claims
                .iter()
                .any(|(item, run)| item == "denkhaus/work#9" && run == &run_id.to_string()),
            "the tracker saw the projection: {claims:?}"
        );
    }

    // The seed's pin: a projection that fails once retries with backoff
    // and heals on the next drain.
    #[tokio::test]
    async fn a_failed_projection_retries_with_backoff_and_heals() {
        let tracker = Arc::new(ScriptedTracker::default());
        tracker
            .fail_project_claim_once
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let (state, store) = state_with(Arc::clone(&tracker)).await;
        let run_id = fabro_types::RunId::new();
        let granted = store
            .grant(&fabro_work::GrantRequest {
                project_id: "fabro".to_string(),
                item_key: "denkhaus/work#10".to_string(),
                run_id,
                lane: "loop".to_string(),
            })
            .await
            .expect("grant");
        let fabro_work::GrantOutcome::Granted(claim) = granted else {
            panic!("grant wins");
        };
        let project = &store.projects().await.unwrap()[0];

        // First drain: the scripted failure fails the entry (backoff,
        // not completion).
        let applied = drain_outbox(&state, project, tracker.as_ref()).await;
        assert_eq!(applied, 0, "nothing applied on failure");
        assert!(tracker.claims.lock().await.is_empty(), "no label was set");
        let entry = store
            .outbox_entry(
                &store
                    .due(Utc::now() + Duration::minutes(10), 10)
                    .await
                    .unwrap()[0]
                    .id,
            )
            .await
            .unwrap()
            .expect("entry survives the failure");
        assert_eq!(entry.attempts, 1);
        assert_eq!(
            entry.last_error.as_deref(),
            Some("scripted projection failure")
        );

        // Fast-forward the backoff window, then drain again: heals.
        sqlx::query("UPDATE work_outbox SET next_attempt_at_ms = ? WHERE id = ?")
            .bind((Utc::now() - Duration::seconds(1)).timestamp_millis())
            .bind(&entry.id)
            .execute(&state.db_pool)
            .await
            .expect("force the retry due");
        let applied = drain_outbox(&state, project, tracker.as_ref()).await;
        assert_eq!(applied, 1);
        let claims = tracker.claims.lock().await;
        assert!(
            claims
                .iter()
                .any(|(item, run)| item == "denkhaus/work#10" && run == &claim.run_id.to_string()),
            "the label healed after the retry: {claims:?}"
        );
        assert!(
            store
                .due(Utc::now() + Duration::minutes(10), 10)
                .await
                .unwrap()
                .is_empty(),
            "outbox drained"
        );
    }
}
