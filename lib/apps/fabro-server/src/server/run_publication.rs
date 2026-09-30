//! A GitHub-target run's work leaves for its repository from the server.
//!
//! The run's workspaces are checked out from the repository inside their
//! sandboxes, and every checkpoint reaches the run's snapshot repository on
//! this host (`fabro_petri::checkpoint`). When the run ends, the server
//! pushes the final checkpoint to the run branch, `fabro/run/<id>`, on the
//! repository with its own GitHub credentials, then, for a successful run
//! with changes whose settings ask for one, records the pull request request
//! the creation supervisor opens. The sandbox never holds a credential that
//! can push.
//!
//! A run that did not succeed, a dry run, a run whose run branch is not
//! pushed, and a run with no GitHub target publish nothing. A push that
//! fails is a warning notice on the run, as is a pull request that cannot
//! be requested; neither changes the run's outcome.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use fabro_config::Storage;
use fabro_store::platform_records::{PlatformRecord, RunNoticeRecord};
use fabro_types::settings::run::RunMode;
use fabro_types::{RunId, RunNoticeLevel, RunSpec, RunTarget};
use tokio::process::Command;
use tokio::{fs, time};
use tracing::{info, warn};

use super::handler::pull_requests::request_pull_request_creation;
use super::{AppState, run_records};
use crate::git_checkout::{self, GitAuthConfig};

/// How long one push to the repository may take.
const PUSH_TIMEOUT: Duration = Duration::from_mins(5);
/// Attempts at the push: a fresh token can take a moment to reach every
/// GitHub replica.
const PUSH_ATTEMPTS: u32 = 3;
const PUSH_RETRY_DELAY: Duration = Duration::from_secs(2);

/// The read-only credential a run's worker fetches its GitHub target with,
/// as the base64 of `username:password`. `None` when the run checks nothing
/// out, or the server has no GitHub credentials for the repository (a
/// public repository is then fetched anonymously).
pub(crate) async fn clone_credential(state: &AppState, spec: &RunSpec) -> Option<String> {
    let Some(RunTarget::Git(target)) = spec.target.as_ref() else {
        return None;
    };
    let settings = &spec.settings.run;
    if !settings.clone.enabled || settings.execution.mode == RunMode::DryRun {
        return None;
    }
    let validated = target.clone().validate().ok()?;
    let github = &state.server_settings().server.integrations.github;
    let credentials = match state.github_credentials(github).await {
        Ok(Some(credentials)) => credentials,
        Ok(None) => return None,
        Err(err) => {
            warn!(error = %err, "GitHub credentials are unavailable; the run's repository is fetched anonymously");
            return None;
        }
    };
    let context = match state.http_client.clone() {
        Some(client) => fabro_github::GitHubContext::with_http_client(
            &credentials,
            &state.github_api_base_url,
            client,
        ),
        None => fabro_github::GitHubContext::new(&credentials, &state.github_api_base_url),
    };
    let repository = validated.repository();
    match fabro_github::resolve_read_only_clone_credentials(
        &context,
        repository.owner(),
        repository.repo(),
    )
    .await
    {
        Ok(credentials) => Some(BASE64_STANDARD.encode(format!(
            "{}:{}",
            credentials.username(),
            credentials.password()
        ))),
        Err(err) => {
            warn!(repository = %repository, error = %err, "no read credential for the run's repository; it is fetched anonymously");
            None
        }
    }
}

/// Publish the run's work once it has ended, in the background.
pub(crate) fn spawn(state: Arc<AppState>, run_id: RunId) {
    tokio::spawn(async move {
        if let Err(err) = publish(&state, run_id).await {
            warn!(run_id = %run_id, error = %err, "the run's work was not published");
            notice(&state, run_id, "run_publish_failed", &format!("{err:#}")).await;
        }
    });
}

/// What an ended run pushes: the repository, the branch, the commit, and
/// whether a pull request follows.
struct Publication {
    repository:   fabro_types::GitHubRepositorySlug,
    run_branch:   String,
    sha:          String,
    pull_request: bool,
    model:        Option<String>,
}

async fn publish(state: &Arc<AppState>, run_id: RunId) -> anyhow::Result<()> {
    state.petri_projector.settle(run_id).await;
    let Some(projection) = run_records::projection(state, run_id).await? else {
        return Ok(());
    };
    let Some(publication) = publication(&projection) else {
        return Ok(());
    };
    let snapshots = Storage::new(state.server_storage_dir())
        .run_scratch(&run_id)
        .root()
        .join("petri")
        .join("snapshots");
    let Some(repository) = snapshot_holding(&snapshots, &publication.sha).await else {
        anyhow::bail!(
            "the run's final commit {} is in none of its snapshot repositories",
            publication.sha
        );
    };
    push(state, &repository, &publication).await?;
    info!(
        run_id = %run_id,
        repository = %publication.repository,
        branch = publication.run_branch,
        sha = publication.sha,
        "run branch pushed"
    );
    if publication.pull_request {
        request_pull_request(state, run_id, publication.model).await;
    }
    Ok(())
}

/// What the ended run publishes, or `None` when it publishes nothing.
fn publication(projection: &fabro_store::RunProjection) -> Option<Publication> {
    let spec = &projection.spec;
    let Some(RunTarget::Git(target)) = spec.target.as_ref() else {
        return None;
    };
    let settings = &spec.settings.run;
    if settings.execution.mode == RunMode::DryRun
        || !settings.run_branch.enabled
        || !settings.run_branch.push
    {
        return None;
    }
    let conclusion = projection.conclusion.as_ref()?;
    if !conclusion.status.is_successful() {
        return None;
    }
    let sha = conclusion
        .final_git_commit_sha
        .clone()
        .filter(|sha| !sha.trim().is_empty())?;
    let run_branch = projection
        .start
        .as_ref()
        .and_then(|start| start.run_branch.clone())?;
    let repository = target.clone().validate().ok()?.repository().clone();
    let has_changes = conclusion
        .diff
        .patch
        .as_deref()
        .is_some_and(|patch| !patch.trim().is_empty());
    let pull_request = has_changes
        && settings
            .pull_request
            .as_ref()
            .is_some_and(|pull_request| pull_request.enabled);
    Some(Publication {
        repository,
        run_branch,
        sha,
        pull_request,
        model: settings.model.name.clone(),
    })
}

/// The snapshot repository of the run that holds `sha`.
async fn snapshot_holding(snapshots: &Path, sha: &str) -> Option<PathBuf> {
    let mut entries = fs::read_dir(snapshots).await.ok()?;
    let mut repositories = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().is_some_and(|extension| extension == "git") {
            repositories.push(path);
        }
    }
    repositories.sort();
    for repository in repositories {
        let held = git(
            &repository,
            &["cat-file", "-e", &format!("{sha}^{{commit}}")],
            &[],
        )
        .await
        .is_ok_and(|output| output.status.success());
        if held {
            return Some(repository);
        }
    }
    None
}

/// Push `sha` from the snapshot repository to the run branch on GitHub,
/// with the server's credentials, retrying a failure that may be a token
/// still replicating.
async fn push(
    state: &AppState,
    repository: &Path,
    publication: &Publication,
) -> anyhow::Result<()> {
    let github = &state.server_settings().server.integrations.github;
    let credentials = state
        .github_credentials(github)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the server has no GitHub credentials to push with"))?;
    let context = match state.http_client.clone() {
        Some(client) => fabro_github::GitHubContext::with_http_client(
            &credentials,
            &state.github_api_base_url,
            client,
        ),
        None => fabro_github::GitHubContext::new(&credentials, &state.github_api_base_url),
    };
    let push_credentials = fabro_github::resolve_clone_credentials(
        &context,
        publication.repository.owner(),
        publication.repository.repo(),
    )
    .await?;
    let auth = GitAuthConfig::new(&push_credentials);
    let url = git_checkout::github_clone_url(&publication.repository);
    let env = auth.git_env(&url);
    let refspec = format!("{}:refs/heads/{}", publication.sha, publication.run_branch);
    let mut last = String::new();
    for attempt in 1..=PUSH_ATTEMPTS {
        let output = git(repository, &["push", "--quiet", &url, &refspec], &env).await?;
        if output.status.success() {
            return Ok(());
        }
        last = redact(
            String::from_utf8_lossy(&output.stderr).trim(),
            auth.sensitive_values(),
        );
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
    anyhow::bail!(
        "the run branch {} could not be pushed to {}: {last}",
        publication.run_branch,
        publication.repository
    )
}

/// Record the pull request request for the run, with the run's model or
/// the catalog's default, and hand it to the creation supervisor.
async fn request_pull_request(state: &Arc<AppState>, run_id: RunId, model: Option<String>) {
    let model = if let Some(model) = model {
        model
    } else {
        let configured = state.ready_llm_provider_ids().await;
        let catalog = state.catalog();
        let Some(entry) = catalog.default_offering_for(&configured) else {
            notice(
                state,
                run_id,
                "pull_request_not_requested",
                "no LLM model is available to write the pull request",
            )
            .await;
            return;
        };
        entry.model.id().to_string()
    };
    let guard = state.pull_request_create_locks.lock(run_id).await;
    let requested = request_pull_request_creation(state, run_id, model, false).await;
    drop(guard);
    match requested {
        Ok(true) => {
            if let Ok(projection) = state.load_run_projection(&run_id).await {
                if let Some(creation) = projection.pull_request_creation.as_ref() {
                    state.enqueue_pull_request_creation(run_id, creation.requested_at);
                    state.notify_pull_request_scheduler();
                }
            }
        }
        Ok(false) => {}
        Err(err) => {
            warn!(run_id = %run_id, error = ?err, "the pull request was not requested");
            notice(
                state,
                run_id,
                "pull_request_not_requested",
                "the pull request could not be requested",
            )
            .await;
        }
    }
}

async fn notice(state: &AppState, run_id: RunId, code: &str, message: &str) {
    let record = PlatformRecord::RunNotice(RunNoticeRecord {
        level:   RunNoticeLevel::Warn,
        code:    code.to_string(),
        message: message.to_string(),
    });
    if let Err(err) = run_records::append(state, run_id, record).await {
        warn!(run_id = %run_id, error = %err, "the run's publication notice was not recorded");
    }
}

/// `git` in `directory` with `env` added, non-interactive, bounded.
async fn git(
    directory: &Path,
    args: &[&str],
    env: &[(String, String)],
) -> anyhow::Result<std::process::Output> {
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(directory)
        .envs(env.iter().map(|(key, value)| (key, value)))
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    Ok(time::timeout(PUSH_TIMEOUT, command.output())
        .await
        .map_err(|_| anyhow::anyhow!("git {} timed out", args.first().unwrap_or(&"")))??)
}

fn redact(text: &str, secrets: &[String]) -> String {
    secrets
        .iter()
        .filter(|secret| !secret.is_empty())
        .fold(text.to_string(), |text, secret| text.replace(secret, "***"))
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use fabro_types::settings::run::PullRequestSettings;
    use fabro_types::{
        Conclusion, GitRunTarget, RunProjection, RunTarget, StageOutcome, StartRecord, test_support,
    };

    use super::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    /// A GitHub-target run that succeeded with a change and a pull request
    /// asked for, pushed to its run branch.
    fn ended() -> RunProjection {
        let mut spec = test_support::test_run_spec();
        spec.target = Some(RunTarget::Git(GitRunTarget {
            repo:   "acme/widgets".to_string(),
            branch: "main".to_string(),
            tag:    None,
            sha:    None,
        }));
        spec.settings.run.run_branch.enabled = true;
        spec.settings.run.run_branch.push = true;
        spec.settings.run.pull_request = Some(PullRequestSettings {
            enabled: true,
            ..PullRequestSettings::default()
        });
        let mut projection = RunProjection::new(String::new(), spec, Utc::now());
        projection.start = Some(StartRecord {
            start_time: Utc::now(),
            run_branch: Some("fabro/run/1".to_string()),
            base_sha:   None,
        });
        let mut conclusion = Conclusion::outcome_only(Utc::now(), StageOutcome::Succeeded, None);
        conclusion.final_git_commit_sha = Some(SHA.to_string());
        conclusion.diff.patch = Some("diff --git a/README.md b/README.md\n".to_string());
        projection.conclusion = Some(conclusion);
        projection
    }

    #[test]
    fn a_successful_github_run_pushes_its_run_branch_and_asks_for_a_pull_request() {
        let publication = publication(&ended()).expect("the run publishes");
        assert_eq!(publication.repository.to_string(), "acme/widgets");
        assert_eq!(publication.run_branch, "fabro/run/1");
        assert_eq!(publication.sha, SHA);
        assert!(publication.pull_request);
    }

    #[test]
    fn a_run_without_changes_or_a_pull_request_setting_pushes_without_one() {
        let mut unchanged = ended();
        unchanged.conclusion.as_mut().unwrap().diff.patch = Some("  \n".to_string());
        assert!(!publication(&unchanged).unwrap().pull_request);

        let mut not_asked = ended();
        not_asked.spec.settings.run.pull_request = None;
        assert!(!publication(&not_asked).unwrap().pull_request);
    }

    #[test]
    fn nothing_is_published_for_a_failed_dry_unpushed_or_non_github_run() {
        let mut failed = ended();
        failed.conclusion.as_mut().unwrap().status = StageOutcome::Failed {
            retry_requested: false,
        };
        assert!(publication(&failed).is_none());

        let mut dry = ended();
        dry.spec.settings.run.execution.mode = RunMode::DryRun;
        assert!(publication(&dry).is_none());

        let mut unpushed = ended();
        unpushed.spec.settings.run.run_branch.push = false;
        assert!(publication(&unpushed).is_none());

        let mut folder = ended();
        folder.spec.target = Some(RunTarget::None {});
        assert!(publication(&folder).is_none());

        let mut no_commit = ended();
        no_commit.conclusion.as_mut().unwrap().final_git_commit_sha = None;
        assert!(publication(&no_commit).is_none());
    }

    #[test]
    fn a_push_failure_never_prints_the_credential() {
        assert_eq!(
            redact("fatal: token s3cret rejected", &["s3cret".to_string()]),
            "fatal: token *** rejected"
        );
    }

    #[tokio::test]
    #[expect(
        clippy::disallowed_methods,
        reason = "the test builds its fixture repositories with synchronous git"
    )]
    async fn the_snapshot_repository_holding_the_final_commit_is_found() {
        let root = tempfile::tempdir().unwrap();
        let snapshots = root.path().join("snapshots");
        let work = root.path().join("work");
        std::fs::create_dir_all(&snapshots).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        let run = |directory: &Path, args: &[&str]| {
            let output = std::process::Command::new("git")
                .current_dir(directory)
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success(), "{args:?}");
            String::from_utf8(output.stdout).unwrap().trim().to_string()
        };
        run(&work, &["init", "-q"]);
        run(&work, &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "one",
        ]);
        let sha = run(&work, &["rev-parse", "HEAD"]);
        for name in ["a.git", "b.git"] {
            run(&snapshots, &["init", "-q", "--bare", name]);
        }
        run(&work, &[
            "push",
            "-q",
            &snapshots.join("b.git").to_string_lossy(),
            "HEAD:refs/checkpoints/0/1/1",
        ]);
        assert_eq!(
            snapshot_holding(&snapshots, &sha).await,
            Some(snapshots.join("b.git"))
        );
        assert_eq!(snapshot_holding(&snapshots, SHA).await, None);
    }
}
