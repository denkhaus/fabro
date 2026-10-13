//! Fork-only pin (fabro-71e6, sprint-35 arch finding N1): the REAL 429
//! bodies observed in production keep parsing as provider windows. The
//! 2026-10-10 evidence tests (fabro-748b) live inline in gateway.rs's
//! upstream test module, which an upstream merge rewrites freely; this
//! file re-pins the live shapes through the public parser API.
//!
//! Source incident: zai 429 run 01M4KMWYRQDJ54ZSS08NJNH74N (2026-10-10).

use std::time::SystemTime;

use fabro_llm::gateway::{
    RateLimitWindow, ResetDeadline, parse_reset_deadline, reset_prose_is_naive, reset_window,
};

/// The exact zai body from the incident: a naive Beijing wallclock plus
/// the engine's `[provider ..., status 429, code 1308]` suffix. The
/// bracket suffix must not defeat the wallclock parse — and the naive
/// wallclock must never claim a wait duration.
#[test]
fn the_live_zai_window_body_parses_as_unknown_eta() {
    let body = "Usage limit reached for 5 hour. Your limit will reset at \
                2026-10-11 05:04:33 [provider zai, status 429, code 1308]";
    assert!(reset_prose_is_naive(body));
    assert_eq!(
        parse_reset_deadline(body),
        Some(ResetDeadline::Naive),
        "the bracket suffix must not defeat the wallclock parse"
    );
    assert_eq!(
        reset_window(body, SystemTime::now()),
        Some(RateLimitWindow::UnknownEta)
    );
}

/// Offset-carrying deadlines keep parsing as a real wait; past ones advise
/// nothing (the retry layer's short-window path stays with the parser's
/// None).
#[test]
fn offset_deadlines_parse_to_a_wait_and_past_ones_to_none() {
    let future = "quota: your usage will reset at 2879-05-29T07:15:00Z";
    assert!(matches!(
        reset_window(future, SystemTime::now()),
        Some(RateLimitWindow::Reopens(_))
    ));
    let past = "quota: your usage will reset at 1970-01-01T00:00:00Z";
    assert_eq!(reset_window(past, SystemTime::now()), None);
}

/// A 429 without reset prose is not a window: OpenAI-style "try again in
/// 20ms" bodies and bare statuses must stay on the retry path.
#[test]
fn bodies_without_reset_prose_are_not_windows() {
    for body in [
        "Rate limit reached for gpt-4 on requests per min: Limit 3, Used 3. Please try again in 20ms.",
        "429 Too Many Requests",
        "too many requests",
    ] {
        assert_eq!(reset_window(body, SystemTime::now()), None, "{body}");
    }
}
