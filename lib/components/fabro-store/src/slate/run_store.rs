use std::sync::{Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard};
use std::time::Duration;

use bytes::Bytes;
use fabro_types::{BlobHash, RunEvent, RunId, SessionId};
use futures::Stream;
use tokio::sync::{Mutex as AsyncMutex, broadcast, mpsc};
use tokio::time::sleep;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::warn;

use crate::run_state::{EventProjectionCache, ProjectedRun, RunProjectionReducer};
use crate::{
    BlobStore, Error, EventEnvelope, EventPayload, Result, RunProjection, RunSummaryStore, StageId,
    run_summary_store,
};

/// Broadcast capacity for live event subscribers; a lagging subscriber refills
/// from SQLite.
const EVENT_BROADCAST_CAPACITY: usize = 1024;

/// Bounded retry budget for SQLite BUSY (code 5) on the run-event write path
/// (fabro-3ef7). The connection-level `busy_timeout` (5s in production via
/// `fabro_db::Database::connect`) absorbs short contention; this loop covers
/// the window past it where a concurrent writer still holds the database
/// write lock. 4 retries at 25ms base, doubling, add ~175ms of bounded
/// backoff on top of the timeout instead of surfacing "database is locked"
/// as a fatal error that kills the writing run.
const BUSY_RETRY_ATTEMPTS: usize = 4;
const BUSY_RETRY_BASE_DELAY: Duration = Duration::from_millis(25);

/// Bounded retry budget for head-heal attempts when the in-memory projection
/// cache is behind the committed SQLite head (interrupted apply, or a writer
/// in another process sharing the database file). Each heal reloads the
/// authoritative projection, so the loop converges as soon as the reloaded
/// head stops advancing; the bound only guards pathological ping-pong.
const HEAD_HEAL_RETRY_LIMIT: usize = 16;

#[derive(Clone)]
pub struct RunDatabase {
    inner:     Arc<RunDatabaseInner>,
    read_only: bool,
}

impl std::fmt::Debug for RunDatabase {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RunDatabase")
            .field("run_id", &self.inner.run_id)
            .field("read_only", &self.read_only)
            .finish_non_exhaustive()
    }
}

pub(crate) struct RunDatabaseInner {
    pub(crate) run_id:     RunId,
    blob_store:            Arc<BlobStore>,
    pub(crate) state_lock: AsyncMutex<()>,
    projection_cache:      StdMutex<EventProjectionCache>,
    run_summary_store:     Arc<RunSummaryStore>,
    event_tx:              broadcast::Sender<EventEnvelope>,
}

impl RunDatabaseInner {
    fn lock_projection_cache(&self) -> StdMutexGuard<'_, EventProjectionCache> {
        self.projection_cache.lock().expect(
            "event projection cache mutex is never poisoned: no code panics while holding this lock",
        )
    }
}

impl RunDatabase {
    pub(crate) async fn build(
        run_id: RunId,
        read_only: bool,
        blob_store: Arc<BlobStore>,
        run_summary_store: Arc<RunSummaryStore>,
    ) -> Result<Self> {
        let projected = run_summary_store.load_projection(&run_id).await?;
        Ok(Self::from_event_projection_cache(
            run_id,
            read_only,
            blob_store,
            run_summary_store,
            projected.into(),
        ))
    }

    pub(crate) fn build_empty(
        run_id: RunId,
        blob_store: Arc<BlobStore>,
        run_summary_store: Arc<RunSummaryStore>,
    ) -> Self {
        Self::from_event_projection_cache(
            run_id,
            false,
            blob_store,
            run_summary_store,
            EventProjectionCache::default(),
        )
    }

    fn from_event_projection_cache(
        run_id: RunId,
        read_only: bool,
        blob_store: Arc<BlobStore>,
        run_summary_store: Arc<RunSummaryStore>,
        projection_cache: EventProjectionCache,
    ) -> Self {
        let (event_tx, _) = broadcast::channel(EVENT_BROADCAST_CAPACITY);
        Self {
            inner: Arc::new(RunDatabaseInner {
                run_id,
                blob_store,
                state_lock: AsyncMutex::new(()),
                projection_cache: StdMutex::new(projection_cache),
                run_summary_store,
                event_tx,
            }),
            read_only,
        }
    }

    pub(crate) fn from_inner(inner: Arc<RunDatabaseInner>) -> Self {
        Self {
            inner,
            read_only: false,
        }
    }

    pub(crate) fn read_only_clone(&self) -> Self {
        Self {
            inner:     Arc::clone(&self.inner),
            read_only: true,
        }
    }

    pub(crate) fn inner_arc(&self) -> Arc<RunDatabaseInner> {
        Arc::clone(&self.inner)
    }

    pub(crate) fn run_id(&self) -> RunId {
        self.inner.run_id
    }

    pub fn subscribe(&self) -> broadcast::Receiver<EventEnvelope> {
        self.inner.event_tx.subscribe()
    }

    pub(super) async fn projection_snapshot(&self) -> Result<Arc<RunProjection>> {
        let _state_guard = self.inner.state_lock.lock().await;
        self.projection_snapshot_locked()
    }

    fn projection_snapshot_locked(&self) -> Result<Arc<RunProjection>> {
        self.inner
            .lock_projection_cache()
            .state
            .clone()
            .ok_or_else(|| {
                Error::InvalidEvent(format!(
                    "run {} has no run.created event",
                    self.inner.run_id
                ))
            })
    }

    pub(crate) fn install_in_memory_state(&self, projected: ProjectedRun) {
        *self.inner.lock_projection_cache() = projected.into();
    }

    pub(crate) fn publish(&self, event: &EventEnvelope) {
        let _ = self.inner.event_tx.send(event.clone());
    }

    pub(crate) async fn commit_first_event(
        &self,
        payload: &EventPayload,
    ) -> Result<(EventEnvelope, ProjectedRun)> {
        payload.validate(&self.inner.run_id)?;
        let event = RunEvent::try_from(payload)?;
        let _state_guard = self.inner.state_lock.lock().await;
        if self.inner.lock_projection_cache().last_seq != 0 {
            return Err(Error::RunAlreadyExists(self.inner.run_id.to_string()));
        }
        Box::pin(self.commit_event_locked(payload, event)).await
    }
}

impl RunDatabase {
    /// Appends an event after validating it against the current run projection.
    ///
    /// Every returned error means the event/current-row transaction did not
    /// commit and is safe to retry. Memory and broadcasts advance only after
    /// the SQLite commit succeeds.
    pub async fn append_event(&self, payload: &EventPayload) -> Result<u32> {
        Ok(Box::pin(self.append_event_envelope(payload)).await?.seq)
    }

    /// Atomically appends `payload` when `predicate` matches the latest run
    /// projection.
    pub async fn append_event_if(
        &self,
        payload: &EventPayload,
        predicate: impl FnOnce(&RunProjection) -> bool,
    ) -> Result<Option<u32>> {
        if self.read_only {
            return Err(Error::ReadOnly);
        }
        payload.validate(&self.inner.run_id)?;
        let event = RunEvent::try_from(payload)?;
        let _state_guard = self.inner.state_lock.lock().await;
        let projection = self.projection_snapshot_locked()?;
        if !predicate(&projection) {
            return Ok(None);
        }
        Ok(Some(
            Box::pin(self.append_event_envelope_locked(payload, event))
                .await?
                .seq,
        ))
    }

    /// Appends and returns the stored event envelope after pre-write reduction.
    pub async fn append_event_envelope(&self, payload: &EventPayload) -> Result<EventEnvelope> {
        if self.read_only {
            return Err(Error::ReadOnly);
        }
        payload.validate(&self.inner.run_id)?;
        let event = RunEvent::try_from(payload)?;
        let _state_guard = self.inner.state_lock.lock().await;
        Box::pin(self.append_event_envelope_locked(payload, event)).await
    }

    async fn append_event_envelope_locked(
        &self,
        payload: &EventPayload,
        event: RunEvent,
    ) -> Result<EventEnvelope> {
        let (envelope, projected) = Box::pin(self.commit_event_locked(payload, event)).await?;
        // Keep post-commit propagation await-free: cancellation after SQLite
        // commits must not leave in-memory state stale or omit the broadcast.
        self.install_in_memory_state(projected);
        self.publish(&envelope);
        Ok(envelope)
    }

    async fn commit_event_locked(
        &self,
        payload: &EventPayload,
        event: RunEvent,
    ) -> Result<(EventEnvelope, ProjectedRun)> {
        let mut head_heals = 0usize;
        loop {
            let (expected_last_seq, mut next_state) = {
                let cache = self.inner.lock_projection_cache();
                (cache.last_seq, cache.state.clone())
            };
            let seq = run_summary_store::next_event_seq_after(expected_last_seq)?;
            let prospective = EventEnvelope {
                seq,
                event: event.clone(),
            };
            apply_cached_projection_event(&mut next_state, &prospective).map_err(event_rejected)?;
            let next_projection =
                next_state.expect("applying a valid event should always produce a projection");
            let projected = ProjectedRun::new(self.inner.run_id, next_projection, seq);

            let envelope = match self
                .commit_event_transaction(expected_last_seq, &projected, payload)
                .await
            {
                Ok(envelope) => envelope,
                Err(error) => {
                    if head_heals < HEAD_HEAL_RETRY_LIMIT
                        && self
                            .heal_stale_projection_head(expected_last_seq, &error)
                            .await?
                    {
                        head_heals += 1;
                        continue;
                    }
                    return Err(error);
                }
            };
            return Ok((envelope, projected));
        }
    }

    /// Commits one event + summary-row transaction, retrying bounded
    /// SQLite BUSY (code 5) errors with exponential backoff (fabro-3ef7).
    /// A BUSY transaction never commits, so retrying the whole
    /// begin/insert/commit sequence is safe.
    async fn commit_event_transaction(
        &self,
        expected_last_seq: u32,
        projected: &ProjectedRun,
        payload: &EventPayload,
    ) -> Result<EventEnvelope> {
        let mut attempt = 0usize;
        loop {
            let result = async {
                let mut transaction = self.inner.run_summary_store.begin().await?;
                let envelope = if expected_last_seq == 0 {
                    RunSummaryStore::insert_first_event_on_connection(
                        &mut transaction,
                        projected,
                        payload,
                    )
                    .await?
                } else {
                    RunSummaryStore::append_event_on_connection(
                        &mut transaction,
                        expected_last_seq,
                        projected,
                        payload,
                    )
                    .await?
                };
                transaction.commit().await?;
                Ok(envelope)
            }
            .await;

            match result {
                Ok(envelope) => return Ok(envelope),
                Err(error) if attempt < BUSY_RETRY_ATTEMPTS && is_sqlite_busy(&error) => {
                    let delay = BUSY_RETRY_BASE_DELAY * (1u32 << attempt);
                    warn!(
                        run_id = %self.inner.run_id,
                        attempt = attempt + 1,
                        delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
                        "SQLite busy on run-event commit; retrying with backoff"
                    );
                    sleep(delay).await;
                    attempt += 1;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Deterministic self-heal for an interrupted apply (fabro-3ef7): when the
    /// optimistic head guard reports a mismatch, the in-memory projection
    /// cache is behind the committed SQLite head — either this writer's task
    /// died between the SQL commit and the cache install in an earlier
    /// attempt, or another process appended to the same database file.
    /// Reloads the authoritative projection from SQLite and installs it into
    /// the cache so the retried append validates against the true head.
    ///
    /// Returns `Ok(true)` when the cache was healed and the caller should
    /// retry, `Ok(false)` when the mismatch is not healable (the SQL head did
    /// not advance, or the error is not a head mismatch) and the original
    /// error must surface.
    async fn heal_stale_projection_head(
        &self,
        expected_last_seq: u32,
        error: &Error,
    ) -> Result<bool> {
        if expected_last_seq == 0 || !matches!(error, Error::RunHeadMismatch { .. }) {
            return Ok(false);
        }
        let healed = self
            .inner
            .run_summary_store
            .load_projection(&self.inner.run_id)
            .await?;
        if healed.last_seq == expected_last_seq {
            return Ok(false);
        }
        self.install_in_memory_state(healed);
        Ok(true)
    }

    pub async fn list_events(&self) -> Result<Vec<EventEnvelope>> {
        self.inner
            .run_summary_store
            .list_events_for_run(&self.inner.run_id)
            .await
    }

    pub async fn last_event_seq(&self) -> Result<Option<u32>> {
        self.inner.run_summary_store.head(&self.inner.run_id).await
    }

    /// Returns up to `limit + 1` events starting at `start_seq`.
    pub async fn list_events_from_with_limit(
        &self,
        start_seq: u32,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>> {
        self.inner
            .run_summary_store
            .list_events_from_with_limit(&self.inner.run_id, start_seq, limit)
            .await
    }

    /// Returns up to `limit + 1` events before `before_seq`, newest first.
    pub async fn list_events_before_with_limit(
        &self,
        before_seq: Option<u32>,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>> {
        self.inner
            .run_summary_store
            .list_events_before_with_limit(&self.inner.run_id, before_seq, limit)
            .await
    }

    pub async fn get_event(&self, seq: u32) -> Result<Option<EventEnvelope>> {
        self.inner
            .run_summary_store
            .get_event_for_run(&self.inner.run_id, seq)
            .await
    }

    pub async fn list_events_for_stage_from_with_limit(
        &self,
        stage_id: &StageId,
        start_seq: u32,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>> {
        self.inner
            .run_summary_store
            .list_events_for_stage_from_with_limit(&self.inner.run_id, stage_id, start_seq, limit)
            .await
    }

    pub async fn list_events_for_session_from_with_limit(
        &self,
        session_id: SessionId,
        start_seq: u32,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>> {
        self.inner
            .run_summary_store
            .list_events_for_session_from_with_limit(
                &self.inner.run_id,
                &session_id,
                start_seq,
                limit,
            )
            .await
    }

    pub fn watch_events_from(
        &self,
        seq: u32,
    ) -> Result<std::pin::Pin<Box<dyn Stream<Item = Result<EventEnvelope>> + Send>>> {
        let inner = Arc::clone(&self.inner);
        // Subscribe before the durable catch-up query to close the read/subscribe race.
        let mut broadcasts = inner.event_tx.subscribe();
        let (sender, receiver) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut next_seq = seq;
            if !refill_from_sql(&inner, &sender, &mut next_seq).await {
                return;
            }
            loop {
                match broadcasts.recv().await {
                    Ok(event) if event.seq < next_seq => {}
                    Ok(event) if event.seq == next_seq => {
                        next_seq = event.seq.saturating_add(1);
                        if sender.send(Ok(event)).is_err() {
                            return;
                        }
                    }
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {
                        if !refill_from_sql(&inner, &sender, &mut next_seq).await {
                            return;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        });
        Ok(Box::pin(UnboundedReceiverStream::new(receiver)))
    }

    pub async fn write_blob(&self, data: &[u8]) -> Result<BlobHash> {
        if self.read_only {
            return Err(Error::ReadOnly);
        }
        self.inner.blob_store.write(data).await
    }

    pub async fn read_blob(&self, blob_hash: &BlobHash) -> Result<Option<Bytes>> {
        self.inner.blob_store.read(blob_hash).await
    }

    pub async fn state(&self) -> Result<RunProjection> {
        Ok(Arc::unwrap_or_clone(self.projection_snapshot().await?))
    }
}

async fn refill_from_sql(
    inner: &RunDatabaseInner,
    sender: &mpsc::UnboundedSender<Result<EventEnvelope>>,
    next_seq: &mut u32,
) -> bool {
    let events = match inner
        .run_summary_store
        .list_events_from_with_limit(&inner.run_id, *next_seq, usize::MAX)
        .await
    {
        Ok(events) => events,
        Err(error) => {
            let _ = sender.send(Err(error));
            return false;
        }
    };
    for event in events {
        *next_seq = event.seq.saturating_add(1);
        if sender.send(Ok(event)).is_err() {
            return false;
        }
    }
    true
}

fn event_rejected(error: Error) -> Error {
    Error::EventRejected {
        source: Box::new(error),
    }
}

/// SQLite primary result code 5 (`SQLITE_BUSY`, "database is locked") — the
/// concurrent-writer contention observed under a child-run event flood
/// (fabro-3ef7). sqlx surfaces the EXTENDED result code (e.g. 517 for
/// `SQLITE_BUSY_SNAPSHOT`), so the primary code is recovered by masking off
/// the extended bits; the message is a defensive fallback for drivers that
/// surface only the text.
fn is_sqlite_busy(error: &Error) -> bool {
    let Error::Sqlite(sqlx::Error::Database(database)) = error else {
        return false;
    };
    is_busy_code_and_message(database.code().as_deref(), database.message())
}

fn is_busy_code_and_message(code: Option<&str>, message: &str) -> bool {
    let primary_busy = code
        .and_then(|code| code.parse::<i64>().ok())
        .is_some_and(|code| code & 0xff == 5);
    primary_busy || message.contains("database is locked")
}

fn apply_cached_projection_event(
    state: &mut Option<Arc<RunProjection>>,
    event: &EventEnvelope,
) -> Result<()> {
    if let Some(projection) = state {
        Arc::make_mut(projection).apply_event(event)?;
    } else {
        *state = Some(Arc::new(RunProjection::apply_events(
            std::slice::from_ref(event),
        )?));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use fabro_types::{Graph, RunId, WorkflowSettings, test_support};
    use futures::StreamExt as _;
    use object_store::memory::InMemory;
    use serde_json::json;
    use tokio::task;

    use crate::run_state::ProjectedRun;
    use crate::{EventPayload, test_support as store_test_support};

    fn run_created_payload(run_id: &RunId) -> EventPayload {
        EventPayload::new(
            json!({
                "id": "evt-created",
                "ts": "2026-04-09T11:59:00Z",
                "run_id": run_id.to_string(),
                "event": "run.created",
                "properties": {
                    "settings": WorkflowSettings::default(),
                    "graph": Graph::new("test"),
                    "provenance": test_support::test_run_provenance(),
                },
            }),
            run_id,
        )
        .unwrap()
    }

    fn stage_payload(run_id: &RunId, index: u32) -> EventPayload {
        EventPayload::new(
            json!({
                "id": format!("evt-{index}"),
                "ts": "2026-04-09T12:00:00Z",
                "run_id": run_id.to_string(),
                "event": "stage.prompt",
                "node_id": "build",
                "stage_id": "build@1",
                "properties": { "visit": 1, "text": format!("prompt {index}") },
            }),
            run_id,
        )
        .unwrap()
    }

    fn store() -> crate::Database {
        store_test_support::test_database(
            Arc::new(InMemory::new()),
            "run-store-sql-tests",
            Duration::from_millis(1),
            None,
        )
    }

    #[tokio::test]
    async fn first_and_later_events_commit_to_sql_before_publication() {
        let store = store();
        let run_id: RunId = "01JT56VE4Z5NZ814GZN2JZD65A".parse().unwrap();

        let run = store
            .create_run_with_first_event(&run_id, &run_created_payload(&run_id))
            .await
            .unwrap();
        assert_eq!(
            run.append_event(&stage_payload(&run_id, 2)).await.unwrap(),
            2
        );
        assert_eq!(
            run.list_events()
                .await
                .unwrap()
                .iter()
                .map(|event| event.seq)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[tokio::test]
    async fn watcher_catches_up_from_sql_without_duplicates() {
        let store = store();
        let run_id: RunId = "01JT56VE4Z5NZ814GZN2JZD65B".parse().unwrap();
        let run = store
            .create_run_with_first_event(&run_id, &run_created_payload(&run_id))
            .await
            .unwrap();
        run.append_event(&stage_payload(&run_id, 2)).await.unwrap();

        let mut stream = run.watch_events_from(1).unwrap();
        assert_eq!(stream.next().await.unwrap().unwrap().seq, 1);
        assert_eq!(stream.next().await.unwrap().unwrap().seq, 2);
        run.append_event(&stage_payload(&run_id, 3)).await.unwrap();
        assert_eq!(stream.next().await.unwrap().unwrap().seq, 3);
    }

    #[tokio::test]
    async fn simultaneous_appends_allocate_one_contiguous_sql_sequence() {
        let store = store();
        let run_id: RunId = "01JT56VE4Z5NZ814GZN2JZD65C".parse().unwrap();
        let run = store
            .create_run_with_first_event(&run_id, &run_created_payload(&run_id))
            .await
            .unwrap();

        let mut tasks = Vec::new();
        for index in 2..=33 {
            let writer = run.clone();
            tasks.push(tokio::spawn(async move {
                writer.append_event(&stage_payload(&run_id, index)).await
            }));
        }
        let mut sequences = Vec::new();
        for task in tasks {
            sequences.push(task.await.unwrap().unwrap());
        }
        sequences.sort_unstable();
        assert_eq!(sequences, (2..=33).collect::<Vec<_>>());
        assert_eq!(run.last_event_seq().await.unwrap(), Some(33));
        assert_eq!(run.list_events().await.unwrap().len(), 33);
    }

    #[tokio::test]
    async fn simultaneous_creation_has_exactly_one_winner() {
        let store = store();
        let run_id: RunId = "01JT56VE4Z5NZ814GZN2JZD65D".parse().unwrap();
        let left_payload = run_created_payload(&run_id);
        let right_payload = run_created_payload(&run_id);
        let left = store.create_run_with_first_event(&run_id, &left_payload);
        let right = store.create_run_with_first_event(&run_id, &right_payload);

        let (left, right) = tokio::join!(left, right);
        assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
        let error = left.err().or_else(|| right.err()).unwrap();
        assert!(matches!(error, crate::Error::RunAlreadyExists(_)));
        assert_eq!(
            store
                .open_run(&run_id)
                .await
                .unwrap()
                .list_events()
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn readers_observe_only_complete_committed_prefixes_during_appends() {
        let store = store();
        let run_id: RunId = "01JT56VE4Z5NZ814GZN2JZD65E".parse().unwrap();
        let run = store
            .create_run_with_first_event(&run_id, &run_created_payload(&run_id))
            .await
            .unwrap();
        let writer = run.clone();
        let write_task = tokio::spawn(async move {
            for index in 2..=25 {
                writer.append_event(&stage_payload(&run_id, index)).await?;
                task::yield_now().await;
            }
            crate::Result::Ok(())
        });

        while !write_task.is_finished() {
            let events = run.list_events().await.unwrap();
            assert!(
                events
                    .iter()
                    .enumerate()
                    .all(|(index, event)| event.seq as usize == index + 1)
            );
            task::yield_now().await;
        }
        write_task.await.unwrap().unwrap();
        assert_eq!(run.list_events().await.unwrap().len(), 25);
    }

    /// fabro-3ef7: an interrupted apply (SQL commit landed, the in-memory
    /// projection install never ran) must not park the run forever — the next
    /// append detects the stale head, heals from SQLite, and converges.
    #[tokio::test]
    async fn interrupted_apply_projection_converges_on_next_append() {
        let directory = tempfile::tempdir().expect("tempdir");
        let object_store: Arc<dyn object_store::ObjectStore> = Arc::new(InMemory::new());
        let store = store_test_support::test_database_at(
            Arc::clone(&object_store),
            "run-store-heal-test",
            Duration::from_millis(1),
            None,
            directory.path(),
        );
        let run_id: RunId = "01JT56VE4Z5NZ814GZN2JZD65F".parse().unwrap();
        let run = store
            .create_run_with_first_event(&run_id, &run_created_payload(&run_id))
            .await
            .unwrap();

        // Capture the post-create (seq 1) projection, then commit event 2 and
        // roll the in-memory cache back to seq 1: the writer died mid-merge
        // after the durable commit but before the cache install.
        let stale_projection = Arc::new(run.state().await.unwrap());
        run.append_event(&stage_payload(&run_id, 2)).await.unwrap();
        run.install_in_memory_state(ProjectedRun::new(run_id, stale_projection, 1));

        // The next append heals from the committed head instead of failing
        // with RunHeadMismatch forever.
        assert_eq!(
            run.append_event(&stage_payload(&run_id, 3)).await.unwrap(),
            3
        );
        let sequences: Vec<u32> = run
            .list_events()
            .await
            .unwrap()
            .iter()
            .map(|event| event.seq)
            .collect();
        assert_eq!(sequences, vec![1, 2, 3]);

        // Restart path: a fresh handle over the same SQLite file rebuilds the
        // projection from the committed head and keeps appending.
        let reopened = store_test_support::test_database_at(
            object_store,
            "run-store-heal-test-reopened",
            Duration::from_millis(1),
            None,
            directory.path(),
        );
        let reopened_run = reopened.open_run(&run_id).await.unwrap();
        assert_eq!(
            reopened_run
                .append_event(&stage_payload(&run_id, 4))
                .await
                .unwrap(),
            4
        );
        let sequences: Vec<u32> = reopened_run
            .list_events()
            .await
            .unwrap()
            .iter()
            .map(|event| event.seq)
            .collect();
        assert_eq!(sequences, vec![1, 2, 3, 4]);
    }

    /// fabro-3ef7: two independent writer handles over one SQLite authority
    /// (modeling a merge child and the server) appending in parallel must not
    /// surface database-locked or head-mismatch fatality — every event lands
    /// and the durable history stays contiguous.
    #[tokio::test]
    async fn parallel_writers_on_one_sqlite_authority_converge() {
        let directory = tempfile::tempdir().expect("tempdir");
        let object_store: Arc<dyn object_store::ObjectStore> = Arc::new(InMemory::new());
        let left = store_test_support::test_database_at(
            Arc::clone(&object_store),
            "run-store-parallel-left",
            Duration::from_millis(1),
            None,
            directory.path(),
        );
        let right = store_test_support::test_database_at(
            object_store,
            "run-store-parallel-right",
            Duration::from_millis(1),
            None,
            directory.path(),
        );
        let run_id: RunId = "01JT56VE4Z5NZ814GZN2JZD65G".parse().unwrap();
        let left_run = left
            .create_run_with_first_event(&run_id, &run_created_payload(&run_id))
            .await
            .unwrap();
        let right_run = right.open_run(&run_id).await.unwrap();

        let mut tasks = Vec::new();
        for index in 2..=9u32 {
            let writer = if index % 2 == 0 {
                left_run.clone()
            } else {
                right_run.clone()
            };
            tasks.push(tokio::spawn(async move {
                writer.append_event(&stage_payload(&run_id, index)).await
            }));
        }
        let mut sequences = Vec::new();
        for task in tasks {
            sequences.push(task.await.unwrap().unwrap());
        }
        sequences.sort_unstable();
        assert_eq!(sequences, (2..=9).collect::<Vec<_>>());
        let durable: Vec<u32> = left_run
            .list_events()
            .await
            .unwrap()
            .iter()
            .map(|event| event.seq)
            .collect();
        assert_eq!(durable, (1..=9).collect::<Vec<_>>());
    }

    /// fabro-3ef7: SQLITE_BUSY classification must match the primary code 5
    /// (including sqlx's extended codes like 517 = SQLITE_BUSY_SNAPSHOT) and
    /// the canonical "database is locked" message, while rejecting adjacent
    /// codes (SQLITE_LOCKED 6 / shared-cache 262, constraint 1555) that must
    /// NOT be retried as busy.
    #[test]
    fn sqlite_busy_codes_and_messages_are_classified_for_retry() {
        use super::is_busy_code_and_message as classify;

        assert!(classify(Some("5"), "database is locked"));
        assert!(classify(Some("517"), "database is locked"));
        assert!(classify(Some("261"), "database is locked"));
        // Message fallback when the code is absent or unparseable.
        assert!(classify(None, "database is locked"));
        assert!(classify(Some("unparseable"), "database is locked"));
        // Adjacent codes must not be classified as busy.
        assert!(!classify(Some("6"), "database table is locked"));
        assert!(!classify(Some("262"), "database table is locked"));
        assert!(!classify(Some("1555"), "UNIQUE constraint failed: runs.id"));
        assert!(!classify(None, "no such table: runs"));
        assert!(!classify(Some("5x"), "unrelated"));
    }
}
