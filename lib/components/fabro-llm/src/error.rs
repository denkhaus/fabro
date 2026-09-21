//! Fork rate-limit classification constants (fabro-a3d8, petri port W3-1).
//!
//! The ErrorData-based helpers of the engine era died with the engine; the
//! park classification now runs in the projection fold over the failure
//! MESSAGE (`gateway::reset_window`), keyed on this threshold.

use std::time::Duration;

/// A rate-limit wait longer than the retry middleware's `Retry-After` cap.
///
/// The retry layer honors provider waits only up to this bound; anything
/// longer cannot be bridged by count-based retries and parks the run
/// (`Blocked { QuotaRateLimit }`, ADR-0021 rev 2).
pub const LONG_RATE_LIMIT_WINDOW: Duration = Duration::from_mins(1);
