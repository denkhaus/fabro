//! The tracker failure classification at the [`super::TrackerProvider`]
//! seam (denkhaus/work#1556).
//!
//! The port's methods return `anyhow::Result`; the CLASSES a caller can
//! act on travel beside the chain as [`TrackerError`] values in the same
//! error: an adapter inserts its classification, and a door (or the
//! materializing decorator, which must pass it through untouched)
//! downcasts it out of the chain. Everything the classification does not
//! cover keeps its full prose chain — no information is folded away.
//!
//! The quota class is the load-bearing one: `RateLimited` carries the
//! forge's announced window (`Retry-After` or the `X-RateLimit-Reset`
//! delta) so doors answer a structured, machine-readable rate-limit
//! error with the window in its meta, and a gear lap whose door answers
//! rate-limited parks with preserved work instead of hard-failing on
//! prose — the established LLM quota posture, mirrored at the tracker
//! seam.

use std::time::Duration;

/// The forge-agnostic failure classes a caller branches on at the
/// tracker port. The adapter maps its private API error into these
/// where callers act on them; every other failure keeps its typed API
/// error as the chain and carries NO classification.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum TrackerError {
    /// The forge cannot serve the call right now (5xx or transport).
    /// `retryable` names whether the same call is worth a later retry.
    #[error("tracker provider unavailable (retryable: {retryable})")]
    Unavailable {
        /// Whether retrying the same call later can succeed (a 5xx can,
        /// a deployment that lost its forge binding cannot).
        retryable: bool,
    },

    /// The forge's quota is exhausted. `window` is the wait the forge
    /// announced — `Retry-After` seconds, or the delta to its
    /// `X-RateLimit-Reset` instant — and `None` when the answer carried
    /// no window. Callers park or back off on this class instead of
    /// failing; the window lets a caller wait precisely instead of
    /// guessing a sleep (the #1521 lap's painpoint).
    #[error("tracker provider rate limited (window: {})", fmt_window(*window))]
    RateLimited {
        /// The announced wait until the quota window reopens, when the
        /// forge announced one.
        window: Option<Duration>,
    },

    /// The forge knows no such item — its own 404 on a key-addressed
    /// call, never a local filter miss.
    #[error("work item not found")]
    NotFound,

    /// The forge refused the write as a conflicting state (409).
    #[error("work item conflict")]
    Conflict,
}

impl TrackerError {
    /// The announced wait until the quota window reopens, when the
    /// forge saw one (denkhaus/work#1556). `None` on every
    /// non-quota class.
    #[must_use]
    pub fn window(&self) -> Option<Duration> {
        match self {
            Self::RateLimited { window } => *window,
            _ => None,
        }
    }

    /// Whether this is the quota class the work doors treat as the
    /// park posture's input (a structured rate-limited answer, never a
    /// hard-failed run).
    #[must_use]
    pub fn is_rate_limited(&self) -> bool {
        matches!(self, Self::RateLimited { .. })
    }
}

/// The honest text for an announced-or-not window in `Display`.
fn fmt_window(window: Option<Duration>) -> String {
    window.map_or_else(
        || "unknown".to_string(),
        |window| format!("{}s", window.as_secs()),
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::TrackerError;

    #[test]
    fn the_window_accessor_reads_the_quota_class_only() {
        let limited = TrackerError::RateLimited {
            window: Some(Duration::from_secs(45)),
        };
        assert_eq!(limited.window(), Some(Duration::from_secs(45)));
        assert!(limited.is_rate_limited());

        let unannounced = TrackerError::RateLimited { window: None };
        assert_eq!(unannounced.window(), None);
        assert!(unannounced.is_rate_limited());

        let down = TrackerError::Unavailable { retryable: true };
        assert_eq!(down.window(), None);
        assert!(!down.is_rate_limited());
    }

    #[test]
    fn the_display_names_the_class_and_the_window() {
        assert_eq!(
            TrackerError::RateLimited {
                window: Some(Duration::from_secs(45)),
            }
            .to_string(),
            "tracker provider rate limited (window: 45s)"
        );
        assert_eq!(
            TrackerError::RateLimited { window: None }.to_string(),
            "tracker provider rate limited (window: unknown)"
        );
        assert_eq!(
            TrackerError::Unavailable { retryable: false }.to_string(),
            "tracker provider unavailable (retryable: false)"
        );
        assert_eq!(TrackerError::NotFound.to_string(), "work item not found");
        assert_eq!(TrackerError::Conflict.to_string(), "work item conflict");
    }
}
