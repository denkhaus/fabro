//! SQLite storage for Ask Fabro conversations.
//!
//! The run event log streams a session's turns live and projects its
//! metadata; the conversation the model sees is pebble's session record,
//! kept here as JSON and written after every turn. Resuming a session reads
//! the record back and continues it on the model it recorded.

use chrono::{DateTime, Utc};
use fabro_types::{RunId, SessionId};
use pebble_coding_agent::state::SessionRecord;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row as _, SqlitePool};

use crate::{Error, Result, sqlite_row};

const RECORD_NAME: &str = "run session record";

/// A stored conversation and when it was last written. `run_id` is `None`
/// for a runless session (work#1677) — the global supervisor's
/// conversations carry no run anchor.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredSessionRecord {
    pub run_id:     Option<RunId>,
    pub record:     SessionRecord,
    pub updated_at: DateTime<Utc>,
}

/// The metadata of a stored conversation, without its JSON payload — the
/// index view `list_for_run` serves (denkhaus/work#1452): a run's sessions
/// and when each was last written, never the record bodies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSessionRecordSummary {
    pub session_id: SessionId,
    pub run_id:     Option<RunId>,
    pub updated_at: DateTime<Utc>,
}

/// Reads and writes pebble session records in SQLite.
pub struct RunSessionRecordStore {
    pool: SqlitePool,
}

impl std::fmt::Debug for RunSessionRecordStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunSessionRecordStore")
            .finish_non_exhaustive()
    }
}

impl RunSessionRecordStore {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Replace the stored record for `session_id`.
    pub async fn put(
        &self,
        session_id: SessionId,
        run_id: Option<RunId>,
        record: &SessionRecord,
        updated_at: DateTime<Utc>,
    ) -> Result<()> {
        let record_json = serde_json::to_string(record)?;
        sqlx::query(
            r"
INSERT INTO run_session_records (session_id, run_id, record_json, updated_at_ms)
VALUES (?, ?, ?, ?)
ON CONFLICT(session_id) DO UPDATE SET
    run_id = excluded.run_id,
    record_json = excluded.record_json,
    updated_at_ms = excluded.updated_at_ms
",
        )
        .bind(session_id.to_string())
        .bind(run_id.map(|run_id| run_id.to_string()))
        .bind(record_json)
        .bind(updated_at.timestamp_millis())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The stored record for `session_id`, if a turn has been persisted.
    pub async fn get(&self, session_id: SessionId) -> Result<Option<StoredSessionRecord>> {
        let row = sqlx::query(
            "SELECT run_id, record_json, updated_at_ms FROM run_session_records WHERE session_id = ?",
        )
        .bind(session_id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(record_from_row).transpose()
    }

    /// Every stored record of `run_id`, as metadata only — the record
    /// payloads stay in the table (denkhaus/work#1452's index seam).
    pub async fn list_for_run(&self, run_id: RunId) -> Result<Vec<StoredSessionRecordSummary>> {
        let rows = sqlx::query(
            r"
SELECT session_id, run_id, updated_at_ms
FROM run_session_records
WHERE run_id = ?
ORDER BY updated_at_ms DESC, session_id ASC
",
        )
        .bind(run_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(summary_from_row).collect()
    }

    /// Forget the stored record for `session_id`.
    pub async fn delete(&self, session_id: SessionId) -> Result<bool> {
        let result = sqlx::query("DELETE FROM run_session_records WHERE session_id = ?")
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Forget every stored record of `run_id` — the deletion-chain mate of
    /// the event store's `delete_for_run`, so run deletion leaves no
    /// orphaned conversations behind.
    pub async fn delete_for_run(&self, run_id: RunId) -> Result<u64> {
        let result = sqlx::query("DELETE FROM run_session_records WHERE run_id = ?")
            .bind(run_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }
}

fn record_from_row(row: &SqliteRow) -> Result<StoredSessionRecord> {
    let run_id: Option<String> = row.try_get("run_id")?;
    let run_id = run_id
        .map(|run_id| {
            run_id.parse::<RunId>().map_err(|error| {
                Error::InvalidEvent(format!("stored {RECORD_NAME} run id: {error}"))
            })
        })
        .transpose()?;
    let record_json: String = row.try_get("record_json")?;
    let record: SessionRecord = serde_json::from_str(&record_json)?;
    let updated_at = sqlite_row::timestamp_from_row(row, RECORD_NAME, "updated_at_ms")?;
    Ok(StoredSessionRecord {
        run_id,
        record,
        updated_at,
    })
}

fn summary_from_row(row: &SqliteRow) -> Result<StoredSessionRecordSummary> {
    let session_id: String = row.try_get("session_id")?;
    let session_id = session_id.parse::<SessionId>().map_err(|error| {
        Error::InvalidEvent(format!("stored {RECORD_NAME} session id: {error}"))
    })?;
    let run_id: Option<String> = row.try_get("run_id")?;
    let run_id = run_id
        .map(|run_id| {
            run_id.parse::<RunId>().map_err(|error| {
                Error::InvalidEvent(format!("stored {RECORD_NAME} run id: {error}"))
            })
        })
        .transpose()?;
    let updated_at = sqlite_row::timestamp_from_row(row, RECORD_NAME, "updated_at_ms")?;
    Ok(StoredSessionRecordSummary {
        session_id,
        run_id,
        updated_at,
    })
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use fabro_types::fixtures;
    use pebble_coding_agent::SessionScope;
    use pebble_coding_agent::state::SESSION_RECORD_FORMAT_VERSION;

    use super::*;
    use crate::test_support;

    fn record(session_id: &SessionId) -> SessionRecord {
        let mut record = SessionRecord::new(SessionScope::root(
            pebble_coding_agent::SessionId::new(session_id.to_string()),
        ));
        record.provider = Some("openai".to_string());
        record.model = Some("gpt-5.4".to_string());
        record.last_event_seq = 7;
        // The record stores timestamps at millisecond precision, so a fixture
        // that expects to read back what it wrote must not carry finer ones.
        let millis = std::time::UNIX_EPOCH + std::time::Duration::from_millis(1_789_156_874_678);
        record.created_at = millis;
        record.updated_at = millis;
        record
    }

    #[tokio::test]
    async fn put_then_get_round_trips_the_record() {
        let store = RunSessionRecordStore::new(test_support::in_memory_pool_with(&[
            fabro_db::RUN_SESSION_RECORDS_MIGRATION_SQL,
            fabro_db::RUN_SESSION_RECORDS_RUNLESS_MIGRATION_SQL,
        ]));
        let session_id = SessionId::new();
        let record = record(&session_id);
        let updated_at = Utc.with_ymd_and_hms(2026, 9, 11, 12, 0, 0).unwrap();

        store
            .put(session_id, Some(fixtures::RUN_1), &record, updated_at)
            .await
            .unwrap();

        let stored = store.get(session_id).await.unwrap().expect("stored record");
        assert_eq!(stored.run_id, Some(fixtures::RUN_1));
        assert_eq!(stored.record, record);
        assert_eq!(stored.updated_at, updated_at);
    }

    /// A record an older build wrote is read back as it was stored: the
    /// store parses the previous format, and pebble's version check, not a
    /// parse error, is what refuses it on resume. Old runs get no migration.
    #[tokio::test]
    async fn get_reads_a_record_in_the_previous_format_for_pebble_to_refuse() {
        const PREVIOUS_RECORD: &str = include_str!("fixtures/session_record_v4.json");

        let pool = test_support::in_memory_pool_with(&[
            fabro_db::RUN_SESSION_RECORDS_MIGRATION_SQL,
            fabro_db::RUN_SESSION_RECORDS_RUNLESS_MIGRATION_SQL,
        ]);
        let store = RunSessionRecordStore::new(pool.clone());
        let session_id = SessionId::new();
        sqlx::query(
            "INSERT INTO run_session_records (session_id, run_id, record_json, updated_at_ms) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(session_id.to_string())
        .bind(fixtures::RUN_1.to_string())
        .bind(PREVIOUS_RECORD)
        .bind(1_789_156_874_678_i64)
        .execute(&pool)
        .await
        .unwrap();

        let stored = store.get(session_id).await.unwrap().expect("stored record");
        assert_eq!(stored.run_id, Some(fixtures::RUN_1));
        assert_eq!(
            stored.record.format_version,
            SESSION_RECORD_FORMAT_VERSION - 1,
            "the fixture is the previous format"
        );
        assert!(
            !stored.record.is_supported(),
            "pebble refuses the previous format on resume rather than reading it"
        );
        assert_eq!(stored.record.provider.as_deref(), Some("anthropic"));
        assert_eq!(stored.record.model.as_deref(), Some("claude-sonnet-5"));
    }

    #[tokio::test]
    async fn put_replaces_an_earlier_record() {
        let store = RunSessionRecordStore::new(test_support::in_memory_pool_with(&[
            fabro_db::RUN_SESSION_RECORDS_MIGRATION_SQL,
            fabro_db::RUN_SESSION_RECORDS_RUNLESS_MIGRATION_SQL,
        ]));
        let session_id = SessionId::new();
        let first = record(&session_id);
        let mut second = first.clone();
        second.last_event_seq = 12;
        let now = Utc::now();

        store
            .put(session_id, Some(fixtures::RUN_1), &first, now)
            .await
            .unwrap();
        store
            .put(session_id, Some(fixtures::RUN_1), &second, now)
            .await
            .unwrap();

        let stored = store.get(session_id).await.unwrap().expect("stored record");
        assert_eq!(stored.record.last_event_seq, 12);
        assert!(store.delete(session_id).await.unwrap());
        assert!(store.get(session_id).await.unwrap().is_none());
    }

    /// The index seam (denkhaus/work#1452): `list_for_run` returns the
    /// run's records as metadata only, newest first, and never touches
    /// another run's records.
    #[tokio::test]
    async fn list_for_run_returns_metadata_only_for_its_own_run() {
        let store = RunSessionRecordStore::new(test_support::in_memory_pool_with(&[
            fabro_db::RUN_SESSION_RECORDS_MIGRATION_SQL,
            fabro_db::RUN_SESSION_RECORDS_RUNLESS_MIGRATION_SQL,
        ]));
        let first = SessionId::new();
        let second = SessionId::new();
        let other_run = SessionId::new();
        let earlier = Utc.with_ymd_and_hms(2026, 9, 11, 12, 0, 0).unwrap();
        let later = earlier + chrono::Duration::minutes(5);

        store
            .put(first, Some(fixtures::RUN_1), &record(&first), earlier)
            .await
            .unwrap();
        store
            .put(second, Some(fixtures::RUN_1), &record(&second), later)
            .await
            .unwrap();
        store
            .put(other_run, Some(fixtures::RUN_2), &record(&other_run), later)
            .await
            .unwrap();

        let summaries = store.list_for_run(fixtures::RUN_1).await.unwrap();
        assert_eq!(summaries, vec![
            StoredSessionRecordSummary {
                session_id: second,
                run_id:     Some(fixtures::RUN_1),
                updated_at: later,
            },
            StoredSessionRecordSummary {
                session_id: first,
                run_id:     Some(fixtures::RUN_1),
                updated_at: earlier,
            },
        ]);
        assert_eq!(
            store.list_for_run(fixtures::RUN_2).await.unwrap().len(),
            1,
            "the other run's records stay out of this run's index"
        );
    }

    /// The deletion-chain seam (denkhaus/work#1452): `delete_for_run`
    /// removes exactly the run's records and leaves other runs intact.
    #[tokio::test]
    async fn delete_for_run_removes_only_that_runs_records() {
        let store = RunSessionRecordStore::new(test_support::in_memory_pool_with(&[
            fabro_db::RUN_SESSION_RECORDS_MIGRATION_SQL,
            fabro_db::RUN_SESSION_RECORDS_RUNLESS_MIGRATION_SQL,
        ]));
        let first = SessionId::new();
        let second = SessionId::new();
        let kept = SessionId::new();
        let now = Utc::now();

        store
            .put(first, Some(fixtures::RUN_1), &record(&first), now)
            .await
            .unwrap();
        store
            .put(second, Some(fixtures::RUN_1), &record(&second), now)
            .await
            .unwrap();
        store
            .put(kept, Some(fixtures::RUN_2), &record(&kept), now)
            .await
            .unwrap();

        assert_eq!(store.delete_for_run(fixtures::RUN_1).await.unwrap(), 2);
        assert!(
            store
                .list_for_run(fixtures::RUN_1)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(store.get(first).await.unwrap().is_none());
        assert!(store.get(second).await.unwrap().is_none());
        assert_eq!(
            store.list_for_run(fixtures::RUN_2).await.unwrap().len(),
            1,
            "the other run's records survive"
        );
        assert_eq!(store.delete_for_run(fixtures::RUN_1).await.unwrap(), 0);
    }
}
