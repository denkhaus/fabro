use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use fabro_types::{
    Graph, RunProjection, StageHandler, StageId, StageProjection, StageState, StageTiming,
    usage_is_empty,
};

use super::super::{
    AppState, IntoResponse, Json, ListResponse, PaginationParams, Path, Query, RequiredUser,
    Response, Router, RunId, RunStage, RunUsage, RunUsageStage, RunUsageTotals, State, StatusCode,
    UsageByModel, UsageStageRef, get, parse_run_id_path,
};

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/runs/{id}/stages", get(list_run_stages))
        .route("/runs/{id}/usage", get(get_run_usage))
}

fn run_stage_from_projection(
    stage_id: &StageId,
    stage: &StageProjection,
    graph: &Graph,
    now: DateTime<Utc>,
) -> RunStage {
    let handler = stage.handler.unwrap_or_else(|| {
        StageHandler::from_handler_type(
            graph
                .nodes
                .get(stage_id.node_id())
                .and_then(|node| node.handler_type()),
        )
    });
    let (parallel_group_id, parallel_branch_index) = stage
        .parallel_branch_id
        .as_ref()
        .map(|branch_id| (branch_id.group().clone(), branch_id.index()))
        .unzip();
    RunStage {
        id: stage_id.clone(),
        name: stage_id.node_id().to_owned(),
        handler,
        usage: stage.usage,
        status: stage.effective_state(),
        wall_time_ms: stage.live_wall_time_ms(now),
        node_id: stage_id.node_id().to_owned(),
        visit: std::num::NonZeroU32::new(stage_id.visit())
            .expect("StageId stores a non-zero visit"),
        provider_used: stage.provider_used.clone(),
        started_at: stage.started_at,
        graph_visit: stage.graph_visit.and_then(std::num::NonZeroU32::new),
        resumed_from_stage_id: stage.resumed_from_stage_id.clone(),
        parallel_group_id,
        parallel_branch_index,
    }
}

async fn list_run_stages(
    _auth: RequiredUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(_pagination): Query<PaginationParams>,
) -> Response {
    let id = match parse_run_id_path(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };

    let projection = match state.load_run_projection(&id).await {
        Ok(projection) => projection,
        Err(err) => return err.into_response(),
    };

    let now = Utc::now();
    let graph = projection.spec().graph();
    let stages = projection
        .iter_stages()
        .map(|(stage_id, stage)| run_stage_from_projection(stage_id, stage, graph, now))
        .collect::<Vec<_>>();

    (StatusCode::OK, Json(ListResponse::new(stages))).into_response()
}

async fn get_run_usage(
    _auth: RequiredUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<RunId>,
) -> Response {
    let projection = match state.load_run_projection(&id).await {
        Ok(projection) => projection,
        Err(err) => return err.into_response(),
    };

    let rollup = fabro_workflow::usage_rollup_from_projection(&projection);
    let by_model = rollup
        .by_model
        .iter()
        .map(|model| UsageByModel {
            model:  model.model.clone(),
            stages: model.stages,
            usage:  model.usage,
        })
        .collect::<Vec<_>>();

    let rollup_by_node = rollup
        .stages
        .iter()
        .map(|stage| (stage.node_id.as_str(), stage))
        .collect::<HashMap<_, _>>();
    let live_rows = live_usage_rows(&projection, Utc::now());
    let totals_timing = live_rows.iter().fold(StageTiming::default(), |acc, row| {
        acc.saturating_add(&row.timing)
    });
    let stages = live_rows
        .into_iter()
        .map(|row| {
            let rollup_stage = rollup_by_node.get(row.node_id.as_str());
            RunUsageStage {
                usage:      rollup_stage.map(|stage| stage.usage).unwrap_or_default(),
                model:      rollup_stage.and_then(|stage| stage.model.as_ref()).cloned(),
                timing:     row.timing,
                stage:      UsageStageRef {
                    id:   row.node_id.clone(),
                    name: row.node_id,
                },
                started_at: row.started_at,
                state:      row.state,
            }
        })
        .collect::<Vec<_>>();

    let response = RunUsage {
        by_model,
        stages,
        totals: RunUsageTotals {
            timing: totals_timing.into(),
            usage:  rollup.totals,
        },
    };

    (StatusCode::OK, Json(response)).into_response()
}

struct LiveUsageRow {
    node_id:      String,
    timing:       StageTiming,
    started_at:   Option<DateTime<Utc>>,
    state:        Option<StageState>,
    latest_visit: u32,
}

fn live_usage_rows(projection: &RunProjection, now: DateTime<Utc>) -> Vec<LiveUsageRow> {
    let mut row_indices = HashMap::<String, usize>::new();
    let mut rows = Vec::<LiveUsageRow>::new();

    for (stage_id, stage) in projection.iter_stages() {
        let node_id = stage_id.node_id();
        if projection.is_boundary_stage(node_id) || !stage_has_usage_row(stage) {
            continue;
        }

        let index = *row_indices.entry(node_id.to_string()).or_insert_with(|| {
            let index = rows.len();
            rows.push(LiveUsageRow {
                node_id:      node_id.to_string(),
                timing:       StageTiming::default(),
                started_at:   None,
                state:        None,
                latest_visit: 0,
            });
            index
        });
        let row = &mut rows[index];
        let stage_timing = stage.live_timing(now);
        row.timing = row.timing.saturating_add(&stage_timing);

        if stage_id.visit() >= row.latest_visit {
            row.latest_visit = stage_id.visit();
            row.started_at = stage.started_at;
            row.state = Some(stage.effective_state());
        }
    }

    rows
}

fn stage_has_usage_row(stage: &StageProjection) -> bool {
    stage.completion.is_some()
        || stage.timing.is_some()
        || !usage_is_empty(&stage.usage)
        || stage.started_at.is_some()
}
