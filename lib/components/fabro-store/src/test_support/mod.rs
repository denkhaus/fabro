use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use fabro_types::{BlobHash, RunId};
use object_store::ObjectStore;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

#[cfg(test)]
use crate::{AuthSessionStore, RunSummaryStore};
use crate::keys::SlateKey;
use crate::{BlobStore, Database, Result};

/// Returns an isolated SQLite blob authority backed by its own in-memory
/// database.
///
/// Every call creates a fresh blob table, so tests never observe rows written
/// by other tests in the same process. Reopen-style tests that model one
/// process-wide blob authority across several store handles should call this
/// once and share the result through [`test_database_with_blobs`].
///
/// The pool connects lazily so synchronous fixture builders can remain
/// synchronous. Its single connection installs the production blob schema on
/// first use.
#[must_use]
pub fn test_blob_store() -> Arc<BlobStore> {
    let options = SqliteConnectOptions::new()
        .filename(":memory:")
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        // A single in-memory test connection never needs reaping. Disabling
        // both timers also keeps this lazy fixture constructible from sync
        // tests, where SQLx has no Tokio runtime for maintenance tasks.
        .max_lifetime(None)
        .idle_timeout(None)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query(fabro_db::BLOBS_MIGRATION_SQL)
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect_lazy_with(options);
    Arc::new(BlobStore::new(pool))
}

/// Returns the SQLite file backing [`test_blob_store_at`] for `store_dir`.
#[must_use]
pub fn test_blob_store_path(store_dir: &Path) -> PathBuf {
    fabro_db::append_to_path(store_dir, "-blobs.sqlite3")
}

/// Returns a durable SQLite blob authority stored beside `store_dir`.
///
/// Handles created for the same directory share one blob database file, so
/// reopen-style tests observe blobs across store handles the way production
/// handles share the process-wide blob authority. Tests that reuse a
/// directory must delete [`test_blob_store_path`] (and its `-wal`/`-shm`
/// siblings) when they reset the directory itself.
#[must_use]
pub fn test_blob_store_at(store_dir: &Path) -> Arc<BlobStore> {
    let options = SqliteConnectOptions::new()
        .filename(test_blob_store_path(store_dir))
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .max_lifetime(None)
        .idle_timeout(None)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                let installed: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master \
                     WHERE type = 'table' AND name = 'blobs')",
                )
                .fetch_one(&mut *connection)
                .await?;
                if !installed {
                    sqlx::query(fabro_db::BLOBS_MIGRATION_SQL)
                        .execute(&mut *connection)
                        .await?;
                }
                Ok(())
            })
        })
        .connect_lazy_with(options);
    Arc::new(BlobStore::new(pool))
}

/// Builds a Slate-backed run database with its own isolated blob authority.
#[must_use]
pub fn test_database(
    object_store: Arc<dyn ObjectStore>,
    base_prefix: impl Into<String>,
    flush_interval: Duration,
    cache_path: Option<PathBuf>,
) -> Database {
    test_database_with_blobs(
        object_store,
        base_prefix,
        flush_interval,
        cache_path,
        test_blob_store(),
    )
}

/// Builds a Slate-backed run database sharing an explicit blob authority.
///
/// Use this for reopen-style tests where two store handles must observe the
/// same signed SQLite blob table, mirroring the one blob authority a
/// production process shares across every run handle.
#[must_use]
pub fn test_database_with_blobs(
    object_store: Arc<dyn ObjectStore>,
    base_prefix: impl Into<String>,
    flush_interval: Duration,
    cache_path: Option<PathBuf>,
    blobs: Arc<BlobStore>,
) -> Database {
    Database::new(object_store, base_prefix, flush_interval, cache_path, blobs)
}

/// Seeds one canonical row in the legacy SlateDB blob keyspace.
pub async fn put_legacy_blob(database: &Database, bytes: &[u8]) -> Result<BlobHash> {
    let hash = BlobHash::new(bytes);
    let source = database.open_db().await?;
    source
        .put(SlateKey::new("blobs").with("sha256").with(hash), bytes)
        .await?;
    source.flush().await?;
    Ok(hash)
}

/// Writes an event without append validation to model a log corrupted by an
/// older Fabro version.
pub async fn put_unvalidated_run_event(
    database: &Database,
    run_id: &RunId,
    seq: u32,
    payload: &serde_json::Value,
) -> Result<()> {
    database
        .put_unvalidated_run_event(run_id, seq, payload)
        .await
}

#[cfg(test)]
pub(crate) async fn sqlite_auth_session_store() -> (tempfile::TempDir, AuthSessionStore) {
    let directory = tempfile::tempdir().unwrap();
    let database = fabro_db::Database::connect(directory.path().join("fabro.sqlite3"))
        .await
        .unwrap();
    database.migrate().await.unwrap();
    (directory, AuthSessionStore::new(database.clone_pool()))
}

#[cfg(test)]
pub(crate) async fn sqlite_summary_store() -> (tempfile::TempDir, RunSummaryStore) {
    let directory = tempfile::tempdir().unwrap();
    let store = sqlite_summary_store_at(directory.path()).await;
    (directory, store)
}

#[cfg(test)]
pub(crate) async fn sqlite_summary_store_at(directory: &Path) -> RunSummaryStore {
    let database = fabro_db::Database::connect(directory.join("fabro.sqlite3"))
        .await
        .unwrap();
    database.migrate().await.unwrap();
    RunSummaryStore::new(database.clone_pool())
}
