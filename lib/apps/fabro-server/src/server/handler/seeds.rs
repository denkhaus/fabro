//! The run-scoped seeds read endpoint (fabro-3488 → rework 2026-10-19,
//! user decision): the tracker of the SELECTED run's repository, read
//! live from the run's own sandbox workspace — never a server-side
//! mirror of a fixed repository. List-only v1 (the graph and detail
//! endpoints left with the mirror); the tracker's writes stay with the
//! CLI surfaces.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use axum_extra::extract::Query as ExtraQuery;
use chrono::{DateTime, Utc};
use fabro_api::types as api;
use serde_json::Value;

use super::super::{ApiError, AppState};
use crate::principal_middleware::RequireRunScoped;
use crate::sandbox_access::{
    InspectionSandbox, RunSandboxSeedsSource, attach_running_run_sandbox, run_sandbox_gate,
};

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/runs/{id}/seeds", get(list_run_seeds))
}

/// List filters; every filter composes with AND semantics.
#[derive(Debug, Default, serde::Deserialize)]
struct SeedListParams {
    #[serde(rename = "page[limit]", default = "default_seed_page_limit")]
    limit:    u32,
    #[serde(rename = "page[offset]", default)]
    offset:   u32,
    #[serde(default)]
    status:   Option<String>,
    #[serde(default)]
    r#type:   Option<String>,
    #[serde(default)]
    assignee: Option<String>,
    #[serde(default)]
    label:    Option<String>,
}

fn default_seed_page_limit() -> u32 {
    500
}

/// The tracker of one run's repository, as its sandbox workspace holds it
/// right now. The read attaches the run's sandbox for the request's
/// duration (a terminal run's sandbox stops again when the guard drops).
async fn list_run_seeds(
    RequireRunScoped(run_id): RequireRunScoped,
    State(state): State<Arc<AppState>>,
    ExtraQuery(params): ExtraQuery<SeedListParams>,
) -> Response {
    let projection = match state.load_run_projection(&run_id).await {
        Ok(projection) => projection,
        Err(err) => return err.into_response(),
    };
    let Some(sandbox_record) = projection.sandbox.as_ref() else {
        return ApiError::new(
            StatusCode::NOT_FOUND,
            "run has no sandbox; the run-scoped seeds read needs the run's workspace",
        )
        .into_response();
    };
    let Some(instance) = sandbox_record.instance() else {
        return ApiError::new(
            StatusCode::NOT_FOUND,
            "run sandbox was not created; the run-scoped seeds read needs the run's workspace",
        )
        .into_response();
    };
    let Ok(access) = state.provider_access().await else {
        return ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "sandbox providers are not configured",
        )
        .into_response();
    };
    let handle = match attach_running_run_sandbox(&access, &instance, run_id).await {
        Ok(handle) => handle,
        Err(err) => {
            return ApiError::new(StatusCode::SERVICE_UNAVAILABLE, err.to_string()).into_response();
        }
    };
    // A terminal run's sandbox runs only for this request; a live run owns
    // its liveness (same guard policy as the Ask Fabro attach).
    let _guard = if projection.is_terminal() {
        InspectionSandbox::terminal(Arc::clone(&handle), Some(run_sandbox_gate(&run_id)))
    } else {
        InspectionSandbox::live(Arc::clone(&handle))
    };
    let source = RunSandboxSeedsSource {
        sandbox:      handle,
        tracker_path: format!("{}/.seeds/issues.jsonl", instance.runtime.working_directory),
    };
    let text = match source.tracker_text().await {
        Ok(text) => text,
        Err(err) => {
            return ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
                .into_response();
        }
    };
    let records = match parse_records(&text) {
        Ok(records) => records,
        Err(err) => {
            return ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
                .into_response();
        }
    };
    let matched: Vec<&Value> = records
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

/// Parse the tracker's compact JSON-per-line format. Lines without an
/// `id` are skipped; a non-JSON line is an error (a corrupt tracker must
/// surface, never silently shrink the response).
fn parse_records(text: &str) -> Result<Vec<Value>, String> {
    let mut records = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let record: Value = serde_json::from_str(line)
            .map_err(|err| format!("tracker line is not valid JSON: {err}"))?;
        if record.get("id").and_then(Value::as_str).is_some() {
            records.push(record);
        }
    }
    Ok(records)
}

fn matches_params(record: &Value, params: &SeedListParams) -> bool {
    if let Some(want) = &params.status {
        if record.get("status").and_then(Value::as_str) != Some(want.as_str()) {
            return false;
        }
    }
    if let Some(want) = &params.r#type {
        if record.get("type").and_then(Value::as_str) != Some(want.as_str()) {
            return false;
        }
    }
    if let Some(want) = &params.assignee {
        let assignee = record.get("assignee").and_then(Value::as_str);
        let matches = if want == "none" {
            assignee.is_none()
        } else {
            assignee == Some(want.as_str())
        };
        if !matches {
            return false;
        }
    }
    if let Some(want) = &params.label {
        let has_label = record
            .get("labels")
            .and_then(Value::as_array)
            .is_some_and(|labels| labels.iter().any(|l| l.as_str() == Some(want.as_str())));
        if !has_label {
            return false;
        }
    }
    true
}

fn seed_summary(record: &Value) -> api::SeedSummary {
    api::SeedSummary {
        id:         record
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        title:      record
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        status:     record
            .get("status")
            .and_then(Value::as_str)
            .and_then(|status| serde_json::from_value(Value::String(status.to_string())).ok()),
        type_:      record
            .get("type")
            .and_then(Value::as_str)
            .and_then(|kind| serde_json::from_value(Value::String(kind.to_string())).ok()),
        priority:   record.get("priority").and_then(Value::as_i64).or_else(|| {
            record
                .get("priority")
                .and_then(Value::as_str)
                .and_then(|p| p.parse().ok())
        }),
        assignee:   record
            .get("assignee")
            .filter(|value| !value.is_null())
            .and_then(Value::as_str)
            .map(str::to_string),
        labels:     record
            .get("labels")
            .and_then(Value::as_array)
            .map(|labels| {
                labels
                    .iter()
                    .filter_map(|label| label.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        // The dependency graph left with the mirror: v1 reports no
        // blockers (issues.jsonl carries none; blocks live in .seeds
        // graph files the run-scoped read does not parse).
        blocked_by: Vec::new(),
        created_at: record
            .get("createdAt")
            .and_then(Value::as_str)
            .and_then(rfc3339),
        updated_at: record
            .get("updatedAt")
            .and_then(Value::as_str)
            .and_then(rfc3339),
    }
}

fn rfc3339(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}
