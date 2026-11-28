use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use axum::body::to_bytes;
use axum::http::{HeaderMap, StatusCode};
use chrono::{DateTime, Utc};
use croner::errors::CronError;
use fabro_automation::{
    Automation, AutomationId, AutomationRevision, AutomationTriggerId, parse_schedule_expression,
};
use fabro_types::{AutomationRef, Principal, RunId, SystemActorKind};
use fabro_util::error as error_util;
use tokio::time::{sleep, timeout};
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
    /// work#1435 gear-fire holdover: the earliest slot the running lap
    /// overran — a due boundary that arrived while the automation still
    /// had a non-terminal run, so the fire could not start and the
    /// overlap guard would have eaten it after the cursor moved on.
    /// ONE coalesced slot per cursor (later missed boundaries fold into
    /// the earliest), fired once as a catch-up when the automation goes
    /// idle. In-memory scheduler state on purpose: a pause, a breaker
    /// trip, an expression change, or a restart drops the cursor and the
    /// holdover with it — no catch-up ever crosses those boundaries, and
    /// an unpause re-arms fresh (work#1411's separate contract).
    holdover:            Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DueScheduleTrigger {
    automation: Automation,
    trigger_id: AutomationTriggerId,
    due_at:     DateTime<Utc>,
    /// True when this entry is the coalesced catch-up fire for a slot a
    /// lap overran (work#1435) instead of the cursor's regular boundary.
    catch_up:   bool,
}

#[derive(Debug, Default)]
pub(crate) struct AutomationSchedulePlanner {
    cursors: HashMap<ScheduleTriggerKey, ScheduleCursor>,
}

fn next_occurrence(expression: &str, after: DateTime<Utc>) -> Result<DateTime<Utc>, CronError> {
    parse_schedule_expression(expression)?.find_next_occurrence(&after, false)
}

/// Advance a drained cursor past `now`. `Err` marks the cursor
/// unadvanceable: the caller drops it (invalid expression, already warned
/// about by reconcile).
fn advance_cursor(
    expression: &str,
    next_due_at: &mut DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<(), CronError> {
    *next_due_at = next_occurrence(expression, now)?;
    Ok(())
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
                        // work#1435: a revision-only change must not
                        // discard a pending catch-up either.
                        holdover:            cursor.holdover,
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
                    holdover: None,
                });
            }
        }

        self.cursors = reconciled;
    }

    /// Drain the cursors whose boundary has arrived. `active` maps an
    /// automation id to its non-terminal run — the caller's overlap
    /// snapshot for this tick (the same data fabro-09ea's guard reads): a
    /// due slot of a gear whose lap is still running cannot start, and
    /// letting the guard suppress it after this drain would silently eat
    /// the slot. Instead the boundary becomes the cursor's coalesced
    /// holdover and the cursor moves on (work#1435). `skip` excludes keys
    /// whose catch-up already fired this tick, so one tick never emits
    /// two fires for one trigger — the regular slot stays armed at its
    /// untouched `next_due_at` and is accounted on a later tick.
    fn take_due(
        &mut self,
        automations: &[Automation],
        active: &HashMap<AutomationId, RunId>,
        skip: &HashSet<ScheduleTriggerKey>,
        now: DateTime<Utc>,
    ) -> Vec<DueScheduleTrigger> {
        let mut due_keys = self
            .cursors
            .iter()
            .filter(|(key, cursor)| cursor.next_due_at <= now && !skip.contains(key))
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
            // work#1435: the lap overran this boundary. Keep the EARLIEST
            // suppressed boundary as the cursor's single coalesced
            // holdover (never N pending fires), move the cursor past
            // `now`, and fire once when the gear goes idle.
            if active.contains_key(&key.automation_id) {
                if cursor.holdover.is_none() {
                    cursor.holdover = Some(due_at);
                }
                let advance = advance_cursor(&cursor.expression, &mut cursor.next_due_at, now);
                if let Err(err) = advance {
                    warn!(
                        automation_id = %key.automation_id,
                        trigger_id = %key.trigger_id,
                        error = %err,
                        "Removing automation schedule cursor after next occurrence failed",
                    );
                    self.cursors.remove(&key);
                }
                continue;
            }
            if let Err(err) = advance_cursor(&cursor.expression, &mut cursor.next_due_at, now) {
                warn!(
                    automation_id = %key.automation_id,
                    trigger_id = %key.trigger_id,
                    error = %err,
                    "Removing automation schedule cursor after next occurrence failed",
                );
                self.cursors.remove(&key);
                continue;
            }

            let Some(automation) = automations_by_id.get(&key.automation_id) else {
                continue;
            };
            due.push(DueScheduleTrigger {
                automation: (*automation).clone(),
                trigger_id: key.trigger_id,
                due_at,
                catch_up: false,
            });
        }

        due
    }

    /// Reconcile cursors against the current automation set, resolve
    /// pending holdovers, then drain due triggers. Single entry point
    /// used by the production loop and tests.
    ///
    /// `active` is the caller's overlap snapshot for this tick (automation
    /// id -> its non-terminal run; see [`active_automation_runs`]). The
    /// holdover policy is COALESCED CATCH-UP (work#1435): a lap that
    /// overran its slot costs the schedule exactly ONE fire — the earliest
    /// missed boundary, fired as soon as the gear is idle again — never N
    /// fires and never a loop (the holdover clears when it fires; a new
    /// one only forms from a fresh suppressed boundary).
    fn tick(
        &mut self,
        automations: &[Automation],
        active: &HashMap<AutomationId, RunId>,
        now: DateTime<Utc>,
    ) -> Vec<DueScheduleTrigger> {
        self.reconcile(automations, now);

        // Catch-up pass: a pending holdover of an idle automation fires
        // ONCE, on the first tick after the lap terminalized. The regular
        // boundary of this same tick stays armed (skipped in the drain
        // below): the catch-up's own run would overlap-suppress a second
        // fire, so the slot is accounted on a later tick instead of being
        // dropped.
        let mut catch_ups = Vec::new();
        let mut resolved = HashSet::new();
        let mut holdover_keys = self
            .cursors
            .iter()
            .filter(|(key, cursor)| {
                cursor.holdover.is_some() && !active.contains_key(&key.automation_id)
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        holdover_keys.sort_by(|a, b| {
            a.automation_id
                .cmp(&b.automation_id)
                .then_with(|| a.trigger_id.cmp(&b.trigger_id))
        });
        if !holdover_keys.is_empty() {
            let automations_by_id = automations
                .iter()
                .map(|automation| (&automation.id, automation))
                .collect::<HashMap<_, _>>();
            for key in holdover_keys {
                let Some(automation) = automations_by_id.get(&key.automation_id) else {
                    continue;
                };
                let Some(missed_at) = self
                    .cursors
                    .get_mut(&key)
                    .and_then(|cursor| cursor.holdover.take())
                else {
                    continue;
                };
                resolved.insert(key.clone());
                catch_ups.push(DueScheduleTrigger {
                    automation: (*automation).clone(),
                    trigger_id: key.trigger_id,
                    due_at:     missed_at,
                    catch_up:   true,
                });
            }
        }

        let mut due = self.take_due(automations, active, &resolved, now);
        catch_ups.append(&mut due);
        catch_ups
    }

    /// Put a coalesced slot back after its catch-up fire was withheld
    /// (work#1435 × fabro-986b): the provider-window holds must not
    /// consume the catch-up — re-arming keeps it owed so the first tick
    /// after the window reopens fires it once. Same minimum rule as the
    /// drain: the earliest boundary wins; an already-pending holdover is
    /// never overwritten.
    fn rearm_holdover(
        &mut self,
        automation_id: &AutomationId,
        trigger_id: &AutomationTriggerId,
        missed_at: DateTime<Utc>,
    ) {
        let key = ScheduleTriggerKey {
            automation_id: automation_id.clone(),
            trigger_id:    trigger_id.clone(),
        };
        if let Some(cursor) = self.cursors.get_mut(&key) {
            if cursor.holdover.is_none() {
                cursor.holdover = Some(missed_at);
            }
        }
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

/// Total bound for ONE scheduler tick evaluation (denkhaus/work#1510,
/// the #1143 wedge class). Every incident of this class — fork.30
/// (work#1143), fork.32, fork.44 (work#1510) — had the same shape: an
/// await inside the tick body (a credential refresh, a provider probe,
/// a store read) parked forever on a half-open connection or a stuck
/// resource, the loop never reached its trailing select, and every
/// automation fire-drifted together while the HTTP API kept serving.
/// Bounding individual awaits raced the class (the bounded refresh of
/// work#1143 did not stop the fork.32 recurrence); the tick itself is
/// the boundary. A tick that exceeds the bound is dropped (its fires
/// move on with the schedule, exactly like any other skipped slot) and
/// the loop continues — the NEXT tick re-lists and re-arms, so an
/// unpause's notify deterministically reaches a live select within one
/// bound at the very latest. The bound comfortably exceeds the slowest
/// legitimate tick (30s probes per distinct closed provider, shared
/// across automations); anything slower is a hang, not a slow tick.
const AUTOMATION_SCHEDULER_TICK_TIMEOUT: Duration = Duration::from_mins(10);

/// Run one tick evaluation under [`AUTOMATION_SCHEDULER_TICK_TIMEOUT`].
/// Returns `false` when the tick was dropped at the bound — the caller
/// logs and loops on; the scheduler can no longer park forever.
async fn bounded_scheduler_tick<F>(tick: F) -> bool
where
    F: std::future::Future<Output = ()>,
{
    bounded_scheduler_tick_with_bound(tick, AUTOMATION_SCHEDULER_TICK_TIMEOUT).await
}

/// The bound with an explicit deadline, so the regression pin can drive
/// a hanging tick with a short bound instead of the production ten
/// minutes (the `refresh_token_bounded` precedent).
async fn bounded_scheduler_tick_with_bound<F>(tick: F, bound: Duration) -> bool
where
    F: std::future::Future<Output = ()>,
{
    timeout(bound, tick).await.is_ok()
}

/// One full scheduler pass: reload automations, run the breaker and
/// provider-window gates, drain and spawn due fires, then the lag
/// watchdog. Extracted from the loop so the loop can bound the WHOLE
/// evaluation (work#1510): an unbounded await anywhere in here must
/// cost one dropped tick, never the scheduler.
async fn scheduler_tick(
    state: &Arc<AppState>,
    planner: &mut AutomationSchedulePlanner,
    lag_watchdog: &mut AutomationLagWatchdog,
) {
    let automations = match state.automation_store().list().await {
        Ok(automations) => automations,
        Err(err) => {
            // The loop's own error back-off (the trailing select) owns the
            // retry cadence; a failed tick is simply dropped.
            error!(error = ?err, "Failed to load automations for scheduler");
            return;
        }
    };
    let now = Utc::now();
    // Circuit breaker first (fabro-3d97): a trigger that just tripped
    // must not fire on this very tick, so reload when anything paused.
    let automations =
        if super::automation_breaker::update_automation_breakers(state.as_ref(), &automations, now)
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
    // Fork state (fabro-986b, shared since fabro-b869 step 4):
    // the provider window gate lives on the AppState, so the
    // run-create path and the scheduler feed the same probes.
    // The tick locks the shared gate internally, in short sections,
    // so a provider probe never blocks a concurrent create.
    let tick =
        super::fork_line_recovery::provider_gate_tick(Arc::clone(state), &automations, now).await;
    // The gate lock is released above; the trigger read model is
    // written here, in order, so a clear can never overtake a hold
    // (fabro-b869 step 5b).
    for write in &tick.fact_writes {
        let Some(automation_id) = fabro_automation::AutomationId::new(&write.automation_id).ok()
        else {
            continue;
        };
        if let Err(err) = state
            .automation_store()
            .apply_schedule_provider_window(&automation_id, &write.trigger_id, write.facts.as_ref())
            .await
        {
            warn!(
                automation_id = %write.automation_id,
                error = ?err,
                "line gate: writing provider window facts failed"
            );
        }
    }
    let held = tick.held;
    // work#1435: snapshot the overlap state BEFORE draining due
    // triggers, so a due slot of a gear whose lap overruns it is
    // held over (coalesced) by the planner instead of being
    // silently eaten by the overlap guard after the cursor moved
    // on. The snapshot is pre-tick: a run that starts between this
    // read and the spawn is still caught by the in-fire guard.
    let active = active_automation_runs(state.as_ref(), &automations).await;
    for due in planner.tick(&automations, &active, now) {
        if due.catch_up {
            info!(
                automation_id = %due.automation.id,
                trigger_id = %due.trigger_id,
                missed_slot = %due.due_at,
                "Scheduled catch-up fire: the previous lap overran the slot (work#1435 holdover)"
            );
        }
        if held.contains(due.automation.id.as_str()) {
            // A withheld catch-up stays owed (work#1435): re-arm
            // it so the first tick after the window reopens fires
            // it once. Regular slots keep their own fabro-986b
            // semantics (the boundary is past; the schedule moves
            // on).
            if due.catch_up {
                planner.rearm_holdover(&due.automation.id, &due.trigger_id, due.due_at);
            }
            info!(
                automation_id = %due.automation.id,
                trigger_id = %due.trigger_id,
                "Scheduled fire held: provider window closed (fabro-986b gate)"
            );
            continue;
        }
        if !super::fork_line_recovery::cron_fire_allowed(state.as_ref(), &due.automation, now).await
        {
            if due.catch_up {
                planner.rearm_holdover(&due.automation.id, &due.trigger_id, due.due_at);
            }
            continue;
        }
        let state = Arc::clone(state);
        let span = info_span!(
            "automation_run",
            automation_id = %due.automation.id,
            trigger_id = %due.trigger_id,
        );
        tokio::spawn(
            fire_scheduled_automation_run(state, due.automation, due.trigger_id, due.due_at)
                .instrument(span),
        );
    }
    // fabro-79d8 watchdog: after firing, turn a silent schedule
    // stall into one WARN per episode (ingestible by log monitors).
    check_automation_lag(state, lag_watchdog, &automations, now).await;
}

pub(crate) fn spawn_automation_scheduler(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut planner = AutomationSchedulePlanner::default();
        let mut lag_watchdog = AutomationLagWatchdog::default();
        let shutdown = state.shutdown_token();

        loop {
            if state.is_shutting_down() {
                break;
            }

            // work#1510: the WHOLE tick sits under one bound. A tick that
            // hits the bound is dropped and logged — the loop continues,
            // re-lists, and re-arms (an unpause's notify reaches the
            // select within one bound at the latest). The scheduler can
            // no longer park on any single unbounded await inside the
            // evaluation.
            if !Box::pin(bounded_scheduler_tick(scheduler_tick(
                &state,
                &mut planner,
                &mut lag_watchdog,
            )))
            .await
            {
                error!(
                    tick_bound_secs = AUTOMATION_SCHEDULER_TICK_TIMEOUT.as_secs(),
                    "Automation scheduler tick timed out and was dropped; the loop continues \
                     (work#1510, the #1143 wedge class — this line is the wedge made visible)"
                );
            }

            let sleep_duration = planner.sleep_duration(Utc::now());
            tokio::select! {
                () = shutdown.cancelled() => break,
                () = state.automation_scheduler_notified() => {},
                () = sleep(sleep_duration) => {},
            }
        }
    });
}

/// The overlap snapshot the scheduler's holdover accounting reads
/// (work#1435): automation id -> the newest non-terminal run, for the
/// automations whose effective scheduled overlap policy can suppress a
/// fire (`skip`, the fabro-09ea/fabro-fb16 default). An explicit `fire`
/// policy overlaps on purpose, so its slots never hold over. A store
/// error reads as idle: the authoritative fail-safe stays the overlap
/// guard inside `fire_scheduled_automation_run`, which records
/// `last_error` and skips.
async fn active_automation_runs(
    state: &AppState,
    automations: &[Automation],
) -> HashMap<AutomationId, RunId> {
    let mut active = HashMap::new();
    for automation in automations {
        if automation.effective_scheduled_overlap_policy()
            != fabro_automation::AutomationOverlapPolicy::Skip
        {
            continue;
        }
        match state
            .stores
            .run_summaries
            .active_run_for_automation(automation.id.as_str())
            .await
        {
            Ok(Some(run_id)) => {
                active.insert(automation.id.clone(), run_id);
            }
            Ok(None) => {}
            Err(err) => {
                warn!(
                    automation_id = %automation.id,
                    error = ?err,
                    "Automation overlap snapshot failed; holdover accounting treats the gear as idle",
                );
            }
        }
    }
    active
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
    // fabro-f1a9 binding gate: a fire against a missing or unconfigured
    // work project would burn a run that dies at the claim stage (the
    // overnight tracker-binding outage class). Refuse here — one line,
    // nothing starts. The in-run tracker guard stays the backstop for
    // manual/API fires and for a binding that dies between this check and
    // the run's first claim.
    if let Err(refusal) = super::fork_work_registry::binding_ready(
        state.stores.work.as_ref(),
        automation.work_project.as_deref(),
    )
    .await
    {
        warn!(
            automation_id = %automation_id,
            trigger_id = %trigger_id,
            work_project = automation.work_project.as_deref().unwrap_or("<unset>"),
            %refusal,
            "Scheduled fire refused: work-project binding is not ready (fabro-f1a9)"
        );
        return;
    }
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
        // #1234's equivalence invariant (gap 3): the manual replay —
        // `fabro create --workflow-repo <repo> --workflow-ref <ref>
        // --environment <id> ...` — and this scheduled fire compile
        // through the SAME create-from-intent admission over the SAME
        // collected version. The replay IS the reproduction of the
        // scheduler's intent: reproduce a 422 here by replaying the
        // automation's coordinate, never by guessing at internals.
        // Read the body ONCE (bounded) and surface it: the create
        // response curates the refusal detail (the diagnostics the
        // admission logged), and discarding it cost the 2026-11-09
        // outage diagnosis hours — status-only errors named nothing to
        // fix (denkhaus/work#1234).
        let body = to_bytes(response.into_body(), 16 * 1024)
            .await
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default();
        // fabro-1162 (denkhaus/work#1162): a provider-window refusal from
        // the create path's gate is a HELD fire, never a create failure.
        // The scheduler-side gate fails open for a fire whose provider set
        // is not stored yet (no terminal run with models, no recorded
        // admission set); the create path's admission is then the only
        // consumer that knows the providers, and its refusal records them
        // on the automation (fabro-0611) — so both gates hold off the
        // same stored set from the next tick. Surface the hold exactly
        // like the tick's holds: log only, `last_error` stays empty, and
        // 409 stays reserved for genuine create conflicts.
        if status == StatusCode::CONFLICT
            && super::fork_line_recovery::create_refusal_is_provider_window(body.as_bytes())
        {
            state
                .provider_gate
                .lock()
                .await
                .note_held(automation_id.as_str());
            info!(
                automation_id = %automation_id,
                trigger_id = %trigger_id,
                run_id = %run_id,
                due_at = %due_at,
                "Scheduled fire held: provider window closed (fabro-986b gate)"
            );
            return;
        }
        let detail = refusal_body_detail(&body);
        record_scheduler_error(
            state.as_ref(),
            &automation_id,
            &format!("Failed to create scheduled automation run ({status}): {detail}"),
        )
        .await;
        warn!(
            run_id = %run_id,
            due_at = %due_at,
            status = %status,
            body = %body,
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

/// A compact, bounded extraction of a create refusal's body for the
/// automation's `last_error` (denkhaus/work#1234): `title: detail` of the
/// first error when the standard error shape parses, else the raw text —
/// truncated so a pathological body cannot bloat the stored error.
fn refusal_body_detail(body: &str) -> String {
    const MAX_LEN: usize = 500;
    let compact = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            let error = value.get("errors")?.get(0)?;
            let title = error
                .get("title")
                .and_then(|title| title.as_str())
                .unwrap_or_default();
            let detail = error
                .get("detail")
                .and_then(|detail| detail.as_str())
                .unwrap_or_default();
            Some(if detail.is_empty() {
                title.to_owned()
            } else {
                format!("{title}: {detail}")
            })
        });
    let text = compact.unwrap_or_else(|| body.trim().to_owned());
    if text.len() <= MAX_LEN {
        return text;
    }
    let mut end = MAX_LEN;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
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
        tests::seed_work_project(state.as_ref()).await;
        let automations = state
            .automation_store()
            .list()
            .await
            .expect("test automations should load");
        // Mirror the production loop: the breaker runs before due triggers
        // fire, with a reload when anything paused (fabro-3d97). The
        // provider window gate (fabro-986b) runs in the production loop
        // only — its probe would hit the network; the gate's decision
        // logic is unit-tested in fork_line_recovery. The overlap snapshot
        // (work#1435) runs in BOTH paths so the holdover accounting
        // cannot drift between the loop and this harness.
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
        let active = active_automation_runs(state.as_ref(), &automations).await;
        for trigger in planner.tick(&automations, &active, now) {
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
#[expect(
    clippy::disallowed_methods,
    reason = "scheduler tests stage checkout fixtures with sync std::fs writes"
)]
pub(crate) mod tests {
    use fabro_automation::{
        AutomationDraft, AutomationGitWorkflowSource, AutomationReplace, AutomationTrigger,
        ScheduleTrigger,
    };
    use fabro_static::EnvVars;
    use fabro_types::{GitRunTarget, ResolvedAutomationGitWorkflowSource, RunStatus, RunTarget};

    use super::super::automation_breaker;
    use super::*;
    use crate::server::fork_line_recovery;
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
            provider_window: None,
        })
    }

    /// fabro-f1a9: a scheduled fire consults the work-project binding first,
    /// so a test world that expects runs registers its project and binds its
    /// automations to it. Refusal cases live in `fork_work_registry_tests`.
    pub(crate) async fn seed_work_project(state: &AppState) {
        state
            .stores
            .work
            .upsert_project(&fabro_work::WorkProject {
                project_id:               "fabro".to_owned(),
                display_name:             "Fabro".to_owned(),
                tracker_provider:         "github".to_owned(),
                tracker_repo:             "denkhaus/work".to_owned(),
                target_repo:              Some("fabro-sh/fabro".to_owned()),
                project_label:            "project:fabro".to_owned(),
                lane_labels:              std::collections::BTreeMap::new(),
                claim_label_prefix:       "fabro:run:".to_owned(),
                claim_comment_enabled:    false,
                backpressure_open_pr_cap: 0,
            })
            .await
            .expect("test work project upserts");
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
            model_providers: None,
            target: target(),
            workflow_source: None,
            workflow: "workflow.fabro".to_string(),
            work_project: Some("fabro".to_owned()),
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
        seed_work_project(state).await;
        state
            .automation_store()
            .create(AutomationDraft {
                work_project: Some("fabro".to_owned()),
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

    /// denkhaus/work#1716 (the work#1436 regression): the engine knows no
    /// workflow by name. A scheduled fire creates a run whatever the
    /// workflow is called and whatever the server-side queues hold — a
    /// project decides in its own workflow definition whether a lap is
    /// idle, the scheduler never does. The fixture is the shape the
    /// retired admission gate skipped: the workflow named `merger`, an
    /// empty store, and no session pull request waiting.
    #[tokio::test]
    async fn a_scheduled_fire_creates_a_run_whatever_the_workflow_name_and_worklist() {
        let materializer = succeeding_materializer();
        // The GitHub integration is configured but resolves no
        // credentials: the shape in which the retired gate answered
        // "positively empty" instead of failing open.
        let mut settings = crate::test_support::default_test_server_settings();
        settings.server.integrations.github.strategy =
            fabro_types::settings::server::GithubIntegrationStrategy::App;
        settings.server.integrations.github.app_id = None;
        let state = TestAppStateBuilder::new()
            .runtime_settings(settings, fabro_config::RunLayer::default())
            .env_lookup(|_| None)
            .vault_entries([(EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
            .automation_materializer(materializer.clone())
            .build();
        seed_work_project(state.as_ref()).await;
        state
            .automation_store()
            .create(AutomationDraft {
                work_project:    Some("fabro".to_owned()),
                on_overlap:      None,
                id:              AutomationId::new("named-workflow")
                    .expect("test automation id should be valid"),
                name:            "named workflow".to_string(),
                description:     None,
                environment_id:  Some("default".to_string()),
                target:          target(),
                workflow_source: None,
                workflow:        "merger".to_string(),
                triggers:        vec![schedule_trigger("schedule", "* * * * *", true)],
            })
            .await
            .expect("test automation should be created");

        // Two ticks, the planner contract: the first arms the cursor at
        // the next boundary, the second fires it.
        let mut planner = AutomationSchedulePlanner::default();
        run_due_schedules_once(state.clone(), &mut planner, prime_time()).await;
        run_due_schedules_once(state.clone(), &mut planner, first_due_time()).await;

        assert_eq!(
            materializer.captured_inputs().len(),
            1,
            "the fire reaches the materializer whatever the workflow is called"
        );
        assert_eq!(
            materializer.captured_inputs().len(),
            1,
            "the fire reaches the materializer whatever the workflow is called"
        );
        assert_eq!(
            stored_runs(state.as_ref()).await.len(),
            1,
            "the empty queues hold nobody back — one run exists behind the fire"
        );
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

        let due = planner.take_due(
            std::slice::from_ref(&automation),
            &HashMap::new(),
            &HashSet::new(),
            first_due_time(),
        );
        let second_due = planner.take_due(
            std::slice::from_ref(&automation),
            &HashMap::new(),
            &HashSet::new(),
            first_due_time(),
        );

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

    /// denkhaus/work#1411: production unpauses missed the first
    /// post-unpause slot. The planner contract the loop relies on: pause
    /// drops the cursor, and the unpause re-arm lands on the FIRST cron
    /// boundary after the unpause — no slot between unpause and the next
    /// boundary may be silently skipped.
    #[test]
    fn unpause_rearms_the_cursor_at_the_first_boundary_after_enablement() {
        let mut automation = automation("nightly", "Nightly", vec![schedule_trigger(
            "schedule",
            "* * * * *",
            true,
        )]);
        let mut planner = AutomationSchedulePlanner::default();
        planner.reconcile(std::slice::from_ref(&automation), prime_time());

        // Pause (a schedule replace with enabled = false): the next tick
        // drops the cursor, exactly as `disabled_schedule_trigger_removes_
        // cursor` pins. Ticks keep running while paused (the loop polls at
        // most every 30s).
        automation.triggers = vec![schedule_trigger("schedule", "* * * * *", false)];
        planner.reconcile(
            std::slice::from_ref(&automation),
            dt("2026-05-29T00:00:40Z"),
        );
        assert!(planner.cursors.is_empty());

        // Unpause at 00:03:10 (the replace path notifies the scheduler, so
        // this tick runs immediately, before the next boundary): the first
        // boundary after the unpause (00:04) must arm and fire.
        automation.triggers = vec![schedule_trigger("schedule", "* * * * *", true)];
        planner.reconcile(
            std::slice::from_ref(&automation),
            dt("2026-05-29T00:03:10Z"),
        );
        let due = planner.take_due(
            std::slice::from_ref(&automation),
            &HashMap::new(),
            &HashSet::new(),
            dt("2026-05-29T00:04:00Z"),
        );

        assert_eq!(due.len(), 1);
        assert_eq!(due[0].due_at, dt("2026-05-29T00:04:00Z"));
        // The cursor advanced past the fired boundary.
        assert_eq!(
            planner.cursors.values().next().unwrap().next_due_at,
            dt("2026-05-29T00:05:00Z")
        );
    }

    /// An active-lap snapshot for one automation, the way
    /// `active_automation_runs` builds it for the planner's holdover
    /// accounting (work#1435).
    fn active_runs(automation: &Automation) -> HashMap<AutomationId, RunId> {
        HashMap::from([(automation.id.clone(), RunId::new())])
    }

    /// work#1435, the planner contract: every boundary that arrives while
    /// the gear's lap is still running folds into ONE coalesced holdover
    /// (the earliest missed slot), which fires exactly once as a catch-up
    /// on the first tick after the gear goes idle — never N fires, and
    /// the next tick does not re-fire the catch-up.
    #[test]
    fn overran_slots_coalesce_into_one_catch_up_fire_on_idle() {
        let automation = automation("gear", "Gear", vec![schedule_trigger(
            "schedule",
            "* * * * *",
            true,
        )]);
        let mut planner = AutomationSchedulePlanner::default();
        planner.reconcile(std::slice::from_ref(&automation), prime_time());
        let active = active_runs(&automation);

        // Slot 00:01 arrives while the lap runs: held over, not fired, and
        // the cursor moved on past the boundary.
        let due = planner.tick(std::slice::from_ref(&automation), &active, first_due_time());
        assert!(due.is_empty());
        let cursor = planner.cursors.values().next().unwrap();
        assert_eq!(cursor.holdover, Some(first_due_time()));
        assert_eq!(cursor.next_due_at, second_due_time());

        // Slots 00:02 and 00:03 also overrun: they fold into the SAME
        // earliest holdover — one pending slot, not three.
        let due = planner.tick(
            std::slice::from_ref(&automation),
            &active,
            second_due_time(),
        );
        assert!(due.is_empty());
        let cursor = planner.cursors.values().next().unwrap();
        assert_eq!(cursor.holdover, Some(first_due_time()));
        assert_eq!(cursor.next_due_at, due_minute(3));

        let due = planner.tick(std::slice::from_ref(&automation), &active, due_minute(3));
        assert!(due.is_empty());
        let cursor = planner.cursors.values().next().unwrap();
        assert_eq!(cursor.holdover, Some(first_due_time()));
        assert_eq!(cursor.next_due_at, due_minute(4));

        // The lap terminalizes: the very next tick fires the catch-up ONCE
        // — for the earliest missed slot — while the regular 00:04 slot
        // stays armed (one fire per tick per trigger).
        let idle = HashMap::new();
        let due = planner.tick(std::slice::from_ref(&automation), &idle, due_minute(4));
        assert_eq!(due.len(), 1);
        assert!(due[0].catch_up);
        assert_eq!(due[0].due_at, first_due_time());
        let cursor = planner.cursors.values().next().unwrap();
        assert_eq!(cursor.holdover, None);
        assert_eq!(cursor.next_due_at, due_minute(4));

        // The next tick does not re-fire the catch-up: the armed regular
        // slot fires as itself.
        let due = planner.tick(std::slice::from_ref(&automation), &idle, due_minute(5));
        assert_eq!(due.len(), 1);
        assert!(!due[0].catch_up);
        assert_eq!(due[0].due_at, due_minute(4));
    }

    /// work#1435 × work#1411 boundary: a pending holdover must not cross a
    /// pause. Disabling the trigger drops the cursor (and the holdover
    /// with it); the unpause re-arms fresh at the first boundary — the
    /// catch-up never fires across the pause boundary.
    #[test]
    fn pause_drops_a_pending_holdover_and_unpause_rearms_fresh() {
        let mut automation = automation("nightly", "Nightly", vec![schedule_trigger(
            "schedule",
            "* * * * *",
            true,
        )]);
        let mut planner = AutomationSchedulePlanner::default();
        planner.reconcile(std::slice::from_ref(&automation), prime_time());

        // The 00:01 slot overruns; the holdover is pending when the gear
        // is paused at 00:01:10.
        let active = active_runs(&automation);
        let due = planner.tick(std::slice::from_ref(&automation), &active, first_due_time());
        assert!(due.is_empty());
        assert_eq!(
            planner.cursors.values().next().unwrap().holdover,
            Some(first_due_time())
        );

        automation.triggers = vec![schedule_trigger("schedule", "* * * * *", false)];
        planner.reconcile(
            std::slice::from_ref(&automation),
            dt("2026-05-29T00:01:10Z"),
        );
        assert!(planner.cursors.is_empty());

        automation.triggers = vec![schedule_trigger("schedule", "* * * * *", true)];
        planner.reconcile(
            std::slice::from_ref(&automation),
            dt("2026-05-29T00:03:10Z"),
        );
        let due = planner.tick(
            std::slice::from_ref(&automation),
            &HashMap::new(),
            dt("2026-05-29T00:04:00Z"),
        );
        assert_eq!(due.len(), 1);
        assert!(!due[0].catch_up, "no catch-up across the pause boundary");
        assert_eq!(due[0].due_at, dt("2026-05-29T00:04:00Z"));
    }

    /// work#1435 × fabro-986b: a catch-up fire withheld by a provider
    /// window hold must stay owed — the loop re-arms the holdover, and the
    /// next idle tick emits it once again. Clearing on emission is what
    /// keeps this from looping.
    #[test]
    fn withheld_catch_up_rearms_and_fires_once_after_the_window_reopens() {
        let automation = automation("gear", "Gear", vec![schedule_trigger(
            "schedule",
            "* * * * *",
            true,
        )]);
        let mut planner = AutomationSchedulePlanner::default();
        planner.reconcile(std::slice::from_ref(&automation), prime_time());

        // The 00:01 slot overruns (lap active), the lap terminalizes, and
        // the catch-up fires — but the provider window hold withholds it,
        // so the loop re-arms the holdover.
        let active = active_runs(&automation);
        let due = planner.tick(std::slice::from_ref(&automation), &active, first_due_time());
        assert!(due.is_empty());
        let idle = HashMap::new();
        let due = planner.tick(std::slice::from_ref(&automation), &idle, second_due_time());
        assert_eq!(due.len(), 1);
        assert!(due[0].catch_up);
        planner.rearm_holdover(&automation.id, &due[0].trigger_id, due[0].due_at);

        // Window reopens: the next tick emits the SAME catch-up once, and
        // the regular slot stays armed behind it.
        let due = planner.tick(std::slice::from_ref(&automation), &idle, due_minute(3));
        assert_eq!(due.len(), 1);
        assert!(due[0].catch_up);
        assert_eq!(due[0].due_at, first_due_time());
        let cursor = planner.cursors.values().next().unwrap();
        assert_eq!(cursor.holdover, None);
        assert_eq!(cursor.next_due_at, second_due_time());

        // No re-arm this time: the schedule resumes at the regular slot.
        let due = planner.tick(std::slice::from_ref(&automation), &idle, due_minute(4));
        assert_eq!(due.len(), 1);
        assert!(!due[0].catch_up);
        assert_eq!(due[0].due_at, second_due_time());
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
        let due = planner.take_due(
            std::slice::from_ref(&automation),
            &HashMap::new(),
            &HashSet::new(),
            first_due_time(),
        );

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
                work_project:    automation.work_project.clone(),
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

    /// The 2026-11-09 incident's exact shape (denkhaus/work#1233 + #1234
    /// gap 3): a workflow.toml authoring
    /// `dockerfile = { path = "..." }`. Production materializes by
    /// collecting the repo fresh per fire; the collected bytes must carry
    /// the inline text, and the create path must ADMIT them — manual and
    /// scheduled compiles see the same source.
    #[tokio::test]
    async fn a_path_dockerfile_workflow_fires_green_on_the_scheduled_path() {
        let temp = tempfile::tempdir().unwrap();
        let workflow_dir = temp.path().join(".fabro/workflows/root");
        std::fs::create_dir_all(&workflow_dir).unwrap();
        std::fs::write(
            workflow_dir.join("workflow.toml"),
            r#"_version = 1

[workflow]
graph = "workflow.fabro"

[environments.toolchain]
provider = "docker"

[environments.toolchain.image]
dockerfile = { path = "Dockerfile" }
"#,
        )
        .unwrap();
        std::fs::write(
            workflow_dir.join("workflow.fabro"),
            r"digraph Root { start [shape=Mdiamond] exit [shape=Msquare] start -> exit }",
        )
        .unwrap();
        std::fs::write(workflow_dir.join("Dockerfile"), "FROM probe\n").unwrap();

        let closure =
            fabro_manifest::collect_workflow_versions(std::path::Path::new("root"), temp.path())
                .expect("the { path } workflow should collect");
        let root = closure
            .versions()
            .find(|(_, version)| {
                version.version().entrypoint().as_str() == ".fabro/workflows/root/workflow.fabro"
            })
            .expect("root version collected")
            .1;
        let version = fabro_workflow_version::ValidatedWorkflowVersion::new(
            fabro_types::WorkflowVersion::new(
                root.version().entrypoint().clone(),
                root.version().files().clone(),
                root.version().workflow_dependencies().clone(),
            )
            .expect("collected version reshapes"),
        )
        .expect("collected version revalidates");

        let materializer =
            TestAutomationRunMaterializer::succeed_with_version(git_target(), version);
        let state = test_state_with_materializer(materializer);
        // The launch must SELECT the environment: Petri parses an
        // environment's image only when a launch or `[run.environment]`
        // names it (the production fire carried `--environment
        // toolchain`).
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
        seed_work_project(state.as_ref()).await;
        state
            .automation_store()
            .create(AutomationDraft {
                work_project:    Some("fabro".to_owned()),
                on_overlap:      None,
                id:              AutomationId::new("path-form")
                    .expect("test automation id is valid"),
                name:            "Path form".to_string(),
                description:     None,
                environment_id:  Some("toolchain".to_string()),
                target:          target(),
                workflow_source: None,
                workflow:        "workflow.fabro".to_string(),
                triggers:        vec![schedule_trigger("schedule", "* * * * *", true)],
            })
            .await
            .expect("test automation should be created");
        let mut planner = AutomationSchedulePlanner::default();

        run_due_schedules_once(Arc::clone(&state), &mut planner, prime_time()).await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, first_due_time()).await;

        assert_eq!(
            stored_runs(state.as_ref()).await.len(),
            1,
            "the {{ path }} dockerfile workflow creates — the collector's \
             persist-time inlining feeds Petri inline text"
        );
    }

    #[tokio::test]
    async fn scheduled_fire_is_refused_when_a_required_provider_has_no_stored_credential() {
        // Only openai carries a credential in this state; the automation's
        // workflow needs zai, so the fire is refused at admission and no
        // run is created (fabro-b46e): the miss is a scheduler error, not
        // a run that dies inside its first model stage.
        let mut exact_target = git_target();
        exact_target.sha = Some("0123456789abcdef0123456789abcdef01234567".to_string());
        let materializer = TestAutomationRunMaterializer::succeed_with_workflow(
            exact_target,
            "digraph Test { graph [goal=\"Test\"] start [shape=Mdiamond] work [shape=box, prompt=\"Do the work\", model=\"glm-4.7\", provider=\"zai\"] exit [shape=Msquare] start -> work -> exit }",
        );
        let state = TestAppStateBuilder::new()
            .env_lookup(|_| None)
            .vault_entries([(EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
            .automation_materializer(materializer)
            .build();
        create_automation(state.as_ref(), "needs-zai", "Needs zai", vec![
            schedule_trigger("schedule", "* * * * *", true),
        ])
        .await;
        let mut planner = AutomationSchedulePlanner::default();

        run_due_schedules_once(Arc::clone(&state), &mut planner, prime_time()).await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, first_due_time()).await;

        assert!(
            stored_runs(state.as_ref()).await.is_empty(),
            "a refused fire must create no run"
        );
        let automation = state
            .automation_store()
            .list()
            .await
            .expect("test automations should list")
            .into_iter()
            .find(|automation| automation.id.as_str() == "needs-zai")
            .expect("the automation should still exist");
        let last_error = automation.last_error.as_deref().unwrap_or_default();
        assert!(
            last_error.contains("Failed to create scheduled automation run"),
            "the refusal should surface as the automation's last error: {last_error}"
        );
        // Gap 1 (denkhaus/work#1234): the create response body carries the
        // curated refusal detail; the scheduler error must surface it, not
        // discard it — the 2026-11-09 outage diagnosis read status-only
        // errors for hours while the body named the `{ path }` defect.
        assert!(
            last_error.contains("zai"),
            "the last error carries the refusal body's detail (the missing \
             provider): {last_error}"
        );
    }

    #[tokio::test]
    async fn a_refused_first_fire_teaches_the_gate_the_model_providers() {
        // fabro-0611 (b869 step 3): an automation whose workflow never ran
        // has no run history to derive providers from. The FIRST fire still
        // pays admission — and its refusal records the admission's model
        // providers on the automation, so the NEXT tick can hold cleanly
        // instead of burning another materialization.
        let exact_target = git_target();
        let materializer = TestAutomationRunMaterializer::succeed_with_workflow(
            exact_target,
            "digraph Test { graph [goal=\"Test\"] start [shape=Mdiamond] work [shape=box, prompt=\"Do the work\", model=\"glm-4.7\", provider=\"zai\"] exit [shape=Msquare] start -> work -> exit }",
        );
        let state = TestAppStateBuilder::new()
            .env_lookup(|_| None)
            .vault_entries([(EnvVars::ZAI_API_KEY, "test-zai-key")])
            .automation_materializer(materializer)
            .build();
        create_automation(state.as_ref(), "needs-zai", "Needs zai", vec![
            schedule_trigger("schedule", "* * * * *", true),
        ])
        .await;
        // The gate knows zai's window is closed (probed just now, so the
        // fire's on-demand check reads the remembered answer, no network).
        {
            let mut gate = state.provider_gate.lock().await;
            gate.note_probe(
                "zai",
                fork_line_recovery::ProviderWindow::Closed,
                "zai/glm-4.7",
                chrono::Utc::now(),
            );
        }
        let mut planner = AutomationSchedulePlanner::default();

        run_due_schedules_once(Arc::clone(&state), &mut planner, prime_time()).await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, first_due_time()).await;

        assert!(
            stored_runs(state.as_ref()).await.is_empty(),
            "a refused fire must create no run"
        );
        let automation = state
            .automation_store()
            .get(&fabro_automation::AutomationId::new("needs-zai").unwrap())
            .await
            .unwrap()
            .expect("the automation should still exist");
        let learned = automation
            .model_providers
            .as_ref()
            .expect("the refused fire records the admission's providers");
        assert_eq!(learned.len(), 1, "unexpected learned set: {learned:?}");
        assert_eq!(learned[0].provider, "zai");
        assert_eq!(learned[0].model.as_deref(), Some("glm-4.7"));

        // The learned set holds the NEXT due fire before materialization:
        // the tick holds the automation and writes the trigger facts.
        let automations = state.automation_store().list().await.unwrap();
        let tick = fork_line_recovery::provider_gate_tick(
            Arc::clone(&state),
            &automations,
            second_due_time(),
        )
        .await;
        assert!(
            tick.held.contains("needs-zai"),
            "the never-run automation is held off its learned providers"
        );
        assert!(
            !tick.fact_writes.is_empty(),
            "the hold is readable on the trigger (WINDOW column)"
        );
    }

    #[tokio::test]
    async fn a_provider_window_refused_scheduled_fire_is_a_hold_not_a_create_failure() {
        // fabro-1162 (denkhaus/work#1162): a first fire whose provider set
        // the automation has not stored yet sails through the
        // scheduler-side gate (fail-open) and is refused by the create
        // path's admission gate. That refusal must surface as a HELD fire
        // — the same observable state as every other held automation —
        // never as a create failure leaking "409 Conflict" onto
        // `last_error`. The admission records zai on the automation, and
        // the NEXT tick holds off that stored set: scheduler gate and
        // create-path gate agree on one provider resolution.
        let exact_target = git_target();
        let materializer = TestAutomationRunMaterializer::succeed_with_workflow(
            exact_target,
            "digraph Test { graph [goal=\"Test\"] start [shape=Mdiamond] work [shape=box, prompt=\"Do the work\", model=\"glm-4.7\", provider=\"zai\"] exit [shape=Msquare] start -> work -> exit }",
        );
        let state = TestAppStateBuilder::new()
            .env_lookup(|_| None)
            .vault_entries([(EnvVars::ZAI_API_KEY, "test-zai-key")])
            .automation_materializer(materializer)
            .build();
        create_automation(state.as_ref(), "needs-zai", "Needs zai", vec![
            schedule_trigger("schedule", "* * * * *", true),
        ])
        .await;
        // The gate knows zai's window is closed (probed just now, so the
        // create path's on-demand check reads the remembered answer, no
        // network).
        {
            let mut gate = state.provider_gate.lock().await;
            gate.note_probe(
                "zai",
                fork_line_recovery::ProviderWindow::Closed,
                "zai/glm-4.7",
                chrono::Utc::now(),
            );
        }
        let mut planner = AutomationSchedulePlanner::default();

        run_due_schedules_once(Arc::clone(&state), &mut planner, prime_time()).await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, first_due_time()).await;

        assert!(
            stored_runs(state.as_ref()).await.is_empty(),
            "a refused fire must create no run"
        );
        let automation = state
            .automation_store()
            .get(&fabro_automation::AutomationId::new("needs-zai").unwrap())
            .await
            .unwrap()
            .expect("the automation should still exist");
        assert!(
            automation.last_error.is_none(),
            "a provider-window hold must leave the automation's last_error empty \
             (fabro-1162): {:?}",
            automation.last_error
        );
        // The hold is marked in the gate, so the reopen transition
        // (rewind-or-fire) owns recovery exactly like a tick hold.
        assert!(
            state.provider_gate.lock().await.is_held("needs-zai"),
            "the refused fire marks the automation held in the gate"
        );

        // Scheduler-gate/create-path agreement (fabro-1162): the create
        // path's admission recorded zai on the automation, and the next
        // tick holds off that stored admission set — no second 409.
        let automations = state.automation_store().list().await.unwrap();
        let tick = fork_line_recovery::provider_gate_tick(
            Arc::clone(&state),
            &automations,
            second_due_time(),
        )
        .await;
        assert!(
            tick.held.contains("needs-zai"),
            "the next tick holds the automation off the admission-recorded providers"
        );
    }

    #[tokio::test]
    async fn the_tick_holds_a_never_run_automation_off_its_stored_providers() {
        // The pure gate fallback (fabro-0611): without any run, the stored
        // admission set decides; with a closed window the automation is
        // held and `cron_fire_allowed` refuses — no materialization, no
        // scheduler error. A deterministic workflow (empty set) consults
        // nothing.
        let materializer = succeeding_materializer();
        let state = test_state_with_materializer(materializer);
        create_automation(state.as_ref(), "never-ran", "Never ran", vec![
            schedule_trigger("schedule", "* * * * *", true),
        ])
        .await;
        create_automation(state.as_ref(), "deterministic", "Deterministic", vec![
            schedule_trigger("schedule", "* * * * *", true),
        ])
        .await;
        let now = prime_time();
        state
            .automation_store()
            .set_model_providers(
                &fabro_automation::AutomationId::new("never-ran").unwrap(),
                Some(&[fabro_automation::ModelProviderRequirement {
                    provider: "zai".to_string(),
                    model:    Some("glm-4.7".to_string()),
                }]),
            )
            .await
            .unwrap();
        state
            .automation_store()
            .set_model_providers(
                &fabro_automation::AutomationId::new("deterministic").unwrap(),
                Some(&[]),
            )
            .await
            .unwrap();
        {
            let mut gate = state.provider_gate.lock().await;
            gate.note_probe(
                "zai",
                fork_line_recovery::ProviderWindow::Closed,
                "zai/glm-4.7",
                now,
            );
        }

        let automations = state.automation_store().list().await.unwrap();
        let tick =
            fork_line_recovery::provider_gate_tick(Arc::clone(&state), &automations, now).await;
        assert!(
            tick.held.contains("never-ran"),
            "a never-run workflow is gated off its stored providers"
        );
        assert!(
            !tick.held.contains("deterministic"),
            "a workflow with no model stages is never held"
        );
        assert_eq!(
            tick.fact_writes.len(),
            1,
            "only the held trigger carries facts"
        );

        let never_ran = automations
            .iter()
            .find(|automation| automation.id.as_str() == "never-ran")
            .unwrap();
        assert!(
            !fork_line_recovery::cron_fire_allowed(state.as_ref(), never_ran, now).await,
            "a due cron fire of the never-run automation is refused"
        );
        let deterministic = automations
            .iter()
            .find(|automation| automation.id.as_str() == "deterministic")
            .unwrap();
        assert!(
            fork_line_recovery::cron_fire_allowed(state.as_ref(), deterministic, now).await,
            "a deterministic workflow consults nothing and always fires"
        );
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

    /// work#1435, end to end over the shared tick: a lap overruns slot X
    /// (the gear stays active), the next tick holds the slot over instead
    /// of eating it, the lap terminalizes, the catch-up fires ONCE, and
    /// the following tick does not re-fire (the catch-up's own lap is now
    /// the active run, so the next slot holds over instead).
    #[tokio::test]
    async fn overran_slot_fires_once_as_catch_up_after_the_lap_terminalizes() {
        let materializer = succeeding_materializer();
        let state = test_state_with_materializer(materializer.clone());
        create_automation(state.as_ref(), "overrun", "Overrun", vec![
            schedule_trigger("schedule", "* * * * *", true),
        ])
        .await;
        let mut planner = AutomationSchedulePlanner::default();

        // The 00:01 slot starts the lap (non-terminal in this test state,
        // like a run blocked at a gate).
        run_due_schedules_once(Arc::clone(&state), &mut planner, prime_time()).await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, first_due_time()).await;
        assert_eq!(stored_runs(state.as_ref()).await.len(), 1);

        // Slot 00:02 arrives while the lap runs: no second run — the slot
        // is held over, not silently eaten by the overlap guard.
        run_due_schedules_once(Arc::clone(&state), &mut planner, second_due_time()).await;
        assert_eq!(stored_runs(state.as_ref()).await.len(), 1);
        assert_eq!(materializer.captured_inputs().len(), 1);

        // The lap terminalizes; the next tick fires the held-over slot as
        // ONE catch-up run.
        let lap = stored_runs_chronological(state.as_ref()).await.remove(0);
        seed_run_row(
            state.as_ref(),
            &lap.id,
            RunStatus::Succeeded {
                reason: fabro_types::SuccessReason::Completed,
            },
            lap.automation.clone(),
            None,
        )
        .await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, due_minute(3)).await;
        let runs = stored_runs(state.as_ref()).await;
        assert_eq!(runs.len(), 2);
        assert_eq!(materializer.captured_inputs().len(), 2);

        // The next tick does not re-fire the catch-up: the catch-up's own
        // run is now the active lap, so the 00:03 slot holds over instead
        // of starting a third run.
        run_due_schedules_once(Arc::clone(&state), &mut planner, due_minute(4)).await;
        assert_eq!(stored_runs(state.as_ref()).await.len(), 2);
        assert_eq!(materializer.captured_inputs().len(), 2);
    }

    /// work#1435 scoping: the overlap snapshot covers only automations
    /// whose effective policy can suppress a fire. An explicit `fire`
    /// policy overlaps on purpose (fabro-09ea) — its slots must never
    /// hold over, so the snapshot excludes it and every boundary fires
    /// immediately.
    #[tokio::test]
    async fn overlap_snapshot_covers_only_skip_policy_automations() {
        let state = test_state_with_materializer(succeeding_materializer());
        create_automation_full(
            state.as_ref(),
            "fire-gear",
            "Fire gear",
            None,
            Some(fabro_automation::AutomationOverlapPolicy::Fire),
            vec![schedule_trigger("schedule", "* * * * *", true)],
        )
        .await;
        create_automation(state.as_ref(), "skip-gear", "Skip gear", vec![
            schedule_trigger("schedule", "* * * * *", true),
        ])
        .await;
        // One non-terminal run for each automation (runnable in this test
        // state), the way an overrunning lap would leave them.
        for (id, name) in [("fire-gear", "Fire gear"), ("skip-gear", "Skip gear")] {
            seed_run_row(
                state.as_ref(),
                &RunId::new(),
                RunStatus::Runnable,
                Some(AutomationRef {
                    id:              id.to_string(),
                    name:            Some(name.to_string()),
                    trigger_id:      Some("schedule".to_string()),
                    workflow_source: None,
                }),
                None,
            )
            .await;
        }

        let automations = state
            .automation_store()
            .list()
            .await
            .expect("test automations should load");
        let active = active_automation_runs(state.as_ref(), &automations).await;

        assert!(active.contains_key(
            &AutomationId::new("skip-gear").expect("test automation id should be valid")
        ));
        assert!(!active.contains_key(
            &AutomationId::new("fire-gear").expect("test automation id should be valid")
        ));
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
            provider_window:   None,
        };
        seed_work_project(state).await;
        state
            .automation_store()
            .create(fabro_automation::AutomationDraft {
                work_project:    Some("fabro".to_owned()),
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

    /// fabro-f1a9 fire gate: a scheduled fire whose work-project binding is
    /// missing, unknown, or unwired creates no run and never reaches the
    /// materializer — the overnight outage class (a run burning against a
    /// dead tracker binding) must be impossible. Binding the same
    /// automation fires on the next tick, so the gate refuses only dead
    /// bindings.
    #[tokio::test]
    async fn a_fire_with_a_dead_work_project_binding_creates_no_run() {
        let materializer = succeeding_materializer();
        let state = test_state_with_materializer(materializer.clone());
        seed_work_project(state.as_ref()).await;
        for (id, work_project) in [
            ("unbound", None),
            ("unknown", Some("absent".to_owned())),
            ("unwired", Some("blank".to_owned())),
        ] {
            state
                .automation_store()
                .create(AutomationDraft {
                    work_project,
                    on_overlap: None,
                    id: AutomationId::new(id).expect("test id is valid"),
                    name: id.to_owned(),
                    description: None,
                    environment_id: Some("default".to_owned()),
                    target: target(),
                    workflow: "workflow.fabro".to_owned(),
                    workflow_source: None,
                    triggers: vec![schedule_trigger("schedule", "* * * * *", true)],
                })
                .await
                .expect("test automation should be created");
        }
        state
            .stores
            .work
            .upsert_project(&fabro_work::WorkProject {
                project_id:               "blank".to_owned(),
                display_name:             "Blank".to_owned(),
                tracker_provider:         String::new(),
                tracker_repo:             String::new(),
                target_repo:              None,
                project_label:            "project:blank".to_owned(),
                lane_labels:              std::collections::BTreeMap::new(),
                claim_label_prefix:       "fabro:run:".to_owned(),
                claim_comment_enabled:    false,
                backpressure_open_pr_cap: 0,
            })
            .await
            .expect("blank project stores");

        let mut planner = AutomationSchedulePlanner::default();
        run_due_schedules_once(Arc::clone(&state), &mut planner, prime_time()).await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, first_due_time()).await;

        assert!(stored_runs(state.as_ref()).await.is_empty());
        assert!(materializer.captured_inputs().is_empty());

        // Positive control: the same schedule bound to the wired project
        // fires on the next tick — the gate refuses dead bindings only.
        let bound = state
            .automation_store()
            .get(&AutomationId::new("unbound").expect("id is valid"))
            .await
            .expect("automations load")
            .expect("unbound automation exists");
        state
            .automation_store()
            .replace(&bound.id, &bound.revision, AutomationReplace {
                work_project:    Some("fabro".to_owned()),
                name:            bound.name.clone(),
                description:     bound.description.clone(),
                environment_id:  bound.environment_id.clone(),
                on_overlap:      bound.on_overlap,
                target:          bound.target.clone(),
                workflow:        bound.workflow.clone(),
                workflow_source: bound.workflow_source.clone(),
                triggers:        bound.triggers.clone(),
            })
            .await
            .expect("binding the automation replaces it");
        run_due_schedules_once(Arc::clone(&state), &mut planner, second_due_time()).await;

        assert_eq!(stored_runs(state.as_ref()).await.len(), 1);
        assert_eq!(materializer.captured_inputs().len(), 1);
    }

    /// denkhaus/work#1510 (the #1143 wedge class): a tick evaluation that
    /// hangs forever must be DROPPED at its bound, returning control to the
    /// loop — the fork.30/fork.32/fork.44 incidents all had the loop parked
    /// inside the body while the HTTP API kept serving. A never-resolving
    /// future under a short bound returns false instead of parking.
    #[tokio::test]
    async fn a_hanging_scheduler_tick_is_dropped_at_its_bound() {
        let started = std::time::Instant::now();
        let completed = bounded_scheduler_tick_with_bound(
            std::future::pending::<()>(),
            Duration::from_millis(50),
        )
        .await;
        assert!(
            !completed,
            "a hanging tick must be dropped at the bound, not park the loop"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the bound must actually release the caller"
        );
    }

    /// The completed-tick control for the pin above: a tick that finishes
    /// inside its bound reports completion.
    #[tokio::test]
    async fn a_completing_scheduler_tick_reports_completion() {
        let completed = bounded_scheduler_tick_with_bound(async {}, Duration::from_secs(5)).await;
        assert!(completed, "a completing tick must report completion");
    }

    /// Source-level pin (the fork_line_recovery scheduler-call pin
    /// precedent): the spawned loop must route its tick evaluation through
    /// the bound — an unbounded call of `scheduler_tick` would reintroduce
    /// the wedge class one refactor away.
    #[test]
    fn the_spawned_loop_bounds_the_whole_tick_evaluation() {
        let source = include_str!("automation_scheduler.rs");
        assert!(
            source.contains("bounded_scheduler_tick(scheduler_tick("),
            "spawn_automation_scheduler must run scheduler_tick under the \
             tick bound (work#1510): an unbounded tick body is the wedge class"
        );
        assert!(
            source.contains("const AUTOMATION_SCHEDULER_TICK_TIMEOUT"),
            "the tick bound must stay a named constant"
        );
    }
}
