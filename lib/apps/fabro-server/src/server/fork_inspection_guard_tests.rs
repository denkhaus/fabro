//! The terminal-run inspection guard (fabro-afab; the legacy
//! InspectionSandbox, Terminal-Run-Leak fabro-8d30a): a read-only
//! inspection that reactivates a terminal run's stopped sandbox must
//! stop it again — on finish and on every early drop — while a live
//! run's sandbox is never the inspection's to stop.
//!
//! Fork-only presence pin: upstream has no `InspectionSandbox`, so a
//! merge that drops the guard reds here instead of leaking sandboxes.

use std::sync::Arc;

use sandbox_driver::{Sandbox, SandboxState};
use sandbox_driver_testing::ScriptedSandbox;

use crate::sandbox_access::InspectionSandbox;

fn running_sandbox() -> (Arc<ScriptedSandbox>, Arc<dyn Sandbox>) {
    let inner = Arc::new(
        ScriptedSandbox::with_id_and_working_dir("test-sandbox", "/workspace")
            .state(SandboxState::Running),
    );
    let handle: Arc<dyn Sandbox> = Arc::clone(&inner) as Arc<_>;
    (inner, handle)
}

#[tokio::test(start_paused = true)]
async fn a_terminal_inspection_stops_the_sandbox_again_on_finish() {
    let (inner, sandbox) = running_sandbox();
    InspectionSandbox::terminal(sandbox).finish().await;
    assert_eq!(inner.current_state(), SandboxState::Stopped);
}

#[tokio::test(start_paused = true)]
async fn a_terminal_inspection_stops_once_even_when_drop_follows_finish() {
    let (inner, sandbox) = running_sandbox();
    let guard = InspectionSandbox::terminal(sandbox);
    guard.finish().await;
    drop(guard);
    assert_eq!(
        inner.current_state(),
        SandboxState::Stopped,
        "finish wins, drop is a no-op"
    );
}

#[tokio::test(start_paused = true)]
async fn an_early_drop_of_a_terminal_inspection_stops_the_sandbox() {
    let (inner, sandbox) = running_sandbox();
    {
        let guard = InspectionSandbox::terminal(sandbox);
        assert_eq!(
            guard.sandbox().id().as_str(),
            "test-sandbox",
            "the guard reads through to the sandbox"
        );
        drop(guard);
    }
    // Drop spawns the stop; give the paused runtime a beat to run it.
    tokio::time::advance(std::time::Duration::from_millis(1)).await;
    assert_eq!(inner.current_state(), SandboxState::Stopped);
}

#[tokio::test(start_paused = true)]
async fn a_live_run_inspection_never_stops_the_sandbox() {
    let (inner, sandbox) = running_sandbox();
    let guard = InspectionSandbox::live(sandbox);
    guard.finish().await;
    drop(guard);
    tokio::time::advance(std::time::Duration::from_millis(1)).await;
    assert_eq!(inner.current_state(), SandboxState::Running);
}
