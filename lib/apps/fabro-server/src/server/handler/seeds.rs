//! The read-only seeds endpoints (fabro-3488, ADR-0023 step 5): list,
//! detail, and dependency graph over the tracker's seeds, served from the
//! configured [`SeedsSource`]. No write endpoints in v1 — the tracker's
//! writes stay with the CLI surfaces.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use axum_extra::extract::Query as ExtraQuery;
use chrono::{DateTime, Utc};
use fabro_api::types as api;
use seeds::{SeedRecord, SeedType as RecordSeedType, Status as RecordStatus, Store};

use super::super::seeds_source::{SeedsSnapshot, SeedsSourceError};
use super::super::{ApiError, AppState, RequiredUser};

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/seeds", get(list_seeds))
        .route("/seeds/graph", get(seed_graph))
        .route("/seeds/{id}", get(get_seed))
}

/// List filters; every filter composes with AND semantics.
#[derive(Debug, Default, serde::Deserialize)]
struct SeedListParams {
    #[serde(rename = "page[limit]", default = "default_seed_page_limit")]
    limit:    u32,
    #[serde(rename = "page[offset]", default)]
    offset:   u32,
    #[serde(default)]
    status:   Option<api::SeedStatus>,
    #[serde(default)]
    r#type:   Option<api::SeedType>,
    #[serde(default)]
    assignee: Option<String>,
    #[serde(default)]
    label:    Option<String>,
}

fn default_seed_page_limit() -> u32 {
    500
}

async fn list_seeds(
    _auth: RequiredUser,
    State(state): State<Arc<AppState>>,
    ExtraQuery(params): ExtraQuery<SeedListParams>,
) -> Response {
    let snapshot = match source_snapshot(state.as_ref()).await {
        Ok(snapshot) => snapshot,
        Err(response) => return response,
    };
    let matched: Vec<&SeedRecord> = snapshot
        .store
        .issues
        .iter()
        .filter(|record| matches_params(record, &params))
        .collect();
    let total = matched.len();
    let offset = params.offset as usize;
    let limit = params.limit.clamp(1, 2000) as usize;
    let page: Vec<_> = matched
        .iter()
        .skip(offset)
        .take(limit)
        .map(|record| seed_summary(record))
        .collect();
    let has_more = offset + page.len() < total;
    (
        StatusCode::OK,
        Json(api::PaginatedSeedList {
            data: page,
            meta: api::PaginationMeta {
                has_more,
                total: Some(i64::try_from(total).unwrap_or(i64::MAX)),
            },
        }),
    )
        .into_response()
}

fn matches_params(record: &SeedRecord, params: &SeedListParams) -> bool {
    if let Some(want) = &params.status {
        if record.status().map(record_status_to_api).as_ref() != Some(want) {
            return false;
        }
    }
    if let Some(want) = &params.r#type {
        if record.seed_type().map(record_type_to_api).as_ref() != Some(want) {
            return false;
        }
    }
    if let Some(want) = &params.assignee {
        let matches = if want == "none" {
            record.assignee().is_none()
        } else {
            record.assignee() == Some(want.as_str())
        };
        if !matches {
            return false;
        }
    }
    if let Some(want) = &params.label {
        if !record.labels().contains(&want.as_str()) {
            return false;
        }
    }
    true
}

async fn seed_graph(_auth: RequiredUser, State(state): State<Arc<AppState>>) -> Response {
    let snapshot = match source_snapshot(state.as_ref()).await {
        Ok(snapshot) => snapshot,
        Err(response) => return response,
    };
    let known: std::collections::HashSet<&str> = snapshot
        .store
        .issues
        .iter()
        .map(|record| record.id().as_str())
        .collect();
    let nodes: Vec<api::SeedGraphNode> = snapshot
        .store
        .issues
        .iter()
        .map(|record| api::SeedGraphNode {
            id:     record.id().to_string(),
            title:  record.title().to_string(),
            status: record.status().map(record_status_to_api),
            type_:  record.seed_type().map(record_type_to_api),
        })
        .collect();
    let known = &known;
    let edges: Vec<api::SeedGraphEdge> = snapshot
        .store
        .issues
        .iter()
        .flat_map(|record| {
            let from = record.id().to_string();
            record
                .blocked_by()
                .into_iter()
                .filter(move |to| known.contains(to))
                .map(move |to| api::SeedGraphEdge {
                    from: from.clone(),
                    to:   to.to_string(),
                })
        })
        .collect();
    (StatusCode::OK, Json(api::SeedGraph { nodes, edges })).into_response()
}

async fn get_seed(
    _auth: RequiredUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let snapshot = match source_snapshot(state.as_ref()).await {
        Ok(snapshot) => snapshot,
        Err(response) => return response,
    };
    match snapshot.store.issue(&id) {
        Some(record) => {
            let blocks = reverse_blocks(&snapshot.store, record.id().as_str());
            (StatusCode::OK, Json(seed_detail(record, blocks))).into_response()
        }
        None => ApiError::not_found(format!("Seed {id} not found.")).into_response(),
    }
}

/// Fetch a snapshot or render the documented `503`.
async fn source_snapshot(state: &AppState) -> Result<SeedsSnapshot, Response> {
    state.seeds_source.snapshot().await.map_err(|err| {
        let detail = err.to_string();
        let code = match &err {
            SeedsSourceError::Unconfigured => "seeds_source_unconfigured",
            SeedsSourceError::Unavailable(_) => "seeds_source_unavailable",
        };
        ApiError::with_code(StatusCode::SERVICE_UNAVAILABLE, detail, code).into_response()
    })
}

fn seed_summary(record: &SeedRecord) -> api::SeedSummary {
    api::SeedSummary {
        id:         record.id().to_string(),
        title:      record.title().to_string(),
        status:     record.status().map(record_status_to_api),
        type_:      record.seed_type().map(record_type_to_api),
        priority:   record.priority().map(|priority| i64::from(priority.get())),
        assignee:   record.assignee().map(str::to_string),
        labels:     record.labels().into_iter().map(str::to_string).collect(),
        blocked_by: record
            .blocked_by()
            .into_iter()
            .map(str::to_string)
            .collect(),
        created_at: record.created_at().and_then(rfc3339),
        updated_at: record.updated_at().and_then(rfc3339),
    }
}

/// The seeds format keeps no maintained reverse index: `blocks` is
/// derived here, over the whole store, from every record's `blockedBy`.
fn reverse_blocks(store: &Store, id: &str) -> Vec<String> {
    store
        .issues
        .iter()
        .filter(|other| other.blocked_by().contains(&id))
        .map(|other| other.id().to_string())
        .collect()
}

fn seed_detail(record: &SeedRecord, blocks: Vec<String>) -> api::SeedDetail {
    api::SeedDetail {
        id: record.id().to_string(),
        title: record.title().to_string(),
        description: record.description().map(str::to_string),
        status: record.status().map(record_status_to_api),
        type_: record.seed_type().map(record_type_to_api),
        priority: record.priority().map(|priority| i64::from(priority.get())),
        assignee: record.assignee().map(str::to_string),
        labels: record.labels().into_iter().map(str::to_string).collect(),
        blocked_by: record
            .blocked_by()
            .into_iter()
            .map(str::to_string)
            .collect(),
        blocks,
        created_at: record.created_at().and_then(rfc3339),
        updated_at: record.updated_at().and_then(rfc3339),
        fields: record.fields().clone(),
    }
}

fn record_status_to_api(status: RecordStatus) -> api::SeedStatus {
    match status {
        RecordStatus::Open => api::SeedStatus::Open,
        RecordStatus::InProgress => api::SeedStatus::InProgress,
        RecordStatus::Closed => api::SeedStatus::Closed,
    }
}

fn record_type_to_api(seed_type: RecordSeedType) -> api::SeedType {
    match seed_type {
        RecordSeedType::Task => api::SeedType::Task,
        RecordSeedType::Bug => api::SeedType::Bug,
        RecordSeedType::Feature => api::SeedType::Feature,
        RecordSeedType::Epic => api::SeedType::Epic,
    }
}

/// Parse a record timestamp; a malformed value degrades to absence rather
/// than failing the whole read.
fn rfc3339(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}
