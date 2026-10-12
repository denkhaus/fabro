//! Fork pin (fabro-9dff): the shared SQLite connect options must carry the
//! stream-cache pragmas — mmap_size 2 GiB and a 64 MiB per-connection page
//! cache. Without them every run-stream tail page is a disk read on the
//! multi-GB store while runs write, and concurrent followers/SSE/UI readers
//! starve the pool (2026-10-12 incident: 747% CPU, API timeouts, the web UI
//! unresponsive with "No command output yet"). An upstream merge that drops
//! the pragmas from `Database::connect` fails here.

use sqlx::Row as _;

#[tokio::test]
async fn connect_options_carry_the_stream_cache_pragmas() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let db_path = dir.path().join("fabro.sqlite3");

    let database = fabro_db::Database::connect(&db_path).await?;

    let mmap: i64 = sqlx::query("PRAGMA mmap_size")
        .fetch_one(database.pool())
        .await?
        .get(0);
    let cache: i64 = sqlx::query("PRAGMA cache_size")
        .fetch_one(database.pool())
        .await?
        .get(0);

    // SQLite rounds the requested 2 GiB down to its page alignment; the
    // PRAGMA reads back the EFFECTIVE value (2GiB - 64KiB).
    assert_eq!(
        mmap, 2_147_418_112,
        "mmap_size must stay ~2 GiB — the OS page cache is the stream-read path (fabro-9dff)"
    );
    assert_eq!(
        cache, -65_536,
        "cache_size must stay -65536 KiB (64 MiB per connection) — the default ~2 MiB starves on multi-GB stores (fabro-9dff)"
    );
    Ok(())
}
