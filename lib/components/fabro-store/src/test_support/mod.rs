#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use fabro_types::{BlobHash, RunId};
use object_store::ObjectStore;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

#[cfg(test)]
use crate::{AuthSessionStore, RunSummaryStore};
use crate::keys::SlateKey;
use crate::{BlobStore, Database, Result};

/// Returns the process-wide SQLite blob authority used by Slate-backed tests.
///
/// Production has one blob authority shared by every run handle. Keeping the
/// same shape in test processes also lets helpers reopen the Slate run store
/// without silently switching to an empty blob database. The CAS key is the
/// content hash, so sharing rows across otherwise isolated tests is safe.
///
/// The pool connects lazily so synchronous fixture builders can remain
/// synchronous. Its single connection installs the production blob schema on
/// first use.
#[must_use]
pub fn test_blob_store() -> Arc<BlobStore> {
    static STORE: OnceLock<Arc<BlobStore>> = OnceLock::new();

    Arc::clone(STORE.get_or_init(|| {
        let options = SqliteConnectOptions::new()
            .filename(":memory:")
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            // A single in-memory test connection never needs reaping.
            // Disabling both timers also keeps this lazy fixture constructible
            // from sync tests, where SQLx has no Tokio runtime for maintenance
            // tasks.
            .max_lifetime(None)
            .idle_timeout(None)
            .after_connect(|connection, _metadata| {
                Box::pin(async move {
                    sqlx::query(include_str!(
                        "../../../../foundation/fabro-db/migrations/2026081301_blobs.sql"
                    ))
                    .execute(&mut *connection)
                    .await?;
                    Ok(())
                })
            })
            .connect_lazy_with(options);
        Arc::new(BlobStore::new(pool))
    }))
}

/// Builds a Slate-backed run database using the process-wide test blob
/// authority and its signed SQLite blob table.
pub fn test_database(
    object_store: Arc<dyn ObjectStore>,
    base_prefix: impl Into<String>,
    flush_interval: Duration,
    cache_path: Option<PathBuf>,
) -> Result<Database> {
    Ok(Database::new(
        object_store,
        base_prefix,
        flush_interval,
        cache_path,
        test_blob_store(),
    ))
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
