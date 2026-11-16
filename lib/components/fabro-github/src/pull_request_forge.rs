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

/// Merge through the pulls API, resolving the merge-in-progress race
/// (work#1368): GitHub answers 405 "Merge already in progress" while a
/// merge — ours or an overlapping one — is landing, and the landed
/// state follows seconds later. The port counts an already-merged pull
/// request as a SUCCESS (the trait's `merge_pull_request` contract), so
/// the adapter re-reads the pull request once and lets the landed state
/// answer. Every other refusal stays the error it was.
async fn merge_pull_request_resolving_race(
    client: &impl crate::HttpClient,
    ctx: &GitHubContext<'_>,
    owner: &str,
    repository: &str,
    number: u64,
    method: MergeStrategy,
) -> Result<(), ForgePullRequestError> {
    match crate::merge_pull_request_with_client(client, ctx, owner, repository, number, method)
        .await
    {
        Ok(()) => Ok(()),
        Err(crate::PullRequestApiError::NotFound { .. }) => Err(ForgePullRequestError::NotFound),
        Err(err) => {
            if merge_in_progress_race(&err).is_some() {
                // One re-read decides: a landed merge is success (the
                // port contract), an unlanded one keeps the original
                // refusal. A failed re-read never upgrades the error —
                // the original 405 stays the answer.
                if let Ok(detail) =
                    crate::get_pull_request_with_client(client, ctx, owner, repository, number)
                        .await
                {
                    if detail.merged {
                        return Ok(());
                    }
                }
            }
            Err(ForgePullRequestError::Failed(err.to_string()))
        }
    }
}

/// The 405 "Merge already in progress" race and nothing else: a real
/// conflict (405-not-mergeable, 409) must keep its typed meaning — the
/// merger parks on it (fabro #1117).
fn merge_in_progress_race(err: &crate::PullRequestApiError) -> Option<u64> {
    match err {
        crate::PullRequestApiError::Conflict {
            number,
            status: 405,
            body,
        } if body.contains("Merge already in progress") => Some(*number),
        _ => None,
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
        let ctx = self.context();
        let client = ctx
            .http_client()
            .map_err(|err| ForgePullRequestError::Failed(err.to_string()))?;
        merge_pull_request_resolving_race(
            &client,
            &ctx,
            owner,
            repository,
            number,
            map_merge_method(method),
        )
        .await
    }
}

#[cfg(test)]
mod merge_race_tests {
    use fabro_types::settings::run::MergeStrategy;
    use fabro_work::ForgePullRequestError;

    use super::merge_pull_request_resolving_race;
    use crate::tests_mock::MockHttpClient;
    use crate::{GitHubContext, GitHubCredentials, HttpMethod};

    async fn merge_with(mock: &MockHttpClient) -> Result<(), ForgePullRequestError> {
        let creds = GitHubCredentials::Pat("test-token".to_string());
        let ctx = GitHubContext::new(&creds, "https://api.github.com");
        merge_pull_request_resolving_race(
            mock,
            &ctx,
            "denkhaus",
            "fabro",
            565,
            MergeStrategy::Merge,
        )
        .await
    }

    #[tokio::test]
    async fn a_merge_in_progress_race_lands_as_success() {
        let mock = MockHttpClient::new()
            .on(
                HttpMethod::Put,
                "/pulls/565/merge",
                405,
                r#"{"message":"Merge already in progress"}"#,
            )
            .on(
                HttpMethod::Get,
                "/pulls/565",
                200,
                r#"{"number":565,"state":"closed","draft":false,"merged":true,"title":"landed","body":null,"additions":1,"deletions":1,"changed_files":1,"html_url":"https://github.com/denkhaus/fabro/pull/565","user":{"login":"fabro-the-factory[bot]"},"head":{"ref":"feat/x"},"base":{"ref":"denkhaus"},"created_at":"2026-11-16T10:00:00Z","updated_at":"2026-11-16T11:00:00Z"}"#,
            );
        merge_with(&mock)
            .await
            .expect("a landed merge answers success (work#1368: PR #565)");
    }

    #[tokio::test]
    async fn a_real_conflict_stays_failed_without_a_reread() {
        let mock = MockHttpClient::new().on(
            HttpMethod::Put,
            "/pulls/565/merge",
            405,
            r#"{"message":"Pull Request is not mergeable"}"#,
        );
        let err = merge_with(&mock).await.expect_err("a real conflict fails");
        assert!(
            matches!(err, ForgePullRequestError::Failed(ref detail) if detail.contains("not mergeable"))
        );
        assert_eq!(mock.request_count(), 1, "no re-read for a real conflict");
    }

    #[tokio::test]
    async fn an_unlanded_race_stays_failed() {
        let mock = MockHttpClient::new()
            .on(
                HttpMethod::Put,
                "/pulls/565/merge",
                405,
                r#"{"message":"Merge already in progress"}"#,
            )
            .on(
                HttpMethod::Get,
                "/pulls/565",
                200,
                r#"{"number":565,"state":"open","draft":false,"merged":false,"title":"still open","body":null,"additions":1,"deletions":1,"changed_files":1,"html_url":"https://github.com/denkhaus/fabro/pull/565","user":{"login":"fabro-the-factory[bot]"},"head":{"ref":"feat/x"},"base":{"ref":"denkhaus"},"created_at":"2026-11-16T10:00:00Z","updated_at":"2026-11-16T11:00:00Z"}"#,
            );
        let err = merge_with(&mock).await.expect_err("an unlanded race fails");
        assert!(
            matches!(err, ForgePullRequestError::Failed(ref detail) if detail.contains("Merge already in progress"))
        );
    }
}
