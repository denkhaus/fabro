use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::http::{HeaderMap, StatusCode};
use chrono::{DateTime, Utc};
use croner::errors::CronError;
use fabro_automation::{
    Automation, AutomationId, AutomationRevision, AutomationTriggerId, parse_schedule_expression,
};
use fabro_types::{AutomationRef, Principal, RunId, SystemActorKind};
use fabro_util::error as error_util;
use tokio::time::sleep;
use tracing::{Instrument, error, info, info_span, warn};

use super::{AppState, handler};
use crate::automation_materializer::AutomationRunMaterializeInput;

const AUTOMATION_SCHEDULER_MAX_SLEEP: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ScheduleTriggerKey {
    automation_id: AutomationId,
    trigger_id:    AutomationTriggerId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScheduleCursor {
    automation_revision: AutomationRevision,
    expression:          String,
    next_due_at:         DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DueScheduleTrigger {
    automation: Automation,
    trigger_id: AutomationTriggerId,
    due_at:     DateTime<Utc>,
}

#[derive(Debug, Default)]
struct AutomationSchedulePlanner {
    cursors: HashMap<ScheduleTriggerKey, ScheduleCursor>,
}

fn next_occurrence(expression: &str, after: DateTime<Utc>) -> Result<DateTime<Utc>, CronError> {
    parse_schedule_expression(expression)?.find_next_occurrence(&after, false)
}

impl AutomationSchedulePlanner {
    fn reconcile(&mut self, automations: &[Automation], now: DateTime<Utc>) {
        let mut reconciled = HashMap::new();

        for automation in automations {
            for trigger in automation.enabled_schedule_triggers() {
                let key = ScheduleTriggerKey {
                    automation_id: automation.id.clone(),
                    trigger_id:    trigger.id.clone(),
                };
                if let Some(cursor) = self.cursors.get(&key).filter(|cursor| {
                    cursor.automation_revision == automation.revision
                        && cursor.expression == trigger.expression
                }) {
                    reconciled.insert(key, cursor.clone());
                    continue;
                }

                let next_due_at = match next_occurrence(&trigger.expression, now) {
                    Ok(next_due_at) => next_due_at,
                    Err(err) => {
                        warn!(
                            automation_id = %automation.id,
                            trigger_id = %trigger.id,
                            error = %err,
                            "Skipping invalid automation schedule trigger",
                        );
                        continue;
                    }
                };
                reconciled.insert(key, ScheduleCursor {
                    automation_revision: automation.revision.clone(),
                    expression: trigger.expression.clone(),
                    next_due_at,
                });
            }
        }

        self.cursors = reconciled;
    }

    fn take_due(
        &mut self,
        automations: &[Automation],
        now: DateTime<Utc>,
    ) -> Vec<DueScheduleTrigger> {
        let mut due_keys = self
            .cursors
            .iter()
            .filter(|(_, cursor)| cursor.next_due_at <= now)
            .map(|(key, cursor)| (key.clone(), cursor.next_due_at))
            .collect::<Vec<_>>();
        // Deterministic order for spawn scheduling, log output, and tests.
        due_keys.sort_by(|a, b| {
            a.0.automation_id
                .cmp(&b.0.automation_id)
                .then_with(|| a.0.trigger_id.cmp(&b.0.trigger_id))
        });
        if due_keys.is_empty() {
            return Vec::new();
        }
        let automations_by_id = automations
            .iter()
            .map(|automation| (&automation.id, automation))
            .collect::<HashMap<_, _>>();

        let mut due = Vec::with_capacity(due_keys.len());
        for (key, due_at) in due_keys {
            let Some(cursor) = self.cursors.get_mut(&key) else {
                continue;
            };
            match next_occurrence(&cursor.expression, now) {
                Ok(next_due_at) => {
                    cursor.next_due_at = next_due_at;
                }
                Err(err) => {
                    warn!(
                        automation_id = %key.automation_id,
                        trigger_id = %key.trigger_id,
                        error = %err,
                        "Removing automation schedule cursor after next occurrence failed",
                    );
                    self.cursors.remove(&key);
                    continue;
                }
            }

            let Some(automation) = automations_by_id.get(&key.automation_id) else {
                continue;
            };
            due.push(DueScheduleTrigger {
                automation: (*automation).clone(),
                trigger_id: key.trigger_id,
                due_at,
            });
        }

        due
    }

    /// Reconcile cursors against the current automation set, then drain due
    /// triggers. Single entry point used by the production loop and tests.
    fn tick(&mut self, automations: &[Automation], now: DateTime<Utc>) -> Vec<DueScheduleTrigger> {
        self.reconcile(automations, now);
        self.take_due(automations, now)
    }

    fn sleep_duration(&self, now: DateTime<Utc>) -> Duration {
        let until_next_due = self
            .cursors
            .values()
            .map(|cursor| cursor.next_due_at)
            .min()
            .map_or(AUTOMATION_SCHEDULER_MAX_SLEEP, |next_due_at| {
                if next_due_at <= now {
                    Duration::ZERO
                } else {
                    (next_due_at - now).to_std().unwrap_or(Duration::ZERO)
                }
            });
        until_next_due.min(AUTOMATION_SCHEDULER_MAX_SLEEP)
    }
}

pub(crate) fn spawn_automation_scheduler(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut planner = AutomationSchedulePlanner::default();
        let shutdown = state.shutdown_token();

        loop {
            if state.is_shutting_down() {
                break;
            }

            let automations = match state.automation_store().list().await {
                Ok(automations) => automations,
                Err(err) => {
                    error!(error = ?err, "Failed to load automations for scheduler");
                    tokio::select! {
                        () = shutdown.cancelled() => break,
                        () = state.automation_scheduler_notified() => {},
                        () = sleep(AUTOMATION_SCHEDULER_MAX_SLEEP) => {},
                    }
                    continue;
                }
            };
            let now = Utc::now();
            // Circuit breaker first (fabro-3d97): a trigger that just tripped
            // must not fire on this very tick, so reload when anything paused.
            let automations = if super::automation_breaker::update_automation_breakers(
                state.as_ref(),
                &automations,
                now,
            )
            .await
            {
                match state.automation_store().list().await {
                    Ok(automations) => automations,
                    Err(err) => {
                        error!(error = ?err, "Failed to reload automations after breaker pause");
                        Vec::new()
                    }
                }
            } else {
                automations
            };
            for due in planner.tick(&automations, now) {
                let state = Arc::clone(&state);
                let span = info_span!(
                    "automation_run",
                    automation_id = %due.automation.id,
                    trigger_id = %due.trigger_id,
                );
                tokio::spawn(
                    fire_scheduled_automation_run(
                        state,
                        due.automation,
                        due.trigger_id,
                        due.due_at,
                    )
                    .instrument(span),
                );
            }

            let sleep_duration = planner.sleep_duration(now);
            tokio::select! {
                () = shutdown.cancelled() => break,
                () = state.automation_scheduler_notified() => {},
                () = sleep(sleep_duration) => {},
            }
        }
    });
}

async fn fire_scheduled_automation_run(
    state: Arc<AppState>,
    automation: Automation,
    trigger_id: AutomationTriggerId,
    due_at: DateTime<Utc>,
) {
    let automation_id = automation.id.clone();
    // fabro-09ea overlap guard: a `skip` policy suppresses the fire while
    // a previous run of THIS automation is still non-terminal — running,
    // queued, or blocked at a gate that may wait indefinitely (ADR-0011).
    // A skip is healthy behavior: INFO only, no last_error, no run; the
    // next tick retries. Manual API triggers stay unconditional.
    if automation.on_overlap == Some(fabro_automation::AutomationOverlapPolicy::Skip) {
        match state
            .stores
            .run_summaries
            .active_run_for_automation(automation_id.as_str())
            .await
        {
            Ok(Some(active_run_id)) => {
                info!(
                    automation_id = %automation_id,
                    trigger_id = %trigger_id,
                    active_run_id = %active_run_id,
                    "Scheduled fire skipped: overlapping run still non-terminal (on_overlap=skip)"
                );
                return;
            }
            Ok(None) => {}
            Err(err) => {
                // The overlap check failing must not silently turn into a
                // fire: record it like other scheduler errors and skip.
                record_scheduler_error(
                    state.as_ref(),
                    &automation_id,
                    "Overlap check failed (see logs)",
                )
                .await;
                error!(
                    automation_id = %automation_id,
                    error = ?err,
                    "Scheduled fire skipped: overlap check failed",
                );
                return;
            }
        }
    }
    let run_id = RunId::new();
    let environment_id = match handler::automations::resolve_automation_environment(
        state.as_ref(),
        automation.environment_id.as_deref(),
        StatusCode::CONFLICT,
    ) {
        Ok(environment_id) => environment_id,
        Err(err) => {
            record_scheduler_error(state.as_ref(), &automation_id, err.detail()).await;
            error!(
                due_at = %due_at,
                error = ?err,
                "Scheduled automation environment is not runnable",
            );
            return;
        }
    };
    let Some(target) = automation.git_target().cloned() else {
        record_scheduler_error(
            state.as_ref(),
            &automation_id,
            "Stored automation target is not Git-backed",
        )
        .await;
        error!(
            automation_id = %automation_id,
            "Stored automation target is not Git-backed",
        );
        return;
    };
    let materialized = match state
        .materialize_automation_run(AutomationRunMaterializeInput {
            automation_id: automation_id.clone(),
            target,
            workflow_source: automation.workflow_source,
            workflow: automation.workflow,
            run_id,
            temp_root: state.automation_temp_root(),
        })
        .await
    {
        Ok(materialized) => materialized,
        Err(err) => {
            let message = error_util::collect_chain(&err).join(": ");
            record_scheduler_error(state.as_ref(), &automation_id, &message).await;
            error!(
                due_at = %due_at,
                error = ?err,
                "Failed to materialize scheduled automation run",
            );
            return;
        }
    };

    let actor = Principal::System {
        system_kind: SystemActorKind::Engine,
    };
    let automation_ref = AutomationRef {
        id:              automation_id.to_string(),
        name:            Some(automation.name.clone()),
        trigger_id:      Some(trigger_id.to_string()),
        workflow_source: materialized.workflow_source.clone(),
    };
    // RunIntent admission produces a large future; box it to keep our
    // stack frame small (matches handler/automations.rs).
    let response = Box::pin(handler::runs::create_run_from_intent(
        Arc::clone(&state),
        handler::runs::CreateRunFromIntentRequest {
            intent:          materialized.into_run_intent(environment_id),
            explicit_run_id: Some(run_id),
            actor:           actor.clone(),
            headers:         HeaderMap::new(),
            automation:      Some(automation_ref),
        },
    ))
    .await;

    let status = response.status();
    if !status.is_success() {
        record_scheduler_error(
            state.as_ref(),
            &automation_id,
            &format!("Failed to create scheduled automation run ({status})"),
        )
        .await;
        warn!(
            run_id = %run_id,
            due_at = %due_at,
            status = %status,
            "Failed to create scheduled automation run",
        );
        return;
    }

    if let Err(err) =
        handler::lifecycle::queue_run_start(state.as_ref(), run_id, false, actor).await
    {
        record_scheduler_error(state.as_ref(), &automation_id, err.detail()).await;
        warn!(
            run_id = %run_id,
            due_at = %due_at,
            status = %err.status(),
            code = err.code().unwrap_or(""),
            "Created scheduled automation run but failed to start it",
        );
        return;
    }

    if automation.last_error.is_some() {
        set_scheduler_error(state.as_ref(), &automation_id, None).await;
    }

    info!(
        run_id = %run_id,
        due_at = %due_at,
        "Scheduled automation run queued",
    );
}

async fn record_scheduler_error(state: &AppState, id: &AutomationId, message: &str) {
    set_scheduler_error(state, id, Some(message)).await;
}

async fn set_scheduler_error(state: &AppState, id: &AutomationId, message: Option<&str>) {
    if let Err(err) = state.automation_store().set_last_error(id, message).await {
        error!(
            automation_id = %id,
            error = ?err,
            "Failed to persist automation scheduler status",
        );
    }
}

/// Drive one tick of the scheduler from a test. Boxed so the calling test
/// future stays small (clippy `large_futures`).
#[cfg(test)]
fn run_due_schedules_once<'a>(
    state: Arc<AppState>,
    planner: &'a mut AutomationSchedulePlanner,
    now: DateTime<Utc>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
        let automations = state
            .automation_store()
            .list()
            .await
            .expect("test automations should load");
        // Mirror the production loop: the breaker runs before due triggers
        // fire, with a reload when anything paused (fabro-3d97).
        let automations = if super::automation_breaker::update_automation_breakers(
            state.as_ref(),
            &automations,
            now,
        )
        .await
        {
            state
                .automation_store()
                .list()
                .await
                .expect("test automations should reload after breaker pause")
        } else {
            automations
        };
        for trigger in planner.tick(&automations, now) {
            Box::pin(fire_scheduled_automation_run(
                Arc::clone(&state),
                trigger.automation,
                trigger.trigger_id,
                trigger.due_at,
            ))
            .await;
        }
    })
}

#[cfg(test)]
mod tests {
    use fabro_automation::{
        AutomationDraft, AutomationGitWorkflowSource, AutomationTrigger, ScheduleTrigger,
    };
    use fabro_static::EnvVars;
    use fabro_types::{GitRunTarget, ResolvedAutomationGitWorkflowSource, RunStatus, RunTarget};
    use fabro_workflow::event as workflow_event;

    use super::super::automation_breaker;
    use super::*;
    use crate::test_support::{TestAppStateBuilder, TestAutomationRunMaterializer};

    fn dt(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("test datetime should parse")
            .with_timezone(&Utc)
    }

    fn git_target() -> GitRunTarget {
        GitRunTarget {
            repo:   "fabro-sh/fabro".to_string(),
            branch: "main".to_string(),
            tag:    None,
            sha:    None,
        }
    }

    fn target() -> RunTarget {
        RunTarget::Git(git_target())
    }

    fn schedule_trigger(id: &str, expression: &str, enabled: bool) -> AutomationTrigger {
        AutomationTrigger::Schedule(ScheduleTrigger {
            id: AutomationTriggerId::new(id).expect("test trigger id should be valid"),
            enabled,
            expression: expression.to_string(),
            breaker_threshold: None,
            breaker: None,
        })
    }

    fn automation(id: &str, name: &str, triggers: Vec<AutomationTrigger>) -> Automation {
        Automation {
            on_overlap: None,
            id: AutomationId::new(id).expect("test automation id should be valid"),
            revision: AutomationRevision::from_bytes(format!("{id}:{name}").as_bytes()),
            name: name.to_string(),
            description: None,
            environment_id: Some("default".to_string()),
            last_error: None,
            target: target(),
            workflow_source: None,
            workflow: "workflow.fabro".to_string(),
            triggers,
        }
    }

    async fn create_automation(
        state: &AppState,
        id: &str,
        name: &str,
        triggers: Vec<AutomationTrigger>,
    ) -> Automation {
        create_automation_with_source(state, id, name, None, triggers).await
    }

    async fn create_automation_with_source(
        state: &AppState,
        id: &str,
        name: &str,
        workflow_source: Option<AutomationGitWorkflowSource>,
        triggers: Vec<AutomationTrigger>,
    ) -> Automation {
        create_automation_full(state, id, name, workflow_source, None, triggers).await
    }

    async fn create_automation_full(
        state: &AppState,
        id: &str,
        name: &str,
        workflow_source: Option<AutomationGitWorkflowSource>,
        on_overlap: Option<fabro_automation::AutomationOverlapPolicy>,
        triggers: Vec<AutomationTrigger>,
    ) -> Automation {
        state
            .automation_store()
            .create(AutomationDraft {
                on_overlap,
                id: AutomationId::new(id).expect("test automation id should be valid"),
                name: name.to_string(),
                description: None,
                environment_id: Some("default".to_string()),
                target: target(),
                workflow_source,
                workflow: "workflow.fabro".to_string(),
                triggers,
            })
            .await
            .expect("test automation should be created")
    }

    fn succeeding_materializer() -> TestAutomationRunMaterializer {
        let mut exact_target = git_target();
        exact_target.sha = Some("0123456789abcdef0123456789abcdef01234567".to_string());
        TestAutomationRunMaterializer::succeed(exact_target)
    }

    fn test_state_with_materializer(materializer: TestAutomationRunMaterializer) -> Arc<AppState> {
        TestAppStateBuilder::new()
            .env_lookup(|_| None)
            .vault_entries([(EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
            .automation_materializer(materializer)
            .build()
    }

    async fn stored_runs(state: &AppState) -> Vec<fabro_types::Run> {
        state
            .stores
            .run_summaries
            .list_all(Utc::now())
            .await
            .expect("stored runs should list")
    }

    /// Stored runs oldest-first, so index `minute - 1` is the newest fire.
    async fn stored_runs_chronological(state: &AppState) -> Vec<fabro_types::Run> {
        let mut runs = stored_runs(state).await;
        runs.sort_by_key(|run| run.timestamps.created_at);
        runs
    }

    fn prime_time() -> DateTime<Utc> {
        dt("2026-05-29T00:00:30Z")
    }

    fn first_due_time() -> DateTime<Utc> {
        dt("2026-05-29T00:01:00Z")
    }

    fn second_due_time() -> DateTime<Utc> {
        dt("2026-05-29T00:02:00Z")
    }

    #[test]
    fn new_cursor_starts_at_next_future_occurrence_without_backfill() {
        let now = dt("2026-05-29T00:00:30Z");
        let automation = automation("nightly", "Nightly", vec![schedule_trigger(
            "schedule",
            "* * * * *",
            true,
        )]);
        let mut planner = AutomationSchedulePlanner::default();

        planner.reconcile(&[automation], now);

        assert_eq!(planner.cursors.len(), 1);
        let cursor = planner.cursors.values().next().unwrap();
        assert_eq!(cursor.next_due_at, dt("2026-05-29T00:01:00Z"));
    }

    #[test]
    fn due_cursor_is_returned_once_and_advanced_beyond_now() {
        let automation = automation("nightly", "Nightly", vec![schedule_trigger(
            "schedule",
            "* * * * *",
            true,
        )]);
        let mut planner = AutomationSchedulePlanner::default();
        planner.reconcile(std::slice::from_ref(&automation), prime_time());

        let due = planner.take_due(std::slice::from_ref(&automation), first_due_time());
        let second_due = planner.take_due(std::slice::from_ref(&automation), first_due_time());

        assert_eq!(due.len(), 1);
        assert_eq!(due[0].trigger_id.as_str(), "schedule");
        assert_eq!(due[0].due_at, first_due_time());
        assert!(second_due.is_empty());
        let cursor = planner.cursors.values().next().unwrap();
        assert_eq!(cursor.next_due_at, second_due_time());
    }

    #[test]
    fn disabled_schedule_trigger_removes_cursor() {
        let mut automation = automation("nightly", "Nightly", vec![schedule_trigger(
            "schedule",
            "* * * * *",
            true,
        )]);
        let mut planner = AutomationSchedulePlanner::default();
        planner.reconcile(std::slice::from_ref(&automation), prime_time());
        assert_eq!(planner.cursors.len(), 1);

        automation.triggers = vec![schedule_trigger("schedule", "* * * * *", false)];
        planner.reconcile(std::slice::from_ref(&automation), first_due_time());
        assert!(planner.cursors.is_empty());
    }

    #[test]
    fn replacing_automation_revision_or_expression_resets_cursor() {
        let mut automation = automation("nightly", "Nightly", vec![schedule_trigger(
            "schedule",
            "* * * * *",
            true,
        )]);
        let mut planner = AutomationSchedulePlanner::default();
        planner.reconcile(std::slice::from_ref(&automation), prime_time());

        let original_due = planner.cursors.values().next().unwrap().next_due_at;
        automation.revision = AutomationRevision::from_bytes(b"new revision");
        planner.reconcile(std::slice::from_ref(&automation), first_due_time());
        let reset_due = planner.cursors.values().next().unwrap().next_due_at;

        assert_eq!(original_due, first_due_time());
        assert_eq!(reset_due, second_due_time());

        automation.triggers = vec![schedule_trigger("schedule", "*/5 * * * *", true)];
        planner.reconcile(std::slice::from_ref(&automation), second_due_time());
        let expression_reset_due = planner.cursors.values().next().unwrap().next_due_at;
        assert_eq!(expression_reset_due, dt("2026-05-29T00:05:00Z"));
    }

    #[test]
    fn multiple_schedule_triggers_on_one_automation_have_independent_cursors() {
        let automation = automation("nightly", "Nightly", vec![
            schedule_trigger("every_minute", "* * * * *", true),
            schedule_trigger("every_five", "*/5 * * * *", true),
        ]);
        let mut planner = AutomationSchedulePlanner::default();

        planner.reconcile(std::slice::from_ref(&automation), prime_time());
        let due = planner.take_due(std::slice::from_ref(&automation), first_due_time());

        assert_eq!(planner.cursors.len(), 2);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].trigger_id.as_str(), "every_minute");
        let five_minute_cursor = planner
            .cursors
            .iter()
            .find(|(key, _)| key.trigger_id.as_str() == "every_five")
            .map(|(_, cursor)| cursor)
            .unwrap();
        assert_eq!(five_minute_cursor.next_due_at, dt("2026-05-29T00:05:00Z"));
    }

    #[test]
    fn sleep_duration_uses_nearest_due_time_capped_at_thirty_seconds() {
        let automation = automation("nightly", "Nightly", vec![schedule_trigger(
            "schedule",
            "* * * * *",
            true,
        )]);
        let mut planner = AutomationSchedulePlanner::default();
        planner.reconcile(std::slice::from_ref(&automation), prime_time());

        assert_eq!(
            planner.sleep_duration(prime_time()),
            Duration::from_secs(30)
        );
        assert_eq!(
            planner.sleep_duration(dt("2026-05-29T00:00:45Z")),
            Duration::from_secs(15)
        );
    }

    #[tokio::test]
    async fn due_schedule_only_automation_creates_started_run_with_automation_metadata() {
        let materializer = succeeding_materializer();
        let state = test_state_with_materializer(materializer);
        let automation = create_automation(state.as_ref(), "nightly", "Nightly", vec![
            schedule_trigger("schedule", "* * * * *", true),
        ])
        .await;
        state
            .automation_store()
            .set_last_error(&automation.id, Some("old failure"))
            .await
            .unwrap();
        let mut planner = AutomationSchedulePlanner::default();

        run_due_schedules_once(Arc::clone(&state), &mut planner, prime_time()).await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, first_due_time()).await;

        let runs = stored_runs(state.as_ref()).await;
        assert_eq!(runs.len(), 1);
        assert_eq!(
            state
                .automation_store()
                .get(&automation.id)
                .await
                .unwrap()
                .unwrap()
                .last_error,
            None,
        );
        let automation_ref = runs[0].automation.as_ref().unwrap();
        assert_eq!(automation_ref.id, "nightly");
        assert_eq!(automation_ref.name.as_deref(), Some("Nightly"));
        assert_eq!(automation_ref.trigger_id.as_deref(), Some("schedule"));
        let run_id = runs[0].id;
        let run_store = state.stores.runs.open_run_reader(&run_id).await.unwrap();
        let projection = run_store.state().await.unwrap();
        assert!(projection.spec.workflow_version_id.is_some());
        assert_eq!(
            projection.spec.target,
            Some(RunTarget::Git(fabro_types::GitRunTarget {
                repo:   "fabro-sh/fabro".to_string(),
                branch: "main".to_string(),
                tag:    None,
                sha:    Some("0123456789abcdef0123456789abcdef01234567".to_string()),
            }))
        );
        assert_eq!(
            run_store
                .list_events()
                .await
                .unwrap()
                .iter()
                .filter(|event| event.event.event_name() == "run.start_requested")
                .count(),
            1
        );
        assert!(matches!(
            state
                .runs
                .lock()
                .expect("runs lock should not be poisoned")
                .get(&run_id)
                .map(|run| run.status),
            Some(RunStatus::Runnable)
        ));
    }

    #[tokio::test]
    async fn schedule_only_automation_fires_without_api_trigger() {
        let materializer = succeeding_materializer();
        let state = test_state_with_materializer(materializer);
        create_automation(state.as_ref(), "schedule-only", "Schedule only", vec![
            schedule_trigger("schedule", "* * * * *", true),
        ])
        .await;
        let mut planner = AutomationSchedulePlanner::default();

        run_due_schedules_once(Arc::clone(&state), &mut planner, prime_time()).await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, first_due_time()).await;

        assert_eq!(stored_runs(state.as_ref()).await.len(), 1);
    }

    #[tokio::test]
    async fn scheduled_fire_skips_while_previous_run_is_non_terminal() {
        let materializer = succeeding_materializer();
        let state = test_state_with_materializer(materializer.clone());
        create_automation_full(
            state.as_ref(),
            "skip-on-overlap",
            "Skip on overlap",
            None,
            Some(fabro_automation::AutomationOverlapPolicy::Skip),
            vec![schedule_trigger("schedule", "* * * * *", true)],
        )
        .await;
        let mut planner = AutomationSchedulePlanner::default();

        // First due fire creates a run; it stays non-terminal in this test
        // state (submitted/runnable), exactly like a run blocked at a gate.
        run_due_schedules_once(Arc::clone(&state), &mut planner, prime_time()).await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, first_due_time()).await;
        assert_eq!(stored_runs(state.as_ref()).await.len(), 1);

        // Second due fire: suppressed by on_overlap=skip — no second run,
        // and the materializer saw only the first request.
        run_due_schedules_once(Arc::clone(&state), &mut planner, second_due_time()).await;
        assert_eq!(stored_runs(state.as_ref()).await.len(), 1);
        assert_eq!(materializer.captured_inputs().len(), 1);
    }

    #[tokio::test]
    async fn scheduled_fire_without_skip_policy_fires_despite_overlap() {
        let materializer = succeeding_materializer();
        let state = test_state_with_materializer(materializer.clone());
        create_automation(state.as_ref(), "fire-on-overlap", "Fire on overlap", vec![
            schedule_trigger("schedule", "* * * * *", true),
        ])
        .await;
        let mut planner = AutomationSchedulePlanner::default();

        run_due_schedules_once(Arc::clone(&state), &mut planner, prime_time()).await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, first_due_time()).await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, second_due_time()).await;

        // Default policy (None = fire) keeps the unchanged behavior: both
        // fires created runs even though run one is still non-terminal.
        assert_eq!(stored_runs(state.as_ref()).await.len(), 2);
    }

    #[tokio::test]
    async fn scheduled_run_passes_saved_workflow_source_to_materialization() {
        let materializer = succeeding_materializer();
        let state = test_state_with_materializer(materializer.clone());
        let workflow_source = AutomationGitWorkflowSource {
            repo:   "fabro-sh/workflows".to_string(),
            branch: "context-only".to_string(),
            tag:    Some("v1".to_string()),
            sha:    Some("0123456789abcdef0123456789abcdef01234567".to_string()),
        };
        create_automation_with_source(
            state.as_ref(),
            "scheduled-source",
            "scheduled-source",
            Some(workflow_source.clone()),
            vec![schedule_trigger("schedule", "* * * * *", true)],
        )
        .await;
        let mut planner = AutomationSchedulePlanner::default();

        run_due_schedules_once(Arc::clone(&state), &mut planner, prime_time()).await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, first_due_time()).await;

        let captured = materializer.captured_inputs();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].workflow_source, Some(workflow_source.clone()));
        let runs = stored_runs(state.as_ref()).await;
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0]
                .automation
                .as_ref()
                .and_then(|automation| automation.workflow_source.clone()),
            Some(Box::new(
                ResolvedAutomationGitWorkflowSource::from_requested(
                    workflow_source,
                    "ffffffffffffffffffffffffffffffffffffffff".to_string(),
                )
            ))
        );
    }

    #[tokio::test]
    async fn disabled_schedule_trigger_does_not_create_run() {
        let materializer = succeeding_materializer();
        let state = test_state_with_materializer(materializer);
        create_automation(
            state.as_ref(),
            "disabled-trigger",
            "Disabled trigger",
            vec![schedule_trigger("schedule", "* * * * *", false)],
        )
        .await;
        let mut planner = AutomationSchedulePlanner::default();

        run_due_schedules_once(Arc::clone(&state), &mut planner, prime_time()).await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, first_due_time()).await;

        assert!(stored_runs(state.as_ref()).await.is_empty());
    }

    #[tokio::test]
    async fn multiple_due_triggers_create_multiple_runs() {
        let materializer = succeeding_materializer();
        let state = test_state_with_materializer(materializer);
        create_automation(state.as_ref(), "nightly", "Nightly", vec![
            schedule_trigger("first", "* * * * *", true),
            schedule_trigger("second", "* * * * *", true),
        ])
        .await;
        let mut planner = AutomationSchedulePlanner::default();

        run_due_schedules_once(Arc::clone(&state), &mut planner, prime_time()).await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, first_due_time()).await;

        let mut trigger_ids = stored_runs(state.as_ref())
            .await
            .into_iter()
            .map(|run| run.automation.unwrap().trigger_id.unwrap())
            .collect::<Vec<_>>();
        trigger_ids.sort();
        assert_eq!(trigger_ids, ["first", "second"]);
    }

    #[tokio::test]
    async fn queued_prior_run_does_not_suppress_new_due_run() {
        let materializer = succeeding_materializer();
        let state = test_state_with_materializer(materializer);
        create_automation(state.as_ref(), "nightly", "Nightly", vec![
            schedule_trigger("schedule", "* * * * *", true),
        ])
        .await;
        let mut planner = AutomationSchedulePlanner::default();

        run_due_schedules_once(Arc::clone(&state), &mut planner, prime_time()).await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, first_due_time()).await;
        assert_eq!(stored_runs(state.as_ref()).await.len(), 1);
        assert!(
            state
                .runs
                .lock()
                .expect("runs lock should not be poisoned")
                .values()
                .any(|run| run.status == RunStatus::Runnable)
        );

        run_due_schedules_once(Arc::clone(&state), &mut planner, second_due_time()).await;

        assert_eq!(stored_runs(state.as_ref()).await.len(), 2);
    }

    #[tokio::test]
    async fn failing_materializer_waits_until_next_cron_occurrence() {
        let materializer = TestAutomationRunMaterializer::fail_invalid_target();
        let state = test_state_with_materializer(materializer.clone());
        let automation = create_automation(state.as_ref(), "nightly", "Nightly", vec![
            schedule_trigger("schedule", "* * * * *", true),
        ])
        .await;
        let mut planner = AutomationSchedulePlanner::default();

        run_due_schedules_once(Arc::clone(&state), &mut planner, prime_time()).await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, first_due_time()).await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, first_due_time()).await;

        assert!(stored_runs(state.as_ref()).await.is_empty());
        assert_eq!(materializer.captured_inputs().len(), 1);
        assert!(
            state
                .automation_store()
                .get(&automation.id)
                .await
                .unwrap()
                .unwrap()
                .last_error
                .is_some()
        );

        run_due_schedules_once(Arc::clone(&state), &mut planner, second_due_time()).await;

        assert!(stored_runs(state.as_ref()).await.is_empty());
        assert_eq!(materializer.captured_inputs().len(), 2);
    }

    // --- fabro-3d97: automation circuit breaker integration tests ---

    /// Capturing sink for breaker-pause notifications.
    #[derive(Clone, Default)]
    struct CapturedBreakerNotices(
        std::sync::Arc<std::sync::Mutex<Vec<automation_breaker::BreakerPauseNotice>>>,
    );

    #[async_trait::async_trait]
    impl automation_breaker::AutomationBreakerNotifier for CapturedBreakerNotices {
        async fn notify_breaker_pause(&self, notice: &automation_breaker::BreakerPauseNotice) {
            self.0
                .lock()
                .expect("captured notices lock should not be poisoned")
                .push(notice.clone());
        }
    }

    fn breaker_test_state(
        materializer: TestAutomationRunMaterializer,
    ) -> (Arc<AppState>, CapturedBreakerNotices) {
        let notices = CapturedBreakerNotices::default();
        let state = TestAppStateBuilder::new()
            .env_lookup(|_| None)
            .vault_entries([(EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
            .automation_materializer(materializer)
            .automation_breaker_notifier(std::sync::Arc::new(notices.clone()))
            .build();
        (state, notices)
    }

    fn schedule_trigger_with_breaker(
        id: &str,
        expression: &str,
        threshold: Option<u32>,
    ) -> AutomationTrigger {
        AutomationTrigger::Schedule(ScheduleTrigger {
            id:                AutomationTriggerId::new(id)
                .expect("test trigger id should be valid"),
            enabled:           true,
            expression:        expression.to_string(),
            breaker_threshold: threshold,
            breaker:           None,
        })
    }

    async fn create_breakable_automation(
        state: &AppState,
        id: &str,
        threshold: Option<u32>,
    ) -> Automation {
        state
            .automation_store()
            .create(AutomationDraft {
                on_overlap:      None,
                id:              AutomationId::new(id).expect("test automation id should be valid"),
                name:            "Breakable".to_string(),
                description:     None,
                environment_id:  Some("default".to_string()),
                target:          target(),
                workflow_source: None,
                workflow:        "workflow.fabro".to_string(),
                triggers:        vec![schedule_trigger_with_breaker(
                    "schedule",
                    "* * * * *",
                    threshold,
                )],
            })
            .await
            .expect("test automation should be created")
    }

    /// Drive one terminal failure onto a fired run, with a failure-detail
    /// signature exactly like the run-level breaker records.
    async fn park_run_with_signature(state: &AppState, run_id: &RunId, signature: &str) {
        append_run_lifecycle_prefix(state, run_id).await;
        let run_store = state.stores.runs.open_run(run_id).await.unwrap();
        let mut detail = fabro_types::FailureDetail::new(
            "zai quota exhausted",
            fabro_types::FailureCategory::TransientInfra,
        );
        detail.signature = Some(fabro_types::FailureSignature(signature.to_string()));
        workflow_event::append_event(
            &run_store,
            run_id,
            &workflow_event::Event::WorkflowRunFailed {
                failure:              fabro_types::RunFailure {
                    reason: fabro_types::FailureReason::SoftStop,
                    detail,
                },
                timing:               fabro_types::RunTiming::wall_only(1_000),
                final_git_commit_sha: None,
                final_patch:          None,
                diff_summary:         None,
                billing:              None,
            },
        )
        .await
        .unwrap();
    }

    /// Drive one terminal success onto a fired run.
    async fn succeed_run(state: &AppState, run_id: &RunId) {
        append_run_lifecycle_prefix(state, run_id).await;
        let run_store = state.stores.runs.open_run(run_id).await.unwrap();
        workflow_event::append_event(
            &run_store,
            run_id,
            &workflow_event::Event::WorkflowRunCompleted {
                timing:               fabro_types::RunTiming::wall_only(1_000),
                artifact_count:       0,
                status:               "succeeded".to_string(),
                reason:               fabro_types::SuccessReason::Completed,
                failure:              None,
                total_usd_micros:     None,
                final_git_commit_sha: None,
                final_patch:          None,
                diff_summary:         None,
                billing:              None,
            },
        )
        .await
        .unwrap();
    }

    async fn append_run_lifecycle_prefix(state: &AppState, run_id: &RunId) {
        let run_store = state.stores.runs.open_run(run_id).await.unwrap();
        workflow_event::append_event(&run_store, run_id, &workflow_event::Event::RunStarting)
            .await
            .unwrap();
        workflow_event::append_event(&run_store, run_id, &workflow_event::Event::RunRunning)
            .await
            .unwrap();
    }

    fn stored_breaker_trigger(automation: &Automation) -> fabro_automation::ScheduleTrigger {
        automation
            .triggers
            .iter()
            .find_map(|trigger| match trigger {
                AutomationTrigger::Schedule(trigger) => Some(trigger.clone()),
                AutomationTrigger::Api(_) => None,
            })
            .expect("automation should keep its schedule trigger")
    }

    fn due_minute(minute: u32) -> DateTime<Utc> {
        dt(&format!("2026-05-29T00:{minute:02}:00Z"))
    }

    #[tokio::test]
    async fn same_signature_parks_pause_the_schedule_with_one_aggregated_notification() {
        let materializer = succeeding_materializer();
        let (state, notices) = breaker_test_state(materializer);
        create_breakable_automation(state.as_ref(), "breakable", Some(2)).await;
        let mut planner = AutomationSchedulePlanner::default();
        let signature = "api_transient|zai|rate_limited";

        // Prime, then fire run 1 and park it.
        run_due_schedules_once(Arc::clone(&state), &mut planner, prime_time()).await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, due_minute(1)).await;
        let runs = stored_runs_chronological(state.as_ref()).await;
        assert_eq!(runs.len(), 1);
        park_run_with_signature(state.as_ref(), &runs[0].id, signature).await;

        // Pass 2: baseline absorbs run 1; run 2 fires and parks the same way.
        run_due_schedules_once(Arc::clone(&state), &mut planner, due_minute(2)).await;
        let runs = stored_runs_chronological(state.as_ref()).await;
        assert_eq!(runs.len(), 2);
        park_run_with_signature(state.as_ref(), &runs[1].id, signature).await;

        // Pass 3: run 2 counts (1 < 2); run 3 fires and parks.
        run_due_schedules_once(Arc::clone(&state), &mut planner, due_minute(3)).await;
        let runs = stored_runs_chronological(state.as_ref()).await;
        assert_eq!(runs.len(), 3);
        park_run_with_signature(state.as_ref(), &runs[2].id, signature).await;

        // Pass 4: run 3 counts (2 = threshold) — the breaker pauses the
        // trigger BEFORE the due fire, so run 4 never exists.
        run_due_schedules_once(Arc::clone(&state), &mut planner, due_minute(4)).await;
        let runs = stored_runs_chronological(state.as_ref()).await;
        assert_eq!(runs.len(), 3);

        // Further passes fire nothing and notify nothing more.
        run_due_schedules_once(Arc::clone(&state), &mut planner, due_minute(5)).await;
        assert_eq!(stored_runs(state.as_ref()).await.len(), 3);

        // The paused state is visible through the automation surface with
        // recorded breaker facts.
        let automation = state
            .automation_store()
            .get(&AutomationId::new("breakable").unwrap())
            .await
            .unwrap()
            .expect("automation should exist");
        assert!(automation.enabled_schedule_triggers().next().is_none());
        let trigger = stored_breaker_trigger(&automation);
        assert!(!trigger.enabled);
        let facts = trigger.breaker.expect("pause facts should be recorded");
        assert_eq!(facts.signature, signature);
        assert_eq!(facts.consecutive_count, 2);
        assert_eq!(facts.last_run_id, runs[2].id.to_string());
        assert!(facts.paused_at.is_some());

        // Exactly ONE aggregated notification, naming the signature, the
        // consecutive count, and the last run.
        let captured = notices
            .0
            .lock()
            .expect("captured notices lock should not be poisoned");
        assert_eq!(captured.len(), 1, "exactly one aggregated notification");
        let notice = &captured[0];
        assert_eq!(notice.signature, signature);
        assert_eq!(notice.consecutive, 2);
        assert_eq!(notice.last_run_id, runs[2].id.to_string());
        assert_eq!(notice.trigger_id.as_str(), "schedule");
    }

    #[tokio::test]
    async fn succeeded_run_resets_the_breaker_count() {
        let materializer = succeeding_materializer();
        let (state, _notices) = breaker_test_state(materializer);
        create_breakable_automation(state.as_ref(), "resettable", Some(2)).await;
        let mut planner = AutomationSchedulePlanner::default();
        let signature = "api_transient|zai|rate_limited";

        run_due_schedules_once(Arc::clone(&state), &mut planner, prime_time()).await;
        // Runs 1..5: park(baseline), park(count 1), succeed(reset), park
        // (count 1), park (count 2 -> pause). Without the reset the breaker
        // would pause while processing run 3.
        for minute in 1..=5u32 {
            run_due_schedules_once(Arc::clone(&state), &mut planner, due_minute(minute)).await;
            let runs = stored_runs_chronological(state.as_ref()).await;
            assert_eq!(runs.len(), usize::try_from(minute).unwrap());
            let run_id = runs[runs.len() - 1].id;
            match minute {
                3 => succeed_run(state.as_ref(), &run_id).await,
                _ => park_run_with_signature(state.as_ref(), &run_id, signature).await,
            }
        }

        // The next pass processes run 5 (count 2 = threshold) and pauses
        // before firing anything further.
        run_due_schedules_once(Arc::clone(&state), &mut planner, due_minute(6)).await;
        assert_eq!(stored_runs(state.as_ref()).await.len(), 5);
        let automation = state
            .automation_store()
            .get(&AutomationId::new("resettable").unwrap())
            .await
            .unwrap()
            .expect("automation should exist");
        let trigger = stored_breaker_trigger(&automation);
        assert!(!trigger.enabled);
        let facts = trigger.breaker.expect("pause facts should be recorded");
        assert_eq!(facts.consecutive_count, 2);
        assert_eq!(
            facts.last_run_id,
            stored_runs_chronological(state.as_ref()).await[4]
                .id
                .to_string()
        );
    }

    #[tokio::test]
    async fn re_enabling_the_trigger_resumes_firing_with_a_reset_counter() {
        let materializer = succeeding_materializer();
        let (state, notices) = breaker_test_state(materializer);
        let automation = create_breakable_automation(state.as_ref(), "resumable", Some(1)).await;
        let mut planner = AutomationSchedulePlanner::default();
        let signature = "api_transient|zai|rate_limited";

        run_due_schedules_once(Arc::clone(&state), &mut planner, prime_time()).await;
        // Threshold 1: run 1 is the baseline, run 2 parks and pauses.
        run_due_schedules_once(Arc::clone(&state), &mut planner, due_minute(1)).await;
        let runs = stored_runs(state.as_ref()).await;
        park_run_with_signature(state.as_ref(), &runs[0].id, signature).await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, due_minute(2)).await;
        let runs = stored_runs_chronological(state.as_ref()).await;
        assert_eq!(runs.len(), 2);
        park_run_with_signature(state.as_ref(), &runs[1].id, signature).await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, due_minute(3)).await;
        assert_eq!(stored_runs(state.as_ref()).await.len(), 2);
        let paused = state
            .automation_store()
            .get(&automation.id)
            .await
            .unwrap()
            .unwrap();
        assert!(!stored_breaker_trigger(&paused).enabled);

        // A human resumes through the existing enable path (replace); the
        // facts clear and the schedule fires again even though the run
        // history still ends with the same-signature park.
        let resumed = state
            .automation_store()
            .replace(
                &paused.id,
                &paused.revision,
                fabro_automation::AutomationReplace {
                    on_overlap:      None,
                    name:            paused.name.clone(),
                    description:     None,
                    environment_id:  Some("default".to_string()),
                    target:          target(),
                    workflow_source: None,
                    workflow:        "workflow.fabro".to_string(),
                    triggers:        vec![schedule_trigger_with_breaker(
                        "schedule",
                        "* * * * *",
                        Some(1),
                    )],
                },
            )
            .await
            .expect("resume replace should succeed");
        let trigger = stored_breaker_trigger(&resumed);
        assert!(trigger.enabled);
        assert_eq!(trigger.breaker, None, "resume resets the breaker counter");

        // The replace rewrote the trigger row, so the planner recreates its
        // cursor at the next occurrence after the resume pass.
        run_due_schedules_once(Arc::clone(&state), &mut planner, due_minute(4)).await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, due_minute(5)).await;
        assert_eq!(
            stored_runs(state.as_ref()).await.len(),
            3,
            "resumed schedule fires again"
        );
        // Still exactly one notification: the resume itself does not notify,
        // and the fresh baseline absorbs the parked history.
        assert_eq!(
            notices
                .0
                .lock()
                .expect("captured notices lock should not be poisoned")
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn workflow_source_failure_creates_and_starts_no_scheduled_run() {
        let materializer = TestAutomationRunMaterializer::fail_invalid_workflow_source();
        let state = test_state_with_materializer(materializer.clone());
        create_automation_with_source(
            state.as_ref(),
            "failing-source",
            "failing-source",
            Some(AutomationGitWorkflowSource {
                repo:   "fabro-sh/workflows".to_string(),
                branch: "main".to_string(),
                tag:    None,
                sha:    None,
            }),
            vec![schedule_trigger("schedule", "* * * * *", true)],
        )
        .await;
        let mut planner = AutomationSchedulePlanner::default();

        run_due_schedules_once(Arc::clone(&state), &mut planner, prime_time()).await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, first_due_time()).await;

        assert!(stored_runs(state.as_ref()).await.is_empty());
        assert_eq!(materializer.captured_inputs().len(), 1);
    }
}
