//! Shared test fixtures: an in-memory pool carrying the FULL production
//! migration set, applied by the same `MIGRATOR` the server's migration
//! path runs. Fixtures in other crates bootstrap through this instead of
//! hand-listing migration SQL, so a test schema cannot drift from the
//! production shape (the failure class of denkhaus/work#1421).

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

use crate::{DbPool, MIGRATOR};

/// An in-memory SQLite pool with the full production migration set applied.
///
/// The pool holds a single connection (one in-memory database) and runs the
/// production [`MIGRATOR`] on it, so the fixture's schema is exactly the
/// schema a migrated server database carries.
pub async fn fully_migrated_in_memory_pool() -> anyhow::Result<DbPool> {
    let options = SqliteConnectOptions::new()
        .filename(":memory:")
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        // A single in-memory test connection never needs reaping, and
        // disabling both timers keeps maintenance tasks out of the way.
        .max_lifetime(None)
        .idle_timeout(None)
        .connect_with(options)
        .await?;
    MIGRATOR.run(&pool).await?;
    Ok(pool)
}
