use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use ::fabro_types::{RunEvent, RunId};
use anyhow::Result;
use fabro_redact::SecretRedactor;
use fabro_store::RunDatabase;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::{Mutex as AsyncMutex, mpsc, oneshot};

use super::emitter::Emitter;
use super::redaction::{RedactedRunEvent, build_redacted_event_payload};
use super::{Event, to_run_event};
use crate::runtime_store::RunStoreHandle;

pub async fn append_event(run_store: &RunDatabase, run_id: &RunId, event: &Event) -> Result<()> {
    let stored = to_run_event(run_id, event);
    // Direct RunDatabase appends are server/test lifecycle paths with no
    // per-run declared-secret registry. Worker run output goes through
    // RunEventSink/RunEventLogger with the run redactor.
    let payload = build_redacted_event_payload(&stored, run_id, &SecretRedactor::default())?;
    run_store
        .append_event(&payload)
        .await
        .map(|_| ())
        .map_err(anyhow::Error::from)
}

pub async fn append_event_to_sink(
    sink: &RunEventSink,
    run_id: &RunId,
    event: &Event,
    redactor: &SecretRedactor,
) -> Result<()> {
    let stored = to_run_event(run_id, event);
    sink.write_run_event(&stored, redactor).await
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

    pub async fn write_run_event(&self, event: &RunEvent, redactor: &SecretRedactor) -> Result<()> {
        let mut pending = vec![(self, PendingEvent::Raw(event.clone()))];
        while let Some((sink, event)) = pending.pop() {
            match sink {
                Self::Store(run_store) => {
                    let redacted = event.into_redacted(redactor)?;
                    run_store.append_run_event(&redacted).await?;
                }
                Self::JsonLines(writer) => {
                    let line = event.into_redacted(redactor)?.json_line()?;
                    let mut writer = writer.lock().await;
                    writer.write_all(line.as_bytes()).await?;
                    writer.write_all(b"\n").await?;
                    writer.flush().await?;
                }
                Self::Callback(callback) => {
                    callback(event.into_redacted(redactor)?.event().clone()).await?;
                }
                Self::Map { transform, inner } => {
                    // A transform can add new fields, so its output re-enters
                    // the pipeline as a raw event and is redacted downstream.
                    pending.push((
                        inner.as_ref(),
                        PendingEvent::Raw(transform(event.into_run_event())),
                    ));
                }
                Self::Composite(sinks) => {
                    // Redact once and share the result with every branch.
                    let redacted = event.into_redacted(redactor)?;
                    for sink in sinks.iter().rev() {
                        pending.push((sink, PendingEvent::Redacted(Arc::clone(&redacted))));
                    }
                }
            }
        }
        Ok(())
    }
}

/// Worklist entry for [`RunEventSink::write_run_event`]: events are redacted at
/// most once per fan-out and the shared result reused by every consumer.
#[allow(
    clippy::large_enum_variant,
    reason = "Worklist entries stay inline to avoid boxing hot-path payloads."
)]
enum PendingEvent {
    Raw(RunEvent),
    Redacted(Arc<RedactedRunEvent>),
}

impl PendingEvent {
    fn into_redacted(self, redactor: &SecretRedactor) -> Result<Arc<RedactedRunEvent>> {
        match self {
            Self::Raw(event) => Ok(Arc::new(RedactedRunEvent::new(&event, redactor)?)),
            Self::Redacted(redacted) => Ok(redacted),
        }
    }

    fn into_run_event(self) -> RunEvent {
        match self {
            Self::Raw(event) => event,
            Self::Redacted(redacted) => redacted.event().clone(),
        }
    }
}

#[allow(
    clippy::large_enum_variant,
    reason = "Logger queue messages stay inline to avoid boxing hot-path payloads."
)]
enum RunEventCommand {
    Event(RunEvent),
    Flush(oneshot::Sender<()>),
}

#[derive(Clone)]
pub struct RunEventLogger {
    tx: mpsc::UnboundedSender<RunEventCommand>,
}

impl RunEventLogger {
    #[must_use]
    pub fn new(sink: RunEventSink, redactor: SecretRedactor) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            while let Some(command) = rx.recv().await {
                match command {
                    RunEventCommand::Event(event) => {
                        if let Err(err) = sink.write_run_event(&event, &redactor).await {
                            tracing::warn!(error = %err, "Failed to write run event");
                        }
                    }
                    RunEventCommand::Flush(tx) => {
                        let _ = tx.send(());
                    }
                }
            }
        });

        Self { tx }
    }

    pub fn register(&self, emitter: &Emitter) {
        let tx = self.tx.clone();
        emitter.on_event(move |event| {
            if tx.send(RunEventCommand::Event(event.clone())).is_err() {
                tracing::warn!("Run event logger channel closed while forwarding event");
            }
        });
    }

    pub async fn flush(&self) {
        let (tx, rx) = oneshot::channel();
        if self.tx.send(RunEventCommand::Flush(tx)).is_err() {
            tracing::warn!("Run event logger channel closed before flush");
            return;
        }
        if rx.await.is_err() {
            tracing::warn!("Run event logger flush dropped before completion");
        }
    }
}

#[derive(Clone)]
pub struct StoreProgressLogger {
    inner: RunEventLogger,
}

impl StoreProgressLogger {
    #[must_use]
    pub fn new(run_store: impl Into<RunStoreHandle>, redactor: SecretRedactor) -> Self {
        Self {
            inner: RunEventLogger::new(RunEventSink::backend(run_store.into()), redactor),
        }
    }

    pub fn register(&self, emitter: &Emitter) {
        self.inner.register(emitter);
    }

    pub async fn flush(&self) {
        self.inner.flush().await;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ::fabro_types::{Graph, RunNoticeLevel, WorkflowSettings, fixtures};
    use fabro_types::test_support;
    use tokio::sync::Mutex as AsyncMutex;

    use super::*;
    use crate::event::test_support::user_principal;
    use crate::event::{
        Emitter, Event, append_event, build_redacted_event_payload,
        event_payload_from_redacted_json, to_run_event,
    };

    #[tokio::test]
    async fn append_event_writes_store_event_shape() {
        let store = fabro_store::Database::new(
            std::sync::Arc::new(object_store::memory::InMemory::new()),
            "",
            std::time::Duration::from_millis(1),
            None,
        );
        let run_store = store.create_run(&fixtures::RUN_7).await.unwrap();
        append_event(&run_store, &fixtures::RUN_7, &Event::RunCreated {
            run_id:           fixtures::RUN_7,
            title:            None,
            settings:         serde_json::to_value(WorkflowSettings::default()).unwrap(),
            graph:            serde_json::to_value(Graph::new("test")).unwrap(),
            workflow_source:  None,
            workflow_config:  None,
            labels:           std::collections::BTreeMap::new(),
            run_dir:          "/tmp/test".to_string(),
            source_directory: None,
            workflow_slug:    None,
            automation:       None,
            db_prefix:        None,
            provenance:       test_support::test_run_provenance(),
            manifest_blob:    None,
            git:              None,
            fork_source_ref:  None,
            retried_from:     None,
            parent_id:        None,
            web_url:          None,
        })
        .await
        .unwrap();
        let stored = to_run_event(&fixtures::RUN_7, &Event::RunNotice {
            level:            RunNoticeLevel::Warn,
            code:             "example".to_string(),
            message:          "notice".to_string(),
            exec_output_tail: None,
        });
        let payload =
            build_redacted_event_payload(&stored, &fixtures::RUN_7, &SecretRedactor::default())
                .unwrap();
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
        let redactor = SecretRedactor::default();

        sink.write_run_event(&event, &redactor).await.unwrap();

        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();

        let payload = event_payload_from_redacted_json(line.trim_end(), &fixtures::RUN_7).unwrap();
        assert_eq!(payload.as_value()["event"], "run.pause.requested");
        assert_eq!(payload.as_value()["properties"]["action"], "pause");
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
        let redactor = SecretRedactor::default();

        sink.write_run_event(&event, &redactor).await.unwrap();

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
        let logger = RunEventLogger::new(sink, SecretRedactor::default());
        let emitter = Emitter::new(fixtures::RUN_8);
        logger.register(&emitter);

        emitter.emit(&Event::RunPaused);
        logger.flush().await;

        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();

        let payload = event_payload_from_redacted_json(line.trim_end(), &fixtures::RUN_8).unwrap();
        assert_eq!(payload.as_value()["event"], "run.paused");
    }

    #[tokio::test]
    async fn run_event_sink_callback_receives_redacted_event() {
        let received = Arc::new(AsyncMutex::new(Vec::new()));
        let received_events = Arc::clone(&received);
        let sink = RunEventSink::callback(move |event| {
            let received_events = Arc::clone(&received_events);
            async move {
                received_events.lock().await.push(event);
                Ok(())
            }
        });
        let event = to_run_event(&fixtures::RUN_7, &Event::RunNotice {
            level:            RunNoticeLevel::Warn,
            code:             "example".to_string(),
            message:          "deploy to staging".to_string(),
            exec_output_tail: None,
        });
        let redactor = SecretRedactor::default();
        redactor.register("staging");

        sink.write_run_event(&event, &redactor).await.unwrap();

        let events = received.lock().await;
        let text = serde_json::to_string(&events[0].to_value().unwrap()).unwrap();
        assert!(!text.contains("staging"));
        assert!(text.contains("REDACTED"));
    }
}
