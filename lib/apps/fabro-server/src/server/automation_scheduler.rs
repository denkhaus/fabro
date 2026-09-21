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
pub(crate) struct AutomationSchedulePlanner {
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
                // fabro-b959: a revision-only change (environment flip, name,
                // overlap policy, ... — expression untouched) must not
                // disturb the cursor's timing. Recomputing from `now` here
                // dropped a pending due boundary whenever the update landed
                // between the last tick and that boundary, silently stopping
                // schedule evaluation until the following occurrence. The
                // stored `next_due_at` is never stale by more than one loop
                // sleep (<= 30s) while the trigger stays enabled, because a
                // disabled trigger's cursor is dropped by this same pass —
                // so preserving it is safe. A fresh cursor (new key, or an
                // expression change) still re-arms from `now`; a server
                // restart starts from an empty planner, which intentionally
                // re-arms everything the same way (no backfill).
                if let Some(cursor) = self
                    .cursors
                    .get(&key)
                    .filter(|cursor| cursor.expression == trigger.expression)
                {
                    reconciled.insert(key, ScheduleCursor {
                        automation_revision: automation.revision.clone(),
                        expression:          cursor.expression.clone(),
                        next_due_at:         cursor.next_due_at,
                    });
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

/// fabro-79d8: automation-lag watchdog. The scheduler's own ticks are INFO
/// (nothing ingests them), so a silent schedule stall — no automation-created
/// run for several slot intervals — is only visible by diffing
/// automation_runs against the trigger's cron. This watchdog runs inside the
/// scheduler loop and turns that lag into one WARN event per stall episode.
#[derive(Debug, Default)]
struct AutomationLagWatchdog {
    /// Automations already alerted in the current stall episode. One alert
    /// per episode; the latch re-arms when the automation catches up (a new
    /// run refreshes the newest-run timestamp and the missed count drops
    /// back to <= 2 slots).
    alerted: std::collections::HashSet<AutomationId>,
}

/// A detected schedule stall, returned so tests can assert the warn path
/// without scraping log output.
#[derive(Debug, Clone, PartialEq)]
struct AutomationLagAlert {
    automation_id: AutomationId,
    trigger_id:    AutomationTriggerId,
    expression:    String,
    newest_run_at: DateTime<Utc>,
    expected_at:   DateTime<Utc>,
    lag:           chrono::Duration,
    missed_slots:  u32,
}

impl AutomationLagWatchdog {
    /// Count the cron boundaries that should have produced a run since the
    /// newest automation-created run; > 2 missed slots is a stall.
    fn missed_slot_count(
        trigger_expression: &str,
        newest_run_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Option<u32> {
        let mut expected = newest_run_at;
        let mut missed = 0u32;
        // Bound the walk at the threshold plus one: the decision only needs
        // to know whether more than two slots were missed.
        while missed <= 3 {
            match next_occurrence(trigger_expression, expected) {
                Ok(next) if next <= now => {
                    expected = next;
                    missed += 1;
                }
                Ok(_) => break,
                // Invalid expressions are already warned about by reconcile.
                Err(_) => return None,
            }
        }
        Some(missed)
    }

    /// Pure decision pass over the current automation set. `newest_run_at`
    /// maps automation id -> newest automation-created run's created_at.
    fn check(
        &mut self,
        automations: &[Automation],
        newest_run_at: &HashMap<String, DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Vec<AutomationLagAlert> {
        let mut alerts = Vec::new();
        for automation in automations {
            // The env-update arm (fabro-79d8): there is no durable
            // "environment update in flight" marker server-side, so the
            // watchdog proxies it with `automation.last_error` — a recorded
            // scheduler error (environment not runnable, materialization
            // failure, ...) means the line is already loud through the
            // existing error path, which is exactly the arm the watchdog
            // must not double-report. A silent stall has last_error clear.
            if automation.last_error.is_some() {
                self.alerted.remove(&automation.id);
                continue;
            }
            let Some(&newest_run_at) = newest_run_at.get(automation.id.as_str()) else {
                // No automation-created run yet: nothing to diff against
                // (fresh automations and post-restart re-arm intentionally
                // have no backfill).
                continue;
            };
            for trigger in automation.enabled_schedule_triggers() {
                // A tripped breaker disables the trigger, so
                // enabled_schedule_triggers() is empty on that arm and the
                // watchdog stays silent for it by construction.
                let Some(missed) = Self::missed_slot_count(&trigger.expression, newest_run_at, now)
                else {
                    continue;
                };
                if missed > 2 {
                    if self.alerted.insert(automation.id.clone()) {
                        // The most recent boundary the schedule missed: walk
                        // forward from the first slot after the newest run.
                        let mut expected_at = now;
                        let mut slot = newest_run_at;
                        for _ in 0..missed {
                            match next_occurrence(&trigger.expression, slot) {
                                Ok(next) if next <= now => {
                                    slot = next;
                                    expected_at = next;
                                }
                                _ => break,
                            }
                        }
                        alerts.push(AutomationLagAlert {
                            automation_id: automation.id.clone(),
                            trigger_id: trigger.id.clone(),
                            expression: trigger.expression.clone(),
                            newest_run_at,
                            expected_at,
                            lag: now - newest_run_at,
                            missed_slots: missed,
                        });
                    }
                    continue;
                }
                // Caught up (or never stalled): re-arm the latch so a future
                // stall alerts again.
                self.alerted.remove(&automation.id);
            }
        }
        // Drop latches for automations that no longer exist.
        let live: std::collections::HashSet<&AutomationId> = automations
            .iter()
            .map(|automation| &automation.id)
            .collect();
        self.alerted.retain(|id| live.contains(id));
        alerts
    }
}

/// Drive one watchdog pass against the live stores: newest automation-run
/// timestamps plus the pure decision pass, emitting one WARN per new stall
/// episode so log-ingesting monitors (rootprint-class) see it — unlike the
/// INFO scheduler ticks.
async fn check_automation_lag(
    state: &AppState,
    watchdog: &mut AutomationLagWatchdog,
    automations: &[Automation],
    now: DateTime<Utc>,
) -> Vec<AutomationLagAlert> {
    let runs = match state.stores.run_summaries.list_all(now).await {
        Ok(runs) => runs,
        Err(err) => {
            warn!(error = ?err, "Automation lag watchdog could not list runs");
            return Vec::new();
        }
    };
    let mut newest_run_at: HashMap<String, DateTime<Utc>> = HashMap::new();
    for run in &runs {
        let Some(automation_ref) = &run.automation else {
            continue;
        };
        let entry = newest_run_at
            .entry(automation_ref.id.clone())
            .or_insert(run.timestamps.created_at);
        if run.timestamps.created_at > *entry {
            *entry = run.timestamps.created_at;
        }
    }
    let alerts = watchdog.check(automations, &newest_run_at, now);
    for alert in &alerts {
        warn!(
            automation_id = %alert.automation_id,
            trigger_id = %alert.trigger_id,
            expression = %alert.expression,
            newest_run_at = %alert.newest_run_at,
            expected_at = %alert.expected_at,
            lag_seconds = alert.lag.num_seconds(),
            missed_slots = alert.missed_slots,
            "Automation schedule stall: lag exceeds two slot intervals (fabro-79d8 watchdog)"
        );
    }
    alerts
}

pub(crate) fn spawn_automation_scheduler(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut planner = AutomationSchedulePlanner::default();
        let mut lag_watchdog = AutomationLagWatchdog::default();
        // Fork state (fabro-986b): provider window gate per automation.
        let mut gate = super::fork_line_recovery::GateState::new();
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
            // Fork seam (fabro-986b, user decision 2026-09-14): the
            // provider window gate probes the LLM BEFORE any scheduled
            // fire (one basic completion) — closed windows create no runs;
            // closed lines poll on the fixed 10-minute cadence and fire
            // on reopen. No prose-parsed backoff.
            let held = super::fork_line_recovery::provider_gate_tick(
                Arc::clone(&state),
                &automations,
                now,
                &mut gate,
            )
            .await;
            for due in planner.tick(&automations, now) {
                if held.contains(due.automation.id.as_str()) {
                    info!(
                        automation_id = %due.automation.id,
                        trigger_id = %due.trigger_id,
                        "Scheduled fire held: provider window closed (fabro-986b gate)"
                    );
                    continue;
                }
                if !super::fork_line_recovery::cron_fire_allowed(
                    state.as_ref(),
                    &due.automation,
                    now,
                    &mut gate,
                )
                .await
                {
                    continue;
                }
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
            // fabro-79d8 watchdog: after firing, turn a silent schedule
            // stall into one WARN per episode (ingestible by log monitors).
            check_automation_lag(state.as_ref(), &mut lag_watchdog, &automations, now).await;

            let sleep_duration = planner.sleep_duration(now);
            tokio::select! {
                () = shutdown.cancelled() => break,
                () = state.automation_scheduler_notified() => {},
                () = sleep(sleep_duration) => {},
            }
        }
    });
}

/// Fork visibility seam (fabro-986b): the recheck probes in
/// `fork_line_recovery` fire through this same serialized path.
pub(crate) async fn fire_scheduled_automation_run(
    state: Arc<AppState>,
    automation: Automation,
    trigger_id: AutomationTriggerId,
    due_at: DateTime<Utc>,
) {
    let automation_id = automation.id.clone();
    // fabro-09ea overlap guard: an effective `skip` policy suppresses the
    // fire while a previous run of THIS automation is still non-terminal —
    // running, queued, or blocked at a gate that may wait indefinitely
    // (ADR-0011). Since fabro-fb16 the untagged (`None`) policy resolves
    // to `Skip` (fabro_automation::Automation::effective_scheduled_overlap_policy),
    // so overlapping scheduled passes are impossible by default; only an
    // explicit API-set `Fire` keeps firing. This is the only scheduled fire
    // decision point — manual/API-triggered runs (handler create_run) are
    // unconditional by design and never consult the policy.
    // A skip is healthy behavior: INFO only, no last_error, no run; the
    // next tick retries.
    if automation.effective_scheduled_overlap_policy()
        == fabro_automation::AutomationOverlapPolicy::Skip
    {
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
                // fire (fabro-fb16 audit: this is the fail-safe arm of the
                // Skip default — an undecidable overlap state skips):
                // record it like other scheduler errors and skip.
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
pub(crate) fn run_due_schedules_once<'a>(
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
        // fire, with a reload when anything paused (fabro-3d97). The
        // provider window gate (fabro-986b) runs in the production loop
        // only — its probe would hit the network; the gate's decision
        // logic is unit-tested in fork_line_recovery.
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
pub(crate) mod tests {
    use fabro_automation::{
        AutomationDraft, AutomationGitWorkflowSource, AutomationReplace, AutomationTrigger,
        ScheduleTrigger,
    };
    use fabro_static::EnvVars;
    use fabro_types::{GitRunTarget, ResolvedAutomationGitWorkflowSource, RunStatus, RunTarget};

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

    pub(crate) fn succeeding_materializer() -> TestAutomationRunMaterializer {
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
    pub(crate) async fn stored_runs_chronological(state: &AppState) -> Vec<fabro_types::Run> {
        let mut runs = stored_runs(state).await;
        runs.sort_by_key(|run| run.timestamps.created_at);
        runs
    }

    pub(crate) fn prime_time() -> DateTime<Utc> {
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
    fn expression_change_resets_cursor_revision_only_change_preserves_timing() {
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
        let preserved_due = planner.cursors.values().next().unwrap().next_due_at;

        // fabro-b959: a revision-only change (e.g. an environment flip)
        // preserves the cursor's timing — including a boundary that is
        // already due — instead of re-arming from `now`, so metadata
        // updates never delay or drop the next scheduled fire.
        assert_eq!(original_due, first_due_time());
        assert_eq!(preserved_due, first_due_time());

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
        let projection = super::super::run_records::projection(&state, run_id)
            .await
            .unwrap()
            .expect("the run projects");
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
        let start_requests = state
            .stores
            .run_summaries
            .platform_records()
            .read(&run_id)
            .await
            .unwrap()
            .into_iter()
            .filter(|stored| {
                matches!(
                    &stored.record,
                    fabro_store::platform_records::PlatformRecord::RunLifecycle(record)
                        if record.transition
                            == fabro_store::platform_records::RunLifecycleKind::StartRequested
                )
            })
            .count();
        assert_eq!(start_requests, 1);
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
    async fn environment_only_update_preserves_schedule_evaluation() {
        let materializer = succeeding_materializer();
        let state = test_state_with_materializer(materializer.clone());
        let automation = create_automation_full(
            state.as_ref(),
            "env-flip",
            "Env flip",
            None,
            Some(fabro_automation::AutomationOverlapPolicy::Fire),
            vec![schedule_trigger("schedule", "* * * * *", true)],
        )
        .await;
        let mut planner = AutomationSchedulePlanner::default();

        run_due_schedules_once(Arc::clone(&state), &mut planner, prime_time()).await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, first_due_time()).await;
        assert_eq!(stored_runs(state.as_ref()).await.len(), 1);

        // Seed the incident's target environment so the FK on
        // `automations.environment_id` is satisfiable (the production replace
        // path validates the environment exists before writing).
        state
            .environment_store()
            .create(fabro_environment::EnvironmentDraft {
                id:       fabro_environment::EnvironmentId::new("toolchain")
                    .expect("valid environment id"),
                settings: fabro_types::settings::run::EnvironmentSettings {
                    provider: fabro_types::SandboxProviderKind::DOCKER,
                    ..fabro_types::settings::run::EnvironmentSettings::default()
                },
            })
            .await
            .expect("toolchain environment should persist");

        // Environment-only update through the store's replace path — exactly
        // the incident shape (environment_id flip, trigger untouched, still
        // enabled; revision bumps because the environment is part of the
        // canonical bytes).
        let replaced = state
            .automation_store()
            .replace(&automation.id, &automation.revision, AutomationReplace {
                name:            automation.name.clone(),
                description:     automation.description.clone(),
                environment_id:  Some("toolchain".to_string()),
                target:          automation.target.clone(),
                workflow:        automation.workflow.clone(),
                workflow_source: automation.workflow_source.clone(),
                on_overlap:      automation.on_overlap,
                triggers:        automation.triggers.clone(),
            })
            .await
            .expect("environment-only replace should succeed");
        assert_eq!(replaced.environment_id.as_deref(), Some("toolchain"));
        assert_ne!(replaced.revision, automation.revision);

        // After the update, the next cron boundary must still fire.
        run_due_schedules_once(Arc::clone(&state), &mut planner, second_due_time()).await;
        assert_eq!(
            stored_runs(state.as_ref()).await.len(),
            2,
            "schedule evaluation must survive an environment-only update"
        );
        assert_eq!(materializer.captured_inputs().len(), 2);
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
    async fn scheduled_fire_without_overlap_policy_defaults_to_skip() {
        let materializer = succeeding_materializer();
        let state = test_state_with_materializer(materializer.clone());
        create_automation(
            state.as_ref(),
            "untagged-overlap",
            "Untagged overlap",
            vec![schedule_trigger("schedule", "* * * * *", true)],
        )
        .await;
        let mut planner = AutomationSchedulePlanner::default();

        // First due fire creates a run; it stays non-terminal in this test
        // state (submitted/runnable), exactly like a run blocked at a gate.
        run_due_schedules_once(Arc::clone(&state), &mut planner, prime_time()).await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, first_due_time()).await;
        assert_eq!(stored_runs(state.as_ref()).await.len(), 1);

        // Untagged (`None`) policy resolves to Skip (fabro-fb16): the
        // second due fire is suppressed — no second run, and the
        // materializer saw only the first request.
        run_due_schedules_once(Arc::clone(&state), &mut planner, second_due_time()).await;
        assert_eq!(stored_runs(state.as_ref()).await.len(), 1);
        assert_eq!(materializer.captured_inputs().len(), 1);
    }

    #[tokio::test]
    async fn scheduled_fire_with_explicit_fire_policy_fires_despite_overlap() {
        let materializer = succeeding_materializer();
        let state = test_state_with_materializer(materializer.clone());
        create_automation_full(
            state.as_ref(),
            "fire-on-overlap",
            "Fire on overlap",
            None,
            Some(fabro_automation::AutomationOverlapPolicy::Fire),
            vec![schedule_trigger("schedule", "* * * * *", true)],
        )
        .await;
        let mut planner = AutomationSchedulePlanner::default();

        run_due_schedules_once(Arc::clone(&state), &mut planner, prime_time()).await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, first_due_time()).await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, second_due_time()).await;

        // An explicitly-set Fire policy is user intent: both fires created
        // runs even though run one is still non-terminal.
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
        // Explicit Fire: both due triggers fire in the same tick even
        // though the first run is non-terminal (the Skip default would
        // suppress the second — see
        // scheduled_fire_without_overlap_policy_defaults_to_skip).
        create_automation_full(
            state.as_ref(),
            "nightly",
            "Nightly",
            None,
            Some(fabro_automation::AutomationOverlapPolicy::Fire),
            vec![
                schedule_trigger("first", "* * * * *", true),
                schedule_trigger("second", "* * * * *", true),
            ],
        )
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
        // Explicit Fire: a merely queued (not yet started) prior run does
        // not suppress the next fire. Under the Skip default (untagged)
        // this fire would be suppressed — a queued run is non-terminal.
        create_automation_full(
            state.as_ref(),
            "nightly",
            "Nightly",
            None,
            Some(fabro_automation::AutomationOverlapPolicy::Fire),
            vec![schedule_trigger("schedule", "* * * * *", true)],
        )
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
    pub(crate) struct CapturedBreakerNotices(
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

    /// Create an automation whose schedule trigger arms the breaker at
    /// `threshold` consecutive same-signature failures (fork, fabro-3d97).
    pub(crate) async fn create_breakable_automation(
        state: &AppState,
        id: &str,
        threshold: Option<u32>,
    ) {
        use fabro_automation::{AutomationId, AutomationTriggerId, ScheduleTrigger};

        let trigger = ScheduleTrigger {
            id:                AutomationTriggerId::new(id).expect("test trigger id should parse"),
            enabled:           true,
            expression:        "* * * * *".to_string(),
            breaker_threshold: threshold,
            breaker:           None,
        };
        state
            .automation_store()
            .create(fabro_automation::AutomationDraft {
                on_overlap:      None,
                id:              AutomationId::new(id).expect("test automation id should parse"),
                name:            id.to_string(),
                description:     None,
                environment_id:  Some("default".to_string()),
                target:          target(),
                workflow:        "workflow.fabro".to_string(),
                workflow_source: None,
                triggers:        vec![fabro_automation::AutomationTrigger::Schedule(trigger)],
            })
            .await
            .expect("breakable automation should be created");
    }

    /// Seed a quota-park terminal run (fork, fabro-986b): blocked on the
    /// quota-rate-limit reason, concluded, with a rate_limit failure
    /// signature the breaker/gate classify through `is_quota_park`.
    pub(crate) async fn park_run_with_signature(state: &AppState, run_id: &RunId, signature: &str) {
        use fabro_types::outcome::{FailureCategory, FailureDetail};
        use fabro_types::{Conclusion, FailureReason, FailureSignature, RunFailure, RunStatus};

        // The parked row must stay attached to its automation for the
        // breaker/gate windows (`list_terminal_for_automation` keys on it).
        let automation = Some(fabro_types::AutomationRef {
            id:              "quota-recheck".to_string(),
            name:            Some("quota-recheck".to_string()),
            trigger_id:      Some("quota-recheck".to_string()),
            workflow_source: None,
        });

        // Fork park taxonomy (fabro-e566/986b): quota-class signatures end
        // as a resumable BLOCKED park; non-quota soft stops are ordinary
        // `Failed { SoftStop }` terminals — exactly what the breaker counts.
        let quota_park = signature.contains("rate_limit");
        let status = if quota_park {
            RunStatus::Blocked {
                blocked_reason: fabro_types::BlockedReason::QuotaRateLimit,
            }
        } else {
            RunStatus::Failed {
                reason: FailureReason::SoftStop,
            }
        };
        let _ = &automation; // used below
        seed_run_row(
            state,
            run_id,
            status,
            automation,
            Some(Conclusion {
                timestamp:            Utc::now(),
                status:               fabro_types::StageOutcome::Failed {
                    retry_requested: false,
                },
                timing:               fabro_types::RunTiming::default(),
                failure:              Some(RunFailure {
                    reason: FailureReason::SoftStop,
                    detail: {
                        let mut detail = FailureDetail::new(
                            format!("provider outage: {signature}"),
                            FailureCategory::TransientInfra,
                        );
                        detail.signature =
                            Some(FailureSignature(format!("api_transient|{signature}")));
                        detail
                    },
                }),
                final_git_commit_sha: None,
                stages:               Vec::new(),
                usage:                None,
                diff:                 fabro_types::RunDiff::default(),
                total_retries:        0,
            }),
        )
        .await;
    }

    /// Petri-native run-row seeding for breaker/gate tests: writes a
    /// projection-shaped runs row directly, the way the projector does.
    /// `conclusion` carries the terminal failure (reason + signature) the
    /// breaker reads; a terminal `status` with a conclusion marks the row
    /// terminal-with-completed for the quota-park visibility rule.
    pub(crate) async fn seed_run_row(
        state: &AppState,
        run_id: &RunId,
        status: fabro_types::RunStatus,
        automation: Option<fabro_types::AutomationRef>,
        conclusion: Option<fabro_types::Conclusion>,
    ) {
        use std::collections::HashMap;

        let mut projection = fabro_types::RunProjection::new(
            "breaker seed".to_string(),
            fabro_types::RunSpec {
                run_id: *run_id,
                settings: fabro_types::WorkflowSettings::default(),
                graph: fabro_types::RunGraph::new("test"),
                graph_source: None,
                workflow_slug: Some("test-workflow".to_string()),
                workflow_version_id: None,
                target: None,
                automation,
                source_directory: None,
                labels: HashMap::new(),
                provenance: fabro_types::test_support::test_run_provenance(),
                definition_blob: None,
                spec_blob: None,
                git: None,
                fork_source_ref: None,
                admission: fabro_types::PetriAdmission::default(),
            },
            Utc::now(),
        );
        projection.status = status;
        projection.conclusion = conclusion;
        let pool = state.stores.run_summaries.pool();
        let mut tx = pool.begin().await.expect("test tx should begin");
        fabro_store::RunSummaryStore::write_petri_run_row_on_connection(
            &mut tx,
            run_id,
            &projection,
        )
        .await
        .expect("seeded run row should write");
        tx.commit().await.expect("seed tx should commit");
    }

    pub(crate) fn breaker_test_state(
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

    /// Drive one terminal failure onto a fired run, with a failure-detail
    /// signature exactly like the run-level breaker records.
    /// Drive one terminal success onto a fired run.
    pub(crate) fn stored_breaker_trigger(
        automation: &Automation,
    ) -> fabro_automation::ScheduleTrigger {
        automation
            .triggers
            .iter()
            .find_map(|trigger| match trigger {
                AutomationTrigger::Schedule(trigger) => Some(trigger.clone()),
                AutomationTrigger::Api(_) => None,
            })
            .expect("automation should keep its schedule trigger")
    }

    pub(crate) fn due_minute(minute: u32) -> DateTime<Utc> {
        dt(&format!("2026-05-29T00:{minute:02}:00Z"))
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

    // --- fabro-79d8: automation-lag watchdog integration tests ---

    /// One enabled "* * * * *" automation with an explicit Fire overlap
    /// policy (so a follow-up fire is not suppressed by the non-terminal
    /// first run) and one fired run pinning the newest-run timestamp.
    async fn lag_watchdog_primed_state() -> (Arc<AppState>, AutomationSchedulePlanner) {
        let materializer = succeeding_materializer();
        let state = test_state_with_materializer(materializer);
        create_automation_full(
            state.as_ref(),
            "lagging",
            "Lagging",
            None,
            Some(fabro_automation::AutomationOverlapPolicy::Fire),
            vec![schedule_trigger("schedule", "* * * * *", true)],
        )
        .await;
        let mut planner = AutomationSchedulePlanner::default();
        run_due_schedules_once(Arc::clone(&state), &mut planner, prime_time()).await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, first_due_time()).await;
        assert_eq!(stored_runs(state.as_ref()).await.len(), 1);
        (state, planner)
    }

    async fn newest_run_at(state: &AppState) -> DateTime<Utc> {
        stored_runs_chronological(state)
            .await
            .last()
            .expect("at least one stored run")
            .timestamps
            .created_at
    }

    async fn lag_watchdog_automations(state: &AppState) -> Vec<Automation> {
        state
            .automation_store()
            .list()
            .await
            .expect("test automations should load")
    }

    #[tokio::test]
    async fn automation_lag_watchdog_alerts_after_more_than_two_missed_slots() {
        let (state, _planner) = lag_watchdog_primed_state().await;
        let newest_run_at = newest_run_at(state.as_ref()).await;
        let mut watchdog = AutomationLagWatchdog::default();

        // At exactly two missed minute boundaries the threshold ("exceeds
        // 2 slot intervals") is not crossed yet.
        let two_slots = newest_run_at + chrono::Duration::minutes(2);
        let automations = lag_watchdog_automations(state.as_ref()).await;
        assert!(
            check_automation_lag(state.as_ref(), &mut watchdog, &automations, two_slots)
                .await
                .is_empty()
        );

        // Four missed boundaries: the warn path fires with the full lag
        // picture (automation id, expression, expected vs newest run time).
        let stalled = newest_run_at + chrono::Duration::minutes(4);
        let alerts =
            check_automation_lag(state.as_ref(), &mut watchdog, &automations, stalled).await;
        assert_eq!(alerts.len(), 1);
        let alert = &alerts[0];
        assert_eq!(alert.automation_id.as_str(), "lagging");
        assert_eq!(alert.trigger_id.as_str(), "schedule");
        assert_eq!(alert.expression, "* * * * *");
        assert_eq!(alert.newest_run_at, newest_run_at);
        assert!(alert.expected_at <= stalled);
        assert!(alert.expected_at > newest_run_at);
        assert!(alert.lag >= chrono::Duration::minutes(3));
        assert!(alert.missed_slots > 2);
    }

    #[tokio::test]
    async fn automation_lag_watchdog_alerts_once_per_episode_and_rearms_after_a_fire() {
        let (state, mut planner) = lag_watchdog_primed_state().await;
        let first_run_at = newest_run_at(state.as_ref()).await;
        let mut watchdog = AutomationLagWatchdog::default();

        // Episode opens: one alert.
        let automations = lag_watchdog_automations(state.as_ref()).await;
        assert_eq!(
            check_automation_lag(
                state.as_ref(),
                &mut watchdog,
                &automations,
                first_run_at + chrono::Duration::minutes(4)
            )
            .await
            .len(),
            1
        );
        // Still stalled on the next tick: the latch suppresses repeats.
        assert!(
            check_automation_lag(
                state.as_ref(),
                &mut watchdog,
                &automations,
                first_run_at + chrono::Duration::minutes(5)
            )
            .await
            .is_empty()
        );

        // The automation catches up: a fresh fire moves the newest-run
        // timestamp, silently re-arming the latch.
        run_due_schedules_once(Arc::clone(&state), &mut planner, second_due_time()).await;
        assert_eq!(stored_runs(state.as_ref()).await.len(), 2);
        let second_run_at = newest_run_at(state.as_ref()).await;
        assert!(
            check_automation_lag(
                state.as_ref(),
                &mut watchdog,
                &automations,
                second_run_at + chrono::Duration::seconds(30)
            )
            .await
            .is_empty()
        );

        // A NEW stall after the catch-up alerts again (re-armed).
        assert_eq!(
            check_automation_lag(
                state.as_ref(),
                &mut watchdog,
                &automations,
                second_run_at + chrono::Duration::minutes(4)
            )
            .await
            .len(),
            1
        );
    }
}
