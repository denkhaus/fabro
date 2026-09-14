//! Fork-only line recovery: provider window gate (fabro-986b, user
//! decision 2026-09-14 — revised the same day after review: "die derzeitige
//! implementation ist keine Lösung, vor jedem cron run muss der llm provider
//! auf 429 gecheckt werden").
//!
//! The first revision fired recheck-probe RUNS every 10 minutes while a
//! quota park sat on the line — producing useless run artifacts and noise
//! for as long as the provider window stays closed. This revision gates
//! FIRES instead of creating runs:
//!
//! - BEFORE every scheduled fire (cron or reopen), the scheduler probes the
//!   provider with the same basic model test the server's model-test endpoint
//!   uses (a one-word completion through the vault credential, ~20 input
//!   tokens). Window closed -> NO run is created; the fire is skipped and
//!   logged.
//! - While a window is closed, the gate polls the provider on the fixed
//!   10-minute recheck cadence (user decision 2026-09-14) and fires the real
//!   pass through the normal scheduled-fire path the moment the window reopens.
//!   Zero runs during an outage.
//! - Quota-class parks (SoftStop + TransientInfra + `rate_limit` signature)
//!   remain breaker-exempt, and the workflow-engine park classification stays
//!   as defense in depth: a run whose window closes MID-FLIGHT still parks
//!   resumable instead of burning retries.
//!
//! The gate probe resolves the model from the automation's newest
//! terminal run (`RunModel { provider, name }`); without a resolvable
//! model or client the gate FAILS OPEN (one bounded parked run per
//! window, never a dead line on a client hiccup).
//!
//! Fork-file policy: this file exists only on our fork; upstream does
//! not have it, so no merge can conflict it away.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use fabro_types::outcome::FailureCategory;
use fabro_types::{Run, RunFailure, RunStatus};

use super::AppState;

/// Recheck probe cadence (user decision 2026-09-14: "fixe rechecks alle
/// 10 Minuten").
pub(crate) const RECHECK_INTERVAL_SECS: i64 = 600;

/// Whether a terminal run is a quota-class park the recheck owns.
///
/// Structural detection only: a SoftStop failure with a TransientInfra
/// category and a `rate_limit` signature detail (spelled `rate_limit` or
/// `rate_limited` depending on provider). The message prose is never
/// parsed here.
#[must_use]
pub(crate) fn is_quota_park(status: RunStatus, failure: Option<&RunFailure>) -> bool {
    if !matches!(status, RunStatus::Failed {
        reason: fabro_types::FailureReason::SoftStop,
    }) {
        return false;
    }
    let Some(failure) = failure else {
        return false;
    };
    if failure.detail.category != FailureCategory::TransientInfra {
        return false;
    }
    failure
        .detail
        .signature
        .as_ref()
        .is_some_and(|signature| signature.as_str().contains("rate_limit"))
}

/// In-memory gate bookkeeping (derived, not persisted).
///
/// `last_poll` bounds the closed-window probe cadence; `closed` marks
/// automations whose provider window was last seen closed. A server
/// restart loses the marks — the next due fire probes fresh and re-marks.
#[derive(Default)]
pub(crate) struct GateState {
    last_poll: HashMap<String, DateTime<Utc>>,
    closed:    HashMap<String, bool>,
}

impl GateState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Whether the 10-minute closed-window poll cadence allows a fresh
    /// probe for `automation_id` (user decision 2026-09-14: fixed
    /// 10-minute rechecks).
    #[must_use]
    pub(crate) fn poll_due(&self, automation_id: &str, now: DateTime<Utc>) -> bool {
        self.last_poll.get(automation_id).is_none_or(|last| {
            now.signed_duration_since(*last).num_seconds() >= RECHECK_INTERVAL_SECS
        })
    }

    /// Record a probe result.
    pub(crate) fn note_probe(
        &mut self,
        automation_id: &str,
        window_open: bool,
        now: DateTime<Utc>,
    ) {
        self.last_poll.insert(automation_id.to_string(), now);
        self.closed.insert(automation_id.to_string(), !window_open);
    }

    /// Last-seen closed state; absent = unknown (never probed).
    #[must_use]
    pub(crate) fn window_closed(&self, automation_id: &str) -> Option<bool> {
        self.closed.get(automation_id).copied()
    }

    /// A recovered (open) automation forgets its marks.
    pub(crate) fn note_recovered(&mut self, automation_id: &str) {
        self.last_poll.remove(automation_id);
        self.closed.remove(automation_id);
    }
}

/// The probe selector for one automation: `provider/model` from its
/// newest terminal run's primary model, else `None` (degraded — the gate
/// fails open).
#[must_use]
pub(crate) fn probe_selector(newest_terminal: Option<&Run>) -> Option<String> {
    let run = newest_terminal?;
    let model = run.models.first()?;
    let provider = model.provider.as_deref()?;
    Some(format!("{provider}/{}", model.name))
}

/// Whether the provider window is open, probed with a basic one-word
/// model test (the same primitive as the server's model-test endpoint)
/// through the resolved client and vault credential.
///
/// Fail-open contract: an unresolvable model or client NEVER blocks the
/// line — the cost is at most one parked run per window (SoftStop,
/// breaker-exempt), while a fail-closed gate could deadlock the line on
/// any client hiccup.
pub(crate) async fn provider_window_open(state: &AppState, selector: &str) -> bool {
    use fabro_llm::{probe, selection};

    let Ok(llm) = state.resolve_llm_client().await else {
        tracing::warn!(
            selector,
            "line gate: LLM client unresolvable — failing open (fabro-986b)"
        );
        return true;
    };
    let catalog = state.catalog();
    let eligible = llm
        .provider_ids()
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let Ok(info) = selection::select(&catalog, selector, None, &eligible) else {
        tracing::warn!(
            selector,
            "line gate: model selector unresolvable — failing open (fabro-986b)"
        );
        return true;
    };
    let provider_id = info.provider.id().clone();
    if !llm.has_provider(&provider_id) {
        tracing::warn!(
            selector,
            "line gate: provider not configured — failing open (fabro-986b)"
        );
        return true;
    }
    match probe::run_model_test(
        &llm.client,
        selector,
        fabro_types::ModelTestMode::Basic,
        None,
        Some(Duration::from_secs(30)),
    )
    .await
    .status
    {
        probe::ModelTestStatus::Ok => true,
        probe::ModelTestStatus::Error => false,
    }
}

/// Gate tick, called on every scheduler pass BEFORE due cron fires are
/// spawned.
///
/// 1. Closed-marked automations probe on the fixed 10-minute cadence (no runs
///    are created while the window stays closed); the moment a probe sees the
///    window open, the real pass fires through the normal scheduled-fire path
///    and the automation recovers its marks.
/// 2. Returns the set of automation ids whose window is closed RIGHT NOW — the
///    scheduler skips their due cron fires (a fire would only produce a doomed
///    run and noise).
pub(crate) async fn provider_gate_tick(
    state: Arc<AppState>,
    automations: &[fabro_automation::Automation],
    now: DateTime<Utc>,
    gate: &mut GateState,
) -> std::collections::HashSet<String> {
    let mut skip_fires = std::collections::HashSet::new();
    for automation in automations {
        let automation_id = automation.id.as_str();
        if automation.enabled_schedule_triggers().next().is_none() {
            continue;
        }
        let window_closed = match gate.window_closed(automation_id) {
            Some(true) => {
                // Closed as of the last probe: poll on the cadence, fire
                // on reopen. Between polls the answer stays closed.
                if gate.poll_due(automation_id, now) {
                    let newest = newest_terminal(&state, automation_id, now).await;
                    let selector = probe_selector(newest.as_ref());
                    let open = match selector.as_deref() {
                        Some(selector) => provider_window_open(&state, selector).await,
                        // Degraded (no model known): fail open, recover.
                        None => true,
                    };
                    gate.note_probe(automation_id, open, now);
                    if open {
                        tracing::info!(
                            automation_id,
                            "line gate: provider window reopened — firing (fabro-986b)"
                        );
                        if let Some(trigger) = automation.enabled_schedule_triggers().next() {
                            tokio::spawn(
                                super::automation_scheduler::fire_scheduled_automation_run(
                                    Arc::clone(&state),
                                    automation.clone(),
                                    trigger.id.clone(),
                                    now,
                                ),
                            );
                        }
                        gate.note_recovered(automation_id);
                        continue;
                    }
                    tracing::info!(
                        automation_id,
                        interval_secs = RECHECK_INTERVAL_SECS,
                        "line gate: provider window still closed — holding fires (fabro-986b)"
                    );
                }
                true
            }
            // Unknown or open: nothing to hold yet — the cron-due probe
            // below decides fresh.
            _ => false,
        };
        if window_closed {
            skip_fires.insert(automation_id.to_string());
        }
    }
    skip_fires
}

/// Gate one due cron fire: probe the provider NOW (one basic completion —
/// cheap next to a run start) and return false when the window is closed.
/// A closed result also marks the automation so the 10-minute reopen
/// polling takes over.
pub(crate) async fn cron_fire_allowed(
    state: &AppState,
    automation: &fabro_automation::Automation,
    now: DateTime<Utc>,
    gate: &mut GateState,
) -> bool {
    let automation_id = automation.id.as_str();
    let newest = newest_terminal(state, automation_id, now).await;
    let Some(selector) = probe_selector(newest.as_ref()) else {
        // Degraded: no model known — fail open.
        return true;
    };
    let open = provider_window_open(state, &selector).await;
    gate.note_probe(automation_id, open, now);
    if open {
        tracing::debug!(
            automation_id,
            selector = %selector,
            "line gate: provider window open — allowing scheduled fire (fabro-986b)"
        );
        true
    } else {
        tracing::info!(
            automation_id,
            selector = %selector,
            "line gate: provider window closed — skipping scheduled fire (fabro-986b)"
        );
        false
    }
}

async fn newest_terminal(state: &AppState, automation_id: &str, now: DateTime<Utc>) -> Option<Run> {
    state
        .stores
        .run_summaries
        .list_terminal_for_automation(automation_id, 1, now)
        .await
        .ok()
        .and_then(|runs| runs.into_iter().next())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

    fn quota_failure() -> RunFailure {
        let mut detail = fabro_types::outcome::FailureDetail::new(
            "LLM error: provider zai Usage limit reached for 5 hour",
            FailureCategory::TransientInfra,
        );
        detail.signature = Some(fabro_types::failure_signature::FailureSignature(
            "api_transient|zai|rate_limit".to_string(),
        ));
        RunFailure {
            reason: fabro_types::FailureReason::SoftStop,
            detail,
        }
    }

    fn soft_stop() -> RunStatus {
        RunStatus::Failed {
            reason: fabro_types::FailureReason::SoftStop,
        }
    }

    #[test]
    fn soft_stop_with_quota_signature_is_a_quota_park() {
        assert!(is_quota_park(soft_stop(), Some(&quota_failure())));
    }

    #[test]
    fn soft_stop_without_quota_signature_is_not_a_quota_park() {
        let mut failure = quota_failure();
        failure.detail.signature = None;
        assert!(!is_quota_park(soft_stop(), Some(&failure)));
    }

    #[test]
    fn hard_failures_are_not_quota_parks() {
        let hard = RunStatus::Failed {
            reason: fabro_types::FailureReason::WorkflowError,
        };
        assert!(!is_quota_park(hard, Some(&quota_failure())));
        assert!(!is_quota_park(soft_stop(), None));
    }

    #[test]
    fn closed_gate_poll_cadence_is_ten_minutes() {
        let mut gate = GateState::new();
        let t0 = Utc::now();
        assert!(gate.poll_due("a", t0));
        gate.note_probe("a", false, t0);
        assert!(
            gate.window_closed("a") == Some(true),
            "a closed probe marks the automation"
        );
        assert!(!gate.poll_due("a", t0));
        assert!(gate.poll_due("a", t0 + chrono::Duration::seconds(RECHECK_INTERVAL_SECS)));
        assert_eq!(RECHECK_INTERVAL_SECS, 600, "user decision 2026-09-14");
    }

    #[test]
    fn reopened_gate_recovers_its_marks() {
        let mut gate = GateState::new();
        let t0 = Utc::now();
        gate.note_probe("a", false, t0);
        gate.note_recovered("a");
        assert_eq!(gate.window_closed("a"), None);
        assert!(gate.poll_due("a", t0));
    }

    #[test]
    fn probe_selector_derives_provider_model_from_the_newest_run() {
        let mut run = run_body();
        run.models = vec![fabro_types::RunModel {
            provider: Some("zai".to_string()),
            name:     "glm-5.3".to_string(),
        }];
        assert_eq!(probe_selector(Some(&run)).as_deref(), Some("zai/glm-5.3"));
        assert_eq!(probe_selector(None), None, "no run — degraded fail-open");
        run.models[0].provider = None;
        assert_eq!(probe_selector(Some(&run)), None);
    }

    fn run_body() -> Run {
        Run {
            id:               fabro_types::RunId::new(),
            parent_id:        None,
            children_count:   0,
            title:            "pass".to_string(),
            goal:             "run the line".to_string(),
            workflow:         fabro_types::WorkflowRef {
                workflow_version_id: None,
                slug:                Some("conductor".to_string()),
                name:                Some("Conductor".to_string()),
                graph_name:          None,
                node_count:          0,
                edge_count:          0,
            },
            automation:       None,
            repository:       None,
            created_by:       fabro_types::test_support::test_principal(),
            origin:           fabro_types::RunOrigin::default(),
            labels:           HashMap::new(),
            lifecycle:        fabro_types::RunLifecycle {
                status:          RunStatus::Submitted,
                approval:        None,
                pending_control: None,
                queue_position:  None,
                error:           None,
                archived:        false,
                archived_at:     None,
            },
            sandbox:          None,
            models:           Vec::new(),
            source_directory: None,
            timestamps:       fabro_types::RunTimestamps {
                created_at:    chrono::Utc.with_ymd_and_hms(2026, 9, 14, 0, 0, 0).unwrap(),
                started_at:    None,
                last_event_at: None,
                completed_at:  None,
            },
            timing:           None,
            billing:          None,
            size:             fabro_types::RunSize::default(),
            ask_fabro:        fabro_types::AskFabro::default(),
            diff:             None,
            pull_request:     None,
            current_question: None,
            superseded_by:    None,
            retried_from:     None,
            links:            fabro_types::RunLinks { web: None },
        }
    }
}
