//! Owner of the run-lifecycle event sequence (fabro-3fe4).
//!
//! The protocol `RunStartRequested -> RunRunnable -> RunStarting -> Running`
//! (plus the resubmit and terminal-failure entries) has exactly one emission
//! point: this module. Emitters (`operations::start`, `operations::resume`,
//! the server's terminal writes, and workflow test support) are adapters over
//! these functions; the status semantics of the emitted stream are owned by
//! the shared transition table in `fabro_types::apply_lifecycle_event`.

use fabro_store::RunDatabase;
use fabro_types::{BlobHash, FailureReason, Principal, RunId, RunRunnableSource, RunTiming};

use crate::error::Error;
use crate::event::{
    Event, RunEventPersistenceError, RunEventSink, append_event, append_event_to_sink,
};

/// Request a start (or resubmission) for a run. `resume: true` re-opens a
/// terminal run; `resume: false` is the request marker of a fresh start.
pub async fn request_start(
    sink: &RunEventSink,
    run_id: &RunId,
    resume: bool,
    actor: Option<Principal>,
) -> Result<(), RunEventPersistenceError> {
    append_event_to_sink(sink, run_id, &Event::RunStartRequested { resume, actor }).await
}

/// Mark the run schedulable after a start request or an approval.
pub async fn runnable(
    sink: &RunEventSink,
    run_id: &RunId,
    source: RunRunnableSource,
    actor: Option<Principal>,
) -> Result<(), RunEventPersistenceError> {
    append_event_to_sink(sink, run_id, &Event::RunRunnable { source, actor }).await
}

/// A worker picked the run up and is initializing.
pub async fn starting(sink: &RunEventSink, run_id: &RunId) -> Result<(), RunEventPersistenceError> {
    append_event_to_sink(sink, run_id, &Event::RunStarting).await
}

/// The workflow graph began executing.
pub async fn running(sink: &RunEventSink, run_id: &RunId) -> Result<(), RunEventPersistenceError> {
    append_event_to_sink(sink, run_id, &Event::RunRunning).await
}

/// Resubmit a run's spec (definition blob) ahead of a resume.
pub async fn resubmit(
    sink: &RunEventSink,
    run_id: &RunId,
    definition_blob: Option<BlobHash>,
) -> Result<(), RunEventPersistenceError> {
    append_event_to_sink(sink, run_id, &Event::RunSubmitted { definition_blob }).await
}

/// Persist the terminal `run.failed` event for a run that failed outside the
/// normal worker pipeline (pre-execution rejection or cancellation).
pub async fn terminal_failure(
    run_store: &RunDatabase,
    run_id: &RunId,
    error: &Error,
    reason: FailureReason,
    timing: RunTiming,
) -> anyhow::Result<()> {
    let failure_event =
        Event::workflow_run_failed_from_error(error, timing, reason, None, None, None, None);
    append_event(run_store, run_id, &failure_event).await
}
