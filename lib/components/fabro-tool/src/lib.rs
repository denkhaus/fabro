#![allow(
    dead_code,
    reason = "Tool DTO fields are consumed by serde and schema generation even when not read \
              directly."
)]

mod ask;
mod blob;
mod common;
mod create;
mod events;
pub mod fabro_client;
mod fork_duplicate_child_guard;
#[cfg(test)]
mod fork_duplicate_child_guard_tests;
mod gather;
mod get;
mod interact;
mod manifest;
mod observation;
mod pair;
pub mod run_mark;
mod run_queue;
mod run_title;
mod search;
mod seeds;
mod wait;
mod work;
mod workflow_version;

pub use ask::{
    AskResult, AskTurnCollector, AskTurnOutcome, AskTurnStatus, FabroAskParams, ValidatedAsk,
    ask_run, ask_run_text,
};
pub use blob::{
    BlobResult, FabroBlobParams, ValidatedBlob, blob_page, blob_page_text, count_lines,
};
pub use common::{
    FABRO_ASK_TOOL_NAME, FABRO_BLOB_TOOL_NAME, FABRO_OBSERVATION_LIST_TOOL_NAME,
    FABRO_OBSERVATION_RECORD_TOOL_NAME, FABRO_RUN_CREATE_TOOL_NAME, FABRO_RUN_EVENTS_TOOL_NAME,
    FABRO_RUN_GATHER_TOOL_NAME, FABRO_RUN_GET_TOOL_NAME, FABRO_RUN_INTERACT_TOOL_NAME,
    FABRO_RUN_MARK_TOOL_NAME, FABRO_RUN_PAIR_TOOL_NAME, FABRO_RUN_QUEUE_TOOL_NAME,
    FABRO_RUN_SEARCH_TOOL_NAME, FABRO_RUN_SET_TITLE_TOOL_NAME, FABRO_RUN_WAIT_TOOL_NAME,
    FABRO_SEEDS_SEARCH_TOOL_NAME, FABRO_WORK_CLOSE_TOOL_NAME, FABRO_WORK_COMMENTS_TOOL_NAME,
    FABRO_WORK_CREATE_TOOL_NAME, FABRO_WORK_LIST_TOOL_NAME, FABRO_WORK_NEXT_TOOL_NAME,
    FABRO_WORK_RADIUS_TOOL_NAME, FABRO_WORK_RELEASE_TOOL_NAME, FABRO_WORK_SHOW_TOOL_NAME,
    FABRO_WORK_UPDATE_TOOL_NAME, FABRO_WORKFLOW_VERSION_CREATE_TOOL_NAME, FabroToolBackend,
    RunSeedsSource, RunSummaryResult, ToolDefinition, ToolError, ToolResult, tool_definitions,
};
pub use create::{
    CreateRunOptions, CreateRunSpec, CreateRunsResult, CreatedRunResult, FabroRunCreateParams,
    create_runs, create_runs_text, create_runs_with_options,
};
pub use events::{
    FabroRunEventsParams, RunEventResult, RunEventsAction, RunEventsResult, ValidatedRunEvents,
    run_events, run_events_text,
};
pub use gather::{
    FabroRunGatherParams, GatherRunsResult, ValidatedGatherRuns, gather_runs, gather_runs_text,
};
pub use get::{FabroRunGetParams, RunGetResult, ValidatedRunGet, run_get, run_get_text};
pub use interact::{
    AnswerValue, FabroRunInteractParams, InteractRunResult, RunInteractAction,
    ValidatedInteractAction, ValidatedInteractRun, interact_run, interact_run_text,
};
pub use manifest::json_to_toml_value;
pub use observation::{
    FabroObservationCategory, FabroObservationListParams, FabroObservationRecordParams,
    MAX_OBSERVATION_CHARS, ObservationListResult, ObservationRecordResult, ValidatedObservation,
    list_observations, list_observations_text, record_observation, record_observation_text,
};
pub use pair::{
    FabroRunPairParams, PairRunResult, RunPairAction, ValidatedPairAction, ValidatedPairRun,
    pair_run, pair_run_text,
};
pub use run_mark::{
    FabroOrchestrationField, FabroOrchestrationVerdict, FabroRunMarkParams, RunMarkOutcome,
    RunMarkResult, ValidatedRunMark, mark_run, mark_run_text,
};
pub use run_queue::{
    FabroRunQueueParams, RunQueueResult, RunQueueSummary, ValidatedRunQueue, run_queue,
    run_queue_text,
};
pub use run_title::{
    FabroRunSetTitleParams, RunSetTitleResult, ValidatedRunSetTitle, set_run_title,
    set_run_title_text,
};
pub use search::{
    FabroRunSearchParams, SearchRunSummaryResult, SearchRunsResult, ValidatedSearchRuns,
    search_runs, search_runs_text,
};
pub use seeds::{
    FabroSeedsSearchParams, SeedsRow, SeedsSearchResult, ValidatedSeedsSearchParams,
    apply_tracker_filters, filter_seed_rows, seeds_search, seeds_search_text,
};
pub use wait::{
    FabroRunWaitParams, RunWaitOutcome, RunWaitReached, RunWaitUntil, ValidatedRunWait, run_wait,
    run_wait_text,
};
pub use work::{
    FabroWorkCloseParams, FabroWorkCommentsParams, FabroWorkCreateParams, FabroWorkListParams,
    FabroWorkNextParams, FabroWorkRadiusParams, FabroWorkReleaseParams, FabroWorkShowParams,
    FabroWorkUpdateParams, ValidatedWorkItemRead, ValidatedWorkList, ValidatedWorkNext,
    WorkNextResult, work_close, work_close_text, work_comments, work_comments_text, work_create,
    work_create_text, work_list, work_list_text, work_next, work_next_text, work_radius,
    work_radius_text, work_release, work_release_text, work_show, work_show_text, work_update,
    work_update_text,
};
pub use workflow_version::{
    FabroWorkflowVersionCreateParams, ValidatedWorkflowVersionCreate, WorkflowVersionPackager,
    create_workflow_version, workflow_version_create_text,
};
