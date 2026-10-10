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
//! The gate probe is PROVIDER-scoped since fabro-b869 (2026-10-03): the
//! newest terminal run records the models of ALL its stages (the fold
//! dedups them) — and since fabro-0611 a workflow that never ran states its
//! providers through the set the create path's admission recorded at the
//! automation's last fire — so the gate probes EVERY provider the
//! automation needs and
//! holds it while any of them is closed — one probe per provider per recheck
//! cadence, shared across all automations that need that provider. A workflow
//! with no LLM provider observed (deterministic) is never held. Without a
//! resolvable model or client the gate FAILS OPEN (one bounded parked run per
//! window, never a dead line on a client hiccup), and every window TRANSITION
//! is logged at WARN because INFO is not ingested by the log platform.
//!
//! Fork-file policy: this file exists only on our fork; upstream does
//! not have it, so no merge can conflict it away.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use fabro_llm::gateway::reset_window;
use fabro_types::{Run, RunFailure, RunId, RunStatus};
use lithos_llm::catalog::CatalogProvider;

use super::AppState;
use crate::petri_check::RequiredProvider;

/// Recheck probe cadence (user decision 2026-09-14: "fixe rechecks alle
/// 10 Minuten").
pub(crate) const RECHECK_INTERVAL_SECS: i64 = 600;

/// Whether a terminal run is a quota-class park the recheck owns.
///
/// Structural detection only (fabro-e566): the shared
/// [`fabro_types::is_quota_rate_limit_failure`] classifier — a SoftStop
/// failure with a TransientInfra category and a `rate_limit` signature
/// detail (spelled `rate_limit` or `rate_limited` depending on provider).
/// The message prose is never parsed here. The run-side status shape is
/// either the legacy `Failed { soft_stop }` or the new terminal park
/// `Blocked { quota_rate_limit }` the lifecycle table remaps quota deaths
/// to.
#[must_use]
/// The terminal failure of a summary run, from the conclusion the fold
/// recorded (lifecycle.conclusion_failure, W1-2).
fn park_failure(run: &fabro_types::Run) -> Option<RunFailure> {
    run.lifecycle.conclusion_failure.clone()
}

pub(crate) fn is_quota_park(status: RunStatus, failure: Option<&RunFailure>) -> bool {
    let parked_status = matches!(
        status,
        RunStatus::Failed {
            reason: fabro_types::FailureReason::SoftStop,
        } | RunStatus::Blocked {
            blocked_reason: fabro_types::BlockedReason::QuotaRateLimit,
        }
    );
    parked_status && failure.is_some_and(fabro_types::is_quota_rate_limit_failure)
}

/// Window state of ONE provider: the shared kind the trigger read model
/// also carries (fabro-b869), so gate bookkeeping and the persisted facts
/// cannot drift apart.
pub(crate) use fabro_automation::ProviderWindowKind as ProviderWindow;

/// Per-provider probe answer plus the selector it was probed with.
#[derive(Debug, Clone)]
struct ProviderProbe {
    window:        ProviderWindow,
    last_probe_at: DateTime<Utc>,
    selector:      String,
}

/// In-memory gate bookkeeping (derived, not persisted).
///
/// PROVIDER-scoped since fabro-b869: `probes` is keyed by provider, so one
/// outage is one fact for every automation that needs that provider and the
/// fixed recheck cadence costs ONE probe per provider instead of one per
/// automation. `held` remembers which automations the last tick skipped, which
/// is what detects the reopen transition (rewind-or-fire, ADR-0021 Option C).
/// A server restart loses the marks — the next due fire probes fresh.
#[derive(Default)]
pub(crate) struct GateState {
    probes: HashMap<String, ProviderProbe>,
    held:   std::collections::HashSet<String>,
}

impl GateState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Whether the 10-minute recheck cadence allows a fresh probe for
    /// `provider` (user decision 2026-09-14: fixed 10-minute rechecks).
    #[must_use]
    pub(crate) fn poll_due(&self, provider: &str, now: DateTime<Utc>) -> bool {
        self.probes.get(provider).is_none_or(|probe| {
            now.signed_duration_since(probe.last_probe_at).num_seconds() >= RECHECK_INTERVAL_SECS
        })
    }

    /// Last-seen window; absent = never probed (the gate fails open).
    #[must_use]
    pub(crate) fn window(&self, provider: &str) -> Option<ProviderWindow> {
        self.probes.get(provider).map(|probe| probe.window)
    }

    /// Record a provider probe result. Every TRANSITION is logged at WARN
    /// (fabro-b869 step 5): at most one per provider per cadence, and it is
    /// the only trace of a window that rootprint can ingest (INFO is not).
    pub(crate) fn note_probe(
        &mut self,
        provider: &str,
        window: ProviderWindow,
        selector: &str,
        now: DateTime<Utc>,
    ) {
        let previous = self.probes.insert(provider.to_string(), ProviderProbe {
            window,
            last_probe_at: now,
            selector: selector.to_string(),
        });
        match (previous.as_ref().map(|probe| probe.window), window) {
            (Some(ProviderWindow::Open) | None, ProviderWindow::Closed) => {
                tracing::warn!(
                    provider,
                    selector,
                    recheck_secs = RECHECK_INTERVAL_SECS,
                    "line gate: provider window CLOSED — scheduled fires for workflows that need this provider are held"
                );
            }
            (Some(ProviderWindow::Closed), ProviderWindow::Open) => {
                tracing::warn!(
                    provider,
                    selector,
                    "line gate: provider window REOPENED — held automations resume (rewind-or-fire)"
                );
            }
            _ => {}
        }
    }

    /// The per-provider hold facts as the trigger read model stores them
    /// (fabro-b869 step 5b): one fact per provider, window as last probed,
    /// next probe one recheck cadence after the last.
    pub(crate) fn window_facts(
        &self,
        providers: &[String],
    ) -> Vec<fabro_automation::ProviderWindowFact> {
        providers
            .iter()
            .filter_map(|provider| {
                let probe = self.probes.get(provider)?;
                Some(fabro_automation::ProviderWindowFact {
                    provider:      provider.clone(),
                    window:        probe.window,
                    last_probe_at: probe.last_probe_at,
                    next_probe_at: probe
                        .last_probe_at
                        .checked_add_signed(chrono::Duration::seconds(RECHECK_INTERVAL_SECS))?,
                })
            })
            .collect()
    }

    /// When a provider was last probed, whatever the answer was.
    #[must_use]
    pub(crate) fn last_probe_at(&self, provider: &str) -> Option<DateTime<Utc>> {
        self.probes.get(provider).map(|probe| probe.last_probe_at)
    }

    /// The selector a provider was last probed with (for log/status lines).
    #[must_use]
    pub(crate) fn probe_selector_of(&self, provider: &str) -> Option<&str> {
        self.probes
            .get(provider)
            .map(|probe| probe.selector.as_str())
    }

    /// Whether the last tick held this automation's fires.
    #[cfg(test)]
    fn is_held(&self, automation_id: &str) -> bool {
        self.held.contains(automation_id)
    }

    fn note_held(&mut self, automation_id: &str) {
        self.held.insert(automation_id.to_string());
    }

    /// Forget a hold (window open again); `true` when this call ended a hold,
    /// i.e. this tick IS the reopen transition for the automation.
    fn take_hold(&mut self, automation_id: &str) -> bool {
        self.held.remove(automation_id)
    }
}

/// The providers one run touched, as `(provider, probe selector)` pairs:
/// EVERY model of the run — the fold records the models of all stages and
/// dedups — not just the first (fabro-b869 step 2: a workflow can drive
/// several providers, and holding it for one of them is not enough).
#[must_use]
pub(crate) fn run_providers(newest_terminal: Option<&Run>) -> Vec<(String, String)> {
    let Some(run) = newest_terminal else {
        return Vec::new();
    };
    sorted_providers(run.models.iter().filter_map(|model| {
        let provider = model.provider.as_deref()?;
        Some((provider.to_string(), format!("{provider}/{}", model.name)))
    }))
}

/// One `(provider, probe selector)` pair per provider, alphabetically
/// ordered, the first of its selectors the deterministic representative:
/// every derivation of the gate's provider set normalizes through here.
fn sorted_providers<I: IntoIterator<Item = (String, String)>>(
    providers: I,
) -> Vec<(String, String)> {
    let mut providers = providers.into_iter().collect::<Vec<_>>();
    providers.sort();
    providers.dedup_by(|left, right| left.0 == right.0);
    providers
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
    let outcome = probe::run_model_test(
        &llm.client,
        selector,
        fabro_types::ModelTestMode::Basic,
        None,
        Some(Duration::from_secs(30)),
    )
    .await;
    match outcome.status {
        probe::ModelTestStatus::Ok => true,
        probe::ModelTestStatus::Error => {
            let message = outcome.error_message.unwrap_or_default();
            if probe_failure_is_a_usage_window(&message) {
                false
            } else {
                // Only a USAGE WINDOW closes the gate (fabro-986b's user
                // decision: check the provider for its 429 before a fire).
                // Any other probe failure — a 5xx, a transport hiccup, a
                // mock — leaves the window OPEN: the documented fail-open
                // contract, and since fabro-b869 step 4 the create path
                // reads the same state, so a transient failure must never
                // lock manual fires for a whole cadence.
                tracing::warn!(
                    selector,
                    error = %message,
                    "line gate: provider probe failed without a usage-window signature — failing open (fabro-986b)"
                );
                true
            }
        }
    }
}

/// Whether a failed provider probe describes a usage/quota WINDOW — the
/// only failure the gate treats as closed: the provider answered and said
/// the window is spent (429-shaped), as opposed to failing to answer at all.
fn probe_failure_is_a_usage_window(message: &str) -> bool {
    let lowered = message.to_ascii_lowercase();
    [
        "429",
        "rate limit",
        "rate_limit",
        "usage limit",
        "quota",
        "too many requests",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
}

/// Gate tick, called on every scheduler pass BEFORE due cron fires are
/// spawned.
///
/// Since fabro-b869 the gate is PROVIDER-scoped: every provider the
/// automation's newest terminal run touched is answered once per recheck
/// cadence (shared across automations), and the automation is held while ANY
/// of them is closed. A hold that ends (window open) either rewinds the parked
/// run (ADR-0021 rev 2 Option C) or fires the real pass through the normal
/// scheduled-fire path.
///
/// Returns the set of automation ids whose fires are held RIGHT NOW — the
/// scheduler skips their due cron fires (a fire would only produce a doomed
/// run and noise).
pub(crate) async fn provider_gate_tick(
    state: Arc<AppState>,
    automations: &[fabro_automation::Automation],
    now: DateTime<Utc>,
) -> GateTick {
    let mut skip_fires = std::collections::HashSet::new();
    let mut fact_writes: Vec<ProviderWindowWrite> = Vec::new();
    for automation in automations {
        let automation_id = automation.id.as_str();
        if automation.enabled_schedule_triggers().next().is_none() {
            continue;
        }
        let newest = newest_terminal(&state, automation_id, now).await;
        let required = automation_providers(state.as_ref(), newest.as_ref(), automation);
        if required.is_empty() {
            // No LLM provider observed for this automation (deterministic
            // workflow, or no terminal run yet): never held.
            continue;
        }
        let closed = probe_due_providers(&state, &required, now).await;
        if closed.is_empty() {
            // Every required provider's window is open. If this tick ENDS a
            // hold, recover the line: rewind the quota-parked run, else fire
            // immediately (the normal cron path would wait for the next slot).
            if state.provider_gate.lock().await.take_hold(automation_id) {
                // The hold ended: clear the trigger's window facts so the
                // read model says quiet-for-no-reason (fabro-b869 step 5b).
                // The write is DEFERRED to the caller (never detached): an
                // out-of-order clear could strand a stale "closed" forever.
                if let Some(trigger) = automation.enabled_schedule_triggers().next() {
                    fact_writes.push(ProviderWindowWrite {
                        automation_id: automation_id.to_string(),
                        trigger_id:    trigger.id.clone(),
                        facts:         None,
                    });
                }
                let park_to_rewind = newest
                    .as_ref()
                    .filter(|run| is_quota_park(run.lifecycle.status, park_failure(run).as_ref()))
                    .map(|run| run.id);
                if let Some(parked_run_id) = park_to_rewind {
                    tracing::info!(
                        automation_id,
                        run_id = %parked_run_id,
                        "line gate: provider window reopened — rewinding the parked run (fabro-2e7b Option C)"
                    );
                    let state_for_rewind = Arc::clone(&state);
                    tokio::spawn(async move {
                        let actor = fabro_types::Principal::System {
                            system_kind: fabro_types::SystemActorKind::Engine,
                        };
                        match super::handler::lineage::rewind_run_internal(
                            state_for_rewind.as_ref(),
                            parked_run_id,
                            actor,
                        )
                        .await
                        {
                            Ok((new_run_id, archived)) => {
                                tracing::info!(
                                    source_run_id = %parked_run_id,
                                    new_run_id = %new_run_id,
                                    archived,
                                    "line gate: parked run rewound after window reopen"
                                );
                            }
                            Err(err) => {
                                tracing::warn!(
                                    source_run_id = %parked_run_id,
                                    error = %err,
                                    "line gate: rewind of the parked run failed — the next schedule fire recovers the line"
                                );
                            }
                        }
                    });
                } else {
                    tracing::info!(
                        automation_id,
                        "line gate: provider window reopened — firing (fabro-986b)"
                    );
                    if let Some(trigger) = automation.enabled_schedule_triggers().next() {
                        tokio::spawn(super::automation_scheduler::fire_scheduled_automation_run(
                            Arc::clone(&state),
                            automation.clone(),
                            trigger.id.clone(),
                            now,
                        ));
                    }
                }
                continue;
            }
            // Not held: the cron path decides on its own (cron_fire_allowed
            // re-checks the same provider state). A trigger still carrying
            // stored facts from an earlier hold — or from a restart, which
            // starts the gate empty and so can never observe a reopen
            // transition — is cleared here, so the read model can never say
            // "closed" for a schedule that is firing (fabro-b869 step 5b).
            let stale = automation
                .enabled_schedule_triggers()
                .next()
                .filter(|trigger| trigger.provider_window.is_some())
                .map(|trigger| trigger.id.clone());
            if let Some(trigger_id) = stale {
                fact_writes.push(ProviderWindowWrite {
                    automation_id: automation_id.to_string(),
                    trigger_id,
                    facts: None,
                });
            }
            continue;
        }
        let probed_with = {
            let gate = state.provider_gate.lock().await;
            closed
                .iter()
                .map(|provider| {
                    gate.probe_selector_of(provider)
                        .unwrap_or(provider.as_str())
                        .to_string()
                })
                .collect::<Vec<_>>()
                .join(",")
        };
        tracing::info!(
            automation_id,
            providers = %closed.join(","),
            probed_with,
            interval_secs = RECHECK_INTERVAL_SECS,
            "line gate: provider window closed — holding fires (fabro-b869)"
        );
        state.provider_gate.lock().await.note_held(automation_id);
        skip_fires.insert(automation_id.to_string());
        // fabro-b869 step 5b: the hold is readable on the trigger — one
        // fact per closed provider, next probe one cadence out. Deferred to
        // the caller, which writes them after the gate lock is released.
        if let Some(trigger) = automation.enabled_schedule_triggers().next() {
            fact_writes.push(ProviderWindowWrite {
                automation_id: automation_id.to_string(),
                trigger_id:    trigger.id.clone(),
                facts:         Some(fabro_automation::ProviderWindowState {
                    providers: state.provider_gate.lock().await.window_facts(&closed),
                }),
            });
        }
    }
    GateTick {
        held: skip_fires,
        fact_writes,
    }
}

/// What one gate tick decided (fabro-b869 step 5b): the automations whose
/// fires it holds, and the trigger-fact writes the caller performs after
/// releasing the gate lock.
pub(crate) struct GateTick {
    pub(crate) held:        std::collections::HashSet<String>,
    pub(crate) fact_writes: Vec<ProviderWindowWrite>,
}

/// One deferred write of a trigger's provider-window read model.
pub(crate) struct ProviderWindowWrite {
    pub(crate) automation_id: String,
    pub(crate) trigger_id:    fabro_automation::AutomationTriggerId,
    pub(crate) facts:         Option<fabro_automation::ProviderWindowState>,
}

/// Why a manual/API/CLI fire was refused on a provider window (fabro-b869
/// step 4): the closed provider, the stage's model selector when stated,
/// and when the gate last probed it.
pub(crate) struct WindowRefusal {
    pub provider:         String,
    pub model:            Option<String>,
    pub last_probe_at:    DateTime<Utc>,
    /// The run whose conclusion announced the window, when the refusal
    /// came from first-hand run evidence rather than a gate probe
    /// (fabro-2f03).
    pub learned_from_run: Option<String>,
}

impl WindowRefusal {
    /// The next probe is one recheck cadence after the last, exactly what
    /// the scheduler's poll_due would allow.
    pub(crate) fn next_probe_at(&self) -> DateTime<Utc> {
        self.last_probe_at
            .checked_add_signed(chrono::Duration::seconds(RECHECK_INTERVAL_SECS))
            .unwrap_or(self.last_probe_at)
    }
}

/// The manual/API/CLI create path's window check (fabro-b869 step 4): a
/// run whose model stages need a provider whose window the gate last saw
/// CLOSED is refused before any run record exists. A provider the gate
/// never probed is probed ON DEMAND here (the same bounded one-word probe
/// the scheduler uses, fail-open on unresolvable client or selector) and
/// the answer is recorded, so scheduler and manual fires share one probe
/// per provider per cadence. `force` skips the check entirely.
pub(crate) async fn manual_window_refusal(
    state: &AppState,
    required: &[RequiredProvider],
    force: bool,
    now: DateTime<Utc>,
    last_run_window: Option<&(String, String)>,
) -> Option<WindowRefusal> {
    if force {
        return None;
    }
    // First-hand evidence outranks a stale probe (fabro-2f03): when the
    // workflow's own last terminal run concluded with a usage-window
    // announcement, the window is spent regardless of what the last
    // cadence probe saw — a window that closed mid-run otherwise stays
    // invisible until the next probe.
    if let Some((run_id, message)) = last_run_window {
        for requirement in required {
            tracing::warn!(
                provider = requirement.provider.as_str(),
                run_id,
                "line gate: refusing a manual fire — the workflow's last run concluded on a provider window: {message}"
            );
        }
        if let Some(requirement) = required.first() {
            return Some(WindowRefusal {
                provider:         requirement.provider.to_string(),
                model:            requirement.model.clone(),
                last_probe_at:    now,
                learned_from_run: Some(run_id.clone()),
            });
        }
    }
    for requirement in required {
        let provider = requirement.provider.as_str();
        // The cadence slot is CLAIMED under the lock (fabro-b869 step 4):
        // a provider the gate never probed, or whose answer is older than
        // the recheck cadence, is reserved for this caller — a concurrent
        // create sees a fresh slot and never probes a second time inside
        // one cadence. The reservation keeps the last known window, so a
        // closed window stays closed while its reprobe is in flight.
        let claim = {
            let mut gate = state.provider_gate.lock().await;
            if gate.poll_due(provider, now) {
                let known = gate.window(provider).unwrap_or(ProviderWindow::Open);
                let selector = probe_selector(state, requirement.provider.as_str());
                gate.note_probe(provider, known, &selector, now);
                Some(selector)
            } else {
                None
            }
        };
        if let Some(selector) = claim {
            // The probe runs OUTSIDE the lock: a 30-second provider probe
            // must never block the scheduler or a concurrent create.
            let open = provider_window_open(state, &selector).await;
            let window = if open {
                ProviderWindow::Open
            } else {
                ProviderWindow::Closed
            };
            let mut gate = state.provider_gate.lock().await;
            gate.note_probe(provider, window, &selector, now);
        }
        let (window, last_probe_at) = {
            let gate = state.provider_gate.lock().await;
            (gate.window(provider), gate.last_probe_at(provider))
        };
        if window == Some(ProviderWindow::Closed) {
            return Some(WindowRefusal {
                provider:         requirement.provider.to_string(),
                model:            requirement.model.clone(),
                last_probe_at:    last_probe_at.unwrap_or(now),
                learned_from_run: None,
            });
        }
    }
    None
}

/// The most recent terminal run of `automation_id` whose recorded
/// conclusion announces a provider usage window, as `(run_id, message)`.
///
/// The gate's facts otherwise come only from its own probes, so a window
/// that closes MID-RUN stays invisible until the next cadence probe — a
/// fire inside that cadence used to go through and burn a run
/// (fabro-2f03, the 2026-10-10 zai incident). A run that concluded with a
/// window announcement is first-hand evidence the window is spent; the
/// refusal consults it before trusting a stale "open" probe.
pub(crate) async fn last_run_window_signal(
    state: &AppState,
    automation_id: &str,
    now: DateTime<Utc>,
) -> Option<(String, String)> {
    let run = state
        .stores
        .run_summaries
        .list_terminal_for_automation(automation_id, 1, now)
        .await
        .ok()?
        .into_iter()
        .next()?;
    window_signal(&run.id, run.lifecycle.conclusion_failure.as_ref(), now)
}

/// The window signal a recorded conclusion carries, as `(run_id, message)`:
/// `Some` only when the conclusion names a provider usage window (the same
/// grammar the park predicate reads).
fn window_signal(
    run_id: &RunId,
    conclusion_failure: Option<&RunFailure>,
    now: DateTime<Utc>,
) -> Option<(String, String)> {
    let message = &conclusion_failure?.detail.message;
    reset_window(message, SystemTime::from(now))
        .is_some()
        .then(|| (run_id.to_string(), message.clone()))
}

/// The selector a requirement's probe runs with: the provider's PROBE
/// model from the catalog (the row marked `probe`, else its default
/// offering) — the cheap model the catalog designates for exactly this
/// question. The stage's own model is deliberately not used: probing the
/// stage's model spends it on a one-word request and, in scripted test
/// scenarios, would consume the answer the stage itself expects.
fn probe_selector(state: &AppState, provider: &str) -> String {
    let probe_model = state
        .catalog()
        .enabled_provider(provider)
        .and_then(CatalogProvider::probe_offering)
        .map(|offering| offering.model.id().to_string());
    match probe_model {
        Some(model) => format!("{provider}/{model}"),
        None => provider.to_string(),
    }
}

/// The providers an automation's fires need as `(provider, probe selector)`
/// pairs (fabro-0611, b869 step 3): the newest terminal run's models when
/// one exists — the observed truth, including selectors execution resolved
/// from templated stages — else the model providers the create path's
/// admission recorded at the automation's last fire, so a workflow that
/// never ran is gated like one that did. Deterministic workflows state no
/// providers either way and are never held.
fn automation_providers(
    state: &AppState,
    newest_terminal: Option<&Run>,
    automation: &fabro_automation::Automation,
) -> Vec<(String, String)> {
    let observed = run_providers(newest_terminal);
    if !observed.is_empty() {
        return observed;
    }
    sorted_providers(
        automation
            .model_providers
            .iter()
            .flatten()
            .map(|requirement| {
                let selector = probe_selector(state, &requirement.provider);
                (requirement.provider.clone(), selector)
            }),
    )
}

/// Probe every required provider whose answer is older than the recheck
/// cadence and return the providers that are CLOSED right now.
///
/// Fail-open: a provider whose probe cannot even run (unresolvable client /
/// selector) is treated as open — one bounded parked run per window is
/// cheaper than a dead line. Both provider-set derivations hand every
/// provider a selector (the run's own model, or the catalog probe offering
/// for the stored admission set), and a deterministic workflow consults
/// nothing, so the caller never reaches this without one.
pub(crate) async fn probe_due_providers(
    state: &AppState,
    required: &[(String, String)],
    now: DateTime<Utc>,
) -> Vec<String> {
    let mut closed = Vec::new();
    for (provider, selector) in required {
        // Claim the cadence slot under a SHORT lock, probe OUTSIDE it: a
        // 30-second provider probe must never block a concurrent create or
        // the tick's next automation (fabro-b869 step 4).
        let claimed = {
            let mut gate = state.provider_gate.lock().await;
            if gate.poll_due(provider, now) {
                let known = gate.window(provider).unwrap_or(ProviderWindow::Open);
                gate.note_probe(provider, known, selector, now);
                true
            } else {
                false
            }
        };
        if claimed {
            let open = provider_window_open(state, selector).await;
            let window = if open {
                ProviderWindow::Open
            } else {
                ProviderWindow::Closed
            };
            let mut gate = state.provider_gate.lock().await;
            gate.note_probe(provider, window, selector, now);
        }
        let known_closed = {
            let gate = state.provider_gate.lock().await;
            gate.window(provider) == Some(ProviderWindow::Closed)
        };
        if known_closed && !closed.contains(provider) {
            closed.push(provider.clone());
        }
    }
    closed
}

/// Gate one due cron fire: make sure every provider the automation needs has a
/// FRESH window answer (probing only what the recheck cadence allows —
/// provider-scoped since fabro-b869, so a shared outage costs one probe per
/// provider) and return false while any of them is closed.
pub(crate) async fn cron_fire_allowed(
    state: &AppState,
    automation: &fabro_automation::Automation,
    now: DateTime<Utc>,
) -> bool {
    let automation_id = automation.id.as_str();
    let newest = newest_terminal(state, automation_id, now).await;
    let required = automation_providers(state, newest.as_ref(), automation);
    if required.is_empty() {
        // Degraded: no LLM provider observed — fail open.
        return true;
    }
    let closed = probe_due_providers(state, &required, now).await;
    if closed.is_empty() {
        tracing::debug!(
            automation_id,
            providers = %required.iter().map(|(provider, _)| provider.as_str()).collect::<Vec<_>>().join(","),
            "line gate: provider windows open — allowing scheduled fire (fabro-986b)"
        );
        true
    } else {
        tracing::info!(
            automation_id,
            providers = %closed.join(","),
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
    use fabro_types::outcome::FailureCategory;

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

    fn quota_blocked() -> RunStatus {
        RunStatus::Blocked {
            blocked_reason: fabro_types::BlockedReason::QuotaRateLimit,
        }
    }

    #[test]
    fn soft_stop_with_quota_signature_is_a_quota_park() {
        assert!(is_quota_park(soft_stop(), Some(&quota_failure())));
    }

    #[test]
    fn quota_blocked_park_shape_is_a_quota_park() {
        // fabro-e566: quota deaths end Blocked { quota_rate_limit }; the
        // classifier must recognize the remapped shape as the same park.
        assert!(is_quota_park(quota_blocked(), Some(&quota_failure())));
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
    fn provider_poll_cadence_is_ten_minutes_and_provider_scoped() {
        let mut gate = GateState::new();
        let t0 = Utc::now();
        assert!(gate.poll_due("zai", t0), "never probed -> due");
        gate.note_probe("zai", ProviderWindow::Closed, "zai/glm-5.3", t0);
        assert_eq!(
            gate.window("zai"),
            Some(ProviderWindow::Closed),
            "a closed probe marks the PROVIDER (fabro-b869)"
        );
        assert!(!gate.poll_due("zai", t0));
        assert!(
            gate.poll_due("zai", t0 + chrono::Duration::seconds(RECHECK_INTERVAL_SECS)),
            "the fixed recheck cadence re-opens the probe"
        );
        // Provider scope: another provider's answer is independent.
        assert_eq!(gate.window("openrouter"), None);
        assert!(gate.poll_due("openrouter", t0));
        gate.note_probe("openrouter", ProviderWindow::Open, "openrouter/x", t0);
        assert_eq!(gate.window("openrouter"), Some(ProviderWindow::Open));
        assert_eq!(RECHECK_INTERVAL_SECS, 600, "user decision 2026-09-14");
    }

    /// A terminal run's conclusion feeds the gate only when it names a
    /// window (fabro-2f03): the live zai body must signal, an unrelated
    /// failure must not.
    #[test]
    fn a_runs_conclusion_signals_only_a_usage_window() {
        fn failure_with(message: &str) -> fabro_types::RunFailure {
            fabro_types::RunFailure {
                reason: fabro_types::FailureReason::SoftStop,
                detail: fabro_types::outcome::FailureDetail::new(
                    message,
                    FailureCategory::TransientInfra,
                ),
            }
        }

        let run_id: fabro_types::RunId = "01M4KMWYRQDJ54ZSS08NJNH74N".parse().unwrap();
        let live = "model request failed (rate_limit): provider zai Usage limit reached for 5 hour. Your limit will reset at 2026-10-11 05:04:33 [provider zai, status 429, code 1308]";
        let signal = window_signal(&run_id, Some(&failure_with(live)), Utc::now());
        assert!(
            signal.is_some(),
            "the live zai conclusion must signal a window"
        );
        assert_eq!(signal.unwrap().0, "01M4KMWYRQDJ54ZSS08NJNH74N");

        assert!(
            window_signal(
                &run_id,
                Some(&failure_with("bad_output after 2 repair turns")),
                Utc::now()
            )
            .is_none(),
            "a non-window failure must not feed the gate"
        );
        assert!(window_signal(&run_id, None, Utc::now()).is_none());
    }

    #[test]
    fn only_a_usage_window_signature_closes_the_gate() {
        // The gate exists for usage windows (fabro-986b): a provider that
        // ANSWERS "the window is spent" closes it, a provider that fails to
        // answer must not — since fabro-b869 step 4 the create path reads
        // the same state, so a 5xx would lock manual fires for a cadence.
        for message in [
            "429 Too Many Requests",
            "Usage limit reached for 5 hour. Your limit will reset at 2026-10-05 01:44:59Z",
            "rate_limit exceeded",
            "quota exhausted",
        ] {
            assert!(
                probe_failure_is_a_usage_window(message),
                "a usage window should close the gate: {message}"
            );
        }
        // The exact body zai sent on 2026-10-10 (naive wallclock + the
        // engine's own bracket suffix) must close the gate — the live
        // incident's text, not a cleaned-up cousin.
        let live = "model request failed (rate_limit): provider zai Usage limit reached for 5 hour. Your limit will reset at 2026-10-11 05:04:33 [provider zai, status 429, code 1308]";
        assert!(
            probe_failure_is_a_usage_window(live),
            "the live 429 body should close the gate"
        );
        for message in [
            "connection reset by peer",
            "500 Internal Server Error",
            "probe unavailable",
            "invalid api key",
            "",
        ] {
            assert!(
                !probe_failure_is_a_usage_window(message),
                "a non-window failure must fail open: {message}"
            );
        }
    }

    #[test]
    fn window_facts_carry_each_providers_last_probe_and_cadence() {
        let now = Utc::now();
        let mut gate = GateState::new();
        gate.note_probe("zai", ProviderWindow::Closed, "zai/glm-4.7", now);
        gate.note_probe("openai", ProviderWindow::Open, "gpt", now);

        let facts = gate.window_facts(&["zai".to_string(), "openai".to_string()]);
        assert_eq!(facts.len(), 2);
        let zai = facts.iter().find(|fact| fact.provider == "zai").unwrap();
        assert_eq!(zai.window, ProviderWindow::Closed);
        assert_eq!(zai.last_probe_at, now);
        assert_eq!(
            zai.next_probe_at,
            now.checked_add_signed(chrono::Duration::seconds(RECHECK_INTERVAL_SECS))
                .unwrap()
        );
        // A provider the gate never probed states no fact.
        let unseen = gate.window_facts(&["moonshot".to_string()]);
        assert!(unseen.is_empty());
    }

    #[test]
    fn holds_end_only_on_the_reopen_transition() {
        let mut gate = GateState::new();
        assert!(!gate.is_held("loop-fabro"));
        gate.note_held("loop-fabro");
        gate.note_held("loop-fabro");
        assert!(gate.is_held("loop-fabro"));
        assert!(
            gate.take_hold("loop-fabro"),
            "the first tick after a hold ends returns true (rewind-or-fire arm)"
        );
        assert!(
            !gate.take_hold("loop-fabro"),
            "a second tick is not a transition"
        );
        assert!(!gate.is_held("loop-fabro"));
    }

    #[test]
    fn run_providers_returns_every_provider_not_just_the_first() {
        let mut run = run_body();
        run.models = vec![
            fabro_types::RunModel {
                provider: Some("zai".to_string()),
                name:     "glm-5.3".to_string(),
            },
            fabro_types::RunModel {
                provider: Some("openrouter".to_string()),
                name:     "claude-opus".to_string(),
            },
            // Same provider, second model: deduped to one probe.
            fabro_types::RunModel {
                provider: Some("zai".to_string()),
                name:     "glm-4.7".to_string(),
            },
            // No provider attribution: skipped, never probed.
            fabro_types::RunModel {
                provider: None,
                name:     "unknown".to_string(),
            },
        ];
        let providers = run_providers(Some(&run));
        assert_eq!(
            providers,
            vec![
                (
                    "openrouter".to_string(),
                    "openrouter/claude-opus".to_string()
                ),
                ("zai".to_string(), "zai/glm-4.7".to_string()),
            ],
            "every provider of the run is gated (fabro-b869 step 2); one probe per provider, \
             the alphabetically first of its models as the deterministic representative"
        );
    }

    #[test]
    fn deterministic_runs_have_no_providers_to_gate() {
        let run = run_body();
        assert!(
            run_providers(Some(&run)).is_empty(),
            "no models recorded -> no provider to gate -> deterministic workflows are never held"
        );
        assert!(
            run_providers(None).is_empty(),
            "no run yet — degraded fail-open"
        );
    }

    fn run_body() -> Run {
        Run {
            id:                  fabro_types::RunId::new(),
            parent_id:           None,
            children_count:      0,
            title:               "pass".to_string(),
            goal:                "run the line".to_string(),
            workflow:            fabro_types::WorkflowRef {
                slug:       Some("conductor".to_string()),
                name:       Some("Conductor".to_string()),
                graph_name: None,
                node_count: 0,
                edge_count: 0,
            },
            automation:          None,
            repository:          None,
            created_by:          fabro_types::test_support::test_principal(),
            origin:              fabro_types::RunOrigin::default(),
            labels:              HashMap::new(),
            lifecycle:           fabro_types::RunLifecycle {
                conclusion_failure: None,
                status:             RunStatus::Submitted,
                approval:           None,
                pending_control:    None,
                queue_position:     None,
                error:              None,
                archived:           false,
                archived_at:        None,
            },
            sandbox:             None,
            workflow_version_id: None,
            models:              Vec::new(),
            source_directory:    None,
            timestamps:          fabro_types::RunTimestamps {
                created_at:    chrono::Utc.with_ymd_and_hms(2026, 9, 14, 0, 0, 0).unwrap(),
                started_at:    None,
                last_event_at: None,
                completed_at:  None,
            },
            timing:              None,
            usage:               fabro_api::types::Usage::default(),
            size:                fabro_types::RunSize::default(),
            ask_fabro:           fabro_types::AskFabro::default(),
            diff:                None,
            pull_request:        None,
            current_question:    None,
            superseded_by:       None,
            retried_from:        None,
            links:               fabro_types::RunLinks { web: None },
        }
    }

    // --- seam presence pins (fork-only file, merge-safe) ---
    //
    // The gate's seam call sites live in SHARED files
    // (`automation_scheduler.rs`, `automation_breaker.rs`): an upstream
    // merge that rewrites them can drop the fork calls silently, and the
    // #832 incident class removes inline tests together with the code they
    // covered. These source pins stay in this fork-only file and go red the
    // moment a seam call disappears.

    fn server_src(rel: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("manifest lives at lib/apps/fabro-server")
            .join(rel);
        #[expect(
            clippy::disallowed_methods,
            reason = "presence pin reads shared scheduler source synchronously"
        )]
        std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
    }

    #[test]
    fn the_create_path_records_the_admissions_model_providers() {
        // fabro-0611 (b869 step 3): the write lives in the SHARED run-create
        // handler; an upstream merge that rewrites finalize_created_run can
        // drop it silently and never-run workflows go blind again.
        let runs = server_src("lib/apps/fabro-server/src/server/handler/runs.rs");
        assert!(
            runs.contains("set_model_providers"),
            "the create path must record the admission's model providers on the \
             automation (fabro-0611): without it, the gate cannot cover a \
             workflow that never ran"
        );
    }

    #[test]
    fn scheduler_consults_the_provider_window_gate_before_firing() {
        let scheduler = server_src("lib/apps/fabro-server/src/server/automation_scheduler.rs");
        assert!(
            scheduler.contains("fork_line_recovery::provider_gate_tick("),
            "the scheduler must tick the provider window gate (fabro-986b): \
             without it, closed windows burn probe runs again"
        );
        assert!(
            scheduler.contains("fork_line_recovery::cron_fire_allowed("),
            "the scheduler must ask the gate before every fire (fabro-986b): \
             without it, fires proceed while the provider window is closed"
        );
    }

    #[test]
    fn breaker_keeps_quota_parks_exempt_from_the_schedule_breaker() {
        let breaker = server_src("lib/apps/fabro-server/src/server/automation_breaker.rs");
        assert!(
            breaker.contains("fork_line_recovery::is_quota_park("),
            "quota-class parks must stay breaker-exempt (fabro-986b): the \
             fixed recheck owns their recovery, the breaker must stay armed"
        );
    }
}

/// Breaker-exemption integration pin (fabro-986b rev, fabro-ec00 ARM 3):
/// migrated from inline `automation_scheduler` tests into this fork-only
/// surface so a merge resolution can never drop it silently.
#[cfg(test)]
mod breaker_exemption_pin {
    use std::sync::Arc;

    use fabro_automation::AutomationId;

    use super::super::automation_scheduler::tests::{
        breaker_test_state, create_breakable_automation, due_minute, park_run_with_signature,
        prime_time, stored_breaker_trigger, stored_runs_chronological, succeeding_materializer,
    };
    use super::super::automation_scheduler::{AutomationSchedulePlanner, run_due_schedules_once};

    /// FORK PRESENCE PIN (fabro-986b, user decision 2026-09-14): quota-class
    /// parks (SoftStop + TransientInfra + rate_limit signature) are EXEMPT
    /// from the schedule breaker — the fixed 10-minute recheck in
    /// `server::fork_line_recovery` owns their recovery, so the breaker must
    /// stay armed (trigger enabled, counter clean) while a line rides out a
    /// provider usage window. If this test fails after a merge, the fork
    /// seam in `automation_breaker.rs` was dropped — restore it, never relax
    /// the test.
    #[tokio::test]
    async fn quota_parks_are_breaker_exempt_and_keep_the_schedule_armed() {
        let materializer = succeeding_materializer();
        let (state, _notices) = breaker_test_state(materializer);
        create_breakable_automation(state.as_ref(), "quota-recheck", Some(2)).await;
        let mut planner = AutomationSchedulePlanner::default();
        let quota_signature = "api_transient|zai|rate_limit";

        // Baseline observation, then four quota parks — far beyond the
        // threshold of 2. The breaker must not count any of them.
        run_due_schedules_once(Arc::clone(&state), &mut planner, prime_time()).await;
        for minute in 1..=4u32 {
            run_due_schedules_once(Arc::clone(&state), &mut planner, due_minute(minute)).await;
            let runs = stored_runs_chronological(state.as_ref()).await;
            let run_id = runs[runs.len() - 1].id;
            park_run_with_signature(state.as_ref(), &run_id, quota_signature).await;
        }
        run_due_schedules_once(Arc::clone(&state), &mut planner, due_minute(5)).await;

        let automation = state
            .automation_store()
            .get(&AutomationId::new("quota-recheck").unwrap())
            .await
            .unwrap()
            .expect("automation should exist");
        let trigger = stored_breaker_trigger(&automation);
        assert!(trigger.enabled, "quota parks never pause the schedule");
        let facts = trigger.breaker.expect("high-water mark persists");
        assert_eq!(
            facts.consecutive_count, 0,
            "quota parks count toward neither the latch nor a reset"
        );

        // Control: a NON-quota transient park still counts — the breaker
        // stays armed for real defects.
        let runs = stored_runs_chronological(state.as_ref()).await;
        park_run_with_signature(
            state.as_ref(),
            &runs[runs.len() - 1].id,
            "api_transient|zai|server_error",
        )
        .await;
        run_due_schedules_once(Arc::clone(&state), &mut planner, due_minute(6)).await;
        let automation = state
            .automation_store()
            .get(&AutomationId::new("quota-recheck").unwrap())
            .await
            .unwrap()
            .expect("automation should exist");
        let facts = stored_breaker_trigger(&automation)
            .breaker
            .expect("facts persist");
        assert_eq!(
            facts.consecutive_count, 1,
            "non-quota parks keep counting toward the latch"
        );
    }
}
