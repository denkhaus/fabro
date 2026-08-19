//! Temporary compatibility importer for the legacy SlateDB blob keyspace.
//!
//! Remove this module with the Slate blob backend after the approved legacy
//! support window ends.

use std::error::Error as StdError;
use std::fmt;

use bytes::Bytes;
use fabro_types::BlobHash;
use sqlx::pool::PoolConnection;
use sqlx::{Acquire as _, Sqlite};
use tracing::debug;

use crate::keys::SlateKey;
use crate::{BlobStore, Database};

const MAX_BATCH_ROWS: usize = 100;
const MAX_BATCH_BYTES: u64 = 1024 * 1024;
const PASSIVE_CHECKPOINT_BYTES: u64 = 8 * 1024 * 1024;

/// Aggregate progress from one legacy blob import attempt.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LegacyBlobImportReport {
    /// Source rows observed under the exact legacy blob prefix.
    pub scanned_rows:        u64,
    /// Raw source value bytes observed under the exact legacy blob prefix.
    pub scanned_bytes:       u64,
    /// Destination rows newly inserted by committed transactions.
    pub imported_rows:       u64,
    /// Destination bytes newly inserted by committed transactions.
    pub imported_bytes:      u64,
    /// Byte-equal destination rows accepted by committed transactions.
    pub existing_rows:       u64,
    /// Bytes belonging to byte-equal destination rows.
    pub existing_bytes:      u64,
    /// Source rows rejected for malformed keys or digest mismatches.
    pub invalid_rows:        u64,
    /// Destination rows rejected because their bytes differed.
    pub conflicting_rows:    u64,
    /// Successfully committed import transactions.
    pub committed_batches:   u64,
    /// Successful passive WAL checkpoints during the import.
    pub passive_checkpoints: u64,
}

/// A failed legacy blob import and the durable progress completed before it.
pub struct LegacyBlobImportError {
    report:  LegacyBlobImportReport,
    failure: LegacyBlobImportFailure,
}

impl LegacyBlobImportError {
    /// Returns the aggregate durable progress completed before the failure.
    #[must_use]
    pub fn report(&self) -> &LegacyBlobImportReport {
        &self.report
    }
}

impl fmt::Debug for LegacyBlobImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyBlobImportError")
            .field("report", &self.report)
            .field("failure", &self.failure.kind())
            .finish()
    }
}

impl fmt::Display for LegacyBlobImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "legacy blob import failed after scanning {} rows and committing {} batches: {}",
            self.report.scanned_rows, self.report.committed_batches, self.failure
        )
    }
}

impl StdError for LegacyBlobImportError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(&self.failure)
    }
}

#[derive(thiserror::Error)]
enum LegacyBlobImportFailure {
    #[error("the legacy blob import target is not backed by SQLite")]
    WrongTargetBackend,

    #[error("opening the legacy blob source")]
    OpenSource(#[source] crate::Error),

    #[error("opening the legacy blob scan")]
    OpenSourceScan(#[source] slatedb::Error),

    #[error("reading the legacy blob scan")]
    ReadSourceScan(#[source] slatedb::Error),

    #[error("a legacy blob key is not canonical")]
    InvalidSourceKey,

    #[error("legacy blob bytes do not match their key digest")]
    SourceDigestMismatch,

    #[error("a legacy blob import counter overflowed")]
    CounterOverflow,

    #[error("acquiring the SQLite import connection")]
    AcquireConnection(#[source] sqlx::Error),

    #[error("reading the SQLite automatic checkpoint setting")]
    ReadAutomaticCheckpoint(#[source] sqlx::Error),

    #[error("disabling SQLite automatic checkpointing")]
    DisableAutomaticCheckpoint(#[source] sqlx::Error),

    #[error("starting a SQLite blob import transaction")]
    BeginTransaction(#[source] sqlx::Error),

    #[error("inserting a SQLite blob row")]
    InsertDestination(#[source] sqlx::Error),

    #[error("reading an existing SQLite blob row")]
    ReadDestination(#[source] sqlx::Error),

    #[error("SQLite contains different bytes for a legacy blob hash")]
    DestinationConflict,

    #[error("committing a SQLite blob import transaction")]
    CommitTransaction(#[source] sqlx::Error),

    #[error("rolling back a failed SQLite blob import transaction")]
    RollbackTransaction {
        #[source]
        source: sqlx::Error,
        prior:  Box<Self>,
    },

    #[error("running a passive SQLite WAL checkpoint")]
    PassiveCheckpoint(#[source] sqlx::Error),

    #[error("the passive SQLite WAL checkpoint could not complete")]
    PassiveCheckpointBusy,

    #[error("running the final SQLite WAL checkpoint")]
    FinalCheckpoint(#[source] sqlx::Error),

    #[error("the final SQLite WAL checkpoint could not complete")]
    FinalCheckpointBusy,

    #[error("restoring the SQLite automatic checkpoint setting")]
    RestoreAutomaticCheckpoint {
        #[source]
        source:           sqlx::Error,
        prior:            Option<Box<Self>>,
        retirement_error: Option<sqlx::Error>,
    },
}

impl LegacyBlobImportFailure {
    fn kind(&self) -> &'static str {
        match self {
            Self::WrongTargetBackend => "wrong_target_backend",
            Self::OpenSource(_) => "open_source",
            Self::OpenSourceScan(_) => "open_source_scan",
            Self::ReadSourceScan(_) => "read_source_scan",
            Self::InvalidSourceKey => "invalid_source_key",
            Self::SourceDigestMismatch => "source_digest_mismatch",
            Self::CounterOverflow => "counter_overflow",
            Self::AcquireConnection(_) => "acquire_connection",
            Self::ReadAutomaticCheckpoint(_) => "read_automatic_checkpoint",
            Self::DisableAutomaticCheckpoint(_) => "disable_automatic_checkpoint",
            Self::BeginTransaction(_) => "begin_transaction",
            Self::InsertDestination(_) => "insert_destination",
            Self::ReadDestination(_) => "read_destination",
            Self::DestinationConflict => "destination_conflict",
            Self::CommitTransaction(_) => "commit_transaction",
            Self::RollbackTransaction { .. } => "rollback_transaction",
            Self::PassiveCheckpoint(_) => "passive_checkpoint",
            Self::PassiveCheckpointBusy => "passive_checkpoint_busy",
            Self::FinalCheckpoint(_) => "final_checkpoint",
            Self::FinalCheckpointBusy => "final_checkpoint_busy",
            Self::RestoreAutomaticCheckpoint { .. } => "restore_automatic_checkpoint",
        }
    }
}

impl fmt::Debug for LegacyBlobImportFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = formatter.debug_struct("LegacyBlobImportFailure");
        debug.field("kind", &self.kind());
        match self {
            Self::RollbackTransaction { prior, .. } => {
                debug.field("prior_failure", &prior.kind());
            }
            Self::RestoreAutomaticCheckpoint {
                prior,
                retirement_error,
                ..
            } => {
                debug
                    .field("prior_failure", &prior.as_deref().map(Self::kind))
                    .field("retirement_failed", &retirement_error.is_some());
            }
            _ => {}
        }
        debug.finish()
    }
}

#[derive(Default)]
struct ImportControls {
    #[cfg(test)]
    source_after_rows:            Option<u64>,
    #[cfg(test)]
    passive_checkpoint:           bool,
    #[cfg(test)]
    final_checkpoint:             bool,
    #[cfg(test)]
    restore_automatic_checkpoint: bool,
}

impl ImportControls {
    fn source_scan_error(&self, scanned_rows: u64) -> Option<slatedb::Error> {
        #[cfg(test)]
        if self.source_after_rows == Some(scanned_rows) {
            return Some(slatedb::Error::unavailable(
                "injected legacy source transport failure".to_owned(),
            ));
        }
        let _ = (self, scanned_rows);
        None
    }

    fn passive_checkpoint_error(&self) -> Option<sqlx::Error> {
        #[cfg(test)]
        if self.passive_checkpoint {
            return Some(sqlx::Error::Protocol(
                "injected passive checkpoint failure".to_owned(),
            ));
        }
        let _ = self;
        None
    }

    fn final_checkpoint_error(&self) -> Option<sqlx::Error> {
        #[cfg(test)]
        if self.final_checkpoint {
            return Some(sqlx::Error::Protocol(
                "injected final checkpoint failure".to_owned(),
            ));
        }
        let _ = self;
        None
    }

    fn restore_automatic_checkpoint_error(&self) -> Option<sqlx::Error> {
        #[cfg(test)]
        if self.restore_automatic_checkpoint {
            return Some(sqlx::Error::Protocol(
                "injected automatic checkpoint restoration failure".to_owned(),
            ));
        }
        let _ = self;
        None
    }
}

struct PendingBlob {
    hash:  BlobHash,
    bytes: Bytes,
}

#[derive(Clone, Copy, Default)]
struct BatchReport {
    imported_rows:  u64,
    imported_bytes: u64,
    existing_rows:  u64,
    existing_bytes: u64,
}

impl Database {
    /// Strictly imports the legacy SlateDB blob keyspace into a SQLite blob
    /// store.
    ///
    /// # Errors
    ///
    /// Returns a typed error with partial durable progress if source
    /// validation, destination persistence, checkpointing, or connection
    /// cleanup fails.
    pub async fn import_legacy_blobs_into(
        &self,
        target: &BlobStore,
    ) -> std::result::Result<LegacyBlobImportReport, LegacyBlobImportError> {
        self.import_legacy_blobs_with_controls(target, &ImportControls::default())
            .await
    }

    async fn import_legacy_blobs_with_controls(
        &self,
        target: &BlobStore,
        controls: &ImportControls,
    ) -> std::result::Result<LegacyBlobImportReport, LegacyBlobImportError> {
        let mut report = LegacyBlobImportReport::default();
        let result = self
            .run_legacy_blob_import(target, controls, &mut report)
            .await;

        match result {
            Ok(()) => {
                debug_import_outcome("complete", &report, None);
                Ok(report)
            }
            Err(failure) => {
                debug_import_outcome("failed", &report, Some(failure.kind()));
                Err(LegacyBlobImportError { report, failure })
            }
        }
    }

    async fn run_legacy_blob_import(
        &self,
        target: &BlobStore,
        controls: &ImportControls,
        report: &mut LegacyBlobImportReport,
    ) -> Result<(), LegacyBlobImportFailure> {
        let pool = target
            .sqlite_pool_for_legacy_import()
            .ok_or(LegacyBlobImportFailure::WrongTargetBackend)?;
        let mut connection = pool
            .acquire()
            .await
            .map_err(LegacyBlobImportFailure::AcquireConnection)?;
        let previous_automatic_checkpoint = sqlx::query_scalar("PRAGMA wal_autocheckpoint")
            .fetch_one(&mut *connection)
            .await
            .map_err(LegacyBlobImportFailure::ReadAutomaticCheckpoint)?;

        let import_result = match set_automatic_checkpoint(&mut connection, 0).await {
            Ok(()) => {
                self.copy_legacy_blobs(&mut connection, controls, report)
                    .await
            }
            Err(source) => Err(LegacyBlobImportFailure::DisableAutomaticCheckpoint(source)),
        };

        let restore_result = if let Some(error) = controls.restore_automatic_checkpoint_error() {
            Err(error)
        } else {
            set_automatic_checkpoint(&mut connection, previous_automatic_checkpoint).await
        };

        if let Err(source) = restore_result {
            let retirement_error = connection.close().await.err();
            return Err(LegacyBlobImportFailure::RestoreAutomaticCheckpoint {
                source,
                prior: import_result.err().map(Box::new),
                retirement_error,
            });
        }

        import_result
    }

    async fn copy_legacy_blobs(
        &self,
        connection: &mut PoolConnection<Sqlite>,
        controls: &ImportControls,
        report: &mut LegacyBlobImportReport,
    ) -> Result<(), LegacyBlobImportFailure> {
        let source = self
            .open_db()
            .await
            .map_err(LegacyBlobImportFailure::OpenSource)?;
        let prefix = SlateKey::new("blobs").with("sha256").into_prefix();
        let prefix_bytes = prefix.as_ref().to_vec();
        let mut entries = source
            .scan_prefix(&prefix_bytes)
            .await
            .map_err(LegacyBlobImportFailure::OpenSourceScan)?;
        let mut pending = Vec::with_capacity(MAX_BATCH_ROWS);
        let mut pending_bytes = 0_u64;
        let mut bytes_since_checkpoint = 0_u64;

        loop {
            if let Some(source) = controls.source_scan_error(report.scanned_rows) {
                return Err(LegacyBlobImportFailure::ReadSourceScan(source));
            }
            let Some(entry) = entries
                .next()
                .await
                .map_err(LegacyBlobImportFailure::ReadSourceScan)?
            else {
                break;
            };
            checked_add(&mut report.scanned_rows, 1)?;
            let value_bytes = usize_to_u64(entry.value.len())?;
            checked_add(&mut report.scanned_bytes, value_bytes)?;

            let hash = validate_source_entry(&entry.key, &entry.value, &prefix_bytes, report)?;

            let would_exceed_rows = pending.len() == MAX_BATCH_ROWS;
            let would_exceed_bytes = pending_bytes
                .checked_add(value_bytes)
                .ok_or(LegacyBlobImportFailure::CounterOverflow)?
                > MAX_BATCH_BYTES;
            if !pending.is_empty() && (would_exceed_rows || would_exceed_bytes) {
                commit_pending_batch(
                    connection,
                    &mut pending,
                    &mut pending_bytes,
                    &mut bytes_since_checkpoint,
                    controls,
                    report,
                )
                .await?;
            }

            pending.push(PendingBlob {
                hash,
                bytes: entry.value,
            });
            checked_add(&mut pending_bytes, value_bytes)?;

            if value_bytes > MAX_BATCH_BYTES {
                commit_pending_batch(
                    connection,
                    &mut pending,
                    &mut pending_bytes,
                    &mut bytes_since_checkpoint,
                    controls,
                    report,
                )
                .await?;
            }
        }

        commit_pending_batch(
            connection,
            &mut pending,
            &mut pending_bytes,
            &mut bytes_since_checkpoint,
            controls,
            report,
        )
        .await?;
        run_checkpoint(connection, CheckpointKind::Final, controls).await
    }
}

fn validate_source_entry(
    key: &[u8],
    value: &[u8],
    prefix: &[u8],
    report: &mut LegacyBlobImportReport,
) -> Result<BlobHash, LegacyBlobImportFailure> {
    let Some(suffix) = key.strip_prefix(prefix) else {
        return invalid_source_row(report, LegacyBlobImportFailure::InvalidSourceKey);
    };
    let canonical = suffix.len() == 64
        && suffix
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte));
    if !canonical {
        return invalid_source_row(report, LegacyBlobImportFailure::InvalidSourceKey);
    }

    let hash = std::str::from_utf8(suffix)
        .ok()
        .and_then(|value| value.parse().ok())
        .ok_or(LegacyBlobImportFailure::InvalidSourceKey);
    let hash = match hash {
        Ok(hash) => hash,
        Err(failure) => return invalid_source_row(report, failure),
    };
    if BlobHash::new(value) != hash {
        return invalid_source_row(report, LegacyBlobImportFailure::SourceDigestMismatch);
    }
    Ok(hash)
}

fn invalid_source_row<T>(
    report: &mut LegacyBlobImportReport,
    failure: LegacyBlobImportFailure,
) -> Result<T, LegacyBlobImportFailure> {
    checked_add(&mut report.invalid_rows, 1)?;
    Err(failure)
}

async fn commit_pending_batch(
    connection: &mut PoolConnection<Sqlite>,
    pending: &mut Vec<PendingBlob>,
    pending_bytes: &mut u64,
    bytes_since_checkpoint: &mut u64,
    controls: &ImportControls,
    report: &mut LegacyBlobImportReport,
) -> Result<(), LegacyBlobImportFailure> {
    if pending.is_empty() {
        return Ok(());
    }

    let batch = std::mem::take(pending);
    *pending_bytes = 0;
    let batch_report = commit_batch(connection, batch, report).await?;

    let mut updated = *report;
    checked_add(&mut updated.imported_rows, batch_report.imported_rows)?;
    checked_add(&mut updated.imported_bytes, batch_report.imported_bytes)?;
    checked_add(&mut updated.existing_rows, batch_report.existing_rows)?;
    checked_add(&mut updated.existing_bytes, batch_report.existing_bytes)?;
    checked_add(&mut updated.committed_batches, 1)?;
    *report = updated;

    checked_add(bytes_since_checkpoint, batch_report.imported_bytes)?;
    if *bytes_since_checkpoint >= PASSIVE_CHECKPOINT_BYTES {
        run_checkpoint(connection, CheckpointKind::Passive, controls).await?;
        checked_add(&mut report.passive_checkpoints, 1)?;
        *bytes_since_checkpoint = 0;
    }

    Ok(())
}

async fn commit_batch(
    connection: &mut PoolConnection<Sqlite>,
    batch: Vec<PendingBlob>,
    report: &mut LegacyBlobImportReport,
) -> Result<BatchReport, LegacyBlobImportFailure> {
    let mut transaction = connection
        .begin()
        .await
        .map_err(LegacyBlobImportFailure::BeginTransaction)?;
    let batch_result = async {
        let mut batch_report = BatchReport::default();
        for blob in batch {
            let value_bytes = usize_to_u64(blob.bytes.len())?;
            let result = sqlx::query(
                "INSERT INTO blobs (hash, data) VALUES (?, ?) ON CONFLICT(hash) DO NOTHING",
            )
            .bind(blob.hash.to_string())
            .bind(blob.bytes.as_ref())
            .execute(&mut *transaction)
            .await
            .map_err(LegacyBlobImportFailure::InsertDestination)?;

            if result.rows_affected() == 1 {
                checked_add(&mut batch_report.imported_rows, 1)?;
                checked_add(&mut batch_report.imported_bytes, value_bytes)?;
                continue;
            }

            let stored: Vec<u8> = sqlx::query_scalar("SELECT data FROM blobs WHERE hash = ?")
                .bind(blob.hash.to_string())
                .fetch_one(&mut *transaction)
                .await
                .map_err(LegacyBlobImportFailure::ReadDestination)?;
            if stored != blob.bytes {
                checked_add(&mut report.conflicting_rows, 1)?;
                return Err(LegacyBlobImportFailure::DestinationConflict);
            }
            checked_add(&mut batch_report.existing_rows, 1)?;
            checked_add(&mut batch_report.existing_bytes, value_bytes)?;
        }
        Ok(batch_report)
    }
    .await;

    match batch_result {
        Ok(batch_report) => {
            transaction
                .commit()
                .await
                .map_err(LegacyBlobImportFailure::CommitTransaction)?;
            Ok(batch_report)
        }
        Err(prior) => match transaction.rollback().await {
            Ok(()) => Err(prior),
            Err(source) => Err(LegacyBlobImportFailure::RollbackTransaction {
                source,
                prior: Box::new(prior),
            }),
        },
    }
}

#[derive(Clone, Copy)]
enum CheckpointKind {
    Passive,
    Final,
}

async fn run_checkpoint(
    connection: &mut PoolConnection<Sqlite>,
    kind: CheckpointKind,
    controls: &ImportControls,
) -> Result<(), LegacyBlobImportFailure> {
    let (statement, injected_error) = match kind {
        CheckpointKind::Passive => (
            "PRAGMA wal_checkpoint(PASSIVE)",
            controls.passive_checkpoint_error(),
        ),
        CheckpointKind::Final => (
            "PRAGMA wal_checkpoint(TRUNCATE)",
            controls.final_checkpoint_error(),
        ),
    };
    if let Some(source) = injected_error {
        return Err(match kind {
            CheckpointKind::Passive => LegacyBlobImportFailure::PassiveCheckpoint(source),
            CheckpointKind::Final => LegacyBlobImportFailure::FinalCheckpoint(source),
        });
    }

    let result = sqlx::query_as::<_, (i64, i64, i64)>(statement)
        .fetch_one(&mut **connection)
        .await;
    let (busy, _, _) = result.map_err(|source| match kind {
        CheckpointKind::Passive => LegacyBlobImportFailure::PassiveCheckpoint(source),
        CheckpointKind::Final => LegacyBlobImportFailure::FinalCheckpoint(source),
    })?;
    if busy != 0 {
        return Err(match kind {
            CheckpointKind::Passive => LegacyBlobImportFailure::PassiveCheckpointBusy,
            CheckpointKind::Final => LegacyBlobImportFailure::FinalCheckpointBusy,
        });
    }
    Ok(())
}

async fn set_automatic_checkpoint(
    connection: &mut PoolConnection<Sqlite>,
    pages: i64,
) -> Result<(), sqlx::Error> {
    // `pages` comes directly from SQLite as an integer, so the dynamic PRAGMA
    // contains no caller-controlled text and cannot change the SQL shape.
    let statement = sqlx::AssertSqlSafe(format!("PRAGMA wal_autocheckpoint = {pages}"));
    sqlx::query(statement).execute(&mut **connection).await?;
    Ok(())
}

fn usize_to_u64(value: usize) -> Result<u64, LegacyBlobImportFailure> {
    u64::try_from(value).map_err(|_| LegacyBlobImportFailure::CounterOverflow)
}

fn checked_add(value: &mut u64, amount: u64) -> Result<(), LegacyBlobImportFailure> {
    *value = value
        .checked_add(amount)
        .ok_or(LegacyBlobImportFailure::CounterOverflow)?;
    Ok(())
}

fn debug_import_outcome(
    outcome: &'static str,
    report: &LegacyBlobImportReport,
    failure_kind: Option<&'static str>,
) {
    debug!(
        outcome,
        failure_kind,
        scanned_rows = report.scanned_rows,
        scanned_bytes = report.scanned_bytes,
        imported_rows = report.imported_rows,
        imported_bytes = report.imported_bytes,
        existing_rows = report.existing_rows,
        existing_bytes = report.existing_bytes,
        invalid_rows = report.invalid_rows,
        conflicting_rows = report.conflicting_rows,
        committed_batches = report.committed_batches,
        passive_checkpoints = report.passive_checkpoints,
        "Legacy blob import finished"
    );
}

#[cfg(test)]
mod tests {
    use std::fmt::{self, Write as _};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use bytes::Bytes;
    use fabro_types::BlobHash;
    use object_store::memory::InMemory;
    use tracing::field::{Field, Visit};
    use tracing::instrument::WithSubscriber as _;
    use tracing::span::{Attributes, Id, Record};
    use tracing::{Event, Metadata, Subscriber};

    use super::{
        ImportControls, LegacyBlobImportFailure, LegacyBlobImportReport, MAX_BATCH_BYTES,
        PASSIVE_CHECKPOINT_BYTES,
    };
    use crate::keys::SlateKey;
    use crate::{BlobStore, Database};

    type TestResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

    struct TestContext {
        _dir:      tempfile::TempDir,
        source:    Database,
        source_db: slatedb::Db,
        sqlite:    fabro_db::Database,
        target:    BlobStore,
    }

    impl TestContext {
        async fn new() -> TestResult<Self> {
            let source = Database::new(
                Arc::new(InMemory::new()),
                "legacy-blob-import-tests",
                Duration::from_millis(1),
                None,
            );
            let source_db = source.open_db().await?;
            let dir = tempfile::tempdir()?;
            let sqlite = fabro_db::Database::connect(dir.path().join("fabro.sqlite3")).await?;
            sqlite.migrate().await?;
            let target = BlobStore::new(sqlite.clone_pool());
            Ok(Self {
                _dir: dir,
                source,
                source_db,
                sqlite,
                target,
            })
        }

        async fn put_blob(&self, bytes: &[u8]) -> TestResult<BlobHash> {
            let hash = BlobHash::new(bytes);
            let key = SlateKey::new("blobs").with("sha256").with(hash);
            self.source_db.put(key, bytes).await?;
            Ok(hash)
        }

        async fn put_raw(&self, key: Vec<u8>, bytes: &[u8]) -> TestResult<()> {
            self.source_db.put(key, bytes).await?;
            Ok(())
        }

        async fn source_entries(&self) -> TestResult<Vec<(Vec<u8>, Vec<u8>)>> {
            let mut entries = self.source_db.scan_prefix(Vec::<u8>::new()).await?;
            let mut snapshot = Vec::new();
            while let Some(entry) = entries.next().await? {
                snapshot.push((entry.key.to_vec(), entry.value.to_vec()));
            }
            Ok(snapshot)
        }

        async fn destination_rows(&self) -> TestResult<i64> {
            Ok(sqlx::query_scalar("SELECT COUNT(*) FROM blobs")
                .fetch_one(self.sqlite.pool())
                .await?)
        }

        async fn insert_destination(&self, hash: BlobHash, bytes: &[u8]) -> TestResult<()> {
            sqlx::query("INSERT INTO blobs (hash, data) VALUES (?, ?)")
                .bind(hash.to_string())
                .bind(bytes)
                .execute(self.sqlite.pool())
                .await?;
            Ok(())
        }

        async fn delete_destination(&self, hash: BlobHash) -> TestResult<()> {
            sqlx::query("DELETE FROM blobs WHERE hash = ?")
                .bind(hash.to_string())
                .execute(self.sqlite.pool())
                .await?;
            Ok(())
        }

        async fn set_automatic_checkpoint(&self, pages: i64) -> TestResult<()> {
            let mut connection = self.sqlite.pool().acquire().await?;
            let statement = sqlx::AssertSqlSafe(format!("PRAGMA wal_autocheckpoint = {pages}"));
            sqlx::query(statement).execute(&mut *connection).await?;
            Ok(())
        }

        async fn automatic_checkpoint(&self) -> TestResult<i64> {
            let mut connection = self.sqlite.pool().acquire().await?;
            Ok(sqlx::query_scalar("PRAGMA wal_autocheckpoint")
                .fetch_one(&mut *connection)
                .await?)
        }

        async fn import(&self) -> TestResult<LegacyBlobImportReport> {
            Ok(self.source.import_legacy_blobs_into(&self.target).await?)
        }
    }

    fn legacy_prefix() -> Vec<u8> {
        SlateKey::new("blobs")
            .with("sha256")
            .into_prefix()
            .as_ref()
            .to_vec()
    }

    async fn seed_blobs(
        context: &TestContext,
        count: usize,
    ) -> TestResult<Vec<(BlobHash, Vec<u8>)>> {
        let mut blobs = Vec::with_capacity(count);
        for index in 0..count {
            let bytes = format!("legacy-blob-{index:04}").into_bytes();
            let hash = context.put_blob(&bytes).await?;
            blobs.push((hash, bytes));
        }
        blobs.sort_by_key(|(hash, _)| *hash);
        Ok(blobs)
    }

    #[tokio::test]
    async fn imports_valid_raw_blobs_and_reports_aggregate_progress() -> TestResult<()> {
        let context = TestContext::new().await?;
        let binary = [0_u8, 0xff, 0x80, b'a'];
        let binary_hash = context.put_blob(&binary).await?;
        let empty_hash = context.put_blob(b"").await?;
        let source_before = context.source_entries().await?;

        let report = context.import().await?;

        assert_eq!(report, LegacyBlobImportReport {
            scanned_rows: 2,
            scanned_bytes: 4,
            imported_rows: 2,
            imported_bytes: 4,
            committed_batches: 1,
            ..LegacyBlobImportReport::default()
        });
        assert_eq!(
            context.target.read(&binary_hash).await?,
            Some(Bytes::copy_from_slice(&binary))
        );
        assert_eq!(context.target.read(&empty_hash).await?, Some(Bytes::new()));
        assert_eq!(context.source_entries().await?, source_before);
        Ok(())
    }

    #[tokio::test]
    async fn empty_source_succeeds_without_committing_a_batch() -> TestResult<()> {
        let context = TestContext::new().await?;

        let report = context.import().await?;

        assert_eq!(report, LegacyBlobImportReport::default());
        assert_eq!(context.destination_rows().await?, 0);
        Ok(())
    }

    #[tokio::test]
    async fn scan_is_limited_to_the_exact_legacy_prefix() -> TestResult<()> {
        let context = TestContext::new().await?;
        context.put_blob(b"included").await?;
        context
            .source_db
            .put(
                SlateKey::new("blobs").with("other").with("ignored"),
                b"nearby",
            )
            .await?;

        let report = context.import().await?;

        assert_eq!(report.scanned_rows, 1);
        assert_eq!(report.imported_rows, 1);
        assert_eq!(context.destination_rows().await?, 1);
        Ok(())
    }

    #[tokio::test]
    async fn rejects_every_noncanonical_legacy_key_shape() -> TestResult<()> {
        let valid_hash = BlobHash::new(b"valid-shape").to_string();
        let cases = [
            ("empty", Vec::new()),
            ("short", vec![b'0'; 63]),
            ("long", vec![b'0'; 65]),
            ("uppercase", vec![b'A'; 64]),
            ("non_hex", vec![b'g'; 64]),
            ("non_utf8", vec![0xff; 64]),
            (
                "extra_segment",
                [valid_hash.as_bytes(), b"\0extra"].concat(),
            ),
        ];

        for (case, suffix) in cases {
            let context = TestContext::new().await?;
            let mut key = legacy_prefix();
            key.extend_from_slice(&suffix);
            context.put_raw(key, b"source-bytes").await?;
            let source_before = context.source_entries().await?;

            let error = context
                .source
                .import_legacy_blobs_into(&context.target)
                .await
                .expect_err("noncanonical key should fail import");

            assert_eq!(error.report().scanned_rows, 1, "case {case}");
            assert_eq!(error.report().invalid_rows, 1, "case {case}");
            assert_eq!(error.report().committed_batches, 0, "case {case}");
            assert!(
                matches!(&error.failure, LegacyBlobImportFailure::InvalidSourceKey),
                "case {case}: {error:?}"
            );
            assert_eq!(context.destination_rows().await?, 0, "case {case}");
            assert_eq!(
                context.source_entries().await?,
                source_before,
                "case {case}"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn rejects_source_bytes_that_do_not_match_the_key_digest() -> TestResult<()> {
        let context = TestContext::new().await?;
        let mut key = legacy_prefix();
        key.extend_from_slice(BlobHash::new(b"expected").to_string().as_bytes());
        context.put_raw(key, b"different").await?;

        let error = context
            .source
            .import_legacy_blobs_into(&context.target)
            .await
            .expect_err("digest mismatch should fail import");

        assert_eq!(error.report().scanned_rows, 1);
        assert_eq!(error.report().scanned_bytes, 9);
        assert_eq!(error.report().invalid_rows, 1);
        assert!(matches!(
            &error.failure,
            LegacyBlobImportFailure::SourceDigestMismatch
        ));
        assert_eq!(context.destination_rows().await?, 0);
        Ok(())
    }

    #[tokio::test]
    async fn invalid_source_row_discards_the_uncommitted_pending_batch() -> TestResult<()> {
        let context = TestContext::new().await?;
        context.put_blob(b"valid-but-not-yet-committed").await?;
        let mut invalid_key = legacy_prefix();
        invalid_key.extend_from_slice(&[b'z'; 64]);
        context.put_raw(invalid_key, b"invalid").await?;

        let error = context
            .source
            .import_legacy_blobs_into(&context.target)
            .await
            .expect_err("invalid row should stop the import");

        assert_eq!(error.report().scanned_rows, 2);
        assert_eq!(error.report().invalid_rows, 1);
        assert_eq!(error.report().imported_rows, 0);
        assert_eq!(error.report().committed_batches, 0);
        assert_eq!(context.destination_rows().await?, 0);
        Ok(())
    }

    #[tokio::test]
    async fn equal_destination_rows_are_retry_progress() -> TestResult<()> {
        let context = TestContext::new().await?;
        context.put_blob(b"first").await?;
        context.put_blob(b"second").await?;

        let first = context.import().await?;
        let second = context.import().await?;

        assert_eq!(first.imported_rows, 2);
        assert_eq!(second.scanned_rows, 2);
        assert_eq!(second.imported_rows, 0);
        assert_eq!(second.existing_rows, 2);
        assert_eq!(second.existing_bytes, 11);
        assert_eq!(second.committed_batches, 1);
        assert_eq!(context.destination_rows().await?, 2);
        Ok(())
    }

    #[tokio::test]
    async fn destination_conflict_rolls_back_the_current_batch() -> TestResult<()> {
        let context = TestContext::new().await?;
        let blobs = seed_blobs(&context, 2).await?;
        context
            .insert_destination(blobs[1].0, b"conflicting-destination-bytes")
            .await?;

        let error = context
            .source
            .import_legacy_blobs_into(&context.target)
            .await
            .expect_err("differing destination row should fail import");

        assert_eq!(error.report().scanned_rows, 2);
        assert_eq!(error.report().conflicting_rows, 1);
        assert_eq!(error.report().imported_rows, 0);
        assert_eq!(error.report().committed_batches, 0);
        assert!(matches!(
            &error.failure,
            LegacyBlobImportFailure::DestinationConflict
        ));
        assert_eq!(context.target.read(&blobs[0].0).await?, None);
        assert_eq!(context.destination_rows().await?, 1);
        Ok(())
    }

    #[tokio::test]
    async fn interrupted_import_preserves_commits_and_retry_converges() -> TestResult<()> {
        let context = TestContext::new().await?;
        let blobs = seed_blobs(&context, 101).await?;
        let conflict = blobs[100].0;
        context
            .insert_destination(conflict, b"conflicting-destination-bytes")
            .await?;

        let error = context
            .source
            .import_legacy_blobs_into(&context.target)
            .await
            .expect_err("last-row conflict should interrupt import");

        assert_eq!(error.report().scanned_rows, 101);
        assert_eq!(error.report().imported_rows, 100);
        assert_eq!(error.report().conflicting_rows, 1);
        assert_eq!(error.report().committed_batches, 1);
        assert_eq!(context.destination_rows().await?, 101);

        context.delete_destination(conflict).await?;
        let retry = context.import().await?;
        assert_eq!(retry.scanned_rows, 101);
        assert_eq!(retry.existing_rows, 100);
        assert_eq!(retry.imported_rows, 1);
        assert_eq!(retry.committed_batches, 2);
        assert_eq!(context.destination_rows().await?, 101);
        Ok(())
    }

    #[tokio::test]
    async fn row_limit_splits_one_hundred_and_one_values() -> TestResult<()> {
        let context = TestContext::new().await?;
        seed_blobs(&context, 101).await?;

        let report = context.import().await?;

        assert_eq!(report.imported_rows, 101);
        assert_eq!(report.committed_batches, 2);
        Ok(())
    }

    #[tokio::test]
    async fn byte_limit_splits_batches_and_allows_exact_limit() -> TestResult<()> {
        let split_context = TestContext::new().await?;
        split_context.put_blob(&vec![b'a'; 600 * 1024]).await?;
        split_context.put_blob(&vec![b'b'; 600 * 1024]).await?;
        let split = split_context.import().await?;
        assert_eq!(split.committed_batches, 2);

        let exact_context = TestContext::new().await?;
        exact_context
            .put_blob(&vec![b'x'; usize::try_from(MAX_BATCH_BYTES)?])
            .await?;
        exact_context.put_blob(b"").await?;
        let exact = exact_context.import().await?;
        assert_eq!(exact.committed_batches, 1);
        Ok(())
    }

    #[tokio::test]
    async fn oversized_value_is_committed_alone_between_surrounding_batches() -> TestResult<()> {
        let context = TestContext::new().await?;
        let oversized = vec![b'o'; usize::try_from(MAX_BATCH_BYTES + 1)?];
        let oversized_hash = BlobHash::new(&oversized);
        let mut lower = None;
        let mut upper = None;
        for index in 0_u32..10_000 {
            let bytes = format!("surrounding-{index}").into_bytes();
            let hash = BlobHash::new(&bytes);
            if hash < oversized_hash && lower.is_none() {
                lower = Some(bytes);
            } else if hash > oversized_hash && upper.is_none() {
                upper = Some(bytes);
            }
            if lower.is_some() && upper.is_some() {
                break;
            }
        }
        let lower = lower.expect("search should find a hash below the oversized value");
        let upper = upper.expect("search should find a hash above the oversized value");
        context.put_blob(&lower).await?;
        context.put_blob(&oversized).await?;
        context.put_blob(&upper).await?;

        let report = context.import().await?;

        assert_eq!(report.imported_rows, 3);
        assert_eq!(report.committed_batches, 3);
        Ok(())
    }

    #[tokio::test]
    async fn passive_checkpoint_runs_once_after_crossing_each_batch_threshold() -> TestResult<()> {
        let crossing_context = TestContext::new().await?;
        crossing_context
            .put_blob(&vec![b'c'; usize::try_from(PASSIVE_CHECKPOINT_BYTES + 1)?])
            .await?;
        let crossing = crossing_context.import().await?;
        assert_eq!(crossing.passive_checkpoints, 1);

        let multi_context = TestContext::new().await?;
        multi_context
            .put_blob(&vec![
                b'm';
                usize::try_from(PASSIVE_CHECKPOINT_BYTES * 2 + 1)?
            ])
            .await?;
        let multiple = multi_context.import().await?;
        assert_eq!(multiple.passive_checkpoints, 1);
        Ok(())
    }

    #[tokio::test]
    async fn passive_checkpoint_failure_returns_durable_partial_report() -> TestResult<()> {
        let context = TestContext::new().await?;
        context
            .put_blob(&vec![b'p'; usize::try_from(PASSIVE_CHECKPOINT_BYTES + 1)?])
            .await?;
        let controls = ImportControls {
            passive_checkpoint: true,
            ..ImportControls::default()
        };

        let error = context
            .source
            .import_legacy_blobs_with_controls(&context.target, &controls)
            .await
            .expect_err("injected passive checkpoint should fail import");

        assert_eq!(error.report().imported_rows, 1);
        assert_eq!(error.report().committed_batches, 1);
        assert_eq!(error.report().passive_checkpoints, 0);
        assert!(matches!(
            &error.failure,
            LegacyBlobImportFailure::PassiveCheckpoint(sqlx::Error::Protocol(_))
        ));
        let failure_source = std::error::Error::source(&error)
            .and_then(std::error::Error::source)
            .expect("checkpoint failure should preserve its SQL source");
        assert!(failure_source.downcast_ref::<sqlx::Error>().is_some());
        assert_eq!(context.destination_rows().await?, 1);
        Ok(())
    }

    #[tokio::test]
    async fn final_checkpoint_failure_returns_committed_progress() -> TestResult<()> {
        let context = TestContext::new().await?;
        context
            .put_blob(b"committed-before-final-checkpoint")
            .await?;
        let controls = ImportControls {
            final_checkpoint: true,
            ..ImportControls::default()
        };

        let error = context
            .source
            .import_legacy_blobs_with_controls(&context.target, &controls)
            .await
            .expect_err("injected final checkpoint should fail import");

        assert_eq!(error.report().imported_rows, 1);
        assert_eq!(error.report().committed_batches, 1);
        assert!(matches!(
            &error.failure,
            LegacyBlobImportFailure::FinalCheckpoint(sqlx::Error::Protocol(_))
        ));
        assert_eq!(context.destination_rows().await?, 1);
        Ok(())
    }

    #[tokio::test]
    async fn source_transport_failure_preserves_commits_and_typed_source() -> TestResult<()> {
        let context = TestContext::new().await?;
        seed_blobs(&context, 101).await?;
        let controls = ImportControls {
            source_after_rows: Some(101),
            ..ImportControls::default()
        };

        let error = context
            .source
            .import_legacy_blobs_with_controls(&context.target, &controls)
            .await
            .expect_err("injected source transport failure should stop import");

        assert_eq!(error.report().scanned_rows, 101);
        assert_eq!(error.report().imported_rows, 100);
        assert_eq!(error.report().committed_batches, 1);
        assert!(matches!(
            &error.failure,
            LegacyBlobImportFailure::ReadSourceScan(_)
        ));
        let failure_source = std::error::Error::source(&error)
            .and_then(std::error::Error::source)
            .expect("transport failure should preserve its SlateDB source");
        assert!(failure_source.downcast_ref::<slatedb::Error>().is_some());

        let retry = context.import().await?;
        assert_eq!(retry.existing_rows, 100);
        assert_eq!(retry.imported_rows, 1);
        assert_eq!(context.destination_rows().await?, 101);
        Ok(())
    }

    #[tokio::test]
    async fn automatic_checkpoint_setting_is_restored_after_success_and_failure() -> TestResult<()>
    {
        let success = TestContext::new().await?;
        success.set_automatic_checkpoint(37).await?;
        success.put_blob(b"success").await?;
        success.import().await?;
        assert_eq!(success.automatic_checkpoint().await?, 37);

        let failure = TestContext::new().await?;
        failure.set_automatic_checkpoint(41).await?;
        let mut invalid_key = legacy_prefix();
        invalid_key.extend_from_slice(&[b'z'; 64]);
        failure.put_raw(invalid_key, b"invalid").await?;
        failure
            .source
            .import_legacy_blobs_into(&failure.target)
            .await
            .expect_err("invalid source should fail import");
        assert_eq!(failure.automatic_checkpoint().await?, 41);
        Ok(())
    }

    #[tokio::test]
    async fn failed_connection_restoration_retires_the_connection() -> TestResult<()> {
        let context = TestContext::new().await?;
        context.set_automatic_checkpoint(43).await?;
        context.put_blob(b"committed-before-restore").await?;
        let controls = ImportControls {
            restore_automatic_checkpoint: true,
            ..ImportControls::default()
        };

        let error = context
            .source
            .import_legacy_blobs_with_controls(&context.target, &controls)
            .await
            .expect_err("injected restoration failure should fail import");

        assert_eq!(error.report().imported_rows, 1);
        assert!(matches!(
            &error.failure,
            LegacyBlobImportFailure::RestoreAutomaticCheckpoint {
                source: sqlx::Error::Protocol(_),
                ..
            }
        ));
        assert_ne!(context.automatic_checkpoint().await?, 0);
        Ok(())
    }

    #[tokio::test]
    async fn slate_backed_target_is_rejected_before_scanning() -> TestResult<()> {
        let context = TestContext::new().await?;
        context.put_blob(b"never-scanned").await?;
        let slate_target = context.source.blobs().await?;

        let error = context
            .source
            .import_legacy_blobs_into(&slate_target)
            .await
            .expect_err("Slate target should be rejected");

        assert_eq!(*error.report(), LegacyBlobImportReport::default());
        assert!(matches!(
            &error.failure,
            LegacyBlobImportFailure::WrongTargetBackend
        ));
        assert_eq!(context.destination_rows().await?, 0);
        Ok(())
    }

    #[tokio::test]
    async fn logs_and_error_rendering_expose_counts_but_not_row_data() -> TestResult<()> {
        let context = TestContext::new().await?;
        let sensitive_key_fragment = "sensitive-invalid-legacy-key";
        let sensitive_content = b"sensitive-blob-content";
        let mut key = legacy_prefix();
        key.extend_from_slice(sensitive_key_fragment.as_bytes());
        context.put_raw(key, sensitive_content).await?;
        let capture = CapturedEvents::default();

        let error = context
            .source
            .import_legacy_blobs_into(&context.target)
            .with_subscriber(capture.clone())
            .await
            .expect_err("invalid key should fail import");

        let rendered = format!("{error} {error:?}");
        let events = capture.events().join("\n");
        for output in [&rendered, &events] {
            assert!(!output.contains(sensitive_key_fragment));
            assert!(!output.contains(std::str::from_utf8(sensitive_content)?));
        }
        assert!(events.contains("scanned_rows=1"), "captured: {events}");
        assert!(events.contains("invalid_rows=1"), "captured: {events}");
        assert!(
            events.contains("failure_kind=\"invalid_source_key\""),
            "captured: {events}"
        );
        assert_eq!(capture.events().len(), 1);
        Ok(())
    }

    #[derive(Clone, Default)]
    struct CapturedEvents {
        events:       Arc<Mutex<Vec<String>>>,
        next_span_id: Arc<AtomicU64>,
    }

    impl CapturedEvents {
        fn events(&self) -> Vec<String> {
            self.events
                .lock()
                .expect("captured tracing events mutex should not be poisoned")
                .clone()
        }
    }

    impl Subscriber for CapturedEvents {
        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _span: &Attributes<'_>) -> Id {
            Id::from_u64(self.next_span_id.fetch_add(1, Ordering::Relaxed) + 1)
        }

        fn record(&self, _span: &Id, _values: &Record<'_>) {}

        fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

        fn event(&self, event: &Event<'_>) {
            let mut visitor = CapturedFields::default();
            event.record(&mut visitor);
            self.events
                .lock()
                .expect("captured tracing events mutex should not be poisoned")
                .push(visitor.output);
        }

        fn enter(&self, _span: &Id) {}

        fn exit(&self, _span: &Id) {}
    }

    #[derive(Default)]
    struct CapturedFields {
        output: String,
    }

    impl Visit for CapturedFields {
        fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
            write!(&mut self.output, "{}={value:?};", field.name())
                .expect("writing tracing fields to String cannot fail");
        }
    }
}
