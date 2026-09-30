//! A GitHub-target run's repository work in its worker: the read credential
//! its workspaces are fetched with, and its publication when it succeeds.
//!
//! The worker resolves the server's GitHub credentials itself, as the
//! legacy worker did: the strategy and App id from the server settings the
//! server named (`FABRO_CONFIG`), the App key the server hands the worker,
//! or `GITHUB_TOKEN` from the worker's vault snapshot. From them it mints a
//! read-only token for the checkout inside the sandbox when the run starts,
//! and a push token when the run ends, so a long run never pushes with an
//! expired token.
//!
//! Publication runs in Fabro's `run_finished` hook, after the last stage and
//! before the run's terminal record, as the legacy publish step did: the
//! final checkpoint is pushed from the run's snapshot repository to the run
//! branch on GitHub, and, when the run changed files and its settings ask
//! for one, a pull request is opened and recorded. A failure fails the run
//! with `publish_failed`.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use fabro_config::ServerSettingsBuilder;
use fabro_github::{GitCloneCredentials, GitHubContext, GitHubCredentials};
use fabro_llm::credentials::{CredentialProvider, readiness};
use fabro_llm::lithos_catalog::Catalog;
use fabro_petri::hooks::{Publication, RunPublisher};
use fabro_petri::platform_records::PlatformRecords;
use fabro_petri::source::SourceCredential;
use fabro_static::EnvVars;
use fabro_store::platform_records::{PlatformRecord, PullRequestCreatedRecord};
use fabro_types::settings::run::{PullRequestSettings, RunMode};
use fabro_types::settings::server::GithubIntegrationStrategy;
use fabro_types::{GitHubRepositorySlug, RunId, RunSpec, RunTarget};
use fabro_vault::Vault;
use fabro_workflow::pull_request::{self, AutoMergeOptions, OpenPullRequestRequest};
use tokio::process::Command;
use tokio::time;
use tracing::warn;

/// How long one push to the repository may take.
const PUSH_TIMEOUT: Duration = Duration::from_mins(5);
/// Attempts at the push: a freshly minted token can take a moment to reach
/// every GitHub replica.
const PUSH_ATTEMPTS: u32 = 3;
const PUSH_RETRY_DELAY: Duration = Duration::from_secs(2);

/// The server's GitHub credentials as the worker reaches them, or `None`
/// when none are configured.
pub(super) fn github_credentials(vault: &Vault) -> Result<Option<GitHubCredentials>> {
    let settings = ServerSettingsBuilder::load_default().context("loading the server settings")?;
    let github = &settings.server.integrations.github;
    match github.strategy {
        GithubIntegrationStrategy::App => {
            GitHubCredentials::from_env_with_slug(github.app_id.as_deref(), github.slug.as_deref())
                .map_err(anyhow::Error::msg)
        }
        GithubIntegrationStrategy::Token => {
            let Some(token) = vault
                .get(EnvVars::GITHUB_TOKEN)
                .map(str::trim)
                .filter(|token| !token.is_empty())
            else {
                return Ok(None);
            };
            fabro_github::validate_static_github_token(token)?;
            Ok(Some(GitHubCredentials::Pat(token.to_string())))
        }
    }
}

/// The run's GitHub repository, when its target names one.
fn repository(spec: &RunSpec) -> Option<GitHubRepositorySlug> {
    let Some(RunTarget::Git(target)) = spec.target.as_ref() else {
        return None;
    };
    Some(target.clone().validate().ok()?.repository().clone())
}

/// The read-only credential the run's workspaces are fetched with. `None`
/// when the run has no GitHub target or no credentials resolve; a public
/// repository is then fetched anonymously.
pub(super) async fn source_credential(
    spec: &RunSpec,
    credentials: Option<&GitHubCredentials>,
) -> Option<SourceCredential> {
    let repository = repository(spec)?;
    let credentials = credentials?;
    let base_url = fabro_github::github_api_base_url();
    let context = GitHubContext::new(credentials, &base_url);
    match fabro_github::resolve_read_only_clone_credentials(
        &context,
        repository.owner(),
        repository.repo(),
    )
    .await
    {
        Ok(credentials) => SourceCredential::from_encoded(encode(&credentials)),
        Err(err) => {
            warn!(repository = %repository, error = %err, "no read credential for the run's repository; it is fetched anonymously");
            None
        }
    }
}

fn encode(credentials: &GitCloneCredentials) -> String {
    BASE64_STANDARD.encode(format!(
        "{}:{}",
        credentials.username(),
        credentials.password()
    ))
}

/// A successful GitHub-target run's publication: the run branch pushed, and
/// the pull request its settings ask for opened.
pub(super) struct GitHubPublisher {
    run_id:       RunId,
    spec:         RunSpec,
    repository:   GitHubRepositorySlug,
    credentials:  Option<GitHubCredentials>,
    pull_request: Option<PullRequestSettings>,
    llm_source:   Arc<dyn CredentialProvider>,
    catalog:      Arc<Catalog>,
    records:      Arc<dyn PlatformRecords>,
    client:       fabro_client::Client,
}

impl GitHubPublisher {
    /// The publisher of a run whose target is a GitHub repository and whose
    /// run branch is pushed; `None` for a dry run or any other run.
    pub(super) fn for_run(
        run_id: RunId,
        spec: &RunSpec,
        credentials: Option<GitHubCredentials>,
        llm_source: Arc<dyn CredentialProvider>,
        catalog: Arc<Catalog>,
        records: Arc<dyn PlatformRecords>,
        client: fabro_client::Client,
    ) -> Option<Self> {
        let settings = &spec.settings.run;
        if settings.execution.mode == RunMode::DryRun
            || !settings.run_branch.enabled
            || !settings.run_branch.push
        {
            return None;
        }
        let repository = repository(spec)?;
        Some(Self {
            run_id,
            spec: spec.clone(),
            repository,
            credentials,
            pull_request: settings
                .pull_request
                .clone()
                .filter(|pull_request| pull_request.enabled),
            llm_source,
            catalog,
            records,
            client,
        })
    }

    /// The model that writes the pull request: the run's, or the catalog's
    /// default among the providers whose credentials resolve.
    async fn model(&self) -> Option<String> {
        if let Some(model) = self.spec.settings.run.model.name.clone() {
            return Some(model);
        }
        let ready = readiness(self.catalog.enabled_providers(), self.llm_source.as_ref()).await;
        self.catalog
            .default_offering_for(&ready.ready)
            .map(|entry| entry.model.id().to_string())
    }

    async fn open_pull_request(
        &self,
        context: GitHubContext<'_>,
        settings: &PullRequestSettings,
        publication: &Publication,
    ) -> Result<(), String> {
        let Some(RunTarget::Git(target)) = self.spec.target.as_ref() else {
            return Err("pull request creation requires a GitHub target".to_string());
        };
        let model = self
            .model()
            .await
            .ok_or_else(|| "no LLM model is available to write the pull request".to_string())?;
        // The run so far, for the pull request's details: best effort.
        let run_state = self.client.get_run_state(&self.run_id).await.ok();
        let origin_url = self.repository.https_url();
        let created = pull_request::open_pull_request(OpenPullRequestRequest {
            github:            context,
            origin_url:        &origin_url,
            base_branch:       &target.branch,
            head_branch:       &publication.run_branch,
            expected_head_sha: &publication.head_sha,
            goal:              &self.spec.graph.goal,
            diff:              &publication.patch,
            model:             &model,
            draft:             settings.draft,
            auto_merge:        settings.auto_merge.then_some(AutoMergeOptions {
                merge_strategy: settings.merge_strategy,
            }),
            llm_source:        Arc::clone(&self.llm_source),
            catalog:           Arc::clone(&self.catalog),
            conclusion:        None,
            run_state:         run_state.as_ref(),
        })
        .await
        .map_err(|err| format!("failed to create pull request: {err}"))?;
        let link = &created.link;
        let record = PlatformRecord::PullRequestCreated(PullRequestCreatedRecord {
            number:    link.number,
            owner:     link.owner.clone(),
            repo:      link.repo.clone(),
            html_url:  link.html_url(),
            head_sha:  Some(publication.head_sha.clone()),
            draft:     settings.draft,
            operation: None,
        });
        self.records
            .append(&self.run_id, &record, None)
            .await
            .map_err(|err| format!("the pull request was opened but not recorded: {err}"))?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl RunPublisher for GitHubPublisher {
    async fn publish(&self, publication: &Publication) -> Result<(), String> {
        let credentials = self.credentials.as_ref().ok_or_else(|| {
            "pushing the run branch requires the server's GitHub credentials".to_string()
        })?;
        let base_url = fabro_github::github_api_base_url();
        let context = GitHubContext::new(credentials, &base_url);
        let push_credentials = fabro_github::resolve_clone_credentials(
            &context,
            self.repository.owner(),
            self.repository.repo(),
        )
        .await
        .map_err(|err| format!("no push credential for {}: {err:#}", self.repository))?;
        push(&self.repository, &push_credentials, publication).await?;
        let Some(settings) = &self.pull_request else {
            return Ok(());
        };
        if publication.patch.trim().is_empty() {
            return Ok(());
        }
        Box::pin(self.open_pull_request(context, settings, publication)).await
    }
}

/// Push the run's final commit from its snapshot repository to the run
/// branch on GitHub, retrying a failure that may be a token still
/// replicating. The credential reaches `git` as an HTTP header for the
/// repository alone and never appears in the error.
async fn push(
    repository: &GitHubRepositorySlug,
    credentials: &GitCloneCredentials,
    publication: &Publication,
) -> Result<(), String> {
    let mut url = repository.https_url();
    url.push_str(".git");
    let header = format!("AUTHORIZATION: basic {}", encode(credentials));
    let env = [
        ("GIT_CONFIG_COUNT", "1".to_string()),
        ("GIT_CONFIG_KEY_0", format!("http.{url}.extraheader")),
        ("GIT_CONFIG_VALUE_0", header),
    ];
    let refspec = format!(
        "{}:refs/heads/{}",
        publication.head_sha, publication.run_branch
    );
    let mut last = String::new();
    for attempt in 1..=PUSH_ATTEMPTS {
        let output = git(
            &publication.snapshot_repository,
            &["push", "--quiet", &url, &refspec],
            &env,
        )
        .await?;
        if output.status.success() {
            return Ok(());
        }
        last = String::from_utf8_lossy(&output.stderr)
            .trim()
            .replace(credentials.password(), "***");
        warn!(
            attempt,
            branch = publication.run_branch,
            error = last,
            "pushing the run branch failed"
        );
        if attempt < PUSH_ATTEMPTS {
            time::sleep(PUSH_RETRY_DELAY).await;
        }
    }
    Err(format!(
        "the run branch {} could not be pushed to {repository}: {last}",
        publication.run_branch
    ))
}

/// `git` in `directory` with `env` added, non-interactive, bounded.
async fn git(
    directory: &Path,
    args: &[&str],
    env: &[(&str, String)],
) -> Result<std::process::Output, String> {
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(directory)
        .envs(env.iter().map(|(key, value)| (*key, value)))
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    match time::timeout(PUSH_TIMEOUT, command.output()).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(err)) => Err(format!("git could not run: {err}")),
        Err(_) => Err(format!("git push timed out after {PUSH_TIMEOUT:?}")),
    }
}

#[cfg(test)]
mod tests {
    use fabro_llm::credentials::NoCredentials;
    use fabro_llm::test_support;
    use fabro_petri::test_support::MemoryPlatformRecords;
    use fabro_types::GitRunTarget;
    use fabro_types::test_support::test_run_spec;

    use super::*;

    fn spec() -> RunSpec {
        let mut spec = test_run_spec();
        spec.target = Some(RunTarget::Git(GitRunTarget {
            repo:   "acme/widgets".to_string(),
            branch: "main".to_string(),
            tag:    None,
            sha:    None,
        }));
        spec.settings.run.run_branch.enabled = true;
        spec.settings.run.run_branch.push = true;
        spec
    }

    #[test]
    fn only_a_pushed_github_target_run_is_published() {
        let publishes = |spec: &RunSpec| {
            GitHubPublisher::for_run(
                RunId::new(),
                spec,
                None,
                Arc::new(NoCredentials),
                Arc::new(test_support::test_catalog()),
                Arc::new(MemoryPlatformRecords::new()),
                fabro_client::Client::new_no_proxy("http://127.0.0.1:9").unwrap(),
            )
            .is_some()
        };
        assert!(publishes(&spec()));

        let mut dry = spec();
        dry.settings.run.execution.mode = RunMode::DryRun;
        assert!(!publishes(&dry));

        let mut unpushed = spec();
        unpushed.settings.run.run_branch.push = false;
        assert!(!publishes(&unpushed));

        let mut empty = spec();
        empty.target = Some(RunTarget::None {});
        assert!(!publishes(&empty));
    }
}
