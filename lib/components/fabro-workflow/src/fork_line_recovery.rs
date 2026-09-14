//! Fork-only line-recovery classification (fabro-986b, user decision
//! 2026-09-14).
//!
//! A provider usage-window rate limit (zai's 5-hour hard cut) closes the
//! LLM for hours. Count-based stage retries cannot bridge that window:
//! every attempt re-sends the full stage prompt and preamble (fabro-183f)
//! and fails against the same closed limit, and the exhaustion path then
//! surfaces as a deterministic goal-gate failure that hides the quota
//! cause from every downstream consumer (schedule breaker, line recovery).
//!
//! This module classifies such errors so the engine parks the run instead
//! of burning retries: [`parks_for_usage_window`] builds on fabro-llm's
//! long-window and naive-prose detectors (fabro-a3d8, fabro-0607). The
//! run-level mapping to `FailureReason::SoftStop` already lives in
//! `Error::failure_reason`; the only engine seam is `Error::is_retryable`,
//! which must not retry what cannot succeed before the window reopens.
//!
//! The recovery CADENCE is deliberately NOT derived from provider prose
//! (user decision 2026-09-14): the fabro-server line-recovery recheck
//! probes on a fixed 10-minute interval regardless of any announced reset
//! time. Only the PRESENCE of an announced usage window distinguishes
//! these 429s from ordinary short-window 429s, which stay retryable.
//!
//! Fork-file policy (fabro-986b): this file exists only on our fork —
//! upstream does not have it, so no merge can conflict with it away. The
//! calling seams are one-liners and are pinned by
//! `fork_line_recovery_tests.rs` at the crate root.

use std::time::SystemTime;

use fabro_llm::gateway::{RateLimitWindow, reset_window};
use fabro_llm::{
    ErrorData, LONG_RATE_LIMIT_WINDOW, long_rate_limit_window, rate_limit_window_unknown,
};

use crate::outcome::{FailureCategory, FailureDetail};

/// Whether `err` is a provider usage-window rate limit that must park the
/// run instead of consuming stage retries.
///
/// True for a 429 whose advised wait exceeds the retry middleware's
/// `Retry-After` cap (a multi-hour usage window) or whose announced reset
/// deadline carries no UTC offset (true wait unknown, fabro-0607). Short
/// provider waits keep normal retry behavior.
#[must_use]
pub fn parks_for_usage_window(err: &ErrorData) -> bool {
    long_rate_limit_window(err).is_some() || rate_limit_window_unknown(err)
}

/// Whether a failed outcome's failure detail is a usage-window park.
///
/// The run must END on such a failure (resumable soft stop,
/// `FailureReason::SoftStop` via `pipeline::finalize`'s message mapping)
/// instead of routing on: every LLM re-entry fails against the same closed
/// window, and looping re-enters exactly that until a goal-gate or visit
/// limit turns the quota cause into a deterministic failure
/// (run 01M2E7VZYX8V forensics, fabro-986b).
///
/// Detection keys on the structured facts, never on a parsed backoff
/// duration: the failure must be `TransientInfra` and its message must
/// ANNOUNCE a reset window that is either naive (offset-less wallclock,
/// true wait unknown) or longer than the retry middleware's cap.
#[must_use]
pub fn failure_detail_parks(failure: &FailureDetail) -> bool {
    if failure.category != FailureCategory::TransientInfra {
        return false;
    }
    match reset_window(&failure.message, SystemTime::now()) {
        Some(RateLimitWindow::UnknownEta) => true,
        Some(RateLimitWindow::Reopens(window)) => window > LONG_RATE_LIMIT_WINDOW,
        None => false,
    }
}
