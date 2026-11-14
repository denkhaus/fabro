//! The forge pull-request port: the label door's provider-agnostic half
//! (ADR-0040's session integration, work#1205).
//!
//! Like the tracker port above, this port is deliberately forge-agnostic
//! — no GitHub, Forgejo or vendor vocabulary in its surface. An adapter
//! maps the port onto one forge: `fabro-github` ships the GitHub adapter
//! (labels through the issues endpoint, merges through the pulls
//! endpoint), a Forgejo or GitLab adapter can be added later without
//! touching this port, the server handler, or the merger workflow that
//! reads the door through the CLI.
//!
//! `id` is an opaque provider-stable identity (GitHub and Forgejo both
//! spell it the pull-request number; GitLab would spell its iid) — the
//! wire and the CLI carry it as a string and never parse it.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// How a merge commit lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForgeMergeMethod {
    /// A merge commit joins the branches.
    Merge,
    /// The commits squash into one.
    Squash,
    /// The commits rebase onto the target.
    Rebase,
}

/// One pull request as its forge sees it — the label door's read shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgePullRequest {
    /// Opaque provider-stable identity (GitHub/Forgejo: the number).
    pub id:     String,
    pub title:  String,
    /// The forge's own state word (`open`, `closed`).
    pub state:  String,
    pub url:    String,
    pub labels: Vec<String>,
}

/// What a merge through the port can refuse with — adapter-mapped, so
/// the handler maps statuses without knowing the forge.
#[derive(Debug, thiserror::Error)]
pub enum ForgePullRequestError {
    /// The forge has no pull request with this id in this repository.
    #[error("the forge has no such pull request")]
    NotFound,
    /// The transport or the forge refused for another reason.
    #[error("{0}")]
    Failed(String),
}

/// The pull-request half of a forge: list by state and label, merge by
/// id. Repositories are provider-stable `owner/name` spellings.
#[async_trait]
pub trait PullRequestForge: Send + Sync {
    /// Pull requests of one repository, optionally narrowed by label —
    /// the label door's worklist read.
    async fn list_pull_requests(
        &self,
        repo: &str,
        state: &str,
        label: Option<&str>,
    ) -> Result<Vec<ForgePullRequest>, ForgePullRequestError>;

    /// Merge one pull request. Idempotent at the adapter's discretion (a
    /// forge reporting an already-merged pull request as merged is a
    /// success, not an error).
    async fn merge_pull_request(
        &self,
        repo: &str,
        id: &str,
        method: ForgeMergeMethod,
    ) -> Result<(), ForgePullRequestError>;
}
