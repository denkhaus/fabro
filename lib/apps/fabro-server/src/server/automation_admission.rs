//! The gear admission gate (denkhaus/work#1436): a scheduled fire consults
//! the gear's server-side worklist BEFORE any run-creation work and skips
//! the fire when the worklist is positively empty.
//!
//! The economics: an idle-slot lap costs sandbox create + pack + cook +
//! prune (observed 22-47s per empty lap on the 2026-11-18 line), and every
//! trunk move drags the no-op through the OOM-prone cold cook (22-minute
//! laps, work#1335). The gear graphs keep their internal sweeps as the
//! authoritative re-check — this gate is only an admission optimization,
//! so a stale or approximate empty answer costs one skipped tick, never
//! lost work.
//!
//! Fail-open contract: the gears are the line's recovery path, so a
//! FAILED worklist consult never blocks a fire — only a POSITIVE empty
//! answer skips. Every read error logs (WARN, naming the arm) and the
//! fire proceeds exactly as it does today. The manual/API fire
//! (`handler::automations`) stays unconditional by design: a caller
//! asking for a run gets one.

use std::collections::HashSet;

use chrono::Utc;
use fabro_automation::Automation;
use fabro_store::platform_records::{ObservationTriageState, PlatformRecordKind};
use fabro_types::{OrchestrationField, OrchestrationVerdict};
use fabro_work::{WorkItemFilter, WorkItemStateFilter};
use tracing::warn;

use super::AppState;

/// The workflow slug the run-queue gears sweep (the scripts' `WORKFLOW`
/// const: the adjudicator's fast path and closing pass and the merger's
/// fast path all read `--workflow develop`).
const SWEPT_WORKFLOW: &str = "develop";

/// The observation category the triager sweeps (`--category painpoint`,
/// `.fabro/workflows/triager/scripts/sweep.nu`) — the store matches the
/// serde fragment, so the wire name rides here verbatim.
const PAINPOINT_CATEGORY: &str = "painpoint";

/// The claim door's bounded lane page (`handler::work_items::claim_next`
/// lists the lane with limit 50, page 1) — the gate reuses the same read
/// instead of a cheaper narrower one, so its answer never disagrees with
/// what the lap's own claim would find.
const CLAIM_POOL_PAGE: usize = 50;

/// Why the gate says the gear's worklist is positively empty. One variant
/// per gear arm; the Display is the skip log's reason token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
pub(crate) enum WorklistEmpty {
    /// The merger's worklist — merge-enabled develop runs without a
    /// settled closable verdict — holds nothing.
    #[strum(serialize = "merge_enabled_empty")]
    MergeEnabled,
    /// The adjudicator's worklists hold nothing: neither the unsettled
    /// merge queue (pass one) nor the merged runs awaiting item closure
    /// (the closing pass).
    #[strum(serialize = "adjudication_empty")]
    Adjudication,
    /// The triager's feed holds no untriaged painpoint.
    #[strum(serialize = "untriaged_painpoints_empty")]
    UntriagedPainpoints,
    /// Every lane of the develop gear's claim pool is drained (no free
    /// item anywhere).
    #[strum(serialize = "claim_pool_empty")]
    ClaimPool,
}

/// The gate's verdict for one scheduled fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AdmissionDecision {
    /// The worklist is positively empty — skip the fire; the next tick
    /// re-consults. Healthy behavior: INFO only, no `last_error`, no run.
    Skip(WorklistEmpty),
    /// Work exists, the workflow is not a gated gear, or the consult
    /// could not decide — fire as today.
    Fire,
}

/// Consult the automation's gear worklist. Dispatch is on the workflow
/// slug; only the four worklist-driven gears of denkhaus/work#1436 are
/// gated (the watchdog's heartbeat sweeps its own systems and is left
/// ungated), and every consult error fails OPEN — the fire proceeds.
pub(crate) async fn admission_decision(
    state: &AppState,
    automation: &Automation,
) -> AdmissionDecision {
    let consulted = match automation.workflow.as_str() {
        "merger" => merger_worklist(state).await,
        "adjudicator" => adjudicator_worklists(state).await,
        "triager" => triager_worklist(state).await,
        "develop" => develop_claim_pool(state, automation).await,
        // Not a gated gear: no opinion, fire as today.
        _ => Ok(None),
    };
    match consulted {
        Ok(Some(empty)) => AdmissionDecision::Skip(empty),
        Ok(None) => AdmissionDecision::Fire,
        Err(error) => {
            // Fail-open (work#1436): the gears are the line's recovery
            // path — a flaky worklist read must not wedge them. The
            // distinction is logged here, the caller fires as today.
            warn!(
                automation_id = %automation.id,
                workflow = %automation.workflow,
                error = %error,
                "Gear admission gate could not consult the worklist — firing anyway (fail-open, work#1436)"
            );
            AdmissionDecision::Fire
        }
    }
}

/// The merger's worklist (ADR-0039 stage E): the closable anti-join
/// restricted to `merge = enabled` — the EXACT query the merger's
/// fast-path sweep walks (`run queue --field closable --workflow develop
/// --requires merge=enabled`). The gate asks only "is there anything", so
/// limit 1 answers it.
async fn merger_worklist(state: &AppState) -> Result<Option<WorklistEmpty>, String> {
    let queue = state
        .stores
        .run_summaries
        .adjudication_queue(
            OrchestrationField::Closable,
            Some((OrchestrationField::Merge, OrchestrationVerdict::Enabled)),
            Some(SWEPT_WORKFLOW),
            1,
            Utc::now(),
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(queue.is_empty().then_some(WorklistEmpty::MergeEnabled))
}

/// The adjudicator's two worklists (ADR-0039 stage D): pass one's merge
/// queue — terminal develop runs whose latest merge verdict is not
/// settled (the anti-join the fast path walks) — and the closing pass's
/// verdict-flipped read — merged develop runs (`closable = enabled`)
/// whose work items may close. Only BOTH drained means nothing to do;
/// skipping on the closing list alone would starve pass-one adjudication
/// beside a fresh batch of terminal runs.
async fn adjudicator_worklists(state: &AppState) -> Result<Option<WorklistEmpty>, String> {
    let now = Utc::now();
    let merge_queue = state
        .stores
        .run_summaries
        .adjudication_queue(
            OrchestrationField::Merge,
            None,
            Some(SWEPT_WORKFLOW),
            1,
            now,
        )
        .await
        .map_err(|error| error.to_string())?;
    if !merge_queue.is_empty() {
        return Ok(None);
    }
    let closings = state
        .stores
        .run_summaries
        .verdict_queue(
            OrchestrationField::Closable,
            OrchestrationVerdict::Enabled,
            Some(SWEPT_WORKFLOW),
            1,
            now,
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(closings.is_empty().then_some(WorklistEmpty::Adjudication))
}

/// The triager's worklist (ADR-0044): the sweep's exact feed read
/// (`--category painpoint --triage untriaged`, oldest first) — the page
/// is irrelevant to the gate, the COUNT is the worklist. The store's
/// materialized total is an upper bound on what the sweep's live overlay
/// would keep, so zero is a positive empty answer and any count fires.
async fn triager_worklist(state: &AppState) -> Result<Option<WorklistEmpty>, String> {
    let (_, total) = state
        .stores
        .run_summaries
        .platform_records()
        .read_kind_global(
            PlatformRecordKind::ObservationRecorded,
            false,
            1,
            0,
            None,
            None,
            Some(PAINPOINT_CATEGORY),
            Some(ObservationTriageState::Untriaged),
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok((total == 0).then_some(WorklistEmpty::UntriagedPainpoints))
}

/// The develop claim pool (ADR-0032): the SAME bounded lane reads the
/// claim door serves (`handler::work_items::claim_next` —
/// `tracker.list` with the lane, limit 50, page 1) minus the live
/// claims. Every declared lane must drain before the gate skips: the
/// planner claims whatever lane PROJECT_FACTS names, and the pool
/// policy's run-scoped refinements (the misroute skip, the stale-basis
/// verdicts, claim holds) stay the run's business — the gate only proves
/// there is no FREE item anywhere. Wiring anomalies (no bound project,
/// no declared lanes) are configuration, not worklist state: fail-open
/// and let the lap surface them.
async fn develop_claim_pool(
    state: &AppState,
    automation: &Automation,
) -> Result<Option<WorklistEmpty>, String> {
    let Some(project_id) = automation.work_project.as_deref() else {
        // The binding gate (fabro-f1a9) already refused an unbound
        // automation before this consult; reaching here means no opinion.
        return Ok(None);
    };
    let Some(project) = state
        .work_store()
        .project(project_id)
        .await
        .map_err(|error| error.to_string())?
    else {
        // Same arm: the binding gate proved the project exists; a miss
        // here races a concurrent unregister — no opinion.
        return Ok(None);
    };
    if project.lane_labels.is_empty() {
        return Ok(None);
    }
    let tracker = state
        .work_tracker_for_project(&project)
        .await
        .map_err(|error| error.to_string())?;
    let claimed: HashSet<String> = state
        .work_store()
        .active_claims(&project.project_id)
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|claim| claim.item_key)
        .collect();
    for lane in project.lane_labels.keys() {
        let items = tracker
            .list(&project, WorkItemFilter {
                lane:  Some(lane.clone()),
                limit: CLAIM_POOL_PAGE,
                query: None,
                label: None,
                page:  1,
                state: WorkItemStateFilter::All,
            })
            .await
            .map_err(|error| error.to_string())?;
        if items.iter().any(|item| !claimed.contains(&item.item_key)) {
            return Ok(None);
        }
    }
    Ok(Some(WorklistEmpty::ClaimPool))
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;
    use fabro_automation::{
        AutomationDraft, AutomationId, AutomationRevision, AutomationTrigger, AutomationTriggerId,
        ScheduleTrigger,
    };
    use fabro_static::EnvVars;
    use fabro_types::{GitRunTarget, RunId, RunTarget};

    use super::super::automation_scheduler::{AutomationSchedulePlanner, run_due_schedules_once};
    use super::*;
    use crate::test_support::{TestAppStateBuilder, TestAutomationRunMaterializer};

    /// A lane-scripted tracker: which item keys each lane serves, and a
    /// switch for a tracker read FAILURE (the fail-open arm).
    struct LaneTracker {
        lanes:      std::sync::Mutex<std::collections::HashMap<String, Vec<String>>>,
        fail_reads: bool,
    }

    impl LaneTracker {
        fn with_lane(lane: &str, item_keys: &[&str]) -> Self {
            Self {
                lanes:      std::sync::Mutex::new(std::collections::HashMap::from([(
                    lane.to_owned(),
                    item_keys.iter().map(|key| (*key).to_owned()).collect(),
                )])),
                fail_reads: false,
            }
        }

        fn failing() -> Self {
            Self {
                lanes:      std::sync::Mutex::new(std::collections::HashMap::new()),
                fail_reads: true,
            }
        }

        fn item(key: &str) -> fabro_work::WorkItem {
            fabro_work::WorkItem {
                item_key:   key.to_owned(),
                title:      format!("item {key}"),
                body:       "the spec".to_owned(),
                state:      "open".to_owned(),
                labels:     Vec::new(),
                priority:   1,
                url:        format!("https://github.com/denkhaus/work/issues/{key}"),
                updated_at: Some(Utc::now()),
                created_at: None,
                closed_at:  None,
            }
        }
    }

    #[async_trait::async_trait]
    impl fabro_work::TrackerProvider for LaneTracker {
        async fn list(
            &self,
            _project: &fabro_work::WorkProject,
            filter: WorkItemFilter,
        ) -> anyhow::Result<Vec<fabro_work::WorkItem>> {
            if self.fail_reads {
                anyhow::bail!("tracker read failed (scripted)");
            }
            let lanes = self.lanes.lock().expect("lane tracker lock poisoned");
            let Some(keys) = lanes.get(filter.lane.as_deref().unwrap_or_default()) else {
                return Ok(Vec::new());
            };
            Ok(keys.iter().map(|key| Self::item(key)).collect())
        }

        async fn get(
            &self,
            _project: &fabro_work::WorkProject,
            _item_key: &str,
        ) -> anyhow::Result<Option<fabro_work::WorkItem>> {
            Ok(None)
        }

        async fn create(
            &self,
            _project: &fabro_work::WorkProject,
            _draft: fabro_work::WorkItemDraft,
        ) -> anyhow::Result<fabro_work::WorkItem> {
            anyhow::bail!("not needed")
        }

        async fn update(
            &self,
            _project: &fabro_work::WorkProject,
            _item_key: &str,
            _title: Option<&str>,
            _body: Option<&str>,
        ) -> anyhow::Result<fabro_work::WorkItem> {
            anyhow::bail!("not needed")
        }

        async fn close(
            &self,
            _project: &fabro_work::WorkProject,
            _item_key: &str,
            _reason: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn comment(
            &self,
            _project: &fabro_work::WorkProject,
            _item_key: &str,
            _body: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn project_claim(
            &self,
            _project: &fabro_work::WorkProject,
            _item_key: &str,
            _run_id: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn clear_claim(
            &self,
            _project: &fabro_work::WorkProject,
            _item_key: &str,
            _run_id: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }

    fn automation_with_workflow(workflow: &str) -> Automation {
        Automation {
            on_overlap:      None,
            id:              AutomationId::new("gate-test")
                .expect("test automation id should be valid"),
            revision:        AutomationRevision::from_bytes(b"gate-test"),
            name:            "gate test".to_string(),
            description:     None,
            environment_id:  Some("default".to_string()),
            last_error:      None,
            model_providers: None,
            target:          RunTarget::Git(GitRunTarget {
                repo:   "fabro-sh/fabro".to_string(),
                branch: "main".to_string(),
                tag:    None,
                sha:    None,
            }),
            workflow_source: None,
            workflow:        workflow.to_string(),
            work_project:    Some("fabro".to_owned()),
            triggers:        Vec::new(),
        }
    }

    fn test_state() -> std::sync::Arc<AppState> {
        // The vault entry rides the shape the scheduler tests' state
        // builder pins; the store arms consult only SQLite.
        TestAppStateBuilder::new()
            .env_lookup(|_| None)
            .vault_entries([(EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
            .build()
    }

    fn project_with_lanes(lanes: &[&str]) -> fabro_work::WorkProject {
        let mut project = crate::test_support::test_work_project("fabro");
        project.lane_labels = lanes
            .iter()
            .map(|lane| ((*lane).to_owned(), format!("lane:{lane}")))
            .collect();
        project
    }

    #[tokio::test]
    async fn merger_skips_on_a_drained_closable_queue() {
        let state = test_state();
        let decision = admission_decision(&state, &automation_with_workflow("merger")).await;
        assert_eq!(
            decision,
            AdmissionDecision::Skip(WorklistEmpty::MergeEnabled),
            "a fresh store's closable queue is empty — the idle merger lap skips"
        );
    }

    #[tokio::test]
    async fn adjudicator_skips_only_when_both_worklists_drain() {
        let state = test_state();
        let decision = admission_decision(&state, &automation_with_workflow("adjudicator")).await;
        assert_eq!(
            decision,
            AdmissionDecision::Skip(WorklistEmpty::Adjudication),
            "a fresh store's merge queue and closing list are empty — the idle pass skips"
        );
    }

    #[tokio::test]
    async fn triager_skips_on_an_empty_untriaged_feed() {
        let state = test_state();
        let decision = admission_decision(&state, &automation_with_workflow("triager")).await;
        assert_eq!(
            decision,
            AdmissionDecision::Skip(WorklistEmpty::UntriagedPainpoints),
            "a fresh store's untriaged painpoint count is zero — the idle sweep skips"
        );
    }

    #[tokio::test]
    async fn ungated_workflows_fire_without_a_consult() {
        let state = test_state();
        let decision = admission_decision(&state, &automation_with_workflow("watchdog")).await;
        assert_eq!(
            decision,
            AdmissionDecision::Fire,
            "the watchdog is not a gated gear — its heartbeat fires as today"
        );
    }

    #[tokio::test]
    async fn develop_skips_when_every_declared_lane_drains() {
        let state = TestAppStateBuilder::new()
            .env_lookup(|_| None)
            .work_projects([project_with_lanes(&["develop"])])
            .work_tracker(std::sync::Arc::new(LaneTracker::with_lane("develop", &[
                "denkhaus/work#1",
            ])))
            .build();
        // The one lane item is actively claimed: no free item anywhere.
        state
            .work_store()
            .grant(&fabro_work::GrantRequest {
                project_id: "fabro".to_owned(),
                item_key:   "denkhaus/work#1".to_owned(),
                run_id:     RunId::new(),
                lane:       "develop".to_owned(),
            })
            .await
            .expect("the test claim grants");
        let decision = admission_decision(&state, &automation_with_workflow("develop")).await;
        assert_eq!(
            decision,
            AdmissionDecision::Skip(WorklistEmpty::ClaimPool),
            "every lane item is claimed — the idle lap skips"
        );
    }

    #[tokio::test]
    async fn develop_fires_when_a_lane_holds_a_free_item() {
        let state = TestAppStateBuilder::new()
            .env_lookup(|_| None)
            .work_projects([project_with_lanes(&["develop"])])
            .work_tracker(std::sync::Arc::new(LaneTracker::with_lane("develop", &[
                "denkhaus/work#1",
            ])))
            .build();
        let decision = admission_decision(&state, &automation_with_workflow("develop")).await;
        assert_eq!(
            decision,
            AdmissionDecision::Fire,
            "a free lane item is claimable work — the fire proceeds"
        );
    }

    #[tokio::test]
    async fn develop_fires_open_when_the_tracker_read_fails() {
        let state = TestAppStateBuilder::new()
            .env_lookup(|_| None)
            .work_projects([project_with_lanes(&["develop"])])
            .work_tracker(std::sync::Arc::new(LaneTracker::failing()))
            .build();
        let decision = admission_decision(&state, &automation_with_workflow("develop")).await;
        assert_eq!(
            decision,
            AdmissionDecision::Fire,
            "a failed worklist consult never blocks the fire (fail-open)"
        );
    }

    #[tokio::test]
    async fn develop_fires_open_without_declared_lanes() {
        let state = TestAppStateBuilder::new()
            .env_lookup(|_| None)
            .work_projects([project_with_lanes(&[])])
            .work_tracker(std::sync::Arc::new(LaneTracker::with_lane("develop", &[
                "denkhaus/work#1",
            ])))
            .build();
        let decision = admission_decision(&state, &automation_with_workflow("develop")).await;
        assert_eq!(
            decision,
            AdmissionDecision::Fire,
            "no declared lanes is a wiring anomaly, not a drained worklist — fail-open"
        );
    }

    /// The end-to-end shape (denkhaus/work#1436): a scheduled merger fire
    /// against an empty worklist creates NO run — the materializer never
    /// sees an input and the store holds no run.
    #[tokio::test]
    async fn a_scheduled_merger_fire_skips_run_creation_on_an_empty_worklist() {
        let materializer = TestAutomationRunMaterializer::succeed(GitRunTarget {
            repo:   "fabro-sh/fabro".to_string(),
            branch: "main".to_string(),
            tag:    None,
            sha:    Some("0123456789abcdef0123456789abcdef01234567".to_string()),
        });
        let state = TestAppStateBuilder::new()
            .env_lookup(|_| None)
            .vault_entries([(EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
            .automation_materializer(materializer.clone())
            .build();
        seed_merger_automation(state.as_ref()).await;
        // Two ticks, the planner contract the scheduler tests pin: the
        // first arms the cursor at the next boundary, the second fires it.
        let mut planner = AutomationSchedulePlanner::default();
        run_due_schedules_once(state.clone(), &mut planner, arm_time()).await;
        run_due_schedules_once(state.clone(), &mut planner, due_time()).await;
        assert!(
            materializer.captured_inputs().is_empty(),
            "the empty-worklist fire must not materialize a run"
        );
        let runs = state
            .stores
            .run_summaries
            .list_all(due_time())
            .await
            .expect("stored runs should list");
        assert!(runs.is_empty(), "no run may exist behind the skipped fire");
    }

    /// The binding gate's wired project (the shape
    /// `automation_scheduler::tests::seed_work_project` seeds), so the
    /// fire's fabro-f1a9 gate passes before the admission consult.
    async fn seed_merger_automation(state: &AppState) {
        state
            .work_store()
            .upsert_project(&project_with_lanes(&[]))
            .await
            .expect("test work project upserts");
        state
            .automation_store()
            .create(AutomationDraft {
                work_project:    Some("fabro".to_owned()),
                on_overlap:      None,
                id:              AutomationId::new("merger-gate")
                    .expect("test automation id should be valid"),
                name:            "merger".to_string(),
                description:     None,
                environment_id:  Some("default".to_string()),
                target:          RunTarget::Git(GitRunTarget {
                    repo:   "fabro-sh/fabro".to_string(),
                    branch: "main".to_string(),
                    tag:    None,
                    sha:    None,
                }),
                workflow_source: None,
                // The draft's workflow slug is the gate's dispatch key —
                // the scheduler test helpers pin `workflow.fabro`, which
                // is not a gated gear.
                workflow:        "merger".to_string(),
                triggers:        vec![schedule_trigger("schedule", "* * * * *")],
            })
            .await
            .expect("test automation should be created");
    }

    fn schedule_trigger(id: &str, expression: &str) -> AutomationTrigger {
        AutomationTrigger::Schedule(ScheduleTrigger {
            id:                AutomationTriggerId::new(id)
                .expect("test trigger id should be valid"),
            enabled:           true,
            expression:        expression.to_owned(),
            breaker_threshold: None,
            breaker:           None,
            provider_window:   None,
        })
    }

    fn arm_time() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-06-01T00:00:30Z")
            .expect("test datetime should parse")
            .with_timezone(&Utc)
    }

    fn due_time() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-06-01T00:01:00Z")
            .expect("test datetime should parse")
            .with_timezone(&Utc)
    }
}
