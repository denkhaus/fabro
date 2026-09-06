//! Automation schedule-trigger circuit breaker evaluation (fabro-3d97).
//!
//! Every scheduler pass, before firing due triggers, each enabled schedule
//! trigger's terminal-run tail is replayed chronologically past the persisted
//! high-water mark. Consecutive parks/failures with the same signature raise
//! the counter; a success or a different signature resets it. When the count
//! reaches the trigger's threshold the store disables the trigger (the same
//! visible state as the pause button) and records the breaker facts, and
//! exactly ONE aggregated notification is emitted for that pause.

use std::str::FromStr as _;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fabro_automation::{
    Automation, AutomationId, AutomationTriggerId, BreakerCounter,
    DEFAULT_SCHEDULE_BREAKER_THRESHOLD, breaker_signature, counts_as_breaker_failure,
};
use fabro_slack::blocks as slack_blocks;
use fabro_types::{EventBody, Run, RunId};
use tracing::{error, info, warn};

use super::AppState;

/// How many terminal runs per automation are replayed each pass. The
/// high-water mark keeps the working set to runs completed since the last
/// pass; the bound only caps catch-up after downtime.
const BREAKER_RUN_WINDOW: u32 = 50;

/// Facts about one breaker pause, for logging and the aggregated
/// notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BreakerPauseNotice {
    pub automation_id:   AutomationId,
    pub automation_name: String,
    pub trigger_id:      AutomationTriggerId,
    pub signature:       String,
    pub consecutive:     u32,
    pub last_run_id:     String,
    pub last_run_url:    Option<String>,
    pub paused_at:       DateTime<Utc>,
}

/// Sink for breaker-pause notifications. The production implementation posts
/// one aggregated Slack lifecycle message; tests install a capturing sink.
#[async_trait]
pub(crate) trait AutomationBreakerNotifier: Send + Sync {
    async fn notify_breaker_pause(&self, notice: &BreakerPauseNotice);
}

/// Evaluate every enabled schedule trigger of every automation. Returns
/// whether any trigger was paused by this pass (callers reload automations
/// so newly paused triggers do not fire).
pub(crate) async fn update_automation_breakers(
    state: &AppState,
    automations: &[Automation],
    now: DateTime<Utc>,
) -> bool {
    let mut paused_any = false;
    for automation in automations {
        for trigger in automation.enabled_schedule_triggers() {
            match evaluate_trigger(state, automation, trigger, now).await {
                Ok(paused) => paused_any |= paused,
                Err(err) => warn!(
                    automation_id = %automation.id,
                    trigger_id = %trigger.id,
                    error = %err,
                    "Automation breaker evaluation failed; the trigger keeps firing this pass",
                ),
            }
        }
    }
    paused_any
}

/// Errors during breaker evaluation are logged by the caller; the breaker
/// must never take the schedule down, only pause it deliberately.
type EvaluationResult = Result<bool, String>;

async fn evaluate_trigger(
    state: &AppState,
    automation: &Automation,
    trigger: &fabro_automation::ScheduleTrigger,
    now: DateTime<Utc>,
) -> EvaluationResult {
    let terminal_runs = state
        .stores
        .run_summaries
        .list_terminal_for_automation(automation.id.as_str(), BREAKER_RUN_WINDOW, now)
        .await
        .map_err(|err| err.to_string())?;
    // Newest-first from the store; replay chronologically.
    let mut trigger_runs = terminal_runs
        .iter()
        .filter(|run| {
            run.automation
                .as_ref()
                .and_then(|automation| automation.trigger_id.as_deref())
                .is_some_and(|run_trigger_id| run_trigger_id == trigger.id.as_str())
        })
        .collect::<Vec<_>>();
    trigger_runs.reverse();

    let mut counter = trigger
        .breaker
        .as_ref()
        .map_or_else(BreakerCounter::fresh, BreakerCounter::from_persisted);
    let marker = trigger
        .breaker
        .as_ref()
        .map(|breaker| breaker.last_run_id.clone());

    let processed_index = marker.as_deref().and_then(|marker| {
        trigger_runs
            .iter()
            .position(|run| run.id.to_string() == marker)
    });
    let unprocessed = if let Some(index) = processed_index {
        &trigger_runs[index + 1..]
    } else {
        // Establish the baseline on the first observation (fresh trigger or
        // one re-enabled through replace, which clears the facts): the newest
        // terminal run becomes the high-water mark without counting, so
        // counting starts with runs completing after the (re)enable — the
        // documented resume-resets-the-counter semantics (fabro-3d97).
        let Some(newest) = trigger_runs.last() else {
            return Ok(false);
        };
        persist(
            state,
            automation,
            trigger,
            &counter,
            &newest.id.to_string(),
            false,
            now,
        )
        .await?;
        return Ok(false);
    };

    let threshold = trigger
        .breaker_threshold
        .unwrap_or(DEFAULT_SCHEDULE_BREAKER_THRESHOLD);
    let mut last_processed: Option<String> = None;
    for run in unprocessed {
        last_processed = Some(run.id.to_string());
        let status = run.lifecycle.status;
        if counts_as_breaker_failure(status) {
            let failure = terminal_failure(state, run).await;
            let workflow_label = run.workflow.display_name().unwrap_or("unknown").to_string();
            let signature = breaker_signature(&workflow_label, status, failure.as_ref());
            counter.observe(true, &signature);
        } else {
            counter.observe(false, "");
        }
    }

    let Some(last_run_id) = last_processed else {
        return Ok(false);
    };
    if counter.trips_at(threshold) {
        let signature = counter.signature().unwrap_or_default().to_string();
        let paused_now = persist(
            state,
            automation,
            trigger,
            &counter,
            &last_run_id,
            true,
            now,
        )
        .await?;
        if paused_now {
            info!(
                automation_id = %automation.id,
                trigger_id = %trigger.id,
                signature = %signature,
                consecutive = counter.consecutive_count(),
                last_run_id = %last_run_id,
                "Automation circuit breaker paused schedule trigger",
            );
            emit_pause_notice(state, automation, trigger, &counter, &last_run_id, now).await;
        }
        return Ok(paused_now);
    }

    persist(
        state,
        automation,
        trigger,
        &counter,
        &last_run_id,
        false,
        now,
    )
    .await?;
    Ok(false)
}

/// Load the terminal event's failure detail for one run (the failure
/// signature lives in the `run.completed`/`run.failed` event, not the
/// summary). `None` for successes and unreadable histories — the fallback
/// signature key still groups those runs.
async fn terminal_failure(state: &AppState, run: &Run) -> Option<fabro_types::RunFailure> {
    let reader = state
        .stores
        .runs
        .open_run_reader(&run.id)
        .await
        .map_err(|err| {
            warn!(
                run_id = %run.id,
                error = %err,
                "Automation breaker could not open run events; using fallback signature",
            );
        })
        .ok()?;
    let events = reader
        .list_events()
        .await
        .map_err(|err| {
            warn!(
                run_id = %run.id,
                error = %err,
                "Automation breaker could not list run events; using fallback signature",
            );
        })
        .ok()?;
    events
        .iter()
        .rev()
        .find_map(|envelope| match &envelope.event.body {
            EventBody::RunCompleted(props) => props.failure.clone(),
            EventBody::RunFailed(props) => Some(props.failure.clone()),
            _ => None,
        })
}

async fn persist(
    state: &AppState,
    automation: &Automation,
    trigger: &fabro_automation::ScheduleTrigger,
    counter: &BreakerCounter,
    last_run_id: &str,
    pause: bool,
    now: DateTime<Utc>,
) -> Result<bool, String> {
    state
        .stores
        .automations
        .apply_schedule_breaker(
            &automation.id,
            &trigger.id,
            // A clean counter persists an empty signature so the high-water
            // mark round-trips.
            Some(counter.signature().unwrap_or("")),
            counter.consecutive_count(),
            last_run_id,
            pause,
            now,
        )
        .await
        .map_err(|err| err.to_string())
}

async fn emit_pause_notice(
    state: &AppState,
    automation: &Automation,
    trigger: &fabro_automation::ScheduleTrigger,
    counter: &BreakerCounter,
    last_run_id: &str,
    paused_at: DateTime<Utc>,
) {
    let mut notice = BreakerPauseNotice {
        automation_id: automation.id.clone(),
        automation_name: automation.name.clone(),
        trigger_id: trigger.id.clone(),
        signature: counter.signature().unwrap_or_default().to_string(),
        consecutive: counter.consecutive_count(),
        last_run_id: last_run_id.to_string(),
        last_run_url: None,
        paused_at,
    };
    // The last run's web URL is resolved opportunistically; the notification
    // is complete without it.
    match RunId::from_str(last_run_id) {
        Ok(run_id) => {
            if let Ok(Some(run)) = state.stores.run_summaries.get(&run_id, Utc::now()).await {
                notice.last_run_url.clone_from(&run.links.web);
            }
        }
        Err(err) => {
            warn!(
                run_id = %last_run_id,
                error = %err,
                "Automation breaker could not parse the last run id for its notification",
            );
        }
    }
    if let Some(notifier) = state.automation_breaker_notifier.as_ref() {
        notifier.notify_breaker_pause(&notice).await;
    }
}

/// Production notifier: ONE aggregated Slack lifecycle message per pause,
/// posted to the configured default channel when the Slack integration is
/// enabled. Per-run failure pings are unchanged; this message replaces the
/// N-th repeated one the breaker prevents.
pub(crate) struct SlackBreakerNotifier {
    service: Arc<super::SlackService>,
}

impl SlackBreakerNotifier {
    pub(super) fn new(service: Arc<super::SlackService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl AutomationBreakerNotifier for SlackBreakerNotifier {
    async fn notify_breaker_pause(&self, notice: &BreakerPauseNotice) {
        let Some(channel) = self.service.default_channel.clone() else {
            info!(
                automation_id = %notice.automation_id,
                "Slack has no default channel; automation breaker pause not posted",
            );
            return;
        };
        let blocks = slack_blocks::automation_breaker_pause_blocks(
            &slack_blocks::AutomationBreakerPauseBlocks {
                automation_id:    notice.automation_id.as_str(),
                automation_label: &notice.automation_name,
                trigger_id:       notice.trigger_id.as_str(),
                signature:        &notice.signature,
                consecutive:      notice.consecutive,
                last_run_id:      notice.last_run_id.as_str(),
                last_run_url:     notice.last_run_url.as_deref(),
            },
        );
        if let Err(err) = self.service.post_breaker_message(&channel, &blocks).await {
            error!(
                automation_id = %notice.automation_id,
                error = %err,
                "Failed to post automation breaker pause notification",
            );
        }
    }
}
