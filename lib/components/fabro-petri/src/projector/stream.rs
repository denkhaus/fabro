//! The run's stream as the view tables hold it: one row per folded item,
//! written by a pass and read back in `stream_seq` order.

use fabro_db::DbPool;
use fabro_types::{RunId, RunStreamItem, RunStreamItemKind};
use petri_execution::events::{EventId, EventSource};

use super::{Positions, ProjectError};
use crate::projection::{Item, RunView};

pub(crate) struct StreamRow {
    pub(super) stream_seq: u64,
    pub(super) item_kind:  &'static str,
    pub(super) item_id:    String,
    pub(super) event_json: String,
}

/// Fold the items into the view in order, each at the next delivery
/// sequence, advancing the positions with each, and produce its stream
/// row.
pub(crate) fn stream_rows(
    items: &[Item<'_>],
    view: &mut RunView,
    positions: &mut Positions,
    stream_seq: &mut u64,
) -> Result<Vec<StreamRow>, ProjectError> {
    let mut rows = Vec::with_capacity(items.len());
    for item in items {
        *stream_seq += 1;
        view.fold(item, *stream_seq);
        let row = match item {
            Item::Petri(event) => {
                positions.advance(event.id);
                StreamRow {
                    stream_seq: *stream_seq,
                    item_kind:  "petri",
                    item_id:    event_id_text(&event.id),
                    event_json: serde_json::to_string(event).map_err(ProjectError::Encode)?,
                }
            }
            Item::Platform(record) => {
                positions.platform_seq = record.seq;
                StreamRow {
                    stream_seq: *stream_seq,
                    item_kind:  "platform",
                    item_id:    record.seq.to_string(),
                    event_json: serde_json::to_string(record).map_err(ProjectError::Encode)?,
                }
            }
        };
        rows.push(row);
    }
    Ok(rows)
}

/// The run's stream past the cursor, read from the view tables: up to
/// `limit` rows with `stream_seq > after`, in order, in Fabro's envelope.
pub(super) async fn stream_after(
    views: &DbPool,
    run_id: RunId,
    after: u64,
    limit: usize,
) -> Result<Vec<RunStreamItem>, ProjectError> {
    let rows: Vec<(i64, String, String, String)> = sqlx::query_as(
        "SELECT stream_seq, item_kind, item_id, event_json FROM petri_stream WHERE run_id = ? AND \
         stream_seq > ? ORDER BY stream_seq LIMIT ?",
    )
    .bind(run_id.to_string())
    .bind(column(after))
    .bind(i64::try_from(limit).unwrap_or(i64::MAX))
    .fetch_all(views)
    .await
    .map_err(ProjectError::Database)?;
    rows.into_iter()
        .map(|(stream_seq, item_kind, item_id, event_json)| {
            let item: serde_json::Value =
                serde_json::from_str(&event_json).map_err(ProjectError::Encode)?;
            let kind = match item_kind.as_str() {
                "platform" => RunStreamItemKind::Platform,
                _ => RunStreamItemKind::Petri,
            };
            let recorded_at = item
                .get("recorded_at")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            Ok(RunStreamItem {
                run_id,
                stream_seq: u64::try_from(stream_seq).unwrap_or(0),
                kind,
                id: item_id,
                recorded_at,
                item,
            })
        })
        .collect()
}

/// A Petri event id as the stream names it: `<log>/<seq>/<index>`.
#[must_use]
pub(crate) fn event_id_text(id: &EventId) -> String {
    format!("{}/{}/{}", log_text(&id.source), id.seq, id.index)
}

pub(super) fn log_text(source: &EventSource) -> String {
    match source {
        EventSource::Coordinator => "coordinator".to_string(),
        EventSource::Execution { execution } => format!("execution {execution}"),
    }
}

pub(super) fn column(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}
