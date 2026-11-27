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
use fabro_github::pull_request_forge::GitHubPullRequestForge;
use fabro_store::platform_records::{ObservationTriageState, PlatformRecordKind};
use fabro_types::{OrchestrationField, OrchestrationVerdict};
use fabro_work::{PullRequestForge, WorkItemFilter};
use tracing::warn;

use super::AppState;
use super::gear_worklist::{ClaimPoolSpec, GearWorklistSpec};

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

/// Consult the automation's gear worklist. Dispatch rides the gear
/// worklist table ([`super::gear_worklist`]): only the four
/// worklist-driven gears of denkhaus/work#1436 are in it (the
/// watchdog's heartbeat sweeps its own systems and is left ungated),
/// each arm consumes the table's spec, and every consult error fails
/// OPEN — the fire proceeds.
pub(crate) async fn admission_decision(
    state: &AppState,
    automation: &Automation,
) -> AdmissionDecision {
    let consulted = match super::gear_worklist::by_slug(&automation.workflow) {
        Some(gear) => match gear.spec {
            GearWorklistSpec::Queue {
                workflow,
                field,
                requires,
            } => merger_worklist(state, automation, field, requires, workflow).await,
            GearWorklistSpec::QueueAndClosing {
                workflow,
                queue_field,
                closing,
            } => adjudicator_worklists(state, queue_field, closing, workflow).await,
            GearWorklistSpec::Feed { category, triage } => {
                triager_worklist(state, category, triage).await
            }
            GearWorklistSpec::ClaimPool(pool) => develop_claim_pool(state, automation, pool).await,
        },
        // Not a gated gear: no opinion, fire as today.
        None => Ok(None),
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

/// The merger's worklist (ADR-0039 stage E + ADR-0040's session half,
/// work#1704): the closable anti-join restricted to its settled
/// requirement — the run-queue half of the fast path's sweep
/// (`run queue --field closable --workflow develop --requires
/// merge=enabled`) — PLUS the label door's open `merge:enabled` pull
/// requests on the automation's git target repo (the fast path's label
/// arm; work#1675's drift starved on exactly this half being absent).
/// The gate asks only "is there anything", so limit 1 answers the queue
/// and an `is_empty` answers the listing.
async fn merger_worklist(
    state: &AppState,
    automation: &Automation,
    field: OrchestrationField,
    requires: (OrchestrationField, OrchestrationVerdict),
    workflow: &str,
) -> Result<Option<WorklistEmpty>, String> {
    let queue = state
        .stores
        .run_summaries
        .adjudication_queue(field, Some(requires), Some(workflow), 1, Utc::now())
        .await
        .map_err(|error| error.to_string())?;
    if !queue.is_empty() {
        return Ok(None);
    }
    if merge_enabled_pull_requests_pending(state, automation).await? {
        return Ok(None);
    }
    Ok(Some(WorklistEmpty::MergeEnabled))
}

/// The merger's session-PR half (ADR-0040, work#1704): open pull
/// requests labeled [`super::gear_worklist::MERGE_ENABLED_LABEL`] on the
/// automation's git target repo, read through the same forge the label
/// door serves. Configuration absence counts the half EMPTY — the fast
/// path's label arm skips its session half on a door-less server too,
/// so the worklist truthfully holds no PR work there. An undecidable
/// read (no git target, unresolvable credentials, a failed listing) is
/// the `Err` the caller fails open on.
async fn merge_enabled_pull_requests_pending(
    state: &AppState,
    automation: &Automation,
) -> Result<bool, String> {
    let Some(target) = automation.git_target() else {
        return Err(
            "the automation carries no git target, so the session-PR half is undecidable"
                .to_owned(),
        );
    };
    let settings = state.server_settings();
    let credentials = state
        .github_credentials(&settings.server.integrations.github)
        .await
        .map_err(|error| format!("resolving GitHub credentials failed: {error}"))?;
    let Some(credentials) = credentials else {
        tracing::debug!(
            automation_id = %automation.id,
            "GitHub integration not configured; counting the session-PR half empty"
        );
        return Ok(false);
    };
    let http_client = state.http_client().map_err(|error| error.to_string())?;
    let forge =
        GitHubPullRequestForge::new(credentials, state.github_api_base_url.clone(), http_client);
    let listing = forge
        .list_pull_requests(
            &target.repo,
            "open",
            Some(super::gear_worklist::MERGE_ENABLED_LABEL),
        )
        .await
        .map_err(|error| format!("the merge:enabled pull request listing failed: {error}"))?;
    Ok(!listing.is_empty())
}

/// The adjudicator's two worklists (ADR-0039 stage D): pass one's merge
/// queue — terminal develop runs whose latest merge verdict is not
/// settled (the anti-join the fast path walks) — and the closing pass's
/// verdict-flipped read — merged develop runs (`closable = enabled`)
/// whose work items may close. Only BOTH drained means nothing to do;
/// skipping on the closing list alone would starve pass-one adjudication
/// beside a fresh batch of terminal runs.
async fn adjudicator_worklists(
    state: &AppState,
    queue_field: OrchestrationField,
    closing: (OrchestrationField, OrchestrationVerdict),
    workflow: &str,
) -> Result<Option<WorklistEmpty>, String> {
    let now = Utc::now();
    let merge_queue = state
        .stores
        .run_summaries
        .adjudication_queue(queue_field, None, Some(workflow), 1, now)
        .await
        .map_err(|error| error.to_string())?;
    if !merge_queue.is_empty() {
        return Ok(None);
    }
    let closings = state
        .stores
        .run_summaries
        .verdict_queue(closing.0, closing.1, Some(workflow), 1, now)
        .await
        .map_err(|error| error.to_string())?;
    Ok(closings.is_empty().then_some(WorklistEmpty::Adjudication))
}

/// The triager's worklist (ADR-0044): the sweep's exact feed read
/// (`--category painpoint --triage untriaged`, oldest first) — the page
/// is irrelevant to the gate, the COUNT is the worklist. The store's
/// materialized total is an upper bound on what the sweep's live overlay
/// would keep, so zero is a positive empty answer and any count fires.
async fn triager_worklist(
    state: &AppState,
    category: &str,
    triage: ObservationTriageState,
) -> Result<Option<WorklistEmpty>, String> {
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
            Some(category),
            Some(triage),
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok((total == 0).then_some(WorklistEmpty::UntriagedPainpoints))
}

/// The develop claim pool (ADR-0032): the SAME bounded lane read the
/// claim door serves (`handler::work_items::claim_next` — the table's
/// claim-pool spec: the lane, its page, OPEN items) minus the live
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
    pool: ClaimPoolSpec,
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
                lane:     Some(lane.clone()),
                limit:    pool.page,
                query:    None,
                label:    None,
                priority: None,
                page:     1,
                state:    pool.state,
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
    use fabro_work::WorkItemStateFilter;

    use super::super::automation_scheduler::{AutomationSchedulePlanner, run_due_schedules_once};
    use super::super::gear_worklist::MERGE_ENABLED_LABEL;
    use super::*;
    use crate::test_support::{TestAppStateBuilder, TestAutomationRunMaterializer};

    /// A lane-scripted tracker: which items each lane serves, and a
    /// switch for a tracker read FAILURE (the fail-open arm). The list
    /// read is STATE-FAITHFUL — a real tracker filters by the filter's
    /// state, so the double serves an item only when the filter
    /// enumerates its state — the property the parity pin (work#1488)
    /// needs.
    struct LaneTracker {
        lanes:      std::sync::Mutex<std::collections::HashMap<String, Vec<fabro_work::WorkItem>>>,
        fail_reads: bool,
    }

    impl LaneTracker {
        fn with_lane(lane: &str, item_keys: &[&str]) -> Self {
            Self::with_items(
                lane,
                item_keys
                    .iter()
                    .map(|key| Self::item(key))
                    .collect::<Vec<_>>(),
            )
        }

        fn with_items(lane: &str, items: Vec<fabro_work::WorkItem>) -> Self {
            Self {
                lanes:      std::sync::Mutex::new(std::collections::HashMap::from([(
                    lane.to_owned(),
                    items,
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

        /// The filter's state arm, mirrored the way a real tracker
        /// applies it: `Open` serves open items only, `Closed` closed
        /// items only, `All` everything.
        fn passes_state(item: &fabro_work::WorkItem, state: WorkItemStateFilter) -> bool {
            match state {
                WorkItemStateFilter::Open => item.state == "open",
                WorkItemStateFilter::Closed => item.state == "closed",
                WorkItemStateFilter::All => true,
            }
        }

        fn item(key: &str) -> fabro_work::WorkItem {
            Self::item_in_state(key, "open")
        }

        fn item_in_state(key: &str, state: &str) -> fabro_work::WorkItem {
            fabro_work::WorkItem {
                item_key:   key.to_owned(),
                title:      format!("item {key}"),
                body:       "the spec".to_owned(),
                state:      state.to_owned(),
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
            let Some(items) = lanes.get(filter.lane.as_deref().unwrap_or_default()) else {
                return Ok(Vec::new());
            };
            Ok(items
                .iter()
                .filter(|item| Self::passes_state(item, filter.state))
                .cloned()
                .collect())
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

    /// The merger consult's configured shape (work#1704): the GitHub
    /// integration points at `base_url` (a local httpmock) and a static
    /// PAT resolves from the vault — the session-PR half reads the label
    /// listing there.
    fn merger_state_with_github(base_url: &str) -> std::sync::Arc<AppState> {
        TestAppStateBuilder::new()
            .env_lookup(|_| None)
            .vault_entries([
                (EnvVars::OPENAI_API_KEY, "test-openai-api-key"),
                (EnvVars::GITHUB_TOKEN, "ghu_test"),
            ])
            .github_api_base_url(base_url)
            .build()
    }

    /// The empty merge:enabled listing — the forge truth of the drift
    /// era (work#1675): no session pull requests waiting.
    fn empty_label_listing_mock(github: &httpmock::MockServer) -> httpmock::Mock<'_> {
        github.mock(|when, then| {
            when.method("GET")
                .path("/repos/fabro-sh/fabro/issues")
                .query_param("state", "open")
                .query_param("labels", MERGE_ENABLED_LABEL);
            then.status(200).json_body(serde_json::json!([]));
        })
    }

    /// One open merge:enabled pull request, in the issues-listing shape
    /// the label door parses (an issue with a `pull_request` field).
    fn one_label_listing_mock(github: &httpmock::MockServer) -> httpmock::Mock<'_> {
        github.mock(|when, then| {
            when.method("GET")
                .path("/repos/fabro-sh/fabro/issues")
                .query_param("state", "open")
                .query_param("labels", MERGE_ENABLED_LABEL);
            then.status(200).json_body(serde_json::json!([{
                "number": 710,
                "title": "feat: the waiting session work",
                "labels": [{"name": "merge:enabled"}],
                "html_url": "https://github.com/fabro-sh/fabro/pull/710",
                "state": "open",
                "pull_request": {"merged_at": null}
            }]));
        })
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
        // work#1704: the consult reads the session-PR half too, so the
        // fixture carries the configured integration and its label
        // listing answers empty (no session work waiting).
        let github = httpmock::MockServer::start();
        let _empty = empty_label_listing_mock(&github);
        let state = merger_state_with_github(&github.base_url());
        let decision = admission_decision(&state, &automation_with_workflow("merger")).await;
        assert_eq!(
            decision,
            AdmissionDecision::Skip(WorklistEmpty::MergeEnabled),
            "an empty closable queue and no merge:enabled PR — the idle merger lap skips"
        );
    }

    /// work#1704 (the work#1675 root cause): a waiting merge:enabled
    /// pull request fires the otherwise-empty worklist. The drift starved
    /// 44 hours while #710/#711 sat labeled and every scheduled slot
    /// skipped — this pin is the consult learning the session half.
    #[tokio::test]
    async fn a_merge_enabled_pull_request_fires_an_empty_run_queue() {
        let github = httpmock::MockServer::start();
        let listing = one_label_listing_mock(&github);
        let state = merger_state_with_github(&github.base_url());
        let decision = admission_decision(&state, &automation_with_workflow("merger")).await;
        assert_eq!(
            decision,
            AdmissionDecision::Fire,
            "a labeled PR is merger work the empty run queue cannot see — fire"
        );
        assert_eq!(listing.calls(), 1, "the consult reads the label door once");
    }

    /// The fail-open contract holds on the new half: a FAILED label
    /// listing is an undecidable consult, never a skip — the fire
    /// proceeds and the caller's WARN names the arm.
    #[tokio::test]
    async fn a_failing_pull_request_read_fires_open() {
        let github = httpmock::MockServer::start();
        let _broken = github.mock(|when, then| {
            when.method("GET")
                .path("/repos/fabro-sh/fabro/issues")
                .query_param("state", "open")
                .query_param("labels", MERGE_ENABLED_LABEL);
            then.status(500).body("forge on fire");
        });
        let state = merger_state_with_github(&github.base_url());
        let decision = admission_decision(&state, &automation_with_workflow("merger")).await;
        assert_eq!(
            decision,
            AdmissionDecision::Fire,
            "a failed PR listing cannot answer 'positively empty' — fail-open"
        );
    }

    /// Configuration absence is not degradation: an App strategy without
    /// an app id resolves no credentials, the label door is unavailable
    /// on this server, and the fast path's label arm skips its session
    /// half in exactly that state — so the consult counts the PR half
    /// empty and the idle merger still skips.
    #[tokio::test]
    async fn an_unconfigured_github_integration_counts_the_pr_half_empty() {
        let mut settings = crate::test_support::default_test_server_settings();
        settings.server.integrations.github.strategy =
            fabro_types::settings::server::GithubIntegrationStrategy::App;
        settings.server.integrations.github.app_id = None;
        let state = TestAppStateBuilder::new()
            .runtime_settings(settings, fabro_config::RunLayer::default())
            .env_lookup(|_| None)
            .vault_entries([(EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
            .build();
        let decision = admission_decision(&state, &automation_with_workflow("merger")).await;
        assert_eq!(
            decision,
            AdmissionDecision::Skip(WorklistEmpty::MergeEnabled),
            "a door-less server holds no PR work — the empty worklist skips"
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

    /// The never-stuck pin (work#1585): a PARKED-ONLY feed still admits.
    /// The parked record's materialized `filing_blocked` column rides the
    /// same widened untriaged count the sweep reads, so the gear fires on
    /// a parked-only feed and the sweep picks the record back up after
    /// the TTL — a feed that read as empty here would never re-serve a
    /// parked painpoint. (Accepted economy: the lap exits "Feed empty"
    /// while the park is young; safety beats economy.)
    #[tokio::test]
    async fn triager_admits_on_a_parked_only_feed() {
        use fabro_store::platform_records::{
            ObservationRecordedRecord, ObservationTriageState, PlatformRecord,
        };

        let state = test_state();
        let run = RunId::new();
        let records = state.stores.run_summaries.platform_records();
        let stored = records
            .append(
                &run,
                &PlatformRecord::ObservationRecorded(ObservationRecordedRecord {
                    category:  fabro_types::ObservationCategory::Painpoint,
                    text:      "parked-only painpoint".to_owned(),
                    execution: 1,
                    firing:    1,
                }),
                None,
            )
            .await
            .expect("observation seed");
        // The park write path: the decision row beside the materialized
        // `filing_blocked` column — the shape a real park leaves.
        state
            .work_store()
            .decide_observation(&run, stored.seq, "filing_blocked", "rate-limited", None)
            .await
            .expect("park decision writes");
        records
            .materialize_observation_triage(&run, stored.seq, ObservationTriageState::FilingBlocked)
            .await
            .expect("parked state materializes");

        let decision = admission_decision(&state, &automation_with_workflow("triager")).await;
        assert_eq!(
            decision,
            AdmissionDecision::Fire,
            "a parked-only feed is not an empty feed — the gear must admit, or the \
             record never comes back after the TTL"
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

    /// The parity pin (work#1488): the admission gate's develop worklist
    /// answer equals claim_next's claimable reality. The claim door
    /// reads the lane with `WorkItemStateFilter::Open` — a CLOSED lane
    /// item is not claimable work, so the gate must answer exactly what
    /// claim_next answers for the same lane (an empty pool: skip),
    /// never count the closed item as work.
    #[tokio::test]
    async fn develop_gate_matches_the_claim_pools_claimable_reality() {
        let state = TestAppStateBuilder::new()
            .env_lookup(|_| None)
            .work_projects([project_with_lanes(&["develop"])])
            .work_tracker(std::sync::Arc::new(LaneTracker::with_items(
                "develop",
                vec![LaneTracker::item_in_state("denkhaus/work#1", "closed")],
            )))
            .build();
        let decision = admission_decision(&state, &automation_with_workflow("develop")).await;
        assert_eq!(
            decision,
            AdmissionDecision::Skip(WorklistEmpty::ClaimPool),
            "a closed lane item is not claimable — the gate must agree with \
             claim_next's Open read (an empty claimable pool skips), not \
             answer Fire off an item no lap could claim"
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
    /// sees an input and the store holds no run. work#1704: the consult
    /// reads the session-PR half too, so the fixture carries the
    /// configured integration and its listing answers empty.
    #[tokio::test]
    async fn a_scheduled_merger_fire_skips_run_creation_on_an_empty_worklist() {
        let github = httpmock::MockServer::start();
        let _empty = empty_label_listing_mock(&github);
        let materializer = TestAutomationRunMaterializer::succeed(GitRunTarget {
            repo:   "fabro-sh/fabro".to_string(),
            branch: "main".to_string(),
            tag:    None,
            sha:    Some("0123456789abcdef0123456789abcdef01234567".to_string()),
        });
        let state = TestAppStateBuilder::new()
            .env_lookup(|_| None)
            .vault_entries([
                (EnvVars::OPENAI_API_KEY, "test-openai-api-key"),
                (EnvVars::GITHUB_TOKEN, "ghu_test"),
            ])
            .github_api_base_url(github.base_url())
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
