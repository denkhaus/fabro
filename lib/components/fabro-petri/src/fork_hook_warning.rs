//! The non-blocking hook failure watchdog (fabro-6922): a persistent
//! failure of a non-blocking `[[run.hooks]]` hook must be LOUD.
//!
//! The judgment-shadow host hook exited 1 on every stage of every run
//! since the cutover and nothing surfaced it — not a run failure (the
//! hook is non-blocking, so its decision is ignored), no metric, no
//! warning event; it was found only by reading raw hook reports. This
//! module decorates the run's hook service ([`HookService`], the seam
//! `runtime` installs around Petri's local service) and reads every
//! [`HookReport`] the service returns: a hook run that failed open, could
//! not run, or whose decision was ignored because the hook is not
//! blocking is a failure of that hook, and the first failure — then every
//! [`WARNING_INTERVAL`]-th consecutive one — is recorded as a `run.notice`
//! platform record (level `warn`, code `non_blocking_hook_failed`) on the
//! run's stream. A hook that runs cleanly again resets its streak, so a
//! flapping hook stays visible without flooding the record.
//!
//! The warning is a notice, never a run failure: a non-blocking hook
//! stays non-blocking. A failure to WRITE the notice is logged and
//! swallowed for the same reason — the watchdog must never be the reason
//! a run ends.
//!
//! Fork-owned module (fabro-6922): this behavior is Fabro's, not
//! upstream Petri's, and lives in its own file so an upstream merge
//! cannot overwrite it; the fork test file `tests/fork_hook_warning.rs`
//! presence-pins it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use fabro_store::platform_records::{PlatformRecord, RunNoticeRecord};
use fabro_types::{RunId, RunNoticeCode, RunNoticeLevel};
use petri_execution::hooks::{HookPoint, HookReport, HookRequest, HookRun, HookService};
use tracing::warn;

use crate::platform_records::PlatformRecords;

/// How often a persistently failing hook re-warns: the first consecutive
/// failure, then every 20th, so a hook failing on every stage of every
/// stage-heavy run produces a steady trickle of notices instead of one
/// per stage.
pub const WARNING_INTERVAL: u64 = 20;

/// The warning prefix of a report warning that names a hook whose
/// decision was ignored because the hook is not blocking. Petri's local
/// service emits this exact phrasing (`hook `X` is not blocking; its
/// decision is ignored`); the watchdog matches the stable head of it.
const NOT_BLOCKING_PREFIX: &str = "is not blocking; its decision is ignored";

/// The label a hook without a configured name carries in its notices.
const UNNAMED: &str = "<unnamed>";

/// Where the watchdog's warnings go: the run's platform records, keyed to
/// the run, with the per-hook streak of consecutive failures.
pub struct HookWarningSink {
    records: Arc<dyn PlatformRecords>,
    run_id:  RunId,
    streaks: Mutex<HashMap<String, u64>>,
}

impl HookWarningSink {
    /// The sink for `run_id`, appending through `records`.
    #[must_use]
    pub fn new(records: Arc<dyn PlatformRecords>, run_id: RunId) -> Self {
        Self {
            records,
            run_id,
            streaks: Mutex::new(HashMap::new()),
        }
    }

    /// Whether the streak of `count` consecutive failures earns a notice:
    /// the first one, then every [`WARNING_INTERVAL`]-th.
    #[must_use]
    pub fn earns_notice(count: u64) -> bool {
        count == 1 || count.is_multiple_of(WARNING_INTERVAL)
    }

    /// One hook ran clean: its streak of consecutive failures ends.
    fn reset(&self, hook: &str) {
        sync_lock(&self.streaks).remove(hook);
    }

    /// One hook failed: advance its streak and, when the streak earns a
    /// notice, record the `run.notice` for it. `point` and `node` name
    /// where the failure happened; `message` is the report's own.
    async fn record_failure(
        &self,
        hook: &str,
        point: HookPoint,
        node: Option<&str>,
        message: &str,
    ) {
        let count = *sync_lock(&self.streaks)
            .entry(hook.to_string())
            .or_insert(0)
            + 1;
        sync_lock(&self.streaks).insert(hook.to_string(), count);
        if !Self::earns_notice(count) {
            return;
        }
        let at = match node {
            Some(node) => format!(" at {point:?} on node `{node}`"),
            None => format!(" at {point:?}"),
        };
        let notice = PlatformRecord::RunNotice(RunNoticeRecord {
            level:   RunNoticeLevel::Warn,
            code:    RunNoticeCode::NonBlockingHookFailed.to_string(),
            message: format!("hook `{hook}` failed{at} ({count} consecutive failures): {message}"),
        });
        if let Err(error) = self.records.append(&self.run_id, &notice, None).await {
            warn!(
                run_id = %self.run_id,
                hook,
                error = %error,
                "the non-blocking hook failure notice was not recorded"
            );
        }
    }

    /// Fold one report into the streaks: every hook the report names
    /// either ran clean (its streak resets) or failed (its streak
    /// advances, and a streak that earns a notice writes one).
    async fn observe(&self, report: &HookReport, node: Option<&str>) {
        for (hook, failure) in failures(report) {
            match failure {
                Some(message) => {
                    self.record_failure(&hook, report.point, node, &message)
                        .await;
                }
                None => self.reset(&hook),
            }
        }
    }
}

/// The hooks of `report` that the report names, each with the message of
/// its failure, or `None` when it ran clean. A hook failed when the
/// service recorded it `failed_open` or `unsupported`, or when the report
/// warns that its decision was ignored because it is not blocking — the
/// shape a command hook that exits non-zero takes under Fabro's rules.
fn failures(report: &HookReport) -> Vec<(String, Option<String>)> {
    report
        .hooks
        .iter()
        .map(|run| {
            let hook = hook_name(run);
            let failure = match run.state.as_ref() {
                "failed_open" | "unsupported" => {
                    Some(run.message.clone().unwrap_or_else(|| run.state.to_string()))
                }
                _ => ignored_decision(report, &hook).then(|| {
                    run.message
                        .clone()
                        .unwrap_or_else(|| "the hook's decision was ignored".to_string())
                }),
            };
            (hook, failure)
        })
        .collect()
}

/// Whether the report warns that `hook`'s decision was ignored because
/// the hook is not blocking: the tell of a failing non-blocking hook.
fn ignored_decision(report: &HookReport, hook: &str) -> bool {
    let head = format!("hook `{hook}` ");
    report.warnings.iter().any(|warning| {
        let trimmed = warning.trim_start();
        trimmed.starts_with(&head) && trimmed.contains(NOT_BLOCKING_PREFIX)
    })
}

/// The label a hook run carries: its configured name, or the unnamed
/// tell when it has none.
fn hook_name(run: &HookRun) -> String {
    if run.name.is_empty() {
        UNNAMED.to_string()
    } else {
        run.name.clone()
    }
}

/// `std::sync::Mutex` locked without poisoning the caller.
fn sync_lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The hook service decorated with the watchdog: every report the inner
/// service returns is observed before it travels on, so a persistently
/// failing non-blocking hook reaches the run's stream as a warning
/// notice no matter which point ran it. Installed by
/// [`crate::runtime::RuntimeSpec`] when the run's engine pass wires a
/// [`HookWarningSink`] (every run with Fabro's hooks does,
/// [`crate::engine`]).
pub struct HookWarningWatchdog {
    inner: Arc<dyn HookService>,
    sink:  Arc<HookWarningSink>,
}

impl HookWarningWatchdog {
    /// Wrap `inner`, its reports observed into `sink`.
    #[must_use]
    pub fn new(inner: Arc<dyn HookService>, sink: Arc<HookWarningSink>) -> Self {
        Self { inner, sink }
    }
}

#[async_trait]
impl HookService for HookWarningWatchdog {
    async fn run(&self, request: HookRequest) -> HookReport {
        let node = request
            .view
            .as_ref()
            .map(|view| view.node_name().to_string());
        let report = self.inner.run(request).await;
        self.sink.observe(&report, node.as_deref()).await;
        report
    }

    fn configured_hooks(&self, point: HookPoint) -> Vec<String> {
        self.inner.configured_hooks(point)
    }
}

#[cfg(test)]
mod tests {
    use petri_execution::hooks::HookDecision;

    use super::*;

    fn run_of(name: &str, state: &str, message: Option<&str>) -> HookRun {
        HookRun {
            name:        name.to_string(),
            state:       state.into(),
            duration_ms: None,
            message:     message.map(str::to_string),
            usage:       None,
        }
    }

    fn report(point: HookPoint, hooks: Vec<HookRun>, warnings: Vec<String>) -> HookReport {
        HookReport {
            point,
            decision: HookDecision::Proceed,
            hooks,
            warnings,
            activity: Vec::new(),
        }
    }

    #[test]
    fn a_failing_hook_is_named_and_a_clean_one_is_not() {
        let report = report(
            HookPoint::AfterVisit,
            vec![
                run_of(
                    "judgment-shadow",
                    "executed",
                    Some("block: hook exited with code 1"),
                ),
                run_of("healthy", "executed", None),
                run_of("bridge", "failed_open", Some("HTTP client: boom")),
                run_of("odd-one", "unsupported", None),
            ],
            vec![
                "hook `judgment-shadow` is not blocking; its decision is ignored".to_string(),
                "an unrelated warning".to_string(),
            ],
        );
        let judged = failures(&report);
        assert_eq!(
            judged,
            vec![
                (
                    "judgment-shadow".to_string(),
                    Some("block: hook exited with code 1".to_string())
                ),
                ("healthy".to_string(), None),
                ("bridge".to_string(), Some("HTTP client: boom".to_string())),
                ("odd-one".to_string(), Some("unsupported".to_string())),
            ],
            "an ignored non-blocking decision and a failed-open run are failures; a clean run is not"
        );
    }

    #[test]
    fn a_blocking_hooks_block_decision_is_not_a_watchdog_failure() {
        // No "is not blocking" warning: the hook is blocking, its block is
        // a decision the engine acts on, not a silent failure.
        let report = report(
            HookPoint::BeforeAttempt,
            vec![run_of("gatekeeper", "executed", Some("block: nope"))],
            Vec::new(),
        );
        assert_eq!(
            failures(&report),
            vec![("gatekeeper".to_string(), None)],
            "a blocking hook's decision is the engine's business, not the watchdog's"
        );
    }

    #[test]
    fn the_notice_cadence_warns_on_the_first_and_every_interval_th() {
        assert!(HookWarningSink::earns_notice(1), "the first failure warns");
        assert!(
            !HookWarningSink::earns_notice(2),
            "an unbroken streak does not re-warn immediately"
        );
        assert!(
            HookWarningSink::earns_notice(WARNING_INTERVAL),
            "the interval-th failure re-warns"
        );
        assert!(!HookWarningSink::earns_notice(WARNING_INTERVAL + 1));
    }
}
