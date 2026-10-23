//! The work domain (ADR-0032): forge-agnostic work items, the tracker port,
//! and the server-owned claim ledger.
//!
//! Two planes, two owners: work-item content lives in a forge behind
//! [`TrackerProvider`]; assignment and queue state live in [`WorkStore`].
//! Neither names a forge. The first adapter (`fabro-github`) talks to GitHub
//! Issues in one central private repository; a Forgejo adapter slots in
//! beside it without changes here.

mod store;
mod tracker;

pub use store::{
    ClaimState, DEFAULT_CLAIM_LABEL_PREFIX, GrantOutcome, GrantRequest, WorkClaim, WorkClaimEvent,
    WorkOutboxEntry, WorkProject, WorkStore,
};
pub use tracker::{NullTracker, TrackerProvider, WorkItem, WorkItemDraft, WorkItemFilter};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] sqlx::Error),
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("work project not found: {0}")]
    WorkProjectNotFound(String),
    #[error("work claim not found: {0}")]
    WorkClaimNotFound(String),
    #[error("work claim conflict: {0}")]
    WorkClaimConflict(String),
    #[error("work outbox entry not found: {0}")]
    WorkOutboxEntryNotFound(String),
    #[error("invalid work claim state: {0}")]
    InvalidClaimState(String),
    #[error("invalid work claim transition: {from} -> {to}")]
    InvalidClaimTransition { from: ClaimState, to: ClaimState },
    #[error("stored work record has an invalid {field} timestamp: {value}")]
    InvalidStoredTimestamp {
        record: &'static str,
        field:  &'static str,
        value:  i64,
    },
    #[error("{0}")]
    Other(String),
}
