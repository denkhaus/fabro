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

use std::collections::HashMap;
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
    // The acceptance fold's verdict maps (denkhaus/work#1555): read
    // once per pass, shared by every claim of every project — the
    // per-run `latest_verdict_of_run` scans the records table per call,
    // so a per-claim read would pay a full scan per row. A read failure
    // degrades to EMPTY maps: without verdicts nothing releases (the
    // hold posture — a failed read never frees an item), and the next
    // pass re-reads.
    let (merge_verdicts, closable_verdicts) = match super::acceptance::verdict_maps(state).await {
        Ok(maps) => maps,
        Err(error) => {
            tracing::warn!(
                error = ?error,
                "work reconciler: verdict map read failed; claims hold this pass"
            );
            Default::default()
        }
    };
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
                            reconcile_run_state(
                                state,
                                &project,
                                &claim,
                                (&merge_verdicts, &closable_verdicts),
                                &mut report,
                            )
                            .await;
                        }
                        ClaimState::AwaitingMerge => {
                            // The PR state of the chain, read through the
                            // acceptance fold (denkhaus/work#1555): the
                            // refused arm releases exactly the fold's
                            // `merge_refused` state, and the
                            // `closed_unlanded` arm heals the stuck
                            // window — an item the tracker closed while
                            // this claim still holds it must not pin the
                            // item forever (the close verb's two-phase
                            // failure window, work#1463). Integration
                            // evidence (a `closable` verdict, an
                            // observed-merged PR) outranks a stale
                            // refusal INSIDE the fold, so an externally
                            // merged PR holds for the closing pass
                            // instead of releasing unlanded.
                            let fold = match super::acceptance::live_claim_fold(
                                state,
                                &claim,
                                (&merge_verdicts, &closable_verdicts),
                                None,
                            )
                            .await
                            {
                                Ok(fold) => fold,
                                Err(error) => {
                                    tracing::warn!(
                                        claim_id = %claim.claim_id,
                                        error = ?error,
                                        "work reconciler: acceptance fold read failed; claim holds"
                                    );
                                    continue;
                                }
                            };
                            match fold {
                                fabro_work::AcceptanceState::MergeRefused => {
                                    match state
                                        .work_store()
                                        .release(
                                            &claim.claim_id,
                                            "merge refused: the adjudicator rejected the run",
                                        )
                                        .await
                                    {
                                        Ok(_released) => report.orphans_released += 1,
                                        Err(error) => tracing::warn!(
                                            claim_id = %claim.claim_id,
                                            error = ?error,
                                            "work reconciler: refused-verdict release failed"
                                        ),
                                    }
                                    continue;
                                }
                                fabro_work::AcceptanceState::ClosedUnlanded => {
                                    match state
                                        .work_store()
                                        .release(
                                            &claim.claim_id,
                                            "item closed: healing the claim hold the close left behind",
                                        )
                                        .await
                                    {
                                        Ok(_released) => report.orphans_released += 1,
                                        Err(error) => tracing::warn!(
                                            claim_id = %claim.claim_id,
                                            error = ?error,
                                            "work reconciler: stuck-window release failed"
                                        ),
                                    }
                                    continue;
                                }
                                _ => {}
                            }
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
            // The run-end observation filing arm is GONE (ADR-0039: the
            // hook stripped before any next window; the adjudicator
            // owns filing). Entries of its kind still queued in a
            // production database complete through the unknown-kind
            // arm below, with a warning.
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
                // The full chain, not the outermost context: a nested
                // failure must name its cause in the outbox's durable
                // health view.
                let chain = error
                    .chain()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(": ");
                if let Err(fail_error) = state.work_store().fail(&entry.id, &chain, retry_at).await
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
/// the grace window. The no-PR decision reads the acceptance fold
/// ([`fabro_work::acceptance_fold`], denkhaus/work#1555): the refused
/// arm (`merge_refused`) releases, and `closed_unlanded` — an item the
/// tracker closed while this claim still holds — releases as the stuck
/// window's healing. Every other fold state holds, and the claim-hold
/// seam ([`fabro_work::hold_reason`], denkhaus/work#1457) names WHY
/// (the park pin included, ADR-0045 P1). On this path (live claim,
/// stored projection, terminal run, no pull request) fold and hold seam
/// cannot disagree: the hold seam releases exactly the refused verdict,
/// which is exactly the fold's refused arm. The orphan grace above
/// stays OUTSIDE the fold by design — a gone run record is not an
/// acceptance row (the hold seam's module doc carries the same note).
async fn reconcile_run_state(
    state: &AppState,
    project: &fabro_work::WorkProject,
    claim: &fabro_work::WorkClaim,
    verdicts: (
        &HashMap<fabro_types::RunId, fabro_types::OrchestrationVerdict>,
        &HashMap<fabro_types::RunId, fabro_types::OrchestrationVerdict>,
    ),
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
        return;
    }
    // A terminal run WITHOUT a pull request: the acceptance fold names
    // the chain state; its release arms fire, the hold seam names the
    // hold.
    let fold =
        match super::acceptance::live_claim_fold(state, claim, verdicts, Some(projection.as_ref()))
            .await
        {
            Ok(fold) => fold,
            Err(error) => {
                tracing::warn!(
                    claim_id = %claim.claim_id,
                    error = ?error,
                    "work reconciler: acceptance fold read failed; claim holds"
                );
                return;
            }
        };
    match fold {
        fabro_work::AcceptanceState::MergeRefused => {
            release_verdict(
                state,
                claim,
                "merge refused: the adjudicator rejected the run",
                report,
            )
            .await;
        }
        fabro_work::AcceptanceState::ClosedUnlanded => {
            release_verdict(
                state,
                claim,
                "item closed: healing the claim hold the close left behind",
                report,
            )
            .await;
        }
        other => match fabro_work::hold_reason(
            claim,
            projection.status,
            verdicts.0.get(&claim.run_id).copied(),
        ) {
            Some(reason) => {
                tracing::debug!(
                    claim_id = %claim.claim_id,
                    run_id = %claim.run_id,
                    acceptance = %other,
                    hold = %reason,
                    "work reconciler: terminal run without a pull request holds its claim"
                );
            }
            None => {
                release_verdict(
                    state,
                    claim,
                    "merge refused: the adjudicator rejected the run",
                    report,
                )
                .await;
            }
        },
    }
}

/// The refused / stuck-window release, shared by the fold's release
/// arms: release the claim with the named reason and count it.
async fn release_verdict(
    state: &AppState,
    claim: &fabro_work::WorkClaim,
    reason: &str,
    report: &mut ReconcileReport,
) {
    match state.work_store().release(&claim.claim_id, reason).await {
        Ok(_released) => report.orphans_released += 1,
        Err(error) => tracing::warn!(
            claim_id = %claim.claim_id,
            error = ?error,
            "work reconciler: fold release failed"
        ),
    }
}

/// The run's latest verdict on the `merge` field, when one exists (the
/// same fold the queue reads). Shared with the claim pool's hold arm
/// (`claim_next`, denkhaus/work#1457) so every consumer folds the same
/// way.
pub(crate) async fn merge_verdict_of_run(
    state: &AppState,
    run_id: fabro_types::RunId,
) -> Option<fabro_types::OrchestrationVerdict> {
    state
        .stores
        .run_summaries
        .platform_records()
        .latest_verdict_of_run(fabro_types::OrchestrationField::Merge, &run_id)
        .await
        .ok()
        .flatten()
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

    /// The verdict-driven release (ADR-0039 stage E): a terminal run
    /// WITHOUT a pull request no longer releases its claim at
    /// run-terminal — the claim holds until the adjudicator's verdict.
    /// `merge: refused` releases (the adjudicator rejected the run);
    /// no verdict yet holds (the adjudicator will verdict; the lock span
    /// is claim to merge-complete).
    #[tokio::test]
    async fn a_refused_run_releases_its_claim_and_an_unadjudicated_one_holds() {
        let tracker = Arc::new(ScriptedTracker::default());
        let (state, store) = state_with(Arc::clone(&tracker)).await;
        let refused_run = fabro_types::RunId::new();
        let pending_run = fabro_types::RunId::new();
        for run_id in [refused_run, pending_run] {
            let granted = store
                .grant(&fabro_work::GrantRequest {
                    project_id: "fabro".to_string(),
                    item_key: format!("denkhaus/work/{run_id}"),
                    run_id,
                    lane: "loop".to_string(),
                })
                .await
                .expect("grant");
            assert!(
                matches!(granted, fabro_work::GrantOutcome::Granted(_)),
                "the grant wins"
            );
            // A terminal run row WITHOUT a pull request (the projection
            // the reconciler reads).
            let mut projection = fabro_types::RunProjection::new(
                "failed run".to_string(),
                fabro_types::RunSpec {
                    run_id,
                    settings: fabro_types::WorkflowSettings::default(),
                    graph: fabro_types::RunGraph::new("test"),
                    graph_source: None,
                    workflow_slug: Some("develop".to_string()),
                    workflow_version_id: None,
                    target: None,
                    automation: None,
                    source_directory: None,
                    labels: std::collections::HashMap::new(),
                    provenance: fabro_types::test_support::test_run_provenance(),
                    definition_blob: None,
                    spec_blob: None,
                    git: None,
                    fork_source_ref: None,
                    admission: fabro_types::PetriAdmission::default(),
                },
                chrono::Utc::now(),
            );
            projection.status = fabro_types::RunStatus::Failed {
                reason: fabro_types::FailureReason::WorkflowError,
            };
            let pool = state.stores.run_summaries.pool();
            let mut tx = pool.begin().await.expect("seed tx");
            fabro_store::RunSummaryStore::write_petri_run_row_on_connection(
                &mut tx,
                &run_id,
                &projection,
            )
            .await
            .expect("run row seed");
            let projection_json = serde_json::to_string(&projection).expect("projection json");
            sqlx::query(
                "INSERT INTO petri_projection (run_id, projection_json, fold_json, \
                 positions_json, stream_seq, updated_at_ms) VALUES (?, ?, '{}', '{}', 0, 0) \
                 ON CONFLICT(run_id) DO UPDATE SET projection_json = excluded.projection_json",
            )
            .bind(run_id.to_string())
            .bind(projection_json)
            .execute(&mut *tx)
            .await
            .expect("projection row seed");
            tx.commit().await.expect("seed commit");
        }

        // The adjudicator refused ONE of the two runs.
        state
            .stores
            .run_summaries
            .platform_records()
            .append(
                &refused_run,
                &fabro_store::platform_records::PlatformRecord::RunOrchestration(
                    fabro_store::platform_records::RunOrchestrationRecord {
                        field:   fabro_types::OrchestrationField::Merge,
                        verdict: fabro_types::OrchestrationVerdict::Refused,
                        by_run:  None,
                    },
                ),
                None,
            )
            .await
            .expect("verdict seed");

        let _ = run_pass(&state).await;

        let active = store
            .active_claims("fabro")
            .await
            .expect("active claims")
            .into_iter()
            .map(|claim| claim.run_id)
            .collect::<Vec<_>>();
        assert!(
            active.contains(&pending_run),
            "the unadjudicated run's claim HOLDS until the adjudicator verdicts"
        );
        assert!(
            !active.contains(&refused_run),
            "the refused run's claim released on the verdict"
        );
    }

    /// Owner park pinning (work#1345): a failed run dragged to blocked is
    /// `Blocked { owner_pending }` — NON-terminal — so the reconciler
    /// holds its claim indefinitely: no other run can pick the item while
    /// the owner signals retry intent. The park, not the verdict, decides.
    #[tokio::test]
    async fn an_owner_parked_run_pins_its_claim_across_passes() {
        let tracker = Arc::new(ScriptedTracker::default());
        let (state, store) = state_with(Arc::clone(&tracker)).await;
        let parked_run = fabro_types::RunId::new();
        let granted = store
            .grant(&fabro_work::GrantRequest {
                project_id: "fabro".to_string(),
                item_key:   format!("denkhaus/work/{parked_run}"),
                run_id:     parked_run,
                lane:       "loop".to_string(),
            })
            .await
            .expect("grant");
        assert!(matches!(granted, fabro_work::GrantOutcome::Granted(_)));

        let mut projection = fabro_types::RunProjection::new(
            "owner-parked run".to_string(),
            fabro_types::RunSpec {
                run_id:              parked_run,
                settings:            fabro_types::WorkflowSettings::default(),
                graph:               fabro_types::RunGraph::new("test"),
                graph_source:        None,
                workflow_slug:       Some("develop".to_string()),
                workflow_version_id: None,
                target:              None,
                automation:          None,
                source_directory:    None,
                labels:              std::collections::HashMap::new(),
                provenance:          fabro_types::test_support::test_run_provenance(),
                definition_blob:     None,
                spec_blob:           None,
                git:                 None,
                fork_source_ref:     None,
                admission:           fabro_types::PetriAdmission::default(),
            },
            chrono::Utc::now(),
        );
        projection.status = fabro_types::RunStatus::Blocked {
            blocked_reason: fabro_types::BlockedReason::OwnerPending,
        };
        projection.conclusion = Some(fabro_types::Conclusion {
            timestamp:            chrono::Utc::now(),
            status:               fabro_types::outcome::StageOutcome::Failed {
                retry_requested: false,
            },
            timing:               fabro_types::RunTiming::default(),
            failure:              Some(fabro_types::RunFailure {
                reason: fabro_types::FailureReason::WorkflowError,
                detail: fabro_types::outcome::FailureDetail::new(
                    "workflow error",
                    fabro_types::outcome::FailureCategory::Deterministic,
                ),
            }),
            final_git_commit_sha: None,
            stages:               Vec::new(),
            usage:                None,
            diff:                 fabro_types::RunDiff::default(),
            total_retries:        0,
        });
        let pool = state.stores.run_summaries.pool();
        let mut tx = pool.begin().await.expect("seed tx");
        fabro_store::RunSummaryStore::write_petri_run_row_on_connection(
            &mut tx,
            &parked_run,
            &projection,
        )
        .await
        .expect("run row seed");
        let projection_json = serde_json::to_string(&projection).expect("projection json");
        sqlx::query(
            "INSERT INTO petri_projection (run_id, projection_json, fold_json, \
             positions_json, stream_seq, updated_at_ms) VALUES (?, ?, '{}', '{}', 0, 0) \
             ON CONFLICT(run_id) DO UPDATE SET projection_json = excluded.projection_json",
        )
        .bind(parked_run.to_string())
        .bind(projection_json)
        .execute(&mut *tx)
        .await
        .expect("projection row seed");
        tx.commit().await.expect("seed commit");

        // Two passes: the pin holds — the reconciler neither releases the
        // claim nor moves it to awaiting_merge while the park stands.
        let _ = run_pass(&state).await;
        let _ = run_pass(&state).await;

        let active = store.active_claims("fabro").await.expect("active claims");
        assert!(
            active.iter().any(|claim| claim.run_id == parked_run),
            "the owner-parked run's claim stays pinned across reconciler passes"
        );
        let claim = store
            .active_claim_for_run(&parked_run)
            .await
            .expect("claim read")
            .expect("claim still live");
        assert_ne!(
            claim.state,
            fabro_work::ClaimState::Released,
            "the parked run's claim never releases while parked"
        );
    }

    /// The fold-driven refused arm over an `awaiting_merge` claim
    /// (denkhaus/work#1555): the pass releases exactly the fold's
    /// `merge_refused` state — read through the acceptance fold, not a
    /// raw verdict comparison.
    #[tokio::test]
    async fn the_pass_releases_an_awaiting_merge_claim_the_fold_reads_refused() {
        let tracker = Arc::new(ScriptedTracker::default());
        let (state, store) = state_with(Arc::clone(&tracker)).await;
        let run_id = fabro_types::RunId::new();
        let granted = store
            .grant(&fabro_work::GrantRequest {
                project_id: "fabro".to_string(),
                item_key: "denkhaus/work#1555".to_string(),
                run_id,
                lane: "loop".to_string(),
            })
            .await
            .expect("grant");
        let fabro_work::GrantOutcome::Granted(claim) = granted else {
            panic!("grant wins");
        };
        store.activate(&claim.claim_id).await.expect("activate");
        let pr_url = "https://github.com/denkhaus/fabro/pull/1555";
        store
            .await_merge(&claim.claim_id, pr_url)
            .await
            .expect("await merge");

        // A terminal run row WITH the open pull request.
        seed_projection(&state, run_id, "refused PR run", Some((pr_url, None))).await;
        // The adjudicator refused the run.
        append_verdict(
            &state,
            &run_id,
            fabro_types::OrchestrationField::Merge,
            fabro_types::OrchestrationVerdict::Refused,
        )
        .await;

        let report = run_pass(&state).await;
        assert_eq!(
            report.orphans_released, 1,
            "the fold's refused arm released"
        );
        let released = store.claim(&claim.claim_id).await.unwrap().unwrap();
        assert_eq!(released.state, fabro_work::ClaimState::Released);
        assert_eq!(
            released.release_reason.as_deref(),
            Some("merge refused: the adjudicator rejected the run")
        );
    }

    /// Integration evidence outranks a stale refusal INSIDE the fold
    /// (denkhaus/work#1555): a PR the platform observed MERGED holds
    /// for the closing pass even when the adjudicator's verdict said
    /// refused — the work is in the target; releasing would strand the
    /// land bookkeeping.
    #[tokio::test]
    async fn an_externally_merged_pr_survives_a_stale_refusal() {
        let tracker = Arc::new(ScriptedTracker::default());
        let (state, store) = state_with(Arc::clone(&tracker)).await;
        let run_id = fabro_types::RunId::new();
        let granted = store
            .grant(&fabro_work::GrantRequest {
                project_id: "fabro".to_string(),
                item_key: "denkhaus/work#1556".to_string(),
                run_id,
                lane: "loop".to_string(),
            })
            .await
            .expect("grant");
        let fabro_work::GrantOutcome::Granted(claim) = granted else {
            panic!("grant wins");
        };
        store.activate(&claim.claim_id).await.expect("activate");
        let pr_url = "https://github.com/denkhaus/fabro/pull/1556";
        store
            .await_merge(&claim.claim_id, pr_url)
            .await
            .expect("await merge");

        seed_projection(
            &state,
            run_id,
            "externally merged PR run",
            Some((pr_url, Some(fabro_types::PullRequestState::Merged))),
        )
        .await;
        append_verdict(
            &state,
            &run_id,
            fabro_types::OrchestrationField::Merge,
            fabro_types::OrchestrationVerdict::Refused,
        )
        .await;

        let _ = run_pass(&state).await;
        let held = store.claim(&claim.claim_id).await.unwrap().unwrap();
        assert_eq!(
            held.state,
            fabro_work::ClaimState::AwaitingMerge,
            "the observed merge outranks the stale refusal: the claim holds for the closing pass"
        );
    }

    /// The stuck window heals (denkhaus/work#1555): an item the tracker
    /// closed while a claim still holds it — the close verb's two-phase
    /// failure window, the #1463 class — releases with the healing
    /// reason instead of pinning the item forever.
    #[tokio::test]
    async fn a_closed_item_with_a_live_claim_releases_as_the_stuck_window() {
        let tracker = Arc::new(ScriptedTracker::default());
        let (state, store) = state_with(Arc::clone(&tracker)).await;
        let run_id = fabro_types::RunId::new();
        let granted = store
            .grant(&fabro_work::GrantRequest {
                project_id: "fabro".to_string(),
                item_key: "denkhaus/work#1463".to_string(),
                run_id,
                lane: "loop".to_string(),
            })
            .await
            .expect("grant");
        let fabro_work::GrantOutcome::Granted(claim) = granted else {
            panic!("grant wins");
        };
        store.activate(&claim.claim_id).await.expect("activate");

        // Terminal run, no pull request, no verdict: without the closed
        // item the fold would hold (adjudication pending).
        seed_projection(&state, run_id, "stuck window run", None).await;
        // The view store's row says the item CLOSED — the tracker-side
        // truth the close verb committed before its claim half failed.
        state
            .stores
            .work_item_views
            .put("fabro", &fabro_work::WorkItem {
                item_key:   "denkhaus/work#1463".to_string(),
                title:      "the stuck item".to_string(),
                body:       String::new(),
                state:      "closed".to_string(),
                labels:     Vec::new(),
                priority:   0,
                url:        "https://github.com/denkhaus/work/issues/1463".to_string(),
                updated_at: None,
                created_at: None,
                closed_at:  Some(chrono::Utc::now()),
            })
            .await
            .expect("view row seed");

        let report = run_pass(&state).await;
        assert_eq!(report.orphans_released, 1, "the stuck window released");
        let released = store.claim(&claim.claim_id).await.unwrap().unwrap();
        assert_eq!(released.state, fabro_work::ClaimState::Released);
        assert_eq!(
            released.release_reason.as_deref(),
            Some("item closed: healing the claim hold the close left behind")
        );
    }

    /// Seed one terminal run projection, optionally carrying a pull
    /// request link with an observed state.
    async fn seed_projection(
        state: &AppState,
        run_id: fabro_types::RunId,
        title: &str,
        pr: Option<(&str, Option<fabro_types::PullRequestState>)>,
    ) {
        let mut projection = fabro_types::RunProjection::new(
            title.to_string(),
            fabro_types::RunSpec {
                run_id,
                settings: fabro_types::WorkflowSettings::default(),
                graph: fabro_types::RunGraph::new("test"),
                graph_source: None,
                workflow_slug: Some("develop".to_string()),
                workflow_version_id: None,
                target: None,
                automation: None,
                source_directory: None,
                labels: std::collections::HashMap::new(),
                provenance: fabro_types::test_support::test_run_provenance(),
                definition_blob: None,
                spec_blob: None,
                git: None,
                fork_source_ref: None,
                admission: fabro_types::PetriAdmission::default(),
            },
            chrono::Utc::now(),
        );
        projection.status = fabro_types::RunStatus::Failed {
            reason: fabro_types::FailureReason::WorkflowError,
        };
        if let Some((url, pr_state)) = pr {
            let (owner, repo, number) = parse_pr_url(url);
            projection.pull_request = Some(fabro_types::PullRequestLink {
                owner,
                repo,
                number,
                state: pr_state,
            });
        }
        let pool = state.stores.run_summaries.pool();
        let mut tx = pool.begin().await.expect("seed tx");
        fabro_store::RunSummaryStore::write_petri_run_row_on_connection(
            &mut tx,
            &run_id,
            &projection,
        )
        .await
        .expect("run row seed");
        let projection_json = serde_json::to_string(&projection).expect("projection json");
        sqlx::query(
            "INSERT INTO petri_projection (run_id, projection_json, fold_json, \
             positions_json, stream_seq, updated_at_ms) VALUES (?, ?, '{}', '{}', 0, 0) \
             ON CONFLICT(run_id) DO UPDATE SET projection_json = excluded.projection_json",
        )
        .bind(run_id.to_string())
        .bind(projection_json)
        .execute(&mut *tx)
        .await
        .expect("projection row seed");
        tx.commit().await.expect("seed commit");
    }

    /// Append one orchestration verdict record for the run.
    async fn append_verdict(
        state: &AppState,
        run_id: &fabro_types::RunId,
        field: fabro_types::OrchestrationField,
        verdict: fabro_types::OrchestrationVerdict,
    ) {
        state
            .stores
            .run_summaries
            .platform_records()
            .append(
                run_id,
                &fabro_store::platform_records::PlatformRecord::RunOrchestration(
                    fabro_store::platform_records::RunOrchestrationRecord {
                        field,
                        verdict,
                        by_run: None,
                    },
                ),
                None,
            )
            .await
            .expect("verdict seed");
    }

    /// Split a GitHub PR html url into its link parts (test URLs only).
    fn parse_pr_url(url: &str) -> (String, String, u64) {
        let rest = url
            .strip_prefix("https://github.com/")
            .expect("a github pr url");
        let (owner_repo, number) = rest.split_once("/pull/").expect("a pull path");
        let (owner, repo) = owner_repo.split_once('/').expect("owner/repo");
        (
            owner.to_string(),
            repo.to_string(),
            number.parse().expect("a pr number"),
        )
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
