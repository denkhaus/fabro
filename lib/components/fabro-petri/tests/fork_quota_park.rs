//! Fork presence pin (fabro-2e7b, ADR-0021 rev 2 Option C): quota-class
//! failures park the run as `Blocked { QuotaRateLimit }` instead of
//! failing it. The classification core is the rate-limit-window parser
//! from fabro-llm (carried fork surface); the fold seam applies it to the
//! conclusion's failure message.

use std::time::{Duration, SystemTime};

use fabro_llm::gateway::{RateLimitWindow, reset_window};

#[test]
fn long_usage_window_resets_parse_as_reopens() {
    // zai's multi-hour window prose (fabro-a3d8): a far-future RFC3339
    // deadline with a UTC offset parses as a reopens window.
    let future = "2879-05-29T07:15:00Z";
    let message = format!("quota exceeded: your usage will reset at {future}");
    let window = reset_window(&message, SystemTime::UNIX_EPOCH).expect("a long window parses");
    assert!(
        matches!(window, RateLimitWindow::Reopens(wait) if wait > Duration::from_mins(1)),
        "the park threshold sees a long reopens window: {window:?}"
    );
}

#[test]
fn naive_reset_prose_parks_as_unknown_eta() {
    // zai sends Beijing wallclocks without an offset (fabro-0607): the
    // wait is unknown but the window is closed — UnknownEta, still a park.
    let message = "quota exceeded: your usage will reset at 2879-05-29 15:15:00";
    let window = reset_window(message, SystemTime::UNIX_EPOCH);
    assert!(
        matches!(window, Some(RateLimitWindow::UnknownEta)),
        "naive deadlines must never claim a duration: {window:?}"
    );
}

#[test]
fn short_retryable_429s_do_not_park() {
    // A deadline already in the past or within the retry budget must not
    // produce a park-worthy window.
    let past = "1970-01-01T00:00:00Z";
    let message = format!("quota exceeded: your usage will reset at {past}");
    assert!(
        reset_window(&message, SystemTime::now()).is_none(),
        "past deadlines are not windows"
    );
}

#[test]
fn the_park_taxonomy_exists_on_the_wire() {
    // The blocked reason the fold writes, serialized snake_case (Slack and
    // the gate read the string).
    let status = fabro_types::RunStatus::Blocked {
        blocked_reason: fabro_types::BlockedReason::QuotaRateLimit,
    };
    assert_eq!(
        serde_json::to_string(&status).unwrap(),
        "{\"kind\":\"blocked\",\"blocked_reason\":\"quota_rate_limit\"}"
    );
}
