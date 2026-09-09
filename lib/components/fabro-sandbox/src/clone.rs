//! Fabro's clone orchestration over the sandbox-driver [`Git`] and [`Exec`]
//! facets.
//!
//! The driver clones; fabro decides what to clone, where it lands, which
//! credentials it carries, and how failures retry. The layout is fabro's:
//! the repository checks out under `<repos_root>/<owner>/<repo>` and the
//! run works in `<workspace_root>/<repo>`, a symlink to the checkout. An
//! exact commit goes through the driver's pinned clone; a tag pin runs
//! fabro's own init, fetch, and attach sequence through `Exec`, because a
//! tag must be fetched by its fully qualified ref so a same-named branch is
//! never consulted. Neither path ever falls back to the branch head.

use std::time::Duration;

use fabro_github::token_source::ResolvedToken;
use fabro_redact::DisplaySafeUrl;
use fabro_types::SandboxProviderKind;
use sandbox_driver::{Git as _, GitCloneOptions, GitCredentials, Sandbox as DriverHandle};
use tokio::time;

use crate::clone_source::{self, GitHubRepoLayout, PinnedRevision};
use crate::exec::SandboxExec;
use crate::git_retry::{self, CredentialContext, GitRetryReason, RetryPlan};
use crate::push_credentials::PushCredentialState;
use crate::redact::redact_auth_url;
use crate::sandbox::shell_quote;
use crate::{ExecResult, ExecStreamingRequest};

/// Whole-clone budget, shared by every network and local step.
pub(crate) const GIT_CLONE_TIMEOUT: Duration = Duration::from_mins(5);
const STEP_TIMEOUT: Duration = Duration::from_secs(10);

/// A GitHub clone fabro decided to perform.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GitHubClone {
    pub(crate) origin_url: String,
    pub(crate) branch:     Option<String>,
    pub(crate) tag:        Option<String>,
    pub(crate) commit_sha: Option<String>,
    pub(crate) depth:      Option<u32>,
}

/// What the clone left behind: the layout and the token now embedded in
/// `origin`, if any.
pub(crate) struct CloneOutcome {
    pub(crate) layout: GitHubRepoLayout,
}

/// Whether a failing git step talked to the remote. Local steps cannot fail
/// on credentials, so they must not suggest reconfiguring the GitHub App.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CloneStep {
    Network,
    Local,
}

struct CloneFailure {
    error:        crate::Error,
    retry_reason: Option<GitRetryReason>,
}

/// Clone `plan` into `handle`, laid out under `workspace_root` and
/// `repos_root`, embedding a GitHub App token from `credentials` when one
/// is available.
pub(crate) async fn clone_github_repo(
    kind: &SandboxProviderKind,
    handle: &dyn DriverHandle,
    exec: &SandboxExec<'_>,
    plan: &GitHubClone,
    workspace_root: &str,
    repos_root: &str,
    credentials: &PushCredentialState,
) -> crate::Result<CloneOutcome> {
    verify_git_available(exec).await?;
    let layout = clone_source::github_repo_layout(&plan.origin_url, workspace_root, repos_root)?;
    // The clone mints its own token (never a warm-cache reuse) and seeds the
    // shared source, so the first refresh compares against the clone token
    // instead of believing nothing was ever embedded.
    let resolved_token = match credentials.source() {
        Some(source) => Some(source.mint_for_clone().await.map_err(|err| {
            crate::Error::context_anyhow("Failed to get GitHub App credentials for clone", err)
        })?),
        None => None,
    };
    let credential_context =
        CredentialContext::from_snapshot(resolved_token.as_ref().map(|token| &token.snapshot));
    let auth_url = match &resolved_token {
        Some(token) => Some(
            fabro_github::embed_token_in_url(&plan.origin_url, token.token.expose()).map_err(
                |err| {
                    crate::Error::context_anyhow(
                        "Failed to build authenticated GitHub clone URL",
                        err,
                    )
                },
            )?,
        ),
        None => None,
    };

    let fs = handle.fs();
    for dir in [workspace_root, layout.repos_owner_path.as_str()] {
        fs.create_dir(dir)
            .await
            .map_err(|error| crate::Error::context(format!("Failed to create {dir}"), error))?;
    }

    let deadline = time::Instant::now() + GIT_CLONE_TIMEOUT;
    let has_app = credentials.source().is_some();
    match PinnedRevision::from_selectors(plan.tag.as_deref(), plan.commit_sha.as_deref()) {
        Some(PinnedRevision::Tag(tag)) => {
            // `decide_clone` already requires a branch for a pin; the branch
            // names the checkout the run works on.
            let branch = plan
                .branch
                .as_deref()
                .filter(|branch| !branch.trim().is_empty())
                .ok_or_else(|| {
                    crate::Error::message("Tag checkout requires a repository branch")
                })?;
            clone_pinned_tag(
                kind,
                exec,
                &layout,
                plan,
                &tag,
                branch,
                auth_url.as_ref(),
                credential_context,
                deadline,
                has_app,
            )
            .await?;
        }
        pin => {
            let git = handle.git().ok_or_else(|| {
                crate::Error::message(format!(
                    "sandbox provider `{kind}` does not support git operations"
                ))
            })?;
            let mut options = GitCloneOptions::default();
            options.branch = plan
                .branch
                .clone()
                .filter(|branch| !branch.trim().is_empty());
            options.commit = plan.commit_sha.clone();
            options.depth = plan.depth;
            options.credentials = resolved_token
                .as_ref()
                .map(|token| GitCredentials::new("x-access-token", token.token.expose()));
            let retry_plan = RetryPlan::clone_default(Some(deadline));
            let target = layout.primary_repo_path.clone();
            git_retry::retry_git_operation(
                kind.clone(),
                "clone",
                &retry_plan,
                |_attempt| {
                    let options = options.clone();
                    let target = target.clone();
                    let origin_url = plan.origin_url.clone();
                    let git = &git;
                    async move {
                        git.clone_repo(&origin_url, &target, &options)
                            .await
                            .map_err(|error| CloneFailure {
                                retry_reason: git_retry::classify_driver_failure(
                                    &error,
                                    credential_context,
                                ),
                                error:        clone_failure_error(
                                    crate::Error::driver_error(error),
                                    CloneStep::Network,
                                    has_app,
                                ),
                            })
                    }
                },
                |failure: &CloneFailure| failure.retry_reason,
            )
            .await
            .map_err(|failure| failure.error)?;
            if let Some(pin) = pin {
                let head = run_local_step(
                    exec,
                    &clone_source::exact_head_revision_command(&layout.primary_repo_path),
                    "git rev-parse HEAD (pinned checkout)",
                    deadline,
                    auth_url.as_ref(),
                    has_app,
                )
                .await?;
                pin.verify_head(&head.stdout)?;
            }
        }
    }

    run_local_step(
        exec,
        &clone_source::repo_symlink_command(&layout),
        "create workspace repo symlink",
        deadline,
        auth_url.as_ref(),
        has_app,
    )
    .await?;

    if let Some(token) = resolved_token {
        embed_origin_credentials(exec, &layout, auth_url.as_ref(), token, credentials).await;
    }
    Ok(CloneOutcome { layout })
}

/// Fabro's exact tag checkout: init, fetch the fully qualified tag ref at
/// the same depth a branch clone gets, attach the admitted branch to the
/// fetched commit, and verify HEAD. Every step runs through `Exec`.
#[expect(
    clippy::too_many_arguments,
    reason = "the pinned path threads clone inputs, credentials, and the shared deadline"
)]
async fn clone_pinned_tag(
    kind: &SandboxProviderKind,
    exec: &SandboxExec<'_>,
    layout: &GitHubRepoLayout,
    plan: &GitHubClone,
    tag: &str,
    branch: &str,
    auth_url: Option<&DisplaySafeUrl>,
    credential_context: CredentialContext,
    deadline: time::Instant,
    has_app: bool,
) -> crate::Result<()> {
    let clone_url = auth_url.map_or(plan.origin_url.as_str(), |url| url.as_raw_url().as_str());
    let init = clone_source::exact_repository_init_command(clone_url, &layout.primary_repo_path);
    run_local_step(
        exec,
        &init,
        "initialize pinned repository checkout",
        deadline,
        auth_url,
        has_app,
    )
    .await?;

    let pin = PinnedRevision::Tag(tag.to_string());
    let fetch = clone_source::pinned_fetch_command(
        &layout.primary_repo_path,
        "origin",
        &pin.fetch_refspec(),
        plan.depth.map(|depth| depth as usize),
    );
    let retry_plan = RetryPlan::clone_default(Some(deadline));
    git_retry::retry_git_operation(
        kind.clone(),
        "fetch",
        &retry_plan,
        |_attempt| async {
            let remaining = deadline.saturating_duration_since(time::Instant::now());
            if remaining.is_zero() {
                return Err(CloneFailure {
                    error:        crate::Error::message(
                        "git fetch pinned revision deadline expired before retry",
                    ),
                    retry_reason: None,
                });
            }
            let result = exec
                .run_streaming(ExecStreamingRequest {
                    timeout_ms: Some(millis(remaining)),
                    working_dir: Some("/"),
                    ..ExecStreamingRequest::new(&fetch)
                })
                .await
                .map_err(|error| CloneFailure {
                    error:        crate::Error::context(
                        "git fetch pinned revision transport failed",
                        error,
                    ),
                    retry_reason: None,
                })?
                .result;
            if result.is_success() {
                return Ok(());
            }
            let retry_reason =
                git_retry::classify_output(&result.stderr, &result.stdout, credential_context)
                    .retry_reason();
            Err(CloneFailure {
                error: clone_failure_error(
                    result.into_exec_error_with_redactor("git fetch pinned revision", |output| {
                        redact_auth_url(output, auth_url)
                    }),
                    CloneStep::Network,
                    has_app,
                ),
                retry_reason,
            })
        },
        |failure: &CloneFailure| failure.retry_reason,
    )
    .await
    .map_err(|failure| failure.error)?;

    let checkout = clone_source::exact_checkout_verify_command(
        &layout.primary_repo_path,
        branch,
        clone_source::FETCH_HEAD_COMMIT,
    );
    let head = run_local_step(
        exec,
        &checkout,
        "git checkout pinned revision",
        deadline,
        auth_url,
        has_app,
    )
    .await?;
    pin.verify_head(&head.stdout)?;
    Ok(())
}

async fn verify_git_available(exec: &SandboxExec<'_>) -> crate::Result<()> {
    let result = exec
        .run("git --version", Some(STEP_TIMEOUT), Some("/"), None, None)
        .await?;
    if !result.is_success() {
        return Err(crate::Error::message(
            "The sandbox image must include git for repository clone and git lifecycle \
             operations. Use an image with bash and git, such as buildpack-deps:noble.",
        ));
    }
    Ok(())
}

/// Run a local (non-network) step under the shared clone deadline.
///
/// Materializing a large working tree takes far longer than the short fixed
/// timeout used for trivial commands, so these steps get the same budget the
/// network steps have.
async fn run_local_step(
    exec: &SandboxExec<'_>,
    command: &str,
    label: &'static str,
    deadline: time::Instant,
    auth_url: Option<&DisplaySafeUrl>,
    has_app: bool,
) -> crate::Result<ExecResult> {
    let remaining = deadline.saturating_duration_since(time::Instant::now());
    if remaining.is_zero() {
        return Err(crate::Error::message(format!(
            "{label} deadline expired before the step could run"
        )));
    }
    let result = exec
        .run(command, Some(remaining), Some("/"), None, None)
        .await
        .map_err(|error| crate::Error::context(format!("{label} transport failed"), error))?;
    if result.is_success() {
        return Ok(result);
    }
    Err(clone_failure_error(
        result.into_exec_error_with_redactor(label, |output| redact_auth_url(output, auth_url)),
        CloneStep::Local,
        has_app,
    ))
}

fn clone_failure_error(error: crate::Error, step: CloneStep, has_app: bool) -> crate::Error {
    let message = match step {
        CloneStep::Network if !has_app => {
            "Git clone failed. If this is a private repository, configure a GitHub App with \
             `fabro install` and install it for your organization."
        }
        CloneStep::Network => "Failed to clone repository into the sandbox",
        CloneStep::Local => "Failed to prepare the cloned repository in the sandbox",
    };
    crate::Error::context(message, error)
}

/// Point `origin` at the authenticated URL so pushes from the checkout
/// carry the clone token, and record that generation for refreshes. A
/// failure here is logged, not fatal: the checkout is complete, and the
/// first push will re-embed.
async fn embed_origin_credentials(
    exec: &SandboxExec<'_>,
    layout: &GitHubRepoLayout,
    auth_url: Option<&DisplaySafeUrl>,
    token: ResolvedToken,
    credentials: &PushCredentialState,
) {
    credentials.record_embedded(token).await;
    let Some(auth_url) = auth_url else {
        return;
    };
    let command = format!(
        "git -c maintenance.auto=0 remote set-url origin {}",
        shell_quote(auth_url.as_raw_url().as_str())
    );
    match exec
        .run(
            &command,
            Some(STEP_TIMEOUT),
            Some(&layout.execution_directory),
            None,
            None,
        )
        .await
    {
        Ok(result) if result.is_success() => {}
        Ok(result) => {
            let err = result
                .into_exec_error_with_redactor("git remote set-url origin (post-clone)", |s| {
                    redact_auth_url(s, Some(auth_url))
                });
            tracing::warn!(
                error = %err,
                "Failed to set sandbox push credentials on origin; git push from this sandbox will fail"
            );
        }
        Err(err) => {
            tracing::warn!(
                error = %redact_auth_url(&crate::display_for_log(&err), Some(auth_url)),
                "Failed to set sandbox push credentials on origin; git push from this sandbox will fail"
            );
        }
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
