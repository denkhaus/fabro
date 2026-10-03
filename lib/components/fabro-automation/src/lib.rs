mod breaker;
mod error;
mod id;
mod migrations;
mod model;
mod store;

pub use breaker::{
    BreakerCounter, DEFAULT_SCHEDULE_BREAKER_THRESHOLD, ScheduleBreakerState, breaker_signature,
    counts_as_breaker_failure,
};
pub use error::{AutomationStoreError, AutomationValidationError};
pub use fabro_types::GitHubRepositorySlug;
pub use id::{AutomationId, AutomationRevision, AutomationRevisionParseError, AutomationTriggerId};
pub use migrations::{
    EnvironmentSelectorBackfillReport, ImportReport, backfill_environment_selectors,
    import_legacy_directory_once,
};
pub use model::{
    ApiTrigger, Automation, AutomationDraft, AutomationGitWorkflowSource, AutomationOverlapPolicy,
    AutomationReplace, AutomationTrigger, ScheduleTrigger, parse_schedule_expression,
    validate_workflow_source,
};
pub use store::AutomationStore;
