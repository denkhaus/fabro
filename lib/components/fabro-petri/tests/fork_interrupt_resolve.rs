//! Fork presence pin (fabro-d4f2, sprint 50 part 2): a plain interrupt
//! settles the attempt after a grace instead of wedging the run in
//! `running` forever (incident 01M59MG9XS2AA2S9H6AJYFD9Z5). The engine
//! behavior is pinned in petri's own suite (`an_interrupt_without_
//! follow_up_fails_the_attempt_after_the_grace`); these tests exist only
//! on the fork and guard the fabro-reachable seam — the grace constant,
//! the host override capability, and the terminal failure class a wedged
//! interrupt now records. A merge resolution that drops them dropped the
//! feature — restore it, never relax the test.

use std::time::Duration;

use petri_attractor_steps::agent::{INTERRUPT_RESOLVE_GRACE, InterruptGrace};
use petri_frontend_attractor::kinds::INTERRUPTED_CLASS;

/// The settle window is the grilled 60s default (user decision
/// 2026-10-20), not a stall-timeout derivative: it must outlive a host's
/// legitimate think-then-steer pause while still bounding the park.
#[test]
fn interrupt_resolve_grace_is_sixty_seconds() {
    assert_eq!(INTERRUPT_RESOLVE_GRACE.as_secs(), 60);
    assert!(INTERRUPT_RESOLVE_GRACE.subsec_nanos() == 0);
}

/// The capability a host (and petri's own tests) install to override the
/// window exists and carries a plain duration — fabro installs none, so
/// the 60s default is what every production run uses.
#[test]
fn interrupt_grace_capability_carries_the_override() {
    let override_window = InterruptGrace(Duration::from_millis(300));
    let InterruptGrace(window) = override_window;
    assert_eq!(window.as_millis(), 300);
}

/// The settle class is the human gate's existing terminal vocabulary —
/// never `retry_requested` (retryability follows failure kind, petri
/// 13e4e66): re-sending the same turn is not what an interrupt asks for.
/// The run record and failure surfaces name exactly this string.
#[test]
fn interrupted_class_is_the_terminal_vocabulary() {
    assert_eq!(INTERRUPTED_CLASS, "interrupted");
}
