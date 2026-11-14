//! The GitHub adapter for the forge pull-request port (ADR-0040's label
//! door, work#1205).
//!
//! Like the tracker adapter beside it, this maps the forge-agnostic port
//! onto one forge: pull requests are GitHub pull requests, the opaque
//! `id` is the pull-request number spelled as a string, labels ride the
//! issues endpoint (the pulls endpoint does not carry them), and merges
//! go through the pulls endpoint with GitHub's merge vocabulary. A
//! Forgejo adapter can implement the same port beside this one; the
//! port, the server handler, and the merger workflow change nothing.
//!
//! The adapter OWNS its credentials and HTTP client (the port object is
//! `Send + Sync + 'static` behind an `Arc`); each call binds a borrowed
//! [`GitHubContext`] over them.

use async_trait::async_trait;
use fabro_types::settings::run::MergeStrategy;
use fabro_work::{ForgeMergeMethod, ForgePullRequest, ForgePullRequestError, PullRequestForge};

use crate::{GitHubContext, GitHubCredentials, ListedPullRequest};

/// The label door over GitHub, owning the server-resolved credentials
/// and HTTP client.
pub struct GitHubPullRequestForge {
    creds:       GitHubCredentials,
    base_url:    String,
    http_client: fabro_http::HttpClient,
}

impl GitHubPullRequestForge {
    #[must_use]
    pub fn new(
        creds: GitHubCredentials,
        base_url: impl Into<String>,
        http_client: fabro_http::HttpClient,
    ) -> Self {
        Self {
            creds,
            base_url: base_url.into(),
            http_client,
        }
    }

    fn context(&self) -> GitHubContext<'_> {
        GitHubContext::with_http_client(&self.creds, &self.base_url, self.http_client.clone())
    }
}

fn split_repo(repo: &str) -> Result<(&str, &str), ForgePullRequestError> {
    repo.split_once('/')
        .filter(|(owner, name)| {
            !owner.is_empty() && !name.is_empty() && !owner.contains('/') && !name.contains('/')
        })
        .ok_or_else(|| {
            ForgePullRequestError::Failed(format!(
                "repo must be owner/name (for example denkhaus/fabro), got '{repo}'"
            ))
        })
}

fn parse_id(id: &str) -> Result<u64, ForgePullRequestError> {
    id.parse::<u64>().map_err(|_| {
        ForgePullRequestError::Failed(format!(
            "pull request id '{id}' is not a GitHub pull-request number"
        ))
    })
}

fn listed_into_port(item: ListedPullRequest) -> ForgePullRequest {
    ForgePullRequest {
        id:     item.number.to_string(),
        title:  item.title,
        state:  item.state,
        url:    item.html_url,
        labels: item.labels,
    }
}

fn map_merge_method(method: ForgeMergeMethod) -> MergeStrategy {
    match method {
        ForgeMergeMethod::Merge => MergeStrategy::Merge,
        ForgeMergeMethod::Squash => MergeStrategy::Squash,
        ForgeMergeMethod::Rebase => MergeStrategy::Rebase,
    }
}

#[async_trait]
impl PullRequestForge for GitHubPullRequestForge {
    async fn list_pull_requests(
        &self,
        repo: &str,
        state: &str,
        label: Option<&str>,
    ) -> Result<Vec<ForgePullRequest>, ForgePullRequestError> {
        let (owner, repository) = split_repo(repo)?;
        crate::list_pull_requests(&self.context(), owner, repository, state, label)
            .await
            .map(|listing| listing.into_iter().map(listed_into_port).collect())
            .map_err(|err| ForgePullRequestError::Failed(err.to_string()))
    }

    async fn merge_pull_request(
        &self,
        repo: &str,
        id: &str,
        method: ForgeMergeMethod,
    ) -> Result<(), ForgePullRequestError> {
        let (owner, repository) = split_repo(repo)?;
        let number = parse_id(id)?;
        crate::merge_pull_request(
            &self.context(),
            owner,
            repository,
            number,
            map_merge_method(method),
        )
        .await
        .map_err(|err| match err {
            crate::PullRequestApiError::NotFound { .. } => ForgePullRequestError::NotFound,
            other => ForgePullRequestError::Failed(other.to_string()),
        })
    }
}
