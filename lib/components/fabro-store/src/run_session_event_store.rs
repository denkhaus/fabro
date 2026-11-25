//! SQLite storage for the events of Ask Fabro sessions.
//!
//! A session's events are numbered per session, from 1, in the order they
//! are appended; an append allocates the next number under the write lock,
//! so two writers of one session never share a number. Every committed
//! event is also published to the store's subscribers, which is how a
//! client attached to a session sees a turn as it runs.

use chrono::{DateTime, Utc};
use fabro_types::session_event::{
    SessionCapabilityChangedProps, SessionCreatedProps, SessionTurnStartedProps,
};
use fabro_types::{
    RunId, SessionEvent, SessionEventBody, SessionId, SessionStatus, SessionSummary, SessionTurn,
};
use sqlx::sqlite::{Sqlite, SqliteRow};
use sqlx::{Row as _, SqlitePool};
use tokio::sync::broadcast;

use crate::{Error, Result, sqlite_row};

const RECORD_NAME: &str = "run session event";

/// The rows of the global session index's page: one row per session, put
/// together from a latest-event-per-session projection of the events.
///
/// Every aggregate query here keeps exactly one aggregate in its SELECT,
/// which SQLite documents as pinning each bare column to the row holding
/// that aggregate's min/max: `candidates` reads each session's last
/// projection-relevant event's timestamp — the `updated_at` the fold path
/// ends on. The streaming `assistant_delta` frames are excluded, exactly
/// as the full event load this door replaced excluded them. The `created`
/// join is inner because a session projects nothing without its creation
/// event (the fold only opens an entry on `run.session.created`), and its
/// row holds the run anchor and the creation properties the summary
/// keeps; `capability` and `turn` are the latest of their event kinds,
/// `LEFT`-joined because a session may never leave the default posture
/// or never run a turn.
const SESSIONS_PAGE_SQL: &str = r"
WITH candidates AS (
    SELECT session_id, MAX(seq) AS last_seq, recorded_at_ms AS updated_at_ms
    FROM run_session_events
    WHERE event_name <> 'run.session.assistant_delta'
    GROUP BY session_id
),
created AS (
    SELECT session_id, run_id, recorded_at_ms AS created_at_ms,
           properties_json AS created_json
    FROM run_session_events
    WHERE seq = 1 AND event_name = 'run.session.created'
),
capability AS (
    SELECT session_id, properties_json AS capability_json
    FROM run_session_events AS latest
    WHERE event_name = 'run.session.capability.changed'
      AND seq = (
          SELECT MAX(seq) FROM run_session_events AS same
          WHERE same.session_id = latest.session_id
            AND same.event_name = 'run.session.capability.changed'
      )
),
turn AS (
    SELECT session_id, event_name AS turn_event, recorded_at_ms AS turn_ms,
           properties_json AS turn_json
    FROM run_session_events AS latest
    WHERE event_name IN (
        'run.session.turn.started', 'run.session.turn.succeeded',
        'run.session.turn.failed', 'run.session.turn.interrupted'
    )
      AND seq = (
          SELECT MAX(seq) FROM run_session_events AS same
          WHERE same.session_id = latest.session_id
            AND same.event_name IN (
                'run.session.turn.started', 'run.session.turn.succeeded',
                'run.session.turn.failed', 'run.session.turn.interrupted'
            )
      )
)
SELECT candidates.session_id, candidates.updated_at_ms,
       created.run_id, created.created_at_ms, created.created_json,
       capability.capability_json,
       turn.turn_event, turn.turn_ms, turn.turn_json
FROM candidates
JOIN created ON created.session_id = candidates.session_id
LEFT JOIN capability ON capability.session_id = candidates.session_id
LEFT JOIN turn ON turn.session_id = candidates.session_id
";

/// How many committed events a slow subscriber may fall behind before it
/// is told it lagged and has to replay from the table.
const LIVE_CAPACITY: usize = 1024;

/// Reads and writes session events in SQLite and publishes each committed
/// one.
pub struct RunSessionEventStore {
    pool: SqlitePool,
    live: broadcast::Sender<SessionEvent>,
}

impl std::fmt::Debug for RunSessionEventStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunSessionEventStore")
            .finish_non_exhaustive()
    }
}

/// Which ordering the global session index's page arrives in. Each variant
/// is the SQL ORDER BY that replicates the handler's comparator exactly,
/// including its tie-breaks — a ULID's TEXT order is its `Ord`, so
/// ordering by the stored id string decides ties the same way the fold
/// path's `SessionId: Ord` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionSummaryOrder {
    /// Last activity first: `updated_at` descending, then `created_at`,
    /// then the session id.
    UpdatedDesc,
    /// Newest session first: `created_at` descending, then the session id.
    CreatedDesc,
}

/// One page of the global session index.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSummaryPage {
    pub data:     Vec<SessionSummary>,
    /// Another page exists past this one's `offset`.
    pub has_more: bool,
}

impl RunSessionEventStore {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        let (live, _) = broadcast::channel(LIVE_CAPACITY);
        Self { pool, live }
    }

    /// Record `body` as the session's next event and publish it.
    pub async fn append(
        &self,
        run_id: Option<RunId>,
        session_id: SessionId,
        body: SessionEventBody,
        ts: DateTime<Utc>,
    ) -> Result<SessionEvent> {
        let properties = serde_json::to_value(&body)?;
        let properties_json =
            serde_json::to_string(properties.get("properties").ok_or_else(|| {
                Error::InvalidEvent("session event body has no properties".into())
            })?)?;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let last_seq: Option<i64> =
            sqlx::query_scalar("SELECT MAX(seq) FROM run_session_events WHERE session_id = ?")
                .bind(session_id.to_string())
                .fetch_one(&mut *transaction)
                .await?;
        let seq = u32::try_from(last_seq.unwrap_or(0))
            .ok()
            .and_then(|last| last.checked_add(1))
            .ok_or(Error::EventSequenceExhausted { max_seq: u32::MAX })?;
        sqlx::query(
            r"
INSERT INTO run_session_events (
    session_id, seq, run_id, turn_id, event_name, recorded_at_ms, properties_json
) VALUES (?, ?, ?, ?, ?, ?, ?)
",
        )
        .bind(session_id.to_string())
        .bind(i64::from(seq))
        .bind(run_id.map(|run_id| run_id.to_string()))
        .bind(body.turn_id().map(|turn_id| turn_id.to_string()))
        .bind(body.event_name())
        .bind(ts.timestamp_millis())
        .bind(properties_json)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        let event = SessionEvent {
            seq,
            session_id,
            run_id,
            ts,
            body,
        };
        // No subscriber is not an error: the event is committed either way.
        let _ = self.live.send(event.clone());
        Ok(event)
    }

    /// The session's events from `since_seq` on, at most `limit` of them,
    /// in sequence order.
    pub async fn list_from(
        &self,
        session_id: SessionId,
        since_seq: u32,
        limit: usize,
    ) -> Result<Vec<SessionEvent>> {
        let rows = sqlx::query(
            r"
SELECT session_id, seq, run_id, event_name, recorded_at_ms, properties_json
FROM run_session_events
WHERE session_id = ? AND seq >= ?
ORDER BY seq ASC
LIMIT ?
",
        )
        .bind(session_id.to_string())
        .bind(i64::from(since_seq))
        .bind(i64::try_from(limit).unwrap_or(i64::MAX))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(event_from_row).collect()
    }

    /// Every event of every session of `run_id`, session by session in
    /// sequence order.
    pub async fn list_for_run(&self, run_id: RunId) -> Result<Vec<SessionEvent>> {
        let rows = sqlx::query(
            r"
SELECT session_id, seq, run_id, event_name, recorded_at_ms, properties_json
FROM run_session_events
WHERE run_id = ?
ORDER BY session_id ASC, seq ASC
",
        )
        .bind(run_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(event_from_row).collect()
    }

    /// One page of the global session index — every run's sessions with
    /// the summary the projection fold would give them (denkhaus/work#1452's
    /// global index door, bounded at the SQL level by work#1469).
    ///
    /// The rows are a latest-event-per-session SQL projection, not a full
    /// event load: `candidates` folds every session down to its last
    /// projection-relevant event (the streaming `assistant_delta` frames
    /// are excluded, as the full load they once served was), and the
    /// `created`/`capability`/`turn` joins read the three rows whose
    /// properties the summary keeps. At most `limit + 1` rows cross into
    /// Rust; the extra row is the has-more probe, the same convention the
    /// in-memory pagination this door replaced used. `limit` must be at
    /// least 1; `offset` skips into the ordered session list.
    pub async fn list_session_summaries(
        &self,
        order: SessionSummaryOrder,
        limit: u32,
        offset: u32,
    ) -> Result<SessionSummaryPage> {
        // The ORDER BY variant is the one dynamic piece; `QueryBuilder`
        // carries it the way the run summary store's list does.
        let mut query = sqlx::QueryBuilder::<Sqlite>::new(SESSIONS_PAGE_SQL);
        match order {
            SessionSummaryOrder::UpdatedDesc => {
                query.push(
                    " ORDER BY candidates.updated_at_ms DESC, created.created_at_ms DESC, \
                     candidates.session_id DESC",
                );
            }
            SessionSummaryOrder::CreatedDesc => {
                query.push(" ORDER BY created.created_at_ms DESC, candidates.session_id DESC");
            }
        }
        query.push(" LIMIT ").push_bind(i64::from(limit) + 1);
        query.push(" OFFSET ").push_bind(i64::from(offset));
        let mut rows = query.build().fetch_all(&self.pool).await?;
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        let data = rows
            .iter()
            .map(session_summary_from_row)
            .collect::<Result<Vec<_>>>()?;
        Ok(SessionSummaryPage { data, has_more })
    }

    /// One page of the global session index narrowed to a search keyword
    /// (ADR-0046 phase 2, denkhaus/work#1679): the session archaeology
    /// read. A session matches when the keyword is a case-insensitive
    /// substring of its run anchor, its title, or the text of a recorded
    /// user or assistant message; SQL `LIKE` is case-insensitive for
    /// ASCII here, and the keyword's `%`/`_`/`\` are escaped so a caller
    /// cannot widen the match. The bound is the SQL: the same
    /// latest-event-per-session projection as `list_session_summaries`,
    /// filtered by an `IN` subquery over the events table — at most
    /// `limit + 1` rows cross into Rust.
    pub async fn search_session_summaries(
        &self,
        keyword: &str,
        order: SessionSummaryOrder,
        limit: u32,
        offset: u32,
    ) -> Result<SessionSummaryPage> {
        let pattern = format!("%{}%", like_escape(keyword));
        let mut query = sqlx::QueryBuilder::<Sqlite>::new(SESSIONS_PAGE_SQL);
        query.push(" WHERE candidates.session_id IN (SELECT session_id FROM run_session_events WHERE run_id LIKE ");
        query.push_bind(&pattern);
        query.push(" ESCAPE '\\' OR (event_name = 'run.session.created' AND json_extract(properties_json, '$.title') LIKE ");
        query.push_bind(&pattern);
        query.push(" ESCAPE '\\') OR (event_name IN ('run.session.user_message', 'run.session.assistant_message') AND properties_json LIKE ");
        query.push_bind(&pattern);
        query.push(" ESCAPE '\\'))");
        match order {
            SessionSummaryOrder::UpdatedDesc => {
                query.push(
                    " ORDER BY candidates.updated_at_ms DESC, created.created_at_ms DESC, \
                     candidates.session_id DESC",
                );
            }
            SessionSummaryOrder::CreatedDesc => {
                query.push(" ORDER BY created.created_at_ms DESC, candidates.session_id DESC");
            }
        }
        query.push(" LIMIT ").push_bind(i64::from(limit) + 1);
        query.push(" OFFSET ").push_bind(i64::from(offset));
        let rows = query.build().fetch_all(&self.pool).await?;
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        let has_more = rows.len() > limit;
        let data = rows
            .iter()
            .take(limit)
            .map(session_summary_from_row)
            .collect::<Result<Vec<_>>>()?;
        Ok(SessionSummaryPage { data, has_more })
    }

    /// The sequence number of the session's latest event, if it has any.
    pub async fn last_seq(&self, session_id: SessionId) -> Result<Option<u32>> {
        let last_seq: Option<i64> =
            sqlx::query_scalar("SELECT MAX(seq) FROM run_session_events WHERE session_id = ?")
                .bind(session_id.to_string())
                .fetch_one(&self.pool)
                .await?;
        last_seq
            .map(|seq| {
                u32::try_from(seq).map_err(|_| {
                    Error::InvalidEvent(format!("stored {RECORD_NAME} sequence {seq}"))
                })
            })
            .transpose()
    }

    /// The run that owns `session_id`, from the session's creation event.
    ///
    /// `Ok(None)` names no session; `Ok(Some(None))` names a runless
    /// session (work#1677) — a seq-1 event with a NULL run id.
    pub async fn owner(&self, session_id: SessionId) -> Result<Option<Option<RunId>>> {
        let run_id: Option<Option<String>> = sqlx::query_scalar(
            "SELECT run_id FROM run_session_events WHERE session_id = ? AND seq = 1",
        )
        .bind(session_id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        run_id
            .map(|run_id| {
                run_id
                    .map(|run_id| {
                        run_id.parse::<RunId>().map_err(|error| {
                            Error::InvalidEvent(format!("stored {RECORD_NAME} run id: {error}"))
                        })
                    })
                    .transpose()
            })
            .transpose()
    }

    /// Forget every session event of `run_id`.
    pub async fn delete_for_run(&self, run_id: RunId) -> Result<u64> {
        let result = sqlx::query("DELETE FROM run_session_events WHERE run_id = ?")
            .bind(run_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    /// Forget every event of `session_id` — the explicit-delete door's
    /// store seam (work#1535), the by-session mate of `delete_for_run`:
    /// the session leaves the index and its transcript is unreachable.
    pub async fn delete_for_session(&self, session_id: SessionId) -> Result<u64> {
        let result = sqlx::query("DELETE FROM run_session_events WHERE session_id = ?")
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    /// Every event committed from now on, across all sessions. A receiver
    /// that falls more than the channel's capacity behind gets
    /// `RecvError::Lagged` and replays from `list_from`.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.live.subscribe()
    }
}

/// SQLite `LIKE`'s wildcards escaped for a literal substring match: `\`
/// doubles, `%` and `_` lose their wildcard meaning under the query's
/// `ESCAPE '\'` clause.
fn like_escape(keyword: &str) -> String {
    let mut escaped = String::with_capacity(keyword.len());
    for ch in keyword.chars() {
        if ch == '\\' || ch == '%' || ch == '_' {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

/// The summary of one session, read off the page query's projected rows.
///
/// The mapping mirrors [`project_run_sessions`]' fold over the session's
/// non-delta events: the creation event holds the run anchor, the
/// timestamps and the title/model/provider; the latest capability event
/// holds the posture (default read-only); the latest turn-lifecycle event
/// holds the status and, for a turn still open, the active turn.
fn session_summary_from_row(row: &SqliteRow) -> Result<SessionSummary> {
    let session_id: String = row.try_get("session_id")?;
    let session_id = session_id.parse::<SessionId>().map_err(|error| {
        Error::InvalidEvent(format!("stored {RECORD_NAME} session id: {error}"))
    })?;
    let created_at = sqlite_row::timestamp_from_row(row, RECORD_NAME, "created_at_ms")?;
    let updated_at = sqlite_row::timestamp_from_row(row, RECORD_NAME, "updated_at_ms")?;
    let run_id: Option<String> = row.try_get("run_id")?;
    let run_id = run_id
        .map(|run_id| {
            run_id.parse::<RunId>().map_err(|error| {
                Error::InvalidEvent(format!("stored {RECORD_NAME} run id: {error}"))
            })
        })
        .transpose()?;
    let created_json: String = row.try_get("created_json")?;
    let created: SessionCreatedProps = serde_json::from_str(&created_json)?;
    let capability_json: Option<String> = row.try_get("capability_json")?;
    // The posture a flip set wins; a session that never flipped carries
    // the posture its CREATION recorded (work#1678: a runless session is
    // created full access), not the read-only default.
    let capability = capability_json
        .map(|json| {
            serde_json::from_str::<SessionCapabilityChangedProps>(&json)
                .map(|props| props.capability)
        })
        .transpose()?
        .unwrap_or(created.capability);
    let turn_event: Option<String> = row.try_get("turn_event")?;
    let (status, active_turn) = match turn_event.as_deref() {
        Some("run.session.turn.started") => {
            let turn_json: String = row.try_get("turn_json")?;
            let started: SessionTurnStartedProps = serde_json::from_str(&turn_json)?;
            let started_at = sqlite_row::timestamp_from_row(row, RECORD_NAME, "turn_ms")?;
            (
                SessionStatus::Running,
                Some(SessionTurn {
                    id: started.turn_id,
                    started_at,
                    input: started.input,
                }),
            )
        }
        Some("run.session.turn.failed") => (SessionStatus::Failed, None),
        // A turn that succeeded or was interrupted leaves the session idle,
        // and a session that never ran a turn starts idle.
        Some(_) | None => (SessionStatus::Idle, None),
    };
    Ok(SessionSummary {
        id: session_id,
        run_id,
        title: created.title,
        status,
        model: created.model,
        provider: created.provider,
        active_turn,
        capability,
        created_at,
        updated_at,
    })
}

fn event_from_row(row: &SqliteRow) -> Result<SessionEvent> {
    let session_id: String = row.try_get("session_id")?;
    let session_id = session_id.parse::<SessionId>().map_err(|error| {
        Error::InvalidEvent(format!("stored {RECORD_NAME} session id: {error}"))
    })?;
    let seq: i64 = row.try_get("seq")?;
    let seq = u32::try_from(seq)
        .map_err(|_| Error::InvalidEvent(format!("stored {RECORD_NAME} sequence {seq}")))?;
    let run_id: Option<String> = row.try_get("run_id")?;
    let run_id = run_id
        .map(|run_id| {
            run_id.parse::<RunId>().map_err(|error| {
                Error::InvalidEvent(format!("stored {RECORD_NAME} run id: {error}"))
            })
        })
        .transpose()?;
    let ts = sqlite_row::timestamp_from_row(row, RECORD_NAME, "recorded_at_ms")?;
    let event_name: String = row.try_get("event_name")?;
    let properties_json: String = row.try_get("properties_json")?;
    let properties: serde_json::Value = serde_json::from_str(&properties_json)?;
    let body: SessionEventBody = serde_json::from_value(serde_json::json!({
        "event": event_name,
        "properties": properties,
    }))?;
    Ok(SessionEvent {
        seq,
        session_id,
        run_id,
        ts,
        body,
    })
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use fabro_types::session_event::{
        SessionAssistantDeltaProps, SessionAssistantMessageProps, SessionCapabilityChangedProps,
        SessionCreatedProps, SessionToolCallCompletedProps, SessionToolCallStartedProps,
        SessionTurnFailedCode, SessionTurnFailedProps, SessionTurnStartedProps,
        SessionTurnSucceededProps, SessionUserMessageProps,
    };
    use fabro_types::{AskFabroCapability, TurnId, fixtures};

    use super::*;
    use crate::run_sessions::project_run_sessions;
    use crate::test_support;

    fn store() -> RunSessionEventStore {
        RunSessionEventStore::new(test_support::in_memory_pool_with(&[
            fabro_db::RUN_SESSION_EVENTS_MIGRATION_SQL,
            fabro_db::RUN_SESSION_EVENTS_RUNLESS_MIGRATION_SQL,
        ]))
    }

    fn created() -> SessionEventBody {
        SessionEventBody::Created(SessionCreatedProps {
            title:      Some("Ask Fabro".to_string()),
            model:      Some("test-model".to_string()),
            provider:   None,
            capability: AskFabroCapability::ReadOnly,
        })
    }

    fn turn_started(turn_id: TurnId) -> SessionEventBody {
        SessionEventBody::TurnStarted(SessionTurnStartedProps {
            turn_id,
            input: "What happened?".to_string(),
        })
    }

    fn capability_changed(capability: AskFabroCapability) -> SessionEventBody {
        SessionEventBody::CapabilityChanged(SessionCapabilityChangedProps {
            capability,
            actor: None,
        })
    }

    fn user_message(turn_id: TurnId) -> SessionEventBody {
        SessionEventBody::UserMessage(SessionUserMessageProps {
            turn_id,
            text: "What happened?".to_string(),
        })
    }

    fn assistant_delta(turn_id: TurnId) -> SessionEventBody {
        SessionEventBody::AssistantDelta(SessionAssistantDeltaProps {
            turn_id,
            delta: "streaming bulk".to_string(),
        })
    }

    fn assistant_message(turn_id: TurnId) -> SessionEventBody {
        SessionEventBody::AssistantMessage(SessionAssistantMessageProps {
            turn_id,
            text: "The run finished.".to_string(),
            model: Some("test-model".to_string()),
            usage: serde_json::json!({ "output_tokens": 4 }),
        })
    }

    fn tool_call_started(turn_id: TurnId) -> SessionEventBody {
        SessionEventBody::ToolCallStarted(SessionToolCallStartedProps {
            turn_id,
            tool_name: "read_file".to_string(),
            tool_call_id: "call_1".to_string(),
            arguments: serde_json::json!({ "path": "README.md" }),
        })
    }

    fn tool_call_completed(turn_id: TurnId) -> SessionEventBody {
        SessionEventBody::ToolCallCompleted(SessionToolCallCompletedProps {
            turn_id,
            tool_name: "read_file".to_string(),
            tool_call_id: "call_1".to_string(),
            output: serde_json::json!("readme bytes"),
            is_error: false,
            output_bytes_observed: None,
            output_bytes_retained: None,
            output_bytes_omitted: None,
        })
    }

    fn turn_succeeded(turn_id: TurnId) -> SessionEventBody {
        SessionEventBody::TurnSucceeded(SessionTurnSucceededProps {
            turn_id,
            output: Some("The run finished.".to_string()),
        })
    }

    fn turn_failed(turn_id: TurnId) -> SessionEventBody {
        SessionEventBody::TurnFailed(SessionTurnFailedProps {
            turn_id,
            error: "boom".to_string(),
            output: None,
            code: SessionTurnFailedCode::AgentError,
            retryable: false,
        })
    }

    #[tokio::test]
    async fn appends_number_a_session_from_one_and_read_back_in_order() {
        let store = store();
        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let ts = Utc.with_ymd_and_hms(2026, 9, 18, 12, 0, 0).unwrap();

        let first = store
            .append(Some(fixtures::RUN_1), session_id, created(), ts)
            .await
            .unwrap();
        let second = store
            .append(Some(fixtures::RUN_1), session_id, turn_started(turn_id), ts)
            .await
            .unwrap();
        assert_eq!(first.seq, 1);
        assert_eq!(second.seq, 2);
        assert_eq!(second.body.turn_id(), Some(turn_id));

        let events = store.list_from(session_id, 1, 10).await.unwrap();
        assert_eq!(events, vec![first.clone(), second.clone()]);
        assert_eq!(store.list_from(session_id, 2, 10).await.unwrap(), vec![
            second
        ]);
        assert_eq!(store.list_from(session_id, 1, 1).await.unwrap(), vec![
            first
        ]);
        assert_eq!(store.last_seq(session_id).await.unwrap(), Some(2));
        assert_eq!(store.last_seq(SessionId::new()).await.unwrap(), None);
    }

    #[tokio::test]
    async fn sessions_are_numbered_apart_and_owned_by_their_run() {
        let store = store();
        let first = SessionId::new();
        let second = SessionId::new();
        let ts = Utc::now();
        store
            .append(Some(fixtures::RUN_1), first, created(), ts)
            .await
            .unwrap();
        store
            .append(
                Some(fixtures::RUN_1),
                first,
                turn_started(TurnId::new()),
                ts,
            )
            .await
            .unwrap();
        let other = store
            .append(Some(fixtures::RUN_2), second, created(), ts)
            .await
            .unwrap();
        assert_eq!(other.seq, 1);

        assert_eq!(
            store.owner(first).await.unwrap(),
            Some(Some(fixtures::RUN_1))
        );
        assert_eq!(
            store.owner(second).await.unwrap(),
            Some(Some(fixtures::RUN_2))
        );
        assert_eq!(store.owner(SessionId::new()).await.unwrap(), None);

        let run_events = store.list_for_run(fixtures::RUN_1).await.unwrap();
        assert_eq!(run_events.len(), 2);
        assert!(
            run_events
                .iter()
                .all(|event| event.session_id == first && event.run_id == Some(fixtures::RUN_1))
        );

        assert_eq!(store.delete_for_run(fixtures::RUN_1).await.unwrap(), 2);
        assert!(
            store
                .list_for_run(fixtures::RUN_1)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(store.owner(first).await.unwrap(), None);
        assert_eq!(
            store.owner(second).await.unwrap(),
            Some(Some(fixtures::RUN_2))
        );
    }

    /// The explicit-delete seam (work#1535): `delete_for_session` removes
    /// exactly that session's events — it leaves the index and its owner
    /// run untouched — and a repeat delete removes nothing.
    #[tokio::test]
    async fn delete_for_session_removes_only_that_sessions_events() {
        let store = store();
        let first = SessionId::new();
        let second = SessionId::new();
        let ts = Utc::now();
        store
            .append(Some(fixtures::RUN_1), first, created(), ts)
            .await
            .unwrap();
        store
            .append(
                Some(fixtures::RUN_1),
                first,
                turn_started(TurnId::new()),
                ts,
            )
            .await
            .unwrap();
        store
            .append(Some(fixtures::RUN_1), second, created(), ts)
            .await
            .unwrap();

        assert_eq!(
            store.delete_for_session(first).await.unwrap(),
            2,
            "both of the first session's events die"
        );
        assert!(store.list_from(first, 1, 10).await.unwrap().is_empty());
        assert_eq!(store.owner(first).await.unwrap(), None);
        assert_eq!(
            store.list_from(second, 1, 10).await.unwrap().len(),
            1,
            "the sibling session's events survive"
        );
        assert_eq!(
            store.list_for_run(fixtures::RUN_1).await.unwrap().len(),
            1,
            "the run's remaining session stays"
        );
        assert_eq!(
            store.delete_for_session(first).await.unwrap(),
            0,
            "deleting an already-deleted session removes nothing"
        );
    }

    /// work#1469 pin: the global sessions index door bounds its rows in
    /// the SQL. The observable is the row count crossing the store's
    /// public door: a fixture with more sessions than the page holds hands
    /// back at most the page plus the has-more probe — never every
    /// session's events, which is what the full load this door replaced
    /// did (the red run materialized 30 rows for a 5-row page).
    #[tokio::test]
    async fn the_global_sessions_door_bounds_its_rows_in_sql() {
        let store = store();
        let ts = Utc::now();
        for _ in 0..30 {
            store
                .append(Some(fixtures::RUN_1), SessionId::new(), created(), ts)
                .await
                .unwrap();
        }

        let first = store
            .list_session_summaries(SessionSummaryOrder::UpdatedDesc, 5, 0)
            .await
            .unwrap();
        assert_eq!(first.data.len(), 5);
        assert!(first.has_more, "30 sessions leave more past a 5-row page");

        let last = store
            .list_session_summaries(SessionSummaryOrder::UpdatedDesc, 5, 25)
            .await
            .unwrap();
        assert_eq!(last.data.len(), 5);
        assert!(!last.has_more);

        let past = store
            .list_session_summaries(SessionSummaryOrder::UpdatedDesc, 5, 26)
            .await
            .unwrap();
        assert_eq!(past.data.len(), 4);
        assert!(!past.has_more);

        let beyond = store
            .list_session_summaries(SessionSummaryOrder::UpdatedDesc, 5, 30)
            .await
            .unwrap();
        assert!(beyond.data.is_empty());
        assert!(!beyond.has_more);
    }

    /// work#1469 parity pin: the SQL-projected page equals the summaries
    /// the projection fold gives the same sessions' non-delta events, in
    /// the handler comparator's order. The fixture walks every event kind
    /// the fold reacts to, across two runs.
    #[tokio::test]
    async fn the_sessions_index_page_matches_the_projection_fold() {
        let store = store();
        let base = Utc.with_ymd_and_hms(2026, 11, 20, 9, 0, 0).unwrap();
        let at = |millis: i64| base + chrono::Duration::milliseconds(millis);

        let rich = SessionId::new();
        let rich_turn = TurnId::new();
        for (millis, body) in [
            (0, created()),
            (1, capability_changed(AskFabroCapability::FullAccess)),
            (2, turn_started(rich_turn)),
            (3, user_message(rich_turn)),
            (4, assistant_delta(rich_turn)),
            (5, assistant_message(rich_turn)),
            (6, tool_call_started(rich_turn)),
            (7, tool_call_completed(rich_turn)),
            (8, turn_succeeded(rich_turn)),
        ] {
            store
                .append(Some(fixtures::RUN_1), rich, body, at(millis))
                .await
                .unwrap();
        }

        let running = SessionId::new();
        let running_turn = TurnId::new();
        store
            .append(Some(fixtures::RUN_1), running, created(), at(10))
            .await
            .unwrap();
        store
            .append(
                Some(fixtures::RUN_1),
                running,
                turn_started(running_turn),
                at(11),
            )
            .await
            .unwrap();

        // A session whose only projection-relevant event is its creation:
        // the trailing delta frame must not move the door's `updated_at`.
        let quiet = SessionId::new();
        store
            .append(Some(fixtures::RUN_1), quiet, created(), at(12))
            .await
            .unwrap();
        store
            .append(
                Some(fixtures::RUN_1),
                quiet,
                assistant_delta(TurnId::new()),
                at(100),
            )
            .await
            .unwrap();

        let failed = SessionId::new();
        let failed_turn = TurnId::new();
        store
            .append(Some(fixtures::RUN_2), failed, created(), at(20))
            .await
            .unwrap();
        store
            .append(
                Some(fixtures::RUN_2),
                failed,
                turn_started(failed_turn),
                at(21),
            )
            .await
            .unwrap();
        store
            .append(
                Some(fixtures::RUN_2),
                failed,
                turn_failed(failed_turn),
                at(22),
            )
            .await
            .unwrap();

        // The oracle: the fold over each session's non-delta events —
        // what the door served before its bound moved into the SQL.
        let mut expected = std::collections::BTreeMap::new();
        for run in [fixtures::RUN_1, fixtures::RUN_2] {
            let events: Vec<_> = store
                .list_for_run(run)
                .await
                .unwrap()
                .into_iter()
                .filter(|event| event.body.event_name() != "run.session.assistant_delta")
                .collect();
            for summary in project_run_sessions(&events) {
                expected.insert(summary.id, summary);
            }
        }

        let page = store
            .list_session_summaries(SessionSummaryOrder::UpdatedDesc, 10, 0)
            .await
            .unwrap();
        assert!(!page.has_more, "four sessions fit a 10-row page");
        assert_eq!(page.data.len(), 4, "one row per session, not per event");
        for summary in &page.data {
            assert_eq!(
                Some(summary),
                expected.get(&summary.id),
                "summary of session {} matches the fold",
                summary.id
            );
        }
        let mut ordered: Vec<_> = expected.values().cloned().collect();
        ordered.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| right.created_at.cmp(&left.created_at))
                .then_with(|| right.id.cmp(&left.id))
        });
        assert_eq!(page.data, ordered, "the page arrives in comparator order");

        // The semantics the fold gives, readable on the page itself: the
        // finished arc is idle in full access, the open turn is running
        // with its active turn, the failed turn marks the session failed,
        // and the quiet session's activity stops at its creation event —
        // the delta frame did not move it.
        let by_id = |id| {
            page.data
                .iter()
                .find(|summary| summary.id == id)
                .unwrap_or_else(|| panic!("session {id} should be on the page"))
        };
        assert_eq!(by_id(rich).status, SessionStatus::Idle);
        assert_eq!(by_id(rich).capability, AskFabroCapability::FullAccess);
        assert!(by_id(rich).active_turn.is_none());
        assert_eq!(by_id(running).status, SessionStatus::Running);
        assert_eq!(by_id(running).capability, AskFabroCapability::ReadOnly);
        assert_eq!(
            by_id(running).active_turn.as_ref().map(|turn| turn.id),
            Some(running_turn)
        );
        assert_eq!(by_id(quiet).updated_at, at(12));
        assert_eq!(by_id(failed).status, SessionStatus::Failed);
    }

    /// UpdatedDesc orders by last activity; CreatedDesc by creation. The
    /// orderings differ when the older session is the more recently
    /// active one.
    #[tokio::test]
    async fn the_sessions_page_orders_by_activity_or_creation() {
        let store = store();
        let older = SessionId::new();
        let newer = SessionId::new();
        let at = |hour: u32| Utc.with_ymd_and_hms(2026, 11, 20, hour, 0, 0).unwrap();
        store
            .append(Some(fixtures::RUN_1), older, created(), at(9))
            .await
            .unwrap();
        store
            .append(
                Some(fixtures::RUN_1),
                older,
                turn_started(TurnId::new()),
                at(11),
            )
            .await
            .unwrap();
        store
            .append(Some(fixtures::RUN_1), newer, created(), at(10))
            .await
            .unwrap();

        let by_activity = store
            .list_session_summaries(SessionSummaryOrder::UpdatedDesc, 10, 0)
            .await
            .unwrap();
        assert_eq!(
            by_activity.data.iter().map(|s| s.id).collect::<Vec<_>>(),
            vec![older, newer],
            "the older session was active last"
        );

        let by_creation = store
            .list_session_summaries(SessionSummaryOrder::CreatedDesc, 10, 0)
            .await
            .unwrap();
        assert_eq!(
            by_creation.data.iter().map(|s| s.id).collect::<Vec<_>>(),
            vec![newer, older],
            "the newer session was created last"
        );
    }

    /// The page's tie-break is the session id descending — the TEXT order
    /// of the stored ULID strings, which is the fold path's
    /// `SessionId: Ord` descending.
    #[tokio::test]
    async fn the_sessions_page_breaks_ties_by_session_id_descending() {
        let store = store();
        let ts = Utc.with_ymd_and_hms(2026, 11, 20, 9, 0, 0).unwrap();
        for _ in 0..4 {
            store
                .append(Some(fixtures::RUN_1), SessionId::new(), created(), ts)
                .await
                .unwrap();
        }

        let page = store
            .list_session_summaries(SessionSummaryOrder::UpdatedDesc, 10, 0)
            .await
            .unwrap();
        let ids: Vec<String> = page.data.iter().map(|s| s.id.to_string()).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable_by(|left, right| right.cmp(left));
        assert_eq!(ids, sorted, "ties order by the session id descending");
    }

    /// The archaeology search (work#1679): a session matches by run
    /// anchor, title, or recorded message text, and non-matching sessions
    /// stay off the page. The page bounds in SQL like the index door.
    #[tokio::test]
    async fn search_session_summaries_matches_anchor_title_and_text() {
        let store = store();
        let ts = Utc.with_ymd_and_hms(2026, 11, 25, 9, 0, 0).unwrap();
        let anchored = SessionId::new();
        let titled = SessionId::new();
        let talked = SessionId::new();
        let quiet = SessionId::new();
        let anchor_run = fixtures::RUN_3;

        store
            .append(Some(anchor_run), anchored, created(), ts)
            .await
            .unwrap();
        store
            .append(
                Some(fixtures::RUN_2),
                titled,
                SessionEventBody::Created(SessionCreatedProps {
                    title:      Some("The deploy question".to_string()),
                    model:      Some("test-model".to_string()),
                    provider:   None,
                    capability: AskFabroCapability::ReadOnly,
                }),
                ts,
            )
            .await
            .unwrap();
        let talk_turn = TurnId::new();
        store
            .append(Some(fixtures::RUN_1), talked, created(), ts)
            .await
            .unwrap();
        store
            .append(
                Some(fixtures::RUN_1),
                talked,
                SessionEventBody::UserMessage(SessionUserMessageProps {
                    turn_id: talk_turn,
                    text:    "Why did the nightly run stall?".to_string(),
                }),
                ts,
            )
            .await
            .unwrap();
        store
            .append(Some(fixtures::RUN_1), quiet, created(), ts)
            .await
            .unwrap();

        // By run anchor: only the session anchored to that run matches.
        let by_anchor = store
            .search_session_summaries(
                &anchor_run.to_string(),
                SessionSummaryOrder::UpdatedDesc,
                10,
                0,
            )
            .await
            .unwrap();
        assert_eq!(
            by_anchor.data.iter().map(|s| s.id).collect::<Vec<_>>(),
            vec![anchored]
        );

        // By title, case-insensitively.
        let by_title = store
            .search_session_summaries("DEPLOY", SessionSummaryOrder::UpdatedDesc, 10, 0)
            .await
            .unwrap();
        assert_eq!(
            by_title.data.iter().map(|s| s.id).collect::<Vec<_>>(),
            vec![titled]
        );

        // By recorded message text.
        let by_text = store
            .search_session_summaries("nightly run stall", SessionSummaryOrder::UpdatedDesc, 10, 0)
            .await
            .unwrap();
        assert_eq!(by_text.data.iter().map(|s| s.id).collect::<Vec<_>>(), vec![
            talked
        ]);

        // A keyword matching nothing leaves the page empty.
        let none = store
            .search_session_summaries("no-such-keyword", SessionSummaryOrder::UpdatedDesc, 10, 0)
            .await
            .unwrap();
        assert!(none.data.is_empty());
        assert!(!none.has_more);

        // The model string in the creation event is not a title match.
        let by_model = store
            .search_session_summaries("test-model", SessionSummaryOrder::UpdatedDesc, 10, 0)
            .await
            .unwrap();
        assert!(
            !by_model.data.iter().any(|s| s.id == anchored),
            "the model field alone must not match the title arm"
        );
    }

    /// A keyword's `%` and `_` are literal text, not wildcards: the
    /// escaped match cannot widen.
    #[tokio::test]
    async fn search_escapes_like_wildcards_in_the_keyword() {
        let store = store();
        let ts = Utc::now();
        let session = SessionId::new();
        store
            .append(Some(fixtures::RUN_1), session, created(), ts)
            .await
            .unwrap();

        // `%` would match every session as a wildcard; escaped, it only
        // matches a session that literally contains a percent sign.
        let wildcard = store
            .search_session_summaries("%", SessionSummaryOrder::UpdatedDesc, 10, 0)
            .await
            .unwrap();
        assert!(wildcard.data.is_empty());

        let underscore = store
            .search_session_summaries("_", SessionSummaryOrder::UpdatedDesc, 10, 0)
            .await
            .unwrap();
        assert!(underscore.data.is_empty());
    }

    #[tokio::test]
    async fn a_subscriber_sees_each_committed_event() {
        let store = store();
        let session_id = SessionId::new();
        let mut live = store.subscribe();
        let ts = Utc::now();
        let appended = store
            .append(Some(fixtures::RUN_1), session_id, created(), ts)
            .await
            .unwrap();
        let delta = store
            .append(
                Some(fixtures::RUN_1),
                session_id,
                SessionEventBody::AssistantDelta(SessionAssistantDeltaProps {
                    turn_id: TurnId::new(),
                    delta:   "Hel".to_string(),
                }),
                ts,
            )
            .await
            .unwrap();

        assert_eq!(live.recv().await.unwrap(), appended);
        assert_eq!(live.recv().await.unwrap(), delta);
    }
}
