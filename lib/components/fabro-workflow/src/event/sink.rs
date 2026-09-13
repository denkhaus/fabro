use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use ::fabro_types::{RunEvent, RunId, RunProjection, bound_run_event};
use anyhow::Result;
use chrono::{DateTime, Utc};
use fabro_store::{Database, RunDatabase};
use fabro_util::error::{SharedError, collect_chain};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::{Mutex as AsyncMutex, mpsc, oneshot, watch};

use super::emitter::Emitter;
use super::offload::offload_oversized_run_event;
use super::redaction::{build_redacted_event_payload, redacted_event_json};
use super::{Event, to_run_event, to_run_event_at};
use crate::runtime_store::RunStoreHandle;

pub async fn append_event(run_store: &RunDatabase, run_id: &RunId, event: &Event) -> Result<()> {
    let stored = to_run_event(run_id, event);
    let payload = build_redacted_event_payload(&stored, run_id)?;
    run_store
        .append_event(&payload)
        .await
        .map(|_| ())
        .map_err(anyhow::Error::from)
}

/// Creates a run by committing its redacted `run.created` event and canonical
/// current row in one SQLite transaction.
pub async fn create_run(
    store: &Database,
    run_id: &RunId,
    event: &Event,
    timestamp: DateTime<Utc>,
) -> Result<RunDatabase> {
    let stored = to_run_event_at(run_id, event, timestamp, None);
    let payload = build_redacted_event_payload(&stored, run_id)?;
    Box::pin(store.create_run_with_first_event(run_id, &payload))
        .await
        .map_err(anyhow::Error::from)
}

pub async fn append_event_if(
    run_store: &RunDatabase,
    run_id: &RunId,
    event: &Event,
    predicate: impl FnOnce(&RunProjection) -> bool,
) -> Result<bool> {
    let stored = to_run_event(run_id, event);
    let payload = build_redacted_event_payload(&stored, run_id)?;
    run_store
        .append_event_if(&payload, predicate)
        .await
        .map(|seq| seq.is_some())
        .map_err(anyhow::Error::from)
}

pub async fn append_event_to_sink(
    sink: &RunEventSink,
    run_id: &RunId,
    event: &Event,
) -> Result<(), RunEventPersistenceError> {
    let stored = to_run_event(run_id, event);
    sink.write_run_event(&stored)
        .await
        .map_err(|err| RunEventPersistenceError::Write {
            run_id: *run_id,
            event:  stored.body.event_name().to_string(),
            source: SharedError::new(err),
        })
}

#[derive(Clone)]
pub enum RunEventSink {
    Store(RunStoreHandle),
    JsonLines(Arc<AsyncMutex<Pin<Box<dyn AsyncWrite + Send>>>>),
    Callback(Arc<RunEventSinkCallback>),
    Map {
        transform: Arc<RunEventTransform>,
        inner:     Box<Self>,
    },
    Composite(Vec<Self>),
}

type RunEventSinkFuture = Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>;
type RunEventSinkCallback = dyn Fn(RunEvent) -> RunEventSinkFuture + Send + Sync + 'static;
type RunEventTransform = dyn Fn(RunEvent) -> RunEvent + Send + Sync + 'static;

impl RunEventSink {
    #[must_use]
    pub fn store(run_store: RunDatabase) -> Self {
        Self::Store(RunStoreHandle::local(run_store))
    }

    #[must_use]
    pub fn backend(run_store: RunStoreHandle) -> Self {
        Self::Store(run_store)
    }

    #[must_use]
    pub fn json_lines<W>(writer: W) -> Self
    where
        W: AsyncWrite + Send + 'static,
    {
        Self::JsonLines(Arc::new(AsyncMutex::new(Box::pin(writer))))
    }

    #[must_use]
    pub fn callback<F, Fut>(callback: F) -> Self
    where
        F: Fn(RunEvent) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        Self::Callback(Arc::new(move |event| Box::pin(callback(event))))
    }

    #[must_use]
    pub fn fanout(sinks: Vec<Self>) -> Self {
        let mut flattened = Vec::new();
        for sink in sinks {
            match sink {
                Self::Composite(inner) => flattened.extend(inner),
                other => flattened.push(other),
            }
        }
        Self::Composite(flattened)
    }

    #[must_use]
    pub fn map<F>(transform: F, inner: Self) -> Self
    where
        F: Fn(RunEvent) -> RunEvent + Send + Sync + 'static,
    {
        Self::Map {
            transform: Arc::new(transform),
            inner:     Box::new(inner),
        }
    }

    pub async fn write_run_event(&self, event: &RunEvent) -> Result<()> {
        let mut pending = vec![(self, event.clone())];
        while let Some((sink, mut event)) = pending.pop() {
            match sink {
                Self::Store(run_store) => {
                    bound_event_for_store(&mut event, run_store).await;
                    run_store.append_run_event(&event).await?;
                }
                Self::JsonLines(writer) => {
                    bound_event_for_blobless_sink(&mut event);
                    let line = redacted_event_json(&event)?;
                    let mut writer = writer.lock().await;
                    writer.write_all(line.as_bytes()).await?;
                    writer.write_all(b"\n").await?;
                    writer.flush().await?;
                }
                Self::Callback(callback) => {
                    bound_event_for_blobless_sink(&mut event);
                    callback(event).await?;
                }
                Self::Map { transform, inner } => {
                    pending.push((inner.as_ref(), transform(event)));
                }
                Self::Composite(sinks) => {
                    for sink in sinks.iter().rev() {
                        pending.push((sink, event.clone()));
                    }
                }
            }
        }
        Ok(())
    }
}

/// Bound an event headed for the run store: first offload oversized
/// payloads into the store's blob store losslessly (fabro-5082), then fall
/// back to the lossy `bound_run_event` truncation when the offload fails or
/// the event still exceeds the budget.
async fn bound_event_for_store(event: &mut RunEvent, run_store: &RunStoreHandle) {
    match offload_oversized_run_event(event, run_store).await {
        Ok(Some(offload)) => tracing::info!(
            run_id = %event.run_id,
            event = %event.body.event_name(),
            offloaded_values = offload.values,
            offloaded_bytes = offload.bytes,
            "offloaded oversized run event payloads to the blob store"
        ),
        Ok(None) => {}
        Err(err) => tracing::warn!(
            run_id = %event.run_id,
            event = %event.body.event_name(),
            error = %err,
            "blob offload of oversized run event failed; falling back to lossy truncation"
        ),
    }
    if let Some(bound) = bound_run_event(event) {
        tracing::warn!(
            run_id = %event.run_id,
            event = event.body.event_name(),
            original_bytes = bound.original_bytes,
            bounded_bytes = bound.bounded_bytes,
            "run event body still exceeded the append budget after blob offload; truncated as a \
             fallback to protect the run store"
        );
    }
}

/// Blob-less sinks keep the bounded envelope of fabro-a723: shrink oversized
/// events lossily before they are written.
fn bound_event_for_blobless_sink(event: &mut RunEvent) {
    if let Some(bound) = bound_run_event(event) {
        tracing::warn!(
            run_id = %event.run_id,
            event = event.body.event_name(),
            original_bytes = bound.original_bytes,
            bounded_bytes = bound.bounded_bytes,
            "run event body exceeded the append budget; truncated to protect the run store"
        );
    }
}

#[allow(
    clippy::large_enum_variant,
    reason = "Logger queue messages stay inline to avoid boxing hot-path payloads."
)]
enum RunEventCommand {
    Event(RunEvent),
    /// An event whose writer waits for the store to accept it.
    Acknowledged(
        RunEvent,
        oneshot::Sender<Result<(), RunEventPersistenceError>>,
    ),
    Flush(oneshot::Sender<Result<(), RunEventPersistenceError>>),
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum RunEventPersistenceError {
    #[error("failed to persist run event {event} for run {run_id}")]
    Write {
        run_id: RunId,
        event:  String,
        #[source]
        source: SharedError,
    },
    #[error("run event persistence task stopped")]
    TaskStopped,
}

async fn write_event(
    sink: &RunEventSink,
    event: &RunEvent,
) -> Result<(), RunEventPersistenceError> {
    match sink.write_run_event(event).await {
        Ok(()) => Ok(()),
        Err(err) => {
            let rendered_error = collect_chain(err.as_ref()).join(": ");
            tracing::error!(
                run_id = %event.run_id,
                event = %event.body.event_name(),
                error = %rendered_error,
                "Failed to persist run event; stopping workflow",
            );
            Err(RunEventPersistenceError::Write {
                run_id: event.run_id,
                event:  event.body.event_name().to_string(),
                source: SharedError::new(err),
            })
        }
    }
}

#[derive(Clone)]
pub struct RunEventLogger {
    tx:         mpsc::UnboundedSender<RunEventCommand>,
    failure_rx: watch::Receiver<Option<RunEventPersistenceError>>,
}

impl RunEventLogger {
    #[must_use]
    pub fn new(sink: RunEventSink) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (failure_tx, failure_rx) = watch::channel(None);

        tokio::spawn(async move {
            // The watch channel is the single record of the latched failure:
            // the worker is its only writer, so borrowing it here cannot race.
            while let Some(command) = rx.recv().await {
                match command {
                    RunEventCommand::Event(event) => {
                        if failure_tx.borrow().is_some() {
                            continue;
                        }
                        if let Err(failure) = write_event(&sink, &event).await {
                            failure_tx.send_replace(Some(failure));
                        }
                    }
                    RunEventCommand::Acknowledged(event, tx) => {
                        let latched = failure_tx.borrow().clone();
                        let result = match latched {
                            Some(failure) => Err(failure),
                            None => match write_event(&sink, &event).await {
                                Ok(()) => Ok(()),
                                Err(failure) => {
                                    failure_tx.send_replace(Some(failure.clone()));
                                    Err(failure)
                                }
                            },
                        };
                        let _ = tx.send(result);
                    }
                    RunEventCommand::Flush(tx) => {
                        let result = failure_tx.borrow().clone().map_or(Ok(()), Err);
                        let _ = tx.send(result);
                    }
                }
            }
        });

        Self { tx, failure_rx }
    }

    /// Makes this logger the emitter's persistence path: every emitted event
    /// is queued here, and [`Emitter::emit_durable`] waits for this logger's
    /// acknowledgement.
    pub fn register(&self, emitter: &Emitter) {
        emitter.attach_persistence(self.clone());
    }

    pub(super) fn enqueue(&self, event: &RunEvent) {
        if self.tx.send(RunEventCommand::Event(event.clone())).is_err() {
            tracing::error!(
                run_id = %event.run_id,
                event = %event.body.event_name(),
                "Run event persistence task stopped while forwarding event",
            );
        }
    }

    /// Writes `event` and returns once the sink has accepted it, or with the
    /// failure that stopped persistence.
    ///
    /// Ordering with events queued through the emitter is preserved: the
    /// write goes through the same queue.
    pub async fn write_acknowledged(
        &self,
        event: &RunEvent,
    ) -> Result<(), RunEventPersistenceError> {
        let (tx, rx) = oneshot::channel();
        if self
            .tx
            .send(RunEventCommand::Acknowledged(event.clone(), tx))
            .is_err()
        {
            return Err(RunEventPersistenceError::TaskStopped);
        }
        rx.await
            .unwrap_or(Err(RunEventPersistenceError::TaskStopped))
    }

    pub async fn wait_for_failure(&self) -> RunEventPersistenceError {
        let mut failure_rx = self.failure_rx.clone();
        let failure = failure_rx.wait_for(Option::is_some).await;
        match failure {
            Ok(failure) => failure
                .clone()
                .expect("wait_for only returns values matching the predicate"),
            Err(_) => RunEventPersistenceError::TaskStopped,
        }
    }

    pub async fn flush(&self) -> Result<(), RunEventPersistenceError> {
        let (tx, rx) = oneshot::channel();
        if self.tx.send(RunEventCommand::Flush(tx)).is_err() {
            return Err(RunEventPersistenceError::TaskStopped);
        }
        rx.await
            .unwrap_or(Err(RunEventPersistenceError::TaskStopped))
    }
}

#[derive(Clone)]
pub struct StoreProgressLogger {
    inner: RunEventLogger,
}

impl StoreProgressLogger {
    #[must_use]
    pub fn new(run_store: impl Into<RunStoreHandle>) -> Self {
        Self {
            inner: RunEventLogger::new(RunEventSink::backend(run_store.into())),
        }
    }

    pub fn register(&self, emitter: &Emitter) {
        self.inner.register(emitter);
    }

    pub async fn flush(&self) -> Result<(), RunEventPersistenceError> {
        self.inner.flush().await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use ::fabro_types::run_event::CheckpointCompletedProps;
    use ::fabro_types::{
        EventBody, Graph, Outcome as StoredOutcome, RunNoticeLevel, WorkflowSettings, fixtures,
        parse_blob_ref, run_event_body_budget,
    };
    use async_trait::async_trait;
    use chrono::Utc;
    use fabro_types::test_support;
    use lithos_llm::types::ReasoningOutput;
    use object_store::memory::InMemory;
    use pebble_coding_agent::events::{CodingAgentEvent, CodingEvent, TokenUsage};
    use tokio::sync::Mutex as AsyncMutex;

    use super::*;
    use crate::event::test_support::user_principal;
    use crate::event::{
        Emitter, Event, append_event, build_redacted_event_payload,
        event_payload_from_redacted_json, to_run_event,
    };
    use crate::pipeline::ResumeState;
    use crate::runtime_store::RunStoreBackend;

    #[tokio::test]
    async fn append_event_writes_store_event_shape() {
        let store = fabro_store::test_support::test_database(
            std::sync::Arc::new(object_store::memory::InMemory::new()),
            "",
            std::time::Duration::from_millis(1),
            None,
        );
        let run_store = store.create_run(&fixtures::RUN_7).await.unwrap();
        append_event(&run_store, &fixtures::RUN_7, &Event::RunCreated {
            run_id:              fixtures::RUN_7,
            title:               None,
            settings:            serde_json::to_value(WorkflowSettings::default()).unwrap(),
            graph:               serde_json::to_value(Graph::new("test")).unwrap(),
            workflow_source:     None,
            labels:              std::collections::BTreeMap::new(),
            source_directory:    None,
            workflow_slug:       None,
            workflow_version_id: None,
            target:              None,
            automation:          None,
            provenance:          test_support::test_run_provenance(),
            spec_blob:           None,
            git:                 None,
            fork_source_ref:     None,
            retried_from:        None,
            parent_id:           None,
            web_url:             None,
        })
        .await
        .unwrap();
        let stored = to_run_event(&fixtures::RUN_7, &Event::RunNotice {
            level:            RunNoticeLevel::Warn,
            code:             "example".to_string(),
            message:          "notice".to_string(),
            exec_output_tail: None,
        });
        let payload = build_redacted_event_payload(&stored, &fixtures::RUN_7).unwrap();
        run_store.append_event(&payload).await.unwrap();

        let events = run_store.list_events().await.unwrap();
        let line = events
            .into_iter()
            .find(|event| event.event.event_name() == "run.notice")
            .map(|event| event.event.to_value().unwrap())
            .unwrap();
        assert!(line.get("id").is_some());
        assert_eq!(line["event"], "run.notice");
        assert_eq!(line["properties"]["code"], "example");
    }

    #[tokio::test]
    async fn run_event_sink_json_lines_writes_canonical_event_lines() {
        use tokio::io::{AsyncBufReadExt, BufReader};

        let (writer, reader) = tokio::io::duplex(4096);
        let sink = RunEventSink::json_lines(writer);
        let event = to_run_event(&fixtures::RUN_7, &Event::RunPauseRequested { actor: None });

        sink.write_run_event(&event).await.unwrap();

        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();

        let payload = event_payload_from_redacted_json(line.trim_end(), &fixtures::RUN_7).unwrap();
        assert_eq!(payload.as_value()["event"], "run.pause.requested");
        assert_eq!(payload.as_value()["properties"]["action"], "pause");
    }

    #[tokio::test]
    async fn run_event_sink_json_lines_carries_agent_message_reasoning() {
        use tokio::io::{AsyncBufReadExt, BufReader};

        let (writer, reader) = tokio::io::duplex(4096);
        let sink = RunEventSink::json_lines(writer);
        let event = to_run_event(&fixtures::RUN_7, &Event::Agent {
            stage: "code".to_string(),
            visit: 1,
            event: CodingAgentEvent::new(
                "ses_agent".to_string(),
                CodingEvent::AssistantMessage {
                    text:            String::new(),
                    model:           "gpt-5.4".to_string(),
                    usage:           TokenUsage::default(),
                    cost_usd_micros: None,
                    cost_source:     None,
                    tool_call_count: 1,
                    context_window:  None,
                    reasoning:       Some(ReasoningOutput::new(
                        "inspect the sink first",
                        "write the line, then read it back",
                    )),
                },
                std::time::SystemTime::UNIX_EPOCH,
            ),
        });

        sink.write_run_event(&event).await.unwrap();

        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();

        let payload = event_payload_from_redacted_json(line.trim_end(), &fixtures::RUN_7).unwrap();
        assert_eq!(payload.as_value()["event"], "agent.message");
        let message = &payload.as_value()["properties"]["event"]["AssistantMessage"];
        assert_eq!(message["reasoning"]["summary"], "inspect the sink first");
        assert_eq!(
            message["reasoning"]["trace"],
            "write the line, then read it back"
        );
    }

    #[tokio::test]
    async fn run_event_sink_map_applies_transform_before_fanout() {
        let first = Arc::new(AsyncMutex::new(Vec::new()));
        let second = Arc::new(AsyncMutex::new(Vec::new()));
        let first_events = Arc::clone(&first);
        let second_events = Arc::clone(&second);
        let sink = RunEventSink::map(
            |mut event| {
                event.actor = Some(user_principal("alice"));
                event
            },
            RunEventSink::fanout(vec![
                RunEventSink::callback(move |event| {
                    let first_events = Arc::clone(&first_events);
                    async move {
                        first_events.lock().await.push(event);
                        Ok(())
                    }
                }),
                RunEventSink::callback(move |event| {
                    let second_events = Arc::clone(&second_events);
                    async move {
                        second_events.lock().await.push(event);
                        Ok(())
                    }
                }),
            ]),
        );
        let event = to_run_event(&fixtures::RUN_7, &Event::RunPauseRequested { actor: None });

        sink.write_run_event(&event).await.unwrap();

        let first = first.lock().await;
        let second = second.lock().await;
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_eq!(first[0].actor, Some(user_principal("alice")));
        assert_eq!(second[0].actor, Some(user_principal("alice")));
    }

    #[tokio::test]
    async fn run_event_logger_registers_emitter_events_to_json_lines() {
        use tokio::io::{AsyncBufReadExt, BufReader};

        let (writer, reader) = tokio::io::duplex(4096);
        let sink = RunEventSink::json_lines(writer);
        let logger = RunEventLogger::new(sink);
        let emitter = Emitter::new(fixtures::RUN_8);
        logger.register(&emitter);

        emitter.emit(&Event::RunPaused);
        logger.flush().await.unwrap();

        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();

        let payload = event_payload_from_redacted_json(line.trim_end(), &fixtures::RUN_8).unwrap();
        assert_eq!(payload.as_value()["event"], "run.paused");
    }

    #[tokio::test]
    async fn run_event_logger_latches_write_failure_and_preserves_cause_chain() {
        let writes = Arc::new(AtomicUsize::new(0));
        let writes_for_sink = Arc::clone(&writes);
        let sink = RunEventSink::callback(move |_| {
            writes_for_sink.fetch_add(1, Ordering::SeqCst);
            async {
                Err(
                    anyhow::anyhow!("request failed with status 413 Payload Too Large")
                        .context("worker lost canonical run store during append run event"),
                )
            }
        });
        let logger = RunEventLogger::new(sink);
        let emitter = Emitter::new(fixtures::RUN_8);
        logger.register(&emitter);

        emitter.emit(&Event::RunPaused);

        let failure = logger.wait_for_failure().await;
        let rendered = collect_chain(&failure).join(": ");
        assert!(rendered.contains("run.paused"), "{rendered}");
        assert!(
            rendered.contains("worker lost canonical run store"),
            "{rendered}"
        );
        assert!(rendered.contains("413 Payload Too Large"), "{rendered}");

        emitter.emit(&Event::RunUnpaused);
        let flush_failure = logger.flush().await.unwrap_err();
        assert_eq!(collect_chain(&flush_failure), collect_chain(&failure));
        assert_eq!(writes.load(Ordering::SeqCst), 1);
    }

    // --- oversized checkpoint payload offload (fabro-5082) ---

    async fn offload_test_store() -> (RunStoreHandle, fabro_store::RunDatabase) {
        let store = fabro_store::test_support::test_database(
            Arc::new(InMemory::new()),
            "",
            Duration::from_millis(1),
            None,
        );
        let run_store = store.create_run(&fixtures::RUN_7).await.unwrap();
        append_event(&run_store, &fixtures::RUN_7, &Event::RunCreated {
            run_id:              fixtures::RUN_7,
            title:               None,
            settings:            serde_json::to_value(WorkflowSettings::default()).unwrap(),
            graph:               serde_json::to_value(Graph::new("test")).unwrap(),
            workflow_source:     None,
            labels:              std::collections::BTreeMap::new(),
            source_directory:    None,
            workflow_slug:       None,
            workflow_version_id: None,
            target:              None,
            automation:          None,
            provenance:          test_support::test_run_provenance(),
            spec_blob:           None,
            git:                 None,
            fork_source_ref:     None,
            retried_from:        None,
            parent_id:           None,
            web_url:             None,
        })
        .await
        .unwrap();
        let handle = RunStoreHandle::local(run_store.clone());
        (handle, run_store)
    }

    fn checkpoint_props() -> CheckpointCompletedProps {
        CheckpointCompletedProps {
            status: "succeeded".to_string(),
            current_node: "work".to_string(),
            completed_nodes: Vec::new(),
            node_retries: std::collections::BTreeMap::new(),
            context_values: std::collections::BTreeMap::new(),
            node_outcomes: std::collections::BTreeMap::new(),
            next_node_id: None,
            git_commit_sha: None,
            loop_failure_signatures: std::collections::BTreeMap::new(),
            restart_failure_signatures: std::collections::BTreeMap::new(),
            node_visits: std::collections::BTreeMap::new(),
            diff: None,
            diff_summary: None,
            graph_visit: None,
            resumed_from_stage_id: None,
        }
    }

    fn checkpoint_event(props: CheckpointCompletedProps) -> RunEvent {
        RunEvent {
            id:                 "0196b0ad-7d47-7c7f-8f2a-3f2b1a0c9d11".to_string(),
            ts:                 Utc::now(),
            run_id:             fixtures::RUN_7,
            node_id:            Some("work".to_string()),
            node_label:         None,
            stage_id:           None,
            parallel_group_id:  None,
            parallel_branch_id: None,
            session_id:         None,
            parent_session_id:  None,
            tool_call_id:       None,
            actor:              None,
            body:               EventBody::CheckpointCompleted(props),
        }
    }

    async fn stored_checkpoint(run_store: &fabro_store::RunDatabase) -> RunEvent {
        let events = run_store.list_events().await.unwrap();
        events
            .into_iter()
            .map(|envelope| envelope.event)
            .find(|event| matches!(event.body, EventBody::CheckpointCompleted(_)))
            .expect("checkpoint event was stored")
    }

    const OVERSIZED: usize = 4 * 1024 * 1024;

    fn oversized_props(case: &str) -> (CheckpointCompletedProps, Vec<u8>) {
        let mut props = checkpoint_props();
        match case {
            "diff" => {
                let diff = "d".repeat(OVERSIZED);
                props.diff = Some(diff.clone());
                (props, diff.into_bytes())
            }
            "context_value" => {
                let journal = "j".repeat(OVERSIZED);
                props
                    .context_values
                    .insert("journal".to_string(), serde_json::json!(journal.clone()));
                (
                    props,
                    serde_json::to_vec(&serde_json::json!(journal)).unwrap(),
                )
            }
            "outcome_update" => {
                let payload = "p".repeat(OVERSIZED);
                let outcome = StoredOutcome {
                    context_updates: [("payload".to_string(), serde_json::json!(payload.clone()))]
                        .into_iter()
                        .collect(),
                    ..StoredOutcome::default()
                };
                props.node_outcomes.insert("work".to_string(), outcome);
                (
                    props,
                    serde_json::to_vec(&serde_json::json!(payload)).unwrap(),
                )
            }
            "outcome_notes" => {
                let notes = "n".repeat(OVERSIZED);
                let outcome = StoredOutcome {
                    notes: Some(notes.clone()),
                    ..StoredOutcome::default()
                };
                props.node_outcomes.insert("work".to_string(), outcome);
                (props, notes.into_bytes())
            }
            other => panic!("unknown offload case: {other}"),
        }
    }

    fn stored_blob_ref(case: &str, props: &CheckpointCompletedProps) -> String {
        match case {
            "diff" => props.diff.clone().unwrap_or_default(),
            "context_value" => props.context_values["journal"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            "outcome_update" => props.node_outcomes["work"].context_updates["payload"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            "outcome_notes" => props.node_outcomes["work"]
                .notes
                .clone()
                .unwrap_or_default(),
            other => panic!("unknown offload case: {other}"),
        }
    }

    #[tokio::test]
    async fn oversized_checkpoint_payloads_offload_to_blobs_with_inline_refs() {
        for case in ["diff", "context_value", "outcome_update", "outcome_notes"] {
            let (handle, run_store) = offload_test_store().await;
            let sink = RunEventSink::backend(handle.clone());
            let (props, original) = oversized_props(case);
            let event = checkpoint_event(props);
            assert!(
                serde_json::to_vec(&event).unwrap().len() > run_event_body_budget(),
                "{case}: event must start oversized"
            );

            sink.write_run_event(&event).await.unwrap();

            let stored = stored_checkpoint(&run_store).await;
            let serialized = serde_json::to_vec(&stored).unwrap();
            assert!(
                serialized.len() <= run_event_body_budget(),
                "{case}: stored event is {} bytes",
                serialized.len()
            );
            let EventBody::CheckpointCompleted(props) = &stored.body else {
                panic!("{case}: event stays a checkpoint.completed");
            };
            let blob_ref = stored_blob_ref(case, props);
            let blob_hash = parse_blob_ref(&blob_ref)
                .unwrap_or_else(|| panic!("{case}: payload is an inline blob ref: {blob_ref}"));
            let blob = handle
                .read_blob(&blob_hash)
                .await
                .unwrap()
                .unwrap_or_else(|| panic!("{case}: blob was persisted"));
            assert_eq!(blob.as_ref(), original.as_slice(), "{case}: full payload");
        }
    }

    struct FailingBlobBackend {
        inner: RunStoreHandle,
    }

    #[async_trait]
    impl RunStoreBackend for FailingBlobBackend {
        async fn load_state(&self) -> anyhow::Result<fabro_store::RunProjection> {
            self.inner.state().await
        }

        async fn list_events(&self) -> anyhow::Result<Vec<fabro_store::EventEnvelope>> {
            self.inner.list_events().await
        }

        async fn append_run_event(&self, event: &RunEvent) -> anyhow::Result<()> {
            self.inner.append_run_event(event).await
        }

        async fn write_blob(&self, _data: &[u8]) -> anyhow::Result<fabro_types::BlobHash> {
            Err(anyhow::anyhow!("blob endpoint unavailable"))
        }

        async fn read_blob(
            &self,
            blob_hash: &fabro_types::BlobHash,
        ) -> anyhow::Result<Option<bytes::Bytes>> {
            self.inner.read_blob(blob_hash).await
        }

        async fn read_run_log(&self) -> anyhow::Result<Option<Vec<u8>>> {
            self.inner.read_run_log().await
        }
    }

    #[tokio::test]
    async fn blob_write_failure_falls_back_to_truncation_and_append_succeeds() {
        let (handle, run_store) = offload_test_store().await;
        let sink = RunEventSink::backend(RunStoreHandle::new(Arc::new(FailingBlobBackend {
            inner: handle,
        })));
        let (props, _) = oversized_props("diff");

        sink.write_run_event(&checkpoint_event(props))
            .await
            .expect("append succeeds via the truncation fallback");

        let stored = stored_checkpoint(&run_store).await;
        assert!(serde_json::to_vec(&stored).unwrap().len() <= run_event_body_budget());
        let EventBody::CheckpointCompleted(props) = &stored.body else {
            panic!("event stays a checkpoint.completed");
        };
        let diff = props.diff.as_deref().expect("diff is retained");
        assert!(
            diff.contains("[... fabro truncated"),
            "diff was truncated by the fallback: {}",
            &diff[..diff.len().min(200)]
        );
    }

    /// Rejects every event append, simulating a transport failure that
    /// survives offload and fallback.
    struct FlakyAppendBackend {
        inner: RunStoreHandle,
    }

    #[async_trait]
    impl RunStoreBackend for FlakyAppendBackend {
        async fn load_state(&self) -> anyhow::Result<fabro_store::RunProjection> {
            self.inner.state().await
        }

        async fn list_events(&self) -> anyhow::Result<Vec<fabro_store::EventEnvelope>> {
            self.inner.list_events().await
        }

        async fn append_run_event(&self, _event: &RunEvent) -> anyhow::Result<()> {
            Err(anyhow::anyhow!(
                "append rejected: request failed with status 503"
            ))
        }

        async fn write_blob(&self, data: &[u8]) -> anyhow::Result<fabro_types::BlobHash> {
            self.inner.write_blob(data).await
        }

        async fn read_blob(
            &self,
            blob_hash: &fabro_types::BlobHash,
        ) -> anyhow::Result<Option<bytes::Bytes>> {
            self.inner.read_blob(blob_hash).await
        }

        async fn read_run_log(&self) -> anyhow::Result<Option<Vec<u8>>> {
            self.inner.read_run_log().await
        }
    }

    #[tokio::test]
    async fn checkpoint_append_failure_after_offload_stays_resumable() {
        let (handle, run_store) = offload_test_store().await;

        // A first, small checkpoint persists through the plain store and
        // becomes the run's resume point.
        handle
            .append_run_event(&checkpoint_event(checkpoint_props()))
            .await
            .unwrap();

        // A later oversized checkpoint whose append still fails after
        // offload surfaces as an error instead of destroying the run's
        // resume state.
        let backend = FlakyAppendBackend {
            inner: handle.clone(),
        };
        let sink = RunEventSink::backend(RunStoreHandle::new(Arc::new(backend)));
        let (props, _) = oversized_props("diff");
        let error = sink
            .write_run_event(&checkpoint_event(props))
            .await
            .expect_err("append failure surfaces");

        assert!(error.to_string().contains("append rejected"), "{}", error);

        // The run stays resumable: the earlier checkpoint remains the
        // resume point a retry or `resume` picks up.
        let state = run_store.state().await.unwrap();
        assert!(!state.checkpoints.is_empty());
        assert!(
            ResumeState::from_projection(&state).is_some(),
            "the persisted checkpoint must remain a valid resume state"
        );
    }
}
