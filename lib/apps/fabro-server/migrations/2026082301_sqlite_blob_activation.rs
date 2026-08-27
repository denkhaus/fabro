//! Fail-closed activation of SQLite blob storage.
//!
//! This compatibility bridge remains until at least 30 calendar days after
//! the first successful production activation, and until the cold-start,
//! warm-restart, production-observation, and backup-integrity evidence is
//! complete and Scott explicitly approves its removal. The date is an
//! eligibility floor, never an automatic deletion trigger.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures_util::TryStreamExt as _;
use object_store::ObjectStore;
use sqlx::Connection as _;
use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection};
use tokio::fs;
use tokio::task::{JoinError, spawn_blocking};
use tracing::{debug, info, warn};

use crate::server::resource_sampler;

/// Earliest date this bridge becomes eligible for removal, assuming the first
/// production activation happens no earlier than this change ships. Removal
/// additionally requires the evidence and explicit approval described in the
/// module docs; the date alone never triggers deletion.
pub(crate) const REMOVAL_DEADLINE: &str = "2026-09-22";

const DISK_HEADROOM_BYTES: u64 = 64 * 1024 * 1024;
const BACKUP_SUFFIX: &str = ".pre-blob-activation.bak";
const STAGING_SUFFIX: &str = ".tmp";

#[derive(Debug, thiserror::Error)]
pub(crate) enum BlobActivationError {
    #[error("canonicalizing the SQLite database path {path}")]
    Canonicalize {
        path:   PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("inventorying the legacy blob source")]
    Inventory(#[source] fabro_store::LegacyBlobInventoryError),
    #[error("reading activation backup metadata at {path}")]
    BackupMetadata {
        path:   PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("activation backup is not a regular file at {path}")]
    BackupNotRegular { path: PathBuf },
    #[error("activation backup permissions are not private at {path}")]
    BackupNotPrivate { path: PathBuf },
    #[error(
        "activation backup is missing at {path} while {existing_rows} of {legacy_rows} legacy blob rows are already present in SQLite"
    )]
    MissingBackupAfterImport {
        path:          PathBuf,
        legacy_rows:   u64,
        existing_rows: u64,
    },
    #[error("opening or checking activation backup integrity at {path}")]
    BackupIntegrity {
        path:   PathBuf,
        #[source]
        source: sqlx::Error,
    },
    #[error("activation backup integrity check did not return exactly one ok result at {path}")]
    BackupIntegrityFailed { path: PathBuf },
    #[error("reading SQLite file metadata at {path}")]
    SqliteMetadata {
        path:   PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("the blob activation disk requirement overflowed")]
    DiskRequirementOverflow,
    #[error(
        "insufficient disk space for blob activation: {available_bytes} bytes available, {required_bytes} required"
    )]
    InsufficientDisk {
        required_bytes:  u64,
        available_bytes: u64,
    },
    #[error("staging the pre-activation SQLite backup")]
    StageBackup(#[source] fabro_db::SnapshotStagingError),
    #[error("joining the activation backup publication task")]
    JoinBackupPublication(#[source] JoinError),
    #[error("publishing the activation backup at {path} without overwriting")]
    PublishBackup {
        path:   PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("importing legacy blobs into SQLite")]
    Import(#[source] Box<fabro_store::LegacyBlobImportError>),
    #[error("verifying legacy and SQLite blobs")]
    Verification(#[source] Box<fabro_store::LegacyBlobVerificationError>),
    #[error("running the live SQLite integrity check")]
    LiveIntegrity(#[source] sqlx::Error),
    #[error("the live SQLite integrity check did not return exactly one ok result")]
    LiveIntegrityFailed,
    #[error("running the final SQLite WAL truncate checkpoint")]
    FinalCheckpoint(#[source] sqlx::Error),
}

pub(crate) async fn activate_blob_storage(
    database: &fabro_db::Database,
    sqlite_path: &Path,
    object_store: Arc<dyn ObjectStore>,
    slatedb_prefix: String,
    flush_interval: Duration,
    cache_path: Option<PathBuf>,
) -> Result<Arc<fabro_store::Database>, BlobActivationError> {
    let canonical_path = fs::canonicalize(sqlite_path).await.map_err(|source| {
        BlobActivationError::Canonicalize {
            path: sqlite_path.to_path_buf(),
            source,
        }
    })?;
    let backup_path = fabro_db::append_to_path(&canonical_path, BACKUP_SUFFIX);
    info!(
        database_path = %canonical_path.display(),
        backup_path = %backup_path.display(),
        "Starting SQLite blob storage activation"
    );

    let blob_store = Arc::new(fabro_store::BlobStore::new(database.clone_pool()));
    let run_summary_store = Arc::new(fabro_store::RunSummaryStore::new(database.clone_pool()));
    let store = Arc::new(fabro_store::Database::new(
        object_store,
        slatedb_prefix,
        flush_interval,
        cache_path,
        Arc::clone(&blob_store),
        run_summary_store,
    ));

    let inventory = store
        .legacy_blob_inventory(database.pool())
        .await
        .map_err(BlobActivationError::Inventory)?;
    let backup_exists = backup_exists(&backup_path).await?;
    if backup_exists {
        validate_backup(&backup_path).await?;
    }
    if !backup_exists && inventory.pending_rows < inventory.rows {
        return Err(BlobActivationError::MissingBackupAfterImport {
            path:          backup_path,
            legacy_rows:   inventory.rows,
            existing_rows: inventory.rows - inventory.pending_rows,
        });
    }
    let backup_required = inventory.rows > 0 && !backup_exists;
    // The resource sampler treats a path with no matching mount as an
    // unsupported-but-benign condition (tmpfs or squashfs roots, network
    // filesystems, an unreadable mount table), so the preflight does too:
    // skipping the capacity check must not block a boot the import itself
    // could complete.
    if let Some(available_free_bytes) = resource_sampler::available_space_for_path(&canonical_path)
    {
        let backup_reserve = if backup_required {
            sqlite_file_set_bytes(&canonical_path).await?
        } else {
            0
        };
        // Only the rows the import still has to copy need new space; rows
        // already present in SQLite cost nothing on a warm restart.
        let required_free_bytes = compute_disk_preflight(
            inventory.pending_bytes,
            backup_reserve,
            available_free_bytes,
        )?;
        debug!(
            legacy_rows = inventory.rows,
            legacy_bytes = inventory.bytes,
            pending_rows = inventory.pending_rows,
            pending_bytes = inventory.pending_bytes,
            backup_required,
            backup_reserve,
            required_free_bytes,
            available_free_bytes,
            "Checked SQLite blob activation disk capacity"
        );
    } else {
        warn!(
            database_path = %canonical_path.display(),
            "No filesystem mount matched the SQLite database path; skipping the blob activation disk preflight"
        );
    }

    let retained_backup = if backup_exists {
        Some(backup_path)
    } else if backup_required {
        create_backup(database.pool(), &backup_path).await?;
        Some(backup_path)
    } else {
        None
    };

    let import = store
        .import_legacy_blobs_into(database.pool())
        .await
        .map_err(|source| BlobActivationError::Import(Box::new(source)))?;
    // The import pass already validates every legacy digest and byte-compares
    // every already-present row on each boot, so the independent verification
    // sweep only needs to double-check boots that actually inserted rows.
    let verification = if import.imported_rows > 0 {
        Some(
            store
                .verify_legacy_blobs_in(database.pool())
                .await
                .map_err(|source| BlobActivationError::Verification(Box::new(source)))?,
        )
    } else {
        None
    };
    validate_live_integrity(database.pool()).await?;
    final_truncate_checkpoint(database.pool()).await?;

    info!(
        legacy_rows = inventory.rows,
        legacy_bytes = inventory.bytes,
        imported_rows = import.imported_rows,
        existing_rows = import.existing_rows,
        matched_rows = verification.as_ref().map(|report| report.matched_rows),
        target_rows = verification.as_ref().map(|report| report.target_rows),
        passive_checkpoints = import.passive_checkpoints,
        backup_required,
        backup_path = ?retained_backup,
        removal_deadline = REMOVAL_DEADLINE,
        "Activated SQLite blob storage"
    );
    Ok(store)
}

/// Fail-closed disk capacity check; returns the required free bytes.
fn compute_disk_preflight(
    pending_bytes: u64,
    backup_reserve: u64,
    available_free_bytes: u64,
) -> Result<u64, BlobActivationError> {
    let half = pending_bytes
        .checked_add(1)
        .ok_or(BlobActivationError::DiskRequirementOverflow)?
        / 2;
    let required_free_bytes = backup_reserve
        .checked_add(pending_bytes)
        .and_then(|value| value.checked_add(half))
        .and_then(|value| value.checked_add(DISK_HEADROOM_BYTES))
        .ok_or(BlobActivationError::DiskRequirementOverflow)?;
    if available_free_bytes < required_free_bytes {
        return Err(BlobActivationError::InsufficientDisk {
            required_bytes:  required_free_bytes,
            available_bytes: available_free_bytes,
        });
    }
    Ok(required_free_bytes)
}

async fn backup_exists(path: &Path) -> Result<bool, BlobActivationError> {
    match fs::metadata(path).await {
        Ok(_) => Ok(true),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(BlobActivationError::BackupMetadata {
            path: path.to_path_buf(),
            source,
        }),
    }
}

async fn sqlite_file_set_bytes(path: &Path) -> Result<u64, BlobActivationError> {
    let mut total = required_file_bytes(path).await?;
    for suffix in ["-wal", "-shm"] {
        let sibling = fabro_db::append_to_path(path, suffix);
        let bytes = optional_file_bytes(&sibling).await?;
        total = total
            .checked_add(bytes)
            .ok_or(BlobActivationError::DiskRequirementOverflow)?;
    }
    Ok(total)
}

async fn required_file_bytes(path: &Path) -> Result<u64, BlobActivationError> {
    fs::metadata(path)
        .await
        .map(|metadata| metadata.len())
        .map_err(|source| BlobActivationError::SqliteMetadata {
            path: path.to_path_buf(),
            source,
        })
}

async fn optional_file_bytes(path: &Path) -> Result<u64, BlobActivationError> {
    match fs::metadata(path).await {
        Ok(metadata) => Ok(metadata.len()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(source) => Err(BlobActivationError::SqliteMetadata {
            path: path.to_path_buf(),
            source,
        }),
    }
}

async fn create_backup(
    pool: &sqlx::SqlitePool,
    backup_path: &Path,
) -> Result<(), BlobActivationError> {
    let staging_path = fabro_db::append_to_path(backup_path, STAGING_SUFFIX);
    fabro_db::write_snapshot_to_staging(pool, &staging_path)
        .await
        .map_err(BlobActivationError::StageBackup)?;
    validate_backup(&staging_path).await?;

    let publish_staging = staging_path.clone();
    let publish_backup = backup_path.to_path_buf();
    let already_exists = spawn_blocking(move || {
        let staging = tempfile::TempPath::from_path(publish_staging);
        match staging.persist_noclobber(&publish_backup) {
            Ok(()) => {
                // Make the rename's directory entry durable: the retained
                // backup is the documented rollback artifact, so it must not
                // vanish in a crash after the import has already committed.
                fabro_db::sync_parent_directory(&publish_backup)?;
                Ok(false)
            }
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(true),
            Err(error) => Err(error.error),
        }
    })
    .await
    .map_err(BlobActivationError::JoinBackupPublication)?
    .map_err(|source| BlobActivationError::PublishBackup {
        path: backup_path.to_path_buf(),
        source,
    })?;

    // The staging copy was validated just before the atomic rename, so only a
    // concurrently published file still needs its own validation.
    if already_exists {
        debug!(
            backup_path = %backup_path.display(),
            "Reusing concurrently published SQLite blob activation backup"
        );
        validate_backup(backup_path).await?;
    }
    Ok(())
}

async fn validate_backup(path: &Path) -> Result<(), BlobActivationError> {
    let metadata =
        fs::symlink_metadata(path)
            .await
            .map_err(|source| BlobActivationError::BackupMetadata {
                path: path.to_path_buf(),
                source,
            })?;
    if !metadata.is_file() {
        return Err(BlobActivationError::BackupNotRegular {
            path: path.to_path_buf(),
        });
    }
    validate_private_permissions(path, &metadata)?;

    let options = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .immutable(true)
        .create_if_missing(false);
    let mut connection = SqliteConnection::connect_with(&options)
        .await
        .map_err(|source| BlobActivationError::BackupIntegrity {
            path: path.to_path_buf(),
            source,
        })?;
    let ok = integrity_check_is_ok(&mut connection)
        .await
        .map_err(|source| BlobActivationError::BackupIntegrity {
            path: path.to_path_buf(),
            source,
        })?;
    if !ok {
        return Err(BlobActivationError::BackupIntegrityFailed {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

/// Returns whether `PRAGMA integrity_check` reports exactly one `ok` row.
async fn integrity_check_is_ok<'a, E>(executor: E) -> Result<bool, sqlx::Error>
where
    E: sqlx::Executor<'a, Database = sqlx::Sqlite>,
{
    let mut rows = sqlx::query_scalar::<_, String>("PRAGMA integrity_check").fetch(executor);
    let first = rows.try_next().await?;
    let second = rows.try_next().await?;
    Ok(first.as_deref() == Some("ok") && second.is_none())
}

#[cfg(unix)]
fn validate_private_permissions(
    path: &Path,
    metadata: &std::fs::Metadata,
) -> Result<(), BlobActivationError> {
    use std::os::unix::fs::PermissionsExt as _;

    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(BlobActivationError::BackupNotPrivate {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_permissions(
    _path: &Path,
    _metadata: &std::fs::Metadata,
) -> Result<(), BlobActivationError> {
    Ok(())
}

async fn validate_live_integrity(pool: &sqlx::SqlitePool) -> Result<(), BlobActivationError> {
    let ok = integrity_check_is_ok(pool)
        .await
        .map_err(BlobActivationError::LiveIntegrity)?;
    if !ok {
        return Err(BlobActivationError::LiveIntegrityFailed);
    }
    Ok(())
}

async fn final_truncate_checkpoint(pool: &sqlx::SqlitePool) -> Result<(), BlobActivationError> {
    let (busy, _, _): (i64, i64, i64) = sqlx::query_as("PRAGMA wal_checkpoint(TRUNCATE)")
        .fetch_one(pool)
        .await
        .map_err(BlobActivationError::FinalCheckpoint)?;
    if busy != 0 {
        // A concurrent reader (a backup tool, a replication agent, an
        // operator shell) can keep the WAL from truncating. An untruncated
        // WAL threatens no data integrity, so it must not block startup; a
        // later checkpoint truncates once the reader is gone.
        warn!("The final SQLite WAL truncate checkpoint could not complete; continuing startup");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use fabro_db::append_to_path;
    use object_store::ObjectStore;
    use object_store::memory::InMemory;
    use tokio::fs;

    use super::{
        BACKUP_SUFFIX, BlobActivationError, DISK_HEADROOM_BYTES, activate_blob_storage,
        compute_disk_preflight, create_backup, final_truncate_checkpoint, sqlite_file_set_bytes,
        validate_backup,
    };

    type TestResult<T> = Result<T, Box<dyn std::error::Error>>;

    #[test]
    fn disk_preflight_passes_at_equality_and_fails_one_byte_below() {
        let pending_bytes = 3;
        let backup_reserve = 10;
        let required = backup_reserve + pending_bytes + 2 + DISK_HEADROOM_BYTES;

        let required_free_bytes = compute_disk_preflight(pending_bytes, backup_reserve, required)
            .expect("exact equality must pass");
        assert_eq!(required_free_bytes, required);

        let error = compute_disk_preflight(pending_bytes, backup_reserve, required - 1)
            .expect_err("one byte below must fail");
        assert!(matches!(
            error,
            BlobActivationError::InsufficientDisk { .. }
        ));
    }

    #[test]
    fn disk_preflight_requires_only_headroom_without_a_backup_reserve() {
        let required_free_bytes =
            compute_disk_preflight(2, 0, u64::MAX).expect("available capacity should pass");
        assert_eq!(required_free_bytes, 3 + DISK_HEADROOM_BYTES);
    }

    #[test]
    fn disk_preflight_fails_closed_on_overflow() {
        let error =
            compute_disk_preflight(u64::MAX, 1, u64::MAX).expect_err("overflow must fail closed");
        assert!(matches!(
            error,
            BlobActivationError::DiskRequirementOverflow
        ));
    }

    #[tokio::test]
    async fn disk_preflight_counts_the_sqlite_file_set_for_a_required_backup() -> TestResult<()> {
        let directory = tempfile::tempdir()?;
        let sqlite_path = directory.path().join("fabro.sqlite3");
        fs::write(&sqlite_path, [0_u8; 3]).await?;
        fs::write(append_to_path(&sqlite_path, "-wal"), [0_u8; 5]).await?;
        fs::write(append_to_path(&sqlite_path, "-shm"), [0_u8; 7]).await?;

        assert_eq!(sqlite_file_set_bytes(&sqlite_path).await?, 15);
        Ok(())
    }

    #[tokio::test]
    async fn backup_is_private_integrity_clean_and_does_not_create_journal_siblings()
    -> TestResult<()> {
        let directory = tempfile::tempdir()?;
        let sqlite_path = directory.path().join("fabro.sqlite3");
        let database = fabro_db::Database::connect(&sqlite_path).await?;
        database.migrate().await?;
        let backup_path = append_to_path(&sqlite_path, BACKUP_SUFFIX);

        create_backup(database.pool(), &backup_path).await?;
        validate_backup(&backup_path).await?;

        assert!(backup_path.is_file());
        assert!(!append_to_path(&backup_path, "-wal").exists());
        assert!(!append_to_path(&backup_path, "-shm").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(&backup_path)?.permissions().mode() & 0o077,
                0
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn backup_publication_never_overwrites_an_existing_valid_backup() -> TestResult<()> {
        let directory = tempfile::tempdir()?;
        let sqlite_path = directory.path().join("fabro.sqlite3");
        let database = fabro_db::Database::connect(&sqlite_path).await?;
        database.migrate().await?;
        let backup_path = append_to_path(&sqlite_path, BACKUP_SUFFIX);
        create_backup(database.pool(), &backup_path).await?;
        let original = fs::read(&backup_path).await?;

        sqlx::query("INSERT INTO blobs (hash, data) VALUES (?, ?)")
            .bind(fabro_types::BlobHash::new(b"later").to_string())
            .bind(b"later".as_slice())
            .execute(database.pool())
            .await?;
        create_backup(database.pool(), &backup_path).await?;

        assert_eq!(fs::read(&backup_path).await?, original);
        Ok(())
    }

    #[tokio::test]
    async fn failed_backup_copy_never_publishes_a_destination() -> TestResult<()> {
        let directory = tempfile::tempdir()?;
        let sqlite_path = directory.path().join("fabro.sqlite3");
        let database = fabro_db::Database::connect(&sqlite_path).await?;
        database.migrate().await?;
        let backup_path = append_to_path(&sqlite_path, BACKUP_SUFFIX);
        database.pool().close().await;

        let error = create_backup(database.pool(), &backup_path)
            .await
            .expect_err("a closed pool must fail backup creation");

        assert!(matches!(
            error,
            BlobActivationError::StageBackup(fabro_db::SnapshotStagingError::Write { .. })
        ));
        assert!(!backup_path.exists());
        Ok(())
    }

    #[tokio::test]
    async fn cold_activation_and_warm_restart_share_verified_sqlite_blobs() -> TestResult<()> {
        let directory = tempfile::tempdir()?;
        let sqlite_path = directory.path().join("fabro.sqlite3");
        let database = fabro_db::Database::connect(&sqlite_path).await?;
        database.migrate().await?;
        let object_store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let source = fabro_store::test_support::test_database(
            Arc::clone(&object_store),
            "activation-test",
            Duration::from_millis(1),
            None,
        );
        let legacy_bytes = b"legacy-blob";
        let legacy_hash = fabro_store::test_support::put_legacy_blob(&source, legacy_bytes).await?;
        drop(source);

        let store = activate_blob_storage(
            &database,
            &sqlite_path,
            Arc::clone(&object_store),
            "activation-test".to_string(),
            Duration::from_millis(1),
            None,
        )
        .await?;
        assert_eq!(
            store.blobs().read(&legacy_hash).await?.as_deref(),
            Some(legacy_bytes.as_slice())
        );

        let backup_path = append_to_path(&sqlite_path, BACKUP_SUFFIX);
        let original_backup = fs::read(&backup_path).await?;
        let run_id = fabro_types::RunId::new();
        let writer = store.create_run(&run_id).await?;
        let reader = store.open_run_reader(&run_id).await?;
        let sqlite_only_bytes = b"written-after-activation";
        let sqlite_only_hash = writer.write_blob(sqlite_only_bytes).await?;
        assert_eq!(
            reader.read_blob(&sqlite_only_hash).await?.as_deref(),
            Some(sqlite_only_bytes.as_slice())
        );
        drop(reader);
        drop(writer);
        drop(store);

        let warm = activate_blob_storage(
            &database,
            &sqlite_path,
            object_store,
            "activation-test".to_string(),
            Duration::from_millis(1),
            None,
        )
        .await?;
        assert_eq!(fs::read(&backup_path).await?, original_backup);
        assert_eq!(
            warm.blobs().read(&sqlite_only_hash).await?.as_deref(),
            Some(sqlite_only_bytes.as_slice())
        );
        Ok(())
    }

    #[tokio::test]
    async fn missing_backup_after_prior_import_fails_closed() -> TestResult<()> {
        let directory = tempfile::tempdir()?;
        let sqlite_path = directory.path().join("fabro.sqlite3");
        let database = fabro_db::Database::connect(&sqlite_path).await?;
        database.migrate().await?;
        let object_store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let source = fabro_store::test_support::test_database(
            Arc::clone(&object_store),
            "missing-backup-test",
            Duration::from_millis(1),
            None,
        );
        let bytes = b"already-imported";
        let hash = fabro_store::test_support::put_legacy_blob(&source, bytes).await?;
        sqlx::query("INSERT INTO blobs (hash, data) VALUES (?, ?)")
            .bind(hash.to_string())
            .bind(bytes.as_slice())
            .execute(database.pool())
            .await?;
        drop(source);

        let error = activate_blob_storage(
            &database,
            &sqlite_path,
            object_store,
            "missing-backup-test".to_string(),
            Duration::from_millis(1),
            None,
        )
        .await
        .expect_err("startup must not move the pre-activation rollback boundary");

        assert!(matches!(
            error,
            BlobActivationError::MissingBackupAfterImport {
                legacy_rows: 1,
                existing_rows: 1,
                ..
            }
        ));
        assert!(!append_to_path(&sqlite_path, BACKUP_SUFFIX).exists());
        Ok(())
    }

    #[tokio::test]
    async fn empty_inventory_skips_backup_and_serves_existing_sqlite_rows() -> TestResult<()> {
        let directory = tempfile::tempdir()?;
        let sqlite_path = directory.path().join("fabro.sqlite3");
        let database = fabro_db::Database::connect(&sqlite_path).await?;
        database.migrate().await?;
        let bytes = b"sqlite-only";
        let hash = fabro_types::BlobHash::new(bytes);
        sqlx::query("INSERT INTO blobs (hash, data) VALUES (?, ?)")
            .bind(hash.to_string())
            .bind(bytes.as_slice())
            .execute(database.pool())
            .await?;

        let activated = activate_blob_storage(
            &database,
            &sqlite_path,
            Arc::new(InMemory::new()),
            "empty-activation-test".to_string(),
            Duration::from_millis(1),
            None,
        )
        .await?;

        assert!(!append_to_path(&sqlite_path, BACKUP_SUFFIX).exists());
        assert_eq!(
            activated.blobs().read(&hash).await?.as_deref(),
            Some(bytes.as_slice())
        );
        Ok(())
    }

    #[tokio::test]
    async fn busy_final_checkpoint_warns_and_does_not_fail_startup() -> TestResult<()> {
        use sqlx::Connection as _;
        use sqlx::sqlite::{
            SqliteConnectOptions, SqliteConnection, SqliteJournalMode, SqlitePoolOptions,
        };

        let directory = tempfile::tempdir()?;
        let sqlite_path = directory.path().join("fabro.sqlite3");
        let database = fabro_db::Database::connect(&sqlite_path).await?;
        database.migrate().await?;
        sqlx::query("INSERT INTO blobs (hash, data) VALUES (?, ?)")
            .bind(fabro_types::BlobHash::new(b"wal-content").to_string())
            .bind(b"wal-content".as_slice())
            .execute(database.pool())
            .await?;

        // A reader holding an open snapshot models a backup tool or operator
        // shell that outlives the checkpoint's busy timeout.
        let reader_options = SqliteConnectOptions::new()
            .filename(&sqlite_path)
            .read_only(true)
            .create_if_missing(false);
        let mut reader = SqliteConnection::connect_with(&reader_options).await?;
        sqlx::query("BEGIN").execute(&mut reader).await?;
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM blobs")
            .fetch_one(&mut reader)
            .await?;

        // A short busy timeout keeps the blocked truncate from stalling the
        // test for the production pool's full five seconds.
        let checkpoint_options = SqliteConnectOptions::new()
            .filename(&sqlite_path)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_millis(50))
            .create_if_missing(false);
        let checkpoint_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(checkpoint_options)
            .await?;

        final_truncate_checkpoint(&checkpoint_pool).await?;

        // The reader really did block the truncate: the WAL was not reset.
        let wal_bytes = fs::metadata(append_to_path(&sqlite_path, "-wal"))
            .await?
            .len();
        assert!(wal_bytes > 0, "the WAL should remain untruncated");
        drop(reader);
        Ok(())
    }

    #[tokio::test]
    async fn invalid_retained_backup_fails_before_importing() -> TestResult<()> {
        let directory = tempfile::tempdir()?;
        let sqlite_path = directory.path().join("fabro.sqlite3");
        let database = fabro_db::Database::connect(&sqlite_path).await?;
        database.migrate().await?;
        let object_store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let source = fabro_store::test_support::test_database(
            Arc::clone(&object_store),
            "invalid-backup-test",
            Duration::from_millis(1),
            None,
        );
        fabro_store::test_support::put_legacy_blob(&source, b"must-not-import").await?;
        drop(source);

        let backup_path = append_to_path(&sqlite_path, BACKUP_SUFFIX);
        fs::write(&backup_path, b"not a database").await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&backup_path, std::fs::Permissions::from_mode(0o600)).await?;
        }

        let error = activate_blob_storage(
            &database,
            &sqlite_path,
            object_store,
            "invalid-backup-test".to_string(),
            Duration::from_millis(1),
            None,
        )
        .await
        .expect_err("an invalid retained backup must fail closed");
        assert!(matches!(
            error,
            BlobActivationError::BackupIntegrity { .. }
                | BlobActivationError::BackupIntegrityFailed { .. }
        ));
        let destination_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM blobs")
            .fetch_one(database.pool())
            .await?;
        assert_eq!(destination_rows, 0);
        Ok(())
    }
}
