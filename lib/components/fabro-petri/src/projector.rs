//! The projector: the view pass that folds a Petri run's committed records
//! into its stored projection, and the wake-up that drives it.
//!
//! # Commit rule
//!
//! Records first. Petri's append (the worker's append endpoint, then
//! `SqliteRunStore::append`) and a platform record's insert are the
//! durability boundaries, and both return before any view work. A view pass
//! then reads what is committed, folds the items past the positions the
//! view last committed, and writes the derived rows in one later
//! transaction together with the new positions: the last event consumed per
//! Petri log, the last platform record consumed, and the delivery sequence
//! (`stream_seq`) it assigned to each item. The view therefore trails a
//! committed record and never leads one. No projection state of Petri's is
//! checkpointed: each pass replays the run through `replay_since`, which
//! rebuilds the engine and invocation state the derivation needs and
//! delivers only the events past the held positions.
//!
//! A pass that finds new platform records committed between its read and
//! its write leaves the view alone and runs again, so the `runs` row never
//! moves backwards behind a concurrent lifecycle write.
//!
//! # Where it runs
//!
//! In the server. [`Projector::signal`] schedules a pass for a run: the
//! server calls it after each committed worker append and, through the run
//! summary store's hook, after each committed platform record; signals
//! that arrive while a pass runs coalesce into one more pass. A signal is a
//! wake-up only, never a source of facts: a signal that is lost costs
//! nothing but latency, because the next signal or the startup pass
//! ([`Projector::startup_pass`]) folds everything the view still trails.
//!
//! # A torn tail
//!
//! A record the store holds that Petri cannot read (a gap in a log, a line
//! that does not decode) fails the replay. The pass then advances no Petri
//! position, folds only the platform records, and reports the run's record
//! as incomplete with the replay's error; `inspect_run` decides
//! completeness once the run has recorded its finish.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use fabro_db::DbPool;
use fabro_store::platform_records::{PlatformRecordStore, StoredPlatformRecord, now_ms};
use fabro_store::{RunProjection, RunSummaryStore};
use fabro_types::RunId;
use fabro_util::error::collect_chain;
use petri_execution::events::{self, EventId, EventSource, RunEvent};
use petri_execution::{Access, RunKey, RunStore as _, inspect};
use petri_store::StoreError;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AsyncMutex;
use tokio::time;
use tracing::{debug, info, warn};

use crate::SqliteRunStore;
use crate::projection::{self, FoldState, Item, RecordHealth, RunView};

/// The positions a view committed: the last event consumed per Petri log,
/// and the last platform record consumed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Positions {
    #[serde(default)]
    pub petri:        Vec<EventId>,
    #[serde(default)]
    pub platform_seq: u64,
}

impl Positions {
    fn held(&self) -> BTreeMap<EventSource, EventId> {
        self.petri.iter().map(|id| (id.source, *id)).collect()
    }

    fn advance(&mut self, id: EventId) {
        match self.petri.iter_mut().find(|held| held.source == id.source) {
            Some(held) => {
                if id > *held {
                    *held = id;
                }
            }
            None => self.petri.push(id),
        }
    }
}

/// What one pass did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassReport {
    pub run_id:           RunId,
    /// The pass found nothing past the committed positions and wrote nothing.
    pub skipped:          bool,
    /// The view was left alone because a platform record landed during the
    /// pass; the projector runs the pass again.
    pub contended:        bool,
    pub petri_events:     usize,
    pub platform_records: usize,
    /// The last delivery sequence the view holds.
    pub stream_seq:       u64,
    pub positions:        Positions,
    pub health:           RecordHealth,
}

/// What the startup pass did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StartupReport {
    pub runs:      usize,
    pub projected: usize,
    /// Runs whose pass failed and was left for the next signal.
    pub failed:    usize,
}

/// Why a pass could not run or commit.
#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("the run's Petri record could not be opened")]
    Open(#[source] StoreError),
    #[error("the projection tables could not be read or written")]
    Database(#[source] sqlx::Error),
    #[error("the platform records could not be read or written")]
    Store(#[source] fabro_store::Error),
    #[error("the view could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the pass was stopped before its view transaction (injected)")]
    Injected,
}

/// The stored view of a run, as the projection tables hold it.
struct StoredView {
    view:       RunView,
    positions:  Positions,
    stream_seq: u64,
}

/// A run's pass state under the projector's lock.
#[derive(Default)]
struct Slot {
    running: bool,
    pending: bool,
}

/// The projector over one database: the pool Petri's records are read
/// from, and the pool the view tables (`platform_records`,
/// `petri_projection`, `petri_stream`, `runs`) are read and written on. In
/// the server both are the one database; a test may hand it the run
/// summary store's own pool for the views.
pub struct Projector {
    records:  DbPool,
    pool:     DbPool,
    store:    SqliteRunStore,
    platform: PlatformRecordStore,
    slots:    Mutex<HashMap<RunId, Slot>>,
    /// One pass at a time per run: a signalled pass and the startup pass
    /// over the same run never interleave their reads and writes.
    passes:   Mutex<HashMap<RunId, Arc<AsyncMutex<()>>>>,
    /// Test-only: stop the next pass after its reads, before its view
    /// transaction, as a crash there would.
    fault:    AtomicBool,
}

impl std::fmt::Debug for Projector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Projector").finish_non_exhaustive()
    }
}

impl Projector {
    /// A projector over `records`, the pool Petri's records live in, and
    /// `views`, the pool the view tables live in; both migrated. The server
    /// passes its one pool twice.
    #[must_use]
    pub fn new(records: DbPool, views: DbPool) -> Arc<Self> {
        Arc::new(Self {
            store: SqliteRunStore::new(records.clone()),
            platform: PlatformRecordStore::new(views.clone()),
            records,
            pool: views,
            slots: Mutex::default(),
            passes: Mutex::default(),
            fault: AtomicBool::new(false),
        })
    }

    /// Schedule a pass for the run. A pass already running for it runs once
    /// more when it ends; any number of signals in between coalesce.
    pub fn signal(self: &Arc<Self>, run_id: RunId) {
        {
            let mut slots = lock(&self.slots);
            let slot = slots.entry(run_id).or_default();
            if slot.running {
                slot.pending = true;
                return;
            }
            slot.running = true;
        }
        let projector = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                let again = match projector.project_run(run_id).await {
                    Ok(report) => report.contended,
                    Err(error) => {
                        warn!(
                            run_id = %run_id,
                            error = %collect_chain(&error).join(": "),
                            "Petri projection pass failed; the next signal retries it"
                        );
                        false
                    }
                };
                let mut slots = lock(&projector.slots);
                let slot = slots.entry(run_id).or_default();
                if again || slot.pending {
                    slot.pending = false;
                    continue;
                }
                slot.running = false;
                return;
            }
        });
    }

    /// Wait until no pass is running or pending for the run: a test's way
    /// to observe the view after its signals.
    pub async fn settle(&self, run_id: RunId) {
        loop {
            let idle = {
                let slots = lock(&self.slots);
                slots
                    .get(&run_id)
                    .is_none_or(|slot| !slot.running && !slot.pending)
            };
            if idle {
                return;
            }
            time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// Stop the next pass after its reads and before its view transaction,
    /// as a crash there would, once.
    pub fn fail_before_view(&self) {
        self.fault.store(true, Ordering::SeqCst);
    }

    /// One pass over every Petri run the database holds: the runs with a
    /// Petri record, and the runs with platform records. Runs whose view
    /// already covers every committed record are skipped cheaply.
    pub async fn startup_pass(&self) -> Result<StartupReport, ProjectError> {
        let mut ids: Vec<String> = sqlx::query_scalar("SELECT run_id FROM petri_runs")
            .fetch_all(&self.records)
            .await
            .map_err(ProjectError::Database)?;
        let with_platform: Vec<String> =
            sqlx::query_scalar("SELECT DISTINCT run_id FROM platform_records")
                .fetch_all(&self.pool)
                .await
                .map_err(ProjectError::Database)?;
        ids.extend(with_platform);
        ids.sort();
        ids.dedup();
        let mut report = StartupReport::default();
        for id in ids {
            let Some(run_id) = projection::run_id_of(&id) else {
                debug!(run_key = %id, "Petri run key is not a Fabro run id; not projected");
                continue;
            };
            report.runs += 1;
            match self.project_run(run_id).await {
                Ok(pass) => {
                    if !pass.skipped {
                        report.projected += 1;
                    }
                }
                // One run's view trailing never stops the server: the next
                // signal for the run retries its pass.
                Err(error) => {
                    warn!(
                        run_id = %run_id,
                        error = %collect_chain(&error).join(": "),
                        "Petri projection pass failed at startup; the next signal retries it"
                    );
                    report.failed += 1;
                }
            }
        }
        if report.projected > 0 {
            info!(
                runs = report.runs,
                projected = report.projected,
                "Petri projections caught up at startup"
            );
        }
        Ok(report)
    }

    /// One view pass for the run. Passes over one run run one at a time.
    pub async fn project_run(&self, run_id: RunId) -> Result<PassReport, ProjectError> {
        let pass = Arc::clone(lock(&self.passes).entry(run_id).or_default());
        let _one_at_a_time = pass.lock().await;
        let stored = self.load_view(&run_id).await?;
        let key = RunKey::new(run_id.to_string());
        let platform_head = self
            .platform
            .head(&run_id)
            .await
            .map_err(ProjectError::Store)?
            .unwrap_or(0);
        let petri_heads = self.petri_heads(&run_id).await?;
        let at_head = platform_head == stored.positions.platform_seq
            && petri_heads.iter().all(|(log, head)| {
                stored
                    .positions
                    .petri
                    .iter()
                    .any(|held| log_text(&held.source) == *log && held.seq == *head)
            });
        if at_head && stored.view.projection.is_some() {
            return Ok(PassReport {
                run_id,
                skipped: true,
                contended: false,
                petri_events: 0,
                platform_records: 0,
                stream_seq: stored.stream_seq,
                positions: stored.positions,
                health: stored.view.state.health,
            });
        }

        let StoredView {
            mut view,
            mut positions,
            mut stream_seq,
        } = stored;
        let platform_records = self
            .platform
            .read_after(&run_id, positions.platform_seq)
            .await
            .map_err(ProjectError::Store)?;
        let (events, replay_failure) = match self.store.open(&key, Access::Read).await {
            Ok(logs) => match events::replay_since(&*logs, &positions.held()).await {
                Ok(events) => (events, None),
                Err(error) => {
                    let chain = collect_chain(&error).join(": ");
                    warn!(run_id = %run_id, error = %chain, "Petri run does not replay; the view holds");
                    (Vec::new(), Some(chain))
                }
            },
            Err(StoreError::NotFound { .. }) => (Vec::new(), None),
            Err(error) => return Err(ProjectError::Open(error)),
        };

        let mut items: Vec<(u64, u8, Item<'_>)> =
            Vec::with_capacity(events.len() + platform_records.len());
        for event in &events {
            let rank = match event.id.source {
                EventSource::Coordinator => 0,
                EventSource::Execution { .. } => 1,
            };
            items.push((event.recorded_at, rank, Item::Petri(event)));
        }
        for record in &platform_records {
            items.push((record.recorded_at, 2, Item::Platform(record)));
        }
        items.sort_by_key(|(recorded_at, rank, _)| (*recorded_at, *rank));

        let mut rows: Vec<StreamRow> = Vec::with_capacity(items.len());
        for (_, _, item) in &items {
            stream_seq += 1;
            view.fold(item, stream_seq);
            let row = match item {
                Item::Petri(event) => {
                    positions.advance(event.id);
                    StreamRow {
                        stream_seq,
                        item_kind: "petri",
                        item_id: event_id_text(&event.id),
                        event_json: serde_json::to_string(event).map_err(ProjectError::Encode)?,
                    }
                }
                Item::Platform(record) => {
                    positions.platform_seq = record.seq;
                    StreamRow {
                        stream_seq,
                        item_kind: "platform",
                        item_id: record.seq.to_string(),
                        event_json: serde_json::to_string(record).map_err(ProjectError::Encode)?,
                    }
                }
            };
            rows.push(row);
        }
        view.state.health = self.health(&key, &view.state, replay_failure).await?;

        if self.fault.swap(false, Ordering::SeqCst) {
            return Err(ProjectError::Injected);
        }

        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(ProjectError::Database)?;
        let head_now: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(seq), 0) FROM platform_records WHERE run_id = ?",
        )
        .bind(run_id.to_string())
        .fetch_one(&mut *tx)
        .await
        .map_err(ProjectError::Database)?;
        if u64::try_from(head_now).unwrap_or(0) != positions.platform_seq {
            debug!(run_id = %run_id, "platform records landed during the pass; running it again");
            drop(tx);
            return Ok(PassReport {
                run_id,
                skipped: false,
                contended: true,
                petri_events: 0,
                platform_records: 0,
                stream_seq: 0,
                positions: Positions::default(),
                health: RecordHealth::default(),
            });
        }
        let projection_json =
            serde_json::to_string(&view.projection).map_err(ProjectError::Encode)?;
        let fold_json = serde_json::to_string(&view.state).map_err(ProjectError::Encode)?;
        let positions_json = serde_json::to_string(&positions).map_err(ProjectError::Encode)?;
        sqlx::query(
            "INSERT INTO petri_projection (run_id, projection_json, fold_json, positions_json, \
             stream_seq, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT(run_id) DO UPDATE \
             SET projection_json = excluded.projection_json, fold_json = excluded.fold_json, \
             positions_json = excluded.positions_json, stream_seq = excluded.stream_seq, \
             updated_at_ms = excluded.updated_at_ms",
        )
        .bind(run_id.to_string())
        .bind(projection_json)
        .bind(fold_json)
        .bind(positions_json)
        .bind(column(stream_seq))
        .bind(column(now_ms()))
        .execute(&mut *tx)
        .await
        .map_err(ProjectError::Database)?;
        for row in &rows {
            sqlx::query(
                "INSERT INTO petri_stream (run_id, stream_seq, item_kind, item_id, event_json) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(run_id.to_string())
            .bind(column(row.stream_seq))
            .bind(row.item_kind)
            .bind(&row.item_id)
            .bind(&row.event_json)
            .execute(&mut *tx)
            .await
            .map_err(ProjectError::Database)?;
        }
        if let Some(projection) = view.projection.as_ref() {
            RunSummaryStore::write_petri_run_row_on_connection(&mut tx, &run_id, projection)
                .await
                .map_err(ProjectError::Store)?;
        }
        tx.commit().await.map_err(ProjectError::Database)?;
        debug!(
            run_id = %run_id,
            petri_events = events.len(),
            platform_records = platform_records.len(),
            stream_seq,
            "Petri projection pass committed"
        );
        Ok(PassReport {
            run_id,
            skipped: false,
            contended: false,
            petri_events: events.len(),
            platform_records: platform_records.len(),
            stream_seq,
            positions,
            health: view.state.health.clone(),
        })
    }

    /// The stored view of the run, or an empty one.
    async fn load_view(&self, run_id: &RunId) -> Result<StoredView, ProjectError> {
        let row: Option<(String, String, String, i64)> = sqlx::query_as(
            "SELECT projection_json, fold_json, positions_json, stream_seq FROM petri_projection \
             WHERE run_id = ?",
        )
        .bind(run_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(ProjectError::Database)?;
        let Some((projection_json, fold_json, positions_json, stream_seq)) = row else {
            return Ok(StoredView {
                view:       RunView::new(),
                positions:  Positions::default(),
                stream_seq: 0,
            });
        };
        let projection: Option<RunProjection> =
            serde_json::from_str(&projection_json).map_err(ProjectError::Encode)?;
        let state: FoldState = serde_json::from_str(&fold_json).map_err(ProjectError::Encode)?;
        let positions: Positions =
            serde_json::from_str(&positions_json).map_err(ProjectError::Encode)?;
        Ok(StoredView {
            view: RunView { projection, state },
            positions,
            stream_seq: u64::try_from(stream_seq).unwrap_or(0),
        })
    }

    /// The last seq of every Petri log of the run, by the log column's text.
    async fn petri_heads(&self, run_id: &RunId) -> Result<Vec<(String, u64)>, ProjectError> {
        // The coordinator log and the execution logs are what the projection
        // reads; the resources log is the sandbox ledger and has no events.
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT log, MAX(seq) FROM petri_records WHERE run_id = ? AND (log = 'coordinator' \
             OR log LIKE 'execution %') GROUP BY log",
        )
        .bind(run_id.to_string())
        .fetch_all(&self.records)
        .await
        .map_err(ProjectError::Database)?;
        Ok(rows
            .into_iter()
            .map(|(log, seq)| (log, u64::try_from(seq).unwrap_or(0)))
            .collect())
    }

    /// Whether the run's record is whole: a replay failure says no with its
    /// reason; a run that has not recorded its finish is not yet; a finished
    /// run is what `inspect_run` says, checked until it says complete.
    async fn health(
        &self,
        key: &RunKey,
        state: &FoldState,
        replay_failure: Option<String>,
    ) -> Result<RecordHealth, ProjectError> {
        if let Some(failure) = replay_failure {
            return Ok(RecordHealth {
                complete:   false,
                incomplete: vec![failure],
            });
        }
        if state.finished.is_none() {
            return Ok(RecordHealth {
                complete:   false,
                incomplete: vec!["the run has not recorded its finish".to_string()],
            });
        }
        if state.health.complete {
            return Ok(state.health.clone());
        }
        let logs = match self.store.open(key, Access::Read).await {
            Ok(logs) => logs,
            Err(StoreError::NotFound { .. }) => return Ok(state.health.clone()),
            Err(error) => return Err(ProjectError::Open(error)),
        };
        match inspect::inspect_run(&*logs).await {
            Ok(inspection) => Ok(RecordHealth {
                complete:   inspection.complete,
                incomplete: inspection.incomplete,
            }),
            Err(error) => Ok(RecordHealth {
                complete:   false,
                incomplete: vec![collect_chain(&error).join(": ")],
            }),
        }
    }
}

impl Projector {
    /// A run store whose appends signal this projector: for a run that
    /// executes in the same process as the projector, over the SQLite store
    /// directly, where no append endpoint is there to signal. The signal is
    /// sent after the store's append returned, so the records it covers are
    /// durable before the view sees them.
    pub fn observe_store(
        self: &Arc<Self>,
        inner: Arc<dyn petri_execution::RunStore>,
    ) -> Arc<dyn petri_execution::RunStore> {
        Arc::new(SignallingStore {
            inner,
            projector: Arc::clone(self),
        })
    }
}

/// A run store that signals a projector after each append.
struct SignallingStore {
    inner:     Arc<dyn petri_execution::RunStore>,
    projector: Arc<Projector>,
}

#[async_trait::async_trait]
impl petri_execution::RunStore for SignallingStore {
    async fn open(
        &self,
        key: &RunKey,
        access: Access,
    ) -> Result<Arc<dyn petri_execution::RunLogs>, StoreError> {
        let logs = self.inner.open(key, access).await?;
        Ok(Arc::new(SignallingLogs {
            inner:     logs,
            run_id:    projection::run_id_of(key.as_str()),
            projector: Arc::clone(&self.projector),
        }))
    }
}

struct SignallingLogs {
    inner:     Arc<dyn petri_execution::RunLogs>,
    run_id:    Option<RunId>,
    projector: Arc<Projector>,
}

#[async_trait::async_trait]
impl petri_execution::RunLogs for SignallingLogs {
    fn locator(&self) -> String {
        self.inner.locator()
    }

    async fn append(
        &self,
        log: &petri_execution::LogId,
        records: &[petri_execution::Record],
    ) -> Result<(), StoreError> {
        self.inner.append(log, records).await?;
        if let Some(run_id) = self.run_id {
            self.projector.signal(run_id);
        }
        Ok(())
    }

    async fn read(
        &self,
        log: &petri_execution::LogId,
    ) -> Result<Vec<petri_execution::Record>, StoreError> {
        self.inner.read(log).await
    }

    async fn put_blob(&self, bytes: &[u8]) -> Result<petri_store::Digest, StoreError> {
        self.inner.put_blob(bytes).await
    }

    async fn get_blob(&self, digest: petri_store::Digest) -> Result<Option<Vec<u8>>, StoreError> {
        self.inner.get_blob(digest).await
    }
}

struct StreamRow {
    stream_seq: u64,
    item_kind:  &'static str,
    item_id:    String,
    event_json: String,
}

/// A Petri event id as the stream names it: `<log>/<seq>/<index>`.
#[must_use]
pub fn event_id_text(id: &EventId) -> String {
    format!("{}/{}/{}", log_text(&id.source), id.seq, id.index)
}

fn log_text(source: &EventSource) -> String {
    match source {
        EventSource::Coordinator => "coordinator".to_string(),
        EventSource::Execution { execution } => format!("execution {execution}"),
    }
}

fn column(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The run's projection rebuilt from its records alone, with nothing
/// stored: what a fresh projector would commit over the same records. A test
/// compares it with the live view. `records` and `views` are the two pools
/// [`Projector::new`] takes.
pub async fn rebuild(
    records: &DbPool,
    views: &DbPool,
    run_id: RunId,
) -> Result<(Option<RunProjection>, Positions, u64), ProjectError> {
    let store = SqliteRunStore::new(records.clone());
    let platform = PlatformRecordStore::new(views.clone());
    let key = RunKey::new(run_id.to_string());
    let platform_records = platform.read(&run_id).await.map_err(ProjectError::Store)?;
    let events = match store.open(&key, Access::Read).await {
        Ok(logs) => events::replay_run(&*logs)
            .await
            .inspect_err(|error| {
                warn!(error = %collect_chain(error).join(": "), "rebuild: the run does not replay");
            })
            .unwrap_or_default(),
        Err(StoreError::NotFound { .. }) => Vec::new(),
        Err(error) => return Err(ProjectError::Open(error)),
    };
    let mut items: Vec<(u64, u8, Item<'_>)> = Vec::new();
    for event in &events {
        let rank = match event.id.source {
            EventSource::Coordinator => 0,
            EventSource::Execution { .. } => 1,
        };
        items.push((event.recorded_at, rank, Item::Petri(event)));
    }
    for record in &platform_records {
        items.push((record.recorded_at, 2, Item::Platform(record)));
    }
    items.sort_by_key(|(recorded_at, rank, _)| (*recorded_at, *rank));
    let mut view = RunView::new();
    let mut positions = Positions::default();
    let mut stream_seq = 0;
    for (_, _, item) in &items {
        stream_seq += 1;
        view.fold(item, stream_seq);
        match item {
            Item::Petri(event) => positions.advance(event.id),
            Item::Platform(record) => positions.platform_seq = record.seq,
        }
    }
    Ok((view.projection, positions, stream_seq))
}

/// The stored view's positions and stream sequence, for a test; `views` is
/// the pool the view tables live in.
pub async fn stored_positions(
    views: &DbPool,
    run_id: RunId,
) -> Result<Option<(Positions, u64)>, ProjectError> {
    let row: Option<(String, i64)> =
        sqlx::query_as("SELECT positions_json, stream_seq FROM petri_projection WHERE run_id = ?")
            .bind(run_id.to_string())
            .fetch_optional(views)
            .await
            .map_err(ProjectError::Database)?;
    row.map(|(positions, stream_seq)| {
        Ok((
            serde_json::from_str(&positions).map_err(ProjectError::Encode)?,
            u64::try_from(stream_seq).unwrap_or(0),
        ))
    })
    .transpose()
}

/// The stored view's projection, for a test or a reader outside the store.
pub async fn stored_projection(
    views: &DbPool,
    run_id: RunId,
) -> Result<Option<RunProjection>, ProjectError> {
    let json: Option<String> =
        sqlx::query_scalar("SELECT projection_json FROM petri_projection WHERE run_id = ?")
            .bind(run_id.to_string())
            .fetch_optional(views)
            .await
            .map_err(ProjectError::Database)?;
    json.map(|json| serde_json::from_str(&json).map_err(ProjectError::Encode))
        .transpose()
}

/// The stream rows of a run: `(stream_seq, item_kind, item_id)`, in order.
pub async fn stored_stream(
    views: &DbPool,
    run_id: RunId,
) -> Result<Vec<(u64, String, String)>, ProjectError> {
    let rows: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT stream_seq, item_kind, item_id FROM petri_stream WHERE run_id = ? ORDER BY stream_seq",
    )
    .bind(run_id.to_string())
    .fetch_all(views)
    .await
    .map_err(ProjectError::Database)?;
    Ok(rows
        .into_iter()
        .map(|(seq, kind, id)| (u64::try_from(seq).unwrap_or(0), kind, id))
        .collect())
}

/// Every stored platform record of a run, for a reader outside the store.
pub async fn stored_platform_records(
    views: &DbPool,
    run_id: RunId,
) -> Result<Vec<StoredPlatformRecord>, ProjectError> {
    PlatformRecordStore::new(views.clone())
        .read(&run_id)
        .await
        .map_err(ProjectError::Store)
}

/// A recorded event's projection is what `RunEvent` serializes to.
#[must_use]
pub fn event_json(event: &RunEvent) -> serde_json::Value {
    serde_json::to_value(event).unwrap_or_default()
}
