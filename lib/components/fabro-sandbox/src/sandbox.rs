use std::fmt::Write;
use std::time::Duration;

use fabro_github::token_source::TokenSnapshot;
pub use fabro_types::run_event::GitCredentialAction as RemoteCredentialAction;
use fabro_util::shell;
use sandbox_driver::{Git as _, GitCheckoutOptions, GitFailureKind, GitPushOptions, Termination};
use serde::{Deserialize, Serialize};
use tokio::time;

use crate::driver_sandbox::RunSandbox;
use crate::exec::ExecResultExt;
use crate::git_retry::{self, CredentialContext, GitRetryReason, RetryPlan};
use crate::push_credentials::{CredentialLease, PushCredentialState, RefreshErrorKind};

/// Git command prefix that disables background maintenance.
pub(crate) const GIT: &str = "git -c maintenance.auto=0 -c gc.auto=0";

pub const DEFAULT_EXEC_OUTPUT_TAIL_BYTES: usize = 8 * 1024;

/// Where a clone-based sandbox put its files, as persisted on the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxWorkspaceLayout {
    pub workspace_root:    String,
    pub repos_root:        String,
    /// The repository checkout and its link in the workspace, when a
    /// repository was cloned.
    pub primary_repo_path: Option<String>,
    pub primary_repo_link: Option<String>,
}

/// Information returned when a sandbox sets up git for a workflow run.
#[derive(Debug, Clone)]
pub struct GitRunInfo {
    pub base_sha:    String,
    pub run_branch:  String,
    pub base_branch: Option<String>,
}

/// Git setup requested by the workflow layer.
#[derive(Debug, Clone)]
pub enum GitSetupIntent {
    NewRun {
        run_id: String,
    },
    ForkFromCheckpoint {
        new_run_id:     String,
        source_run_id:  String,
        checkpoint_sha: String,
    },
}

/// Formats file content with line numbers for display.
///
/// Applies optional offset (1-based starting line number) and limit (max lines
/// to return). Line numbers are 1-based and right-aligned.
#[must_use]
pub fn format_lines_numbered(content: &str, offset: Option<usize>, limit: Option<usize>) -> String {
    let all_lines: Vec<&str> = content.lines().collect();
    let skip = offset.unwrap_or(1).saturating_sub(1);
    let take = limit.unwrap_or(all_lines.len());
    let selected: Vec<&str> = all_lines.into_iter().skip(skip).take(take).collect();
    let width = (skip + selected.len()).to_string().len().max(1);
    let mut result = String::new();
    for (i, line) in selected.iter().enumerate() {
        let line_num = skip + i + 1;
        let _ = writeln!(result, "{line_num:>width$} | {line}");
    }
    result
}

/// Build a redacted `ExecOutputTail` from raw stdout/stderr without
/// fabricating a synthetic `ExecResult`. Pass `""` for either stream that
/// isn't relevant. Returns `None` when both streams are empty.
#[must_use]
pub fn redacted_output_tail(
    stdout: &str,
    stderr: &str,
    max_bytes_per_stream: usize,
) -> Option<fabro_types::ExecOutputTail> {
    let (stdout, stdout_truncated) = redacted_tail(stdout, max_bytes_per_stream);
    let (stderr, stderr_truncated) = redacted_tail(stderr, max_bytes_per_stream);
    let tail = fabro_types::ExecOutputTail {
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
    };
    (!tail.is_empty()).then_some(tail)
}

fn redacted_tail(text: &str, max_bytes: usize) -> (Option<String>, bool) {
    if text.is_empty() || max_bytes == 0 {
        return (None, !text.is_empty());
    }

    let redacted = fabro_redact::redact_string(text);
    let sanitized = sanitize_exec_output(&redacted);
    let truncated = sanitized.len() > max_bytes;
    let start = if truncated {
        sanitized.floor_char_boundary(sanitized.len() - max_bytes)
    } else {
        0
    };
    let tail = sanitized[start..].to_string();
    ((!tail.is_empty()).then_some(tail), truncated)
}

fn sanitize_exec_output(text: &str) -> String {
    let mut sanitized = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            match chars.peek().copied() {
                Some('[') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if ('@'..='~').contains(&next) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    let mut saw_esc = false;
                    for next in chars.by_ref() {
                        if next == '\u{7}' || (saw_esc && next == '\\') {
                            break;
                        }
                        saw_esc = next == '\u{1b}';
                    }
                }
                Some('(' | ')' | '*' | '+' | '-' | '.' | '/') => {
                    chars.next();
                    chars.next();
                }
                Some('@'..='_') => {
                    chars.next();
                }
                _ => {}
            }
            continue;
        }
        if ch == '\n' || ch == '\r' || ch == '\t' || !ch.is_control() {
            sanitized.push(ch);
        }
    }
    sanitized
}

/// A regular file discovered inside a sandbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxFile {
    /// Provider-resolved path accepted by sandbox filesystem operations.
    pub path:          String,
    /// `/`-separated path relative to the requested traversal base.
    pub relative_path: String,
    pub size:          u64,
}

/// Outcome of
/// [`RunSandbox::refresh_push_credentials`](crate::RunSandbox::refresh_push_credentials):
/// what this call did to the remote, and the non-secret description of the
/// token embedded in it. `token` is `None` only when `action` is
/// [`RemoteCredentialAction::None`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshOutcome {
    /// No managed credentials exist for this sandbox.
    None,
    /// The remote already carried this token generation.
    Unchanged(TokenSnapshot),
    /// The remote was updated to carry this token generation.
    Embedded(TokenSnapshot),
}

impl RefreshOutcome {
    /// No managed credentials to refresh.
    #[must_use]
    pub const fn none() -> Self {
        Self::None
    }

    #[must_use]
    pub const fn unchanged(token: TokenSnapshot) -> Self {
        Self::Unchanged(token)
    }

    #[must_use]
    pub const fn embedded(token: TokenSnapshot) -> Self {
        Self::Embedded(token)
    }

    #[must_use]
    pub const fn action(self) -> RemoteCredentialAction {
        match self {
            Self::None => RemoteCredentialAction::None,
            Self::Unchanged(_) => RemoteCredentialAction::Unchanged,
            Self::Embedded(_) => RemoteCredentialAction::Embedded,
        }
    }

    #[must_use]
    pub const fn token(self) -> Option<TokenSnapshot> {
        match self {
            Self::None => None,
            Self::Unchanged(token) | Self::Embedded(token) => Some(token),
        }
    }
}

pub(crate) fn resolve_path(path: &str, working_dir: &str) -> String {
    if std::path::Path::new(path).is_absolute() {
        path.to_string()
    } else {
        join_sandbox_path(working_dir, path)
    }
}

pub(crate) fn join_sandbox_path(base: &str, relative_path: &str) -> String {
    if relative_path.is_empty() {
        return base.to_string();
    }
    if base.is_empty() {
        return relative_path.to_string();
    }
    if base == "/" {
        return format!("/{relative_path}");
    }
    format!("{}/{relative_path}", base.trim_end_matches('/'))
}

/// Shell-quote a string using `shlex::try_quote`, with a fallback for edge
/// cases. Re-exported from [`fabro_util::shell::shell_quote`] so sandbox code
/// and the config resolve layer share one audited implementation.
pub fn shell_quote(s: &str) -> String {
    shell::shell_quote(s)
}

/// Creates the run branch in the sandbox's checkout through the driver's
/// git facet: a new run branches from `HEAD`, a fork from the source run's
/// checkpoint. The branch is created at that base, or moved to it when an
/// earlier attempt already created it.
pub async fn setup_git(sandbox: &RunSandbox, intent: &GitSetupIntent) -> crate::Result<GitRunInfo> {
    let git = sandbox.git()?;
    let repo = sandbox.working_directory().to_owned();
    let status = git
        .status(&repo)
        .await
        .map_err(|error| crate::Error::context("git status", error))?;
    let base_branch = status
        .current_branch
        .filter(|name| !name.is_empty() && name != "HEAD");

    let (base_sha, branch_name) = match intent {
        GitSetupIntent::NewRun { run_id } => {
            let head = status.head.ok_or_else(|| {
                crate::Error::message("the repository has no commit to branch the run from")
            })?;
            (head, format!("fabro/run/{run_id}"))
        }
        GitSetupIntent::ForkFromCheckpoint {
            new_run_id,
            source_run_id,
            checkpoint_sha,
        } => {
            fetch_source_run_ref(sandbox, source_run_id, checkpoint_sha).await?;
            (checkpoint_sha.clone(), format!("fabro/run/{new_run_id}"))
        }
    };

    git.checkout(
        &repo,
        &GitCheckoutOptions::new(&branch_name)
            .create_or_reset()
            .start_point(&base_sha),
    )
    .await
    .map_err(|error| crate::Error::context("git checkout -B", error))?;

    Ok(GitRunInfo {
        base_sha,
        run_branch: branch_name,
        base_branch,
    })
}

#[tracing::instrument(name = "git_op", skip_all, fields(op = "fetch"))]
pub(crate) async fn fetch_source_run_ref(
    sandbox: &RunSandbox,
    source_run_id: &str,
    checkpoint_sha: &str,
) -> crate::Result<()> {
    let remote_ref = format!("refs/heads/fabro/run/{source_run_id}");
    let tracking_ref = format!("refs/remotes/origin/fabro/run/{source_run_id}");
    let fetch_cmd = format!(
        "{GIT} fetch origin {}:{}",
        shell_quote(&remote_ref),
        shell_quote(&tracking_ref)
    );
    let check_cmd = format!(
        "{GIT} merge-base --is-ancestor {} {}",
        shell_quote(checkpoint_sha),
        shell_quote(&tracking_ref)
    );

    let mut last_error = String::new();
    for _ in 0..5 {
        let fetch = sandbox
            .exec_command(&fetch_cmd, 30_000, None, None, None)
            .await?;
        if fetch.success() {
            let check = sandbox
                .exec_command(&check_cmd, 10_000, None, None, None)
                .await?;
            if check.success() {
                return Ok(());
            }
            last_error = check
                .into_exec_error(format!(
                    "checkpoint {checkpoint_sha} is not reachable from {remote_ref}"
                ))
                .to_string();
        } else {
            last_error = fetch
                .into_exec_error("git fetch source run ref")
                .to_string();
        }
        time::sleep(Duration::from_millis(500)).await;
    }

    Err(crate::Error::message(last_error))
}

/// One push attempt inside a retried push operation. Runtime detail only —
/// the durable serialized shape lives in `fabro-types` and the workflow layer
/// owns the conversion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushAttempt {
    /// 1-based attempt number within this operation.
    pub attempt:           u32,
    pub started_at:        chrono::DateTime<chrono::Utc>,
    pub success:           bool,
    /// The classifier's verdict for a failed attempt — recorded on the
    /// terminal attempt too; whether a retry actually followed is positional
    /// (every entry except the last).
    pub retry_reason:      Option<GitRetryReason>,
    /// Redacted, bounded output tail; failed attempts only.
    pub exec_output_tail:  Option<fabro_types::ExecOutputTail>,
    /// The token embedded in the remote during this attempt.
    pub token:             Option<TokenSnapshot>,
    /// What `ensure_embedded` did to the remote this attempt.
    pub credential_action: Option<RemoteCredentialAction>,
    /// A mint or `set-url` failure this attempt pushed through.
    pub refresh_error:     Option<RefreshErrorKind>,
}

/// The attempt history of one push operation.
#[derive(Debug, Clone, Default)]
pub struct PushReport {
    pub attempts: Vec<PushAttempt>,
}

/// A failed push operation: the final typed error plus the attempt history.
/// The error type stays the safety boundary for output tails.
#[derive(Debug, thiserror::Error)]
#[error("git push failed")]
pub struct PushError {
    pub report: PushReport,
    #[source]
    pub error:  crate::Error,
}

/// What a failed push attempt means for retrying. The driver classified
/// the failure; a push that did not run to completion (timed out or
/// cancelled) is never retried, because the remote may still be applying
/// it.
fn classify_push_error(error: &crate::Error, cred: CredentialContext) -> Option<GitRetryReason> {
    let driver = error.driver()?;
    if let sandbox_driver::Error::Git(failure) = driver {
        if failure
            .output()
            .is_some_and(|output| output.termination() != Termination::Exited)
        {
            return None;
        }
    }
    git_retry::classify_driver_failure(driver, cred)
}

/// Whether a failed push attempt was rejected as unauthenticated, the shape
/// a drifted or missing embedded token also produces.
fn push_failure_looks_auth_shaped(error: &crate::Error) -> bool {
    matches!(
        error.driver(),
        Some(sandbox_driver::Error::Git(failure)) if failure.kind() == GitFailureKind::AuthRejected
    )
}

/// Pushes a refspec to origin through the driver's git facet, retrying per
/// `plan` with one pinned credential generation for the whole operation.
/// `credentials` is the provider's push-credential state plus the origin
/// URL; `None` pushes with whatever the remote already carries (the local
/// sandbox, or a workspace without managed credentials).
#[tracing::instrument(name = "git_op", skip_all, fields(op = "push"))]
pub(crate) async fn git_push(
    sandbox: &RunSandbox,
    credentials: Option<(&PushCredentialState, &str)>,
    refspec: &str,
    plan: &RetryPlan,
) -> Result<PushReport, PushError> {
    use CredentialContext;
    use CredentialLease;

    let start = time::Instant::now();
    let deadline = plan.effective_deadline(start);
    let git = match sandbox.git() {
        Ok(git) => git,
        Err(error) => {
            return Err(PushError {
                report: PushReport::default(),
                error,
            });
        }
    };
    let repo = sandbox.working_directory().to_owned();

    // The lease pins one token generation and owns the embed mutex for the
    // whole operation; no concurrent refresh can re-embed mid-operation, and
    // no attempt can cross the refresh margin and restart the replication
    // clock.
    let mut lease: Option<(CredentialLease<'_>, &str)> = match credentials {
        Some((state, origin_url)) => match match deadline {
            Some(deadline) => match time::timeout_at(deadline, state.lease()).await {
                Ok(result) => result,
                Err(_) => {
                    return Err(push_deadline_error(
                        Vec::new(),
                        "while acquiring credentials",
                    ));
                }
            },
            None => state.lease().await,
        } {
            Ok(lease) => Some((lease, origin_url)),
            Err(error) => {
                return Err(PushError {
                    report: PushReport::default(),
                    error,
                });
            }
        },
        None => None,
    };

    let mut attempts: Vec<PushAttempt> = Vec::new();
    let mut force_reembed = false;
    let mut drift_repaired = false;
    let label = format!("git push origin {refspec}");

    loop {
        let attempt_number = u32::try_from(attempts.len()).unwrap_or(u32::MAX) + 1;
        let started_at = chrono::Utc::now();
        let attempt_timeout = plan
            .attempt_timeout(deadline)
            .unwrap_or(Duration::from_mins(1));
        if attempt_timeout.is_zero() {
            return Err(push_deadline_error(attempts, "before the next attempt"));
        }
        let attempt_deadline = time::Instant::now() + attempt_timeout;
        let (token, credential_action, refresh_error) = match lease.as_mut() {
            Some((lease, origin_url)) => {
                let ensured = match time::timeout_at(
                    attempt_deadline,
                    lease.ensure_embedded(sandbox, origin_url, force_reembed),
                )
                .await
                {
                    Ok(Ok(ensured)) => ensured,
                    Ok(Err(error)) => {
                        return Err(PushError {
                            report: PushReport { attempts },
                            error,
                        });
                    }
                    Err(_) => {
                        return Err(push_deadline_error(
                            attempts,
                            "while refreshing credentials",
                        ));
                    }
                };
                force_reembed = false;
                (ensured.token, Some(ensured.action), ensured.refresh_error)
            }
            None => (None, None, None),
        };

        let remaining = attempt_deadline.saturating_duration_since(time::Instant::now());
        if remaining.is_zero() {
            return Err(push_deadline_error(attempts, "before running git push"));
        }
        let mut options = GitPushOptions::default();
        options.remote = Some("origin".to_owned());
        options.refspec = Some(refspec.to_owned());
        options.timeout = Some(remaining);
        let push_result = git
            .push(&repo, &options)
            .await
            .map_err(|error| crate::Error::context(label.clone(), error));

        match push_result {
            Ok(()) => {
                attempts.push(PushAttempt {
                    attempt: attempt_number,
                    started_at,
                    success: true,
                    retry_reason: None,
                    exec_output_tail: None,
                    token,
                    credential_action,
                    refresh_error,
                });
                tracing::info!(
                    refspec = %refspec,
                    attempt = attempt_number,
                    token_generation = token.map(|token| token.generation),
                    token_age_ms = token.and_then(|token| token.age_ms()),
                    "Pushed git ref to origin"
                );
                return Ok(PushReport { attempts });
            }
            Err(error) => {
                // Drift recovery: the tracked generation is local belief, and
                // agent code inside the sandbox can rewrite `origin`. The
                // first auth/not-found failure earns one forced re-embed of
                // the pinned token, inside the same retry budget.
                if !drift_repaired && lease.is_some() && push_failure_looks_auth_shaped(&error) {
                    drift_repaired = true;
                    force_reembed = true;
                }
                let cred = CredentialContext::from_snapshot(token.as_ref());
                let retry_reason = classify_push_error(&error, cred);
                attempts.push(PushAttempt {
                    attempt: attempt_number,
                    started_at,
                    success: false,
                    retry_reason,
                    exec_output_tail: error.default_redacted_output_tail(),
                    token,
                    credential_action,
                    refresh_error,
                });

                let exhausted = attempt_number >= plan.max_attempts.max(1);
                let Some(reason) = retry_reason.filter(|_| !exhausted) else {
                    return Err(PushError {
                        report: PushReport { attempts },
                        error,
                    });
                };
                let Some(delay) = plan.retry_delay(attempt_number, deadline) else {
                    return Err(PushError {
                        report: PushReport { attempts },
                        error,
                    });
                };
                // The failure text can carry git stderr, so log the category
                // rather than the message.
                tracing::warn!(
                    refspec = %refspec,
                    attempt = attempt_number,
                    max_attempts = plan.max_attempts,
                    reason = %reason,
                    token_generation = token.map(|token| token.generation),
                    token_age_ms = token.and_then(|token| token.age_ms()),
                    delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
                    "Git push failed, retrying with the same token"
                );
                time::sleep(delay).await;
            }
        }
    }
}

fn push_deadline_error(attempts: Vec<PushAttempt>, stage: &str) -> PushError {
    PushError {
        report: PushReport { attempts },
        error:  crate::Error::message(format!("Git push retry deadline expired {stage}")),
    }
}

#[cfg(test)]
mod push_tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use chrono::Utc;
    use fabro_github::InstallationToken;
    use fabro_github::test_support::{InstallationTokenMinter, installation_token_source};
    use fabro_github::token_source::{InstallationTokenSource, REFRESH_MARGIN};
    use fabro_types::SandboxProviderKind;
    use sandbox_driver::ExecResult;
    use sandbox_driver_testing::ScriptedSandbox;
    use tokio::sync::Mutex as AsyncMutex;

    use super::*;
    use crate::git_retry::{GitRetryReason, RetryPlan};
    use crate::push_credentials::{PushCredentialState, RefreshErrorKind};

    const ORIGIN: &str = "https://github.com/fabro-testing/repo";
    const REFSPEC: &str = "refs/heads/fabro/run/01M0DH033P2XSTHAGVBHG6922F";

    fn ok_exec() -> ExecResult {
        ExecResult::new(Termination::Exited, Some(0), Duration::from_millis(5))
    }

    fn failed_exec(stderr: &str) -> ExecResult {
        let mut result = ExecResult::new(Termination::Exited, Some(128), Duration::from_millis(5));
        result.stderr = stderr.as_bytes().to_vec();
        result
    }

    fn timed_out_exec() -> ExecResult {
        let mut result = ExecResult::new(Termination::TimedOut, None, Duration::from_mins(1));
        result.stderr = b"Command timed out".to_vec();
        result
    }

    /// A run sandbox over a scripted driver double: `git push` answers come
    /// from a script, `git remote set-url` succeeds unless scripted
    /// otherwise, and every command is recorded.
    struct ScriptedGitSandbox {
        run:    RunSandbox,
        driver: Arc<ScriptedSandbox>,
    }

    impl ScriptedGitSandbox {
        fn new(push_results: Vec<ExecResult>) -> Self {
            Self::with_set_url_results(push_results, Vec::new())
        }

        fn with_set_url_results(
            push_results: Vec<ExecResult>,
            set_url_results: Vec<ExecResult>,
        ) -> Self {
            let driver = Arc::new(ScriptedSandbox::with_id_and_working_dir(
                "scripted-git",
                "/workspace",
            ));
            let pushes = Mutex::new(VecDeque::from(push_results));
            let set_urls = Mutex::new(VecDeque::from(set_url_results));
            driver.scripted_exec().respond_with(move |spec| {
                let script = spec.args.last().map(String::as_str).unwrap_or_default();
                if script.contains("remote set-url") {
                    return Some(set_urls.lock().unwrap().pop_front().unwrap_or_else(ok_exec));
                }
                assert!(
                    script.contains("'push' 'origin'"),
                    "unexpected exec: {script}"
                );
                Some(
                    pushes
                        .lock()
                        .unwrap()
                        .pop_front()
                        .expect("push script exhausted"),
                )
            });
            let run = RunSandbox::new(SandboxProviderKind::LOCAL, Arc::clone(&driver) as _);
            Self { run, driver }
        }

        fn commands(&self) -> Vec<String> {
            self.driver.scripted_exec().commands()
        }

        fn push_count(&self) -> usize {
            self.commands()
                .iter()
                .filter(|command| command.contains("'push' 'origin'"))
                .count()
        }

        fn set_url_commands(&self) -> Vec<String> {
            self.commands()
                .into_iter()
                .filter(|command| command.contains("remote set-url"))
                .collect()
        }
    }

    enum MintAction {
        Token(&'static str, chrono::Duration),
        Error(&'static str),
    }

    struct ScriptedMinter {
        calls:  AtomicUsize,
        script: AsyncMutex<VecDeque<MintAction>>,
    }

    impl ScriptedMinter {
        fn new(script: Vec<MintAction>) -> std::sync::Arc<Self> {
            std::sync::Arc::new(Self {
                calls:  AtomicUsize::new(0),
                script: AsyncMutex::new(script.into()),
            })
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl InstallationTokenMinter for ScriptedMinter {
        async fn mint(&self) -> anyhow::Result<InstallationToken> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.script.lock().await.pop_front().expect("mint script") {
                MintAction::Token(token, ttl) => Ok(InstallationToken {
                    token:      token.to_string(),
                    expires_at: Utc::now() + ttl,
                }),
                MintAction::Error(message) => Err(anyhow::anyhow!(message)),
            }
        }
    }

    struct SlowMinter;

    #[async_trait]
    impl InstallationTokenMinter for SlowMinter {
        async fn mint(&self) -> anyhow::Result<InstallationToken> {
            time::sleep(Duration::from_secs(2)).await;
            Ok(InstallationToken {
                token:      "ghs_slow".to_string(),
                expires_at: Utc::now() + chrono::Duration::hours(1),
            })
        }
    }

    fn minting_state(
        script: Vec<MintAction>,
    ) -> (PushCredentialState, std::sync::Arc<ScriptedMinter>) {
        let minter = ScriptedMinter::new(script);
        let source = installation_token_source(
            "fabro-testing/repo",
            std::sync::Arc::clone(&minter) as std::sync::Arc<dyn InstallationTokenMinter>,
        );
        (PushCredentialState::new(Some(source)), minter)
    }

    async fn seed_clone_token(state: &PushCredentialState) {
        let clone_token = state
            .source()
            .expect("state has a source")
            .mint_for_clone()
            .await
            .expect("clone mint succeeds");
        state.record_embedded(clone_token).await;
    }

    /// Regression for run `01M0DH033P2XSTHAGVBHG6922F` (the push variant of
    /// `clone_not_found_after_a_successful_mint_is_retried`): GitHub rejected
    /// pushes with 404 "Repository not found" milliseconds after a token
    /// mint. The retry must reuse the same token — replication of a given
    /// token only makes progress — and recover inside the plan's budget.
    #[tokio::test(start_paused = true)]
    async fn push_not_found_after_a_successful_mint_is_retried_with_the_same_token() {
        let (state, minter) = minting_state(vec![MintAction::Token(
            "ghs_gen1",
            chrono::Duration::minutes(60),
        )]);
        let sandbox = ScriptedGitSandbox::new(vec![
            failed_exec("remote: Repository not found."),
            failed_exec("remote: Repository not found."),
            ok_exec(),
        ]);

        let report = git_push(
            &sandbox.run,
            Some((&state, ORIGIN)),
            REFSPEC,
            &RetryPlan::checkpoint_push(),
        )
        .await
        .expect("push should recover within the checkpoint plan");

        assert_eq!(report.attempts.len(), 3);
        assert_eq!(minter.calls(), 1, "retries must not re-mint");
        for attempt in &report.attempts {
            assert_eq!(attempt.token.expect("token recorded").generation, 1);
        }
        assert_eq!(
            report.attempts[0].retry_reason,
            Some(GitRetryReason::TokenReplication)
        );
        assert!(report.attempts[0].exec_output_tail.is_some());
        assert_eq!(
            report.attempts[0].credential_action,
            Some(RemoteCredentialAction::Embedded),
            "first attempt embeds the resolved token"
        );
        assert!(report.attempts[2].success);
        assert!(report.attempts[2].exec_output_tail.is_none());
        assert_eq!(sandbox.push_count(), 3);
    }

    /// The publish plan gives the terminal push a real budget: four
    /// replication-lag failures still recover on the fifth attempt.
    #[tokio::test(start_paused = true)]
    async fn publish_plan_survives_four_not_found_failures() {
        let (state, minter) = minting_state(vec![MintAction::Token(
            "ghs_gen1",
            chrono::Duration::minutes(60),
        )]);
        let sandbox = ScriptedGitSandbox::new(vec![
            failed_exec("remote: Repository not found."),
            failed_exec("remote: Repository not found."),
            failed_exec("remote: Repository not found."),
            failed_exec("remote: Repository not found."),
            ok_exec(),
        ]);

        let report = git_push(
            &sandbox.run,
            Some((&state, ORIGIN)),
            REFSPEC,
            &RetryPlan::publish_push(),
        )
        .await
        .expect("push should recover within the publish plan");

        assert_eq!(report.attempts.len(), 5);
        assert_eq!(minter.calls(), 1);
        assert!(report.attempts[4].success);
    }

    /// Margin-boundary pinning: a token resolved just above the refresh
    /// margin stays pinned through a full retry sequence — the operation
    /// never re-resolves mid-flight, so no fresh mint can restart the
    /// replication clock.
    #[tokio::test(start_paused = true)]
    async fn token_resolved_just_above_the_margin_stays_pinned_through_retries() {
        let ttl = REFRESH_MARGIN + Duration::from_secs(5);
        let (state, minter) = minting_state(vec![MintAction::Token(
            "ghs_gen1",
            chrono::Duration::from_std(ttl).unwrap(),
        )]);
        let sandbox = ScriptedGitSandbox::new(vec![
            failed_exec("remote: Repository not found."),
            failed_exec("remote: Repository not found."),
            ok_exec(),
        ]);

        let report = git_push(
            &sandbox.run,
            Some((&state, ORIGIN)),
            REFSPEC,
            &RetryPlan::checkpoint_push(),
        )
        .await
        .expect("push should recover");

        assert_eq!(minter.calls(), 1, "no mid-operation mint");
        let generations: Vec<u64> = report
            .attempts
            .iter()
            .map(|attempt| attempt.token.expect("token recorded").generation)
            .collect();
        assert_eq!(generations, vec![1, 1, 1]);
    }

    #[tokio::test(start_paused = true)]
    async fn static_credential_auth_failure_fails_fast() {
        let source = InstallationTokenSource::for_origin(
            &fabro_github::GitHubCredentials::Pat("ghp_pat".to_string()),
            ORIGIN,
            serde_json::json!({ "contents": "write" }),
        )
        .unwrap();
        let state = PushCredentialState::new(Some(source));
        seed_clone_token(&state).await;
        let sandbox = ScriptedGitSandbox::new(vec![failed_exec(
            "fatal: Authentication failed for 'https://github.com/fabro-testing/repo'",
        )]);

        let push_error = git_push(
            &sandbox.run,
            Some((&state, ORIGIN)),
            REFSPEC,
            &RetryPlan::publish_push(),
        )
        .await
        .expect_err("static credentials cannot become valid by waiting");

        assert_eq!(push_error.report.attempts.len(), 1);
        assert_eq!(push_error.report.attempts[0].retry_reason, None);
        assert!(push_error.report.attempts[0].token.unwrap().is_static());
    }

    /// Clone seeding closes the "nothing was ever embedded" hole: when the
    /// first refresh mint fails, the push falls back to the clone token
    /// recorded as last-embedded instead of aborting.
    #[tokio::test(start_paused = true)]
    async fn mint_failure_falls_back_to_the_clone_token() {
        let (state, minter) = minting_state(vec![
            MintAction::Token("ghs_clone", chrono::Duration::minutes(5)),
            // The clone token is inside the margin, so lease acquisition
            // re-mints and fails.
            MintAction::Error("mint failed"),
        ]);
        seed_clone_token(&state).await;
        let sandbox = ScriptedGitSandbox::new(vec![ok_exec()]);

        let report = git_push(
            &sandbox.run,
            Some((&state, ORIGIN)),
            REFSPEC,
            &RetryPlan::checkpoint_push(),
        )
        .await
        .expect("push proceeds with the still-valid clone token");

        assert_eq!(minter.calls(), 2);
        let attempt = &report.attempts[0];
        assert!(attempt.success);
        assert_eq!(attempt.refresh_error, Some(RefreshErrorKind::Mint));
        assert_eq!(
            attempt.token.expect("fallback token recorded").generation,
            1,
            "attempts classify against the embedded clone token, never None"
        );
        assert_eq!(
            attempt.credential_action,
            Some(RemoteCredentialAction::Unchanged)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn acquisition_fails_when_mint_fails_and_nothing_was_embedded() {
        let (state, _minter) = minting_state(vec![MintAction::Error("mint failed")]);
        let sandbox = ScriptedGitSandbox::new(vec![]);

        let push_error = git_push(
            &sandbox.run,
            Some((&state, ORIGIN)),
            REFSPEC,
            &RetryPlan::checkpoint_push(),
        )
        .await
        .expect_err("there is nothing to push with");

        assert!(push_error.report.attempts.is_empty());
        assert!(push_error.error.to_string().contains("token_mint_failed"));
        assert_eq!(sandbox.push_count(), 0);
    }

    /// Late-mint recovery: the fallback push fails on the expired-ish old
    /// token, a later attempt's resolve retry succeeds, the target embeds,
    /// and the push recovers — all inside one operation's budget.
    #[tokio::test(start_paused = true)]
    async fn late_mint_recovery_lands_the_target_inside_the_operation() {
        let (state, minter) = minting_state(vec![
            MintAction::Token("ghs_gen1", chrono::Duration::minutes(5)),
            MintAction::Error("mint failed"),
            MintAction::Token("ghs_gen2", chrono::Duration::minutes(60)),
        ]);
        seed_clone_token(&state).await;
        let sandbox = ScriptedGitSandbox::new(vec![
            failed_exec("fatal: Authentication failed for 'https://github.com'"),
            ok_exec(),
        ]);

        let report = git_push(
            &sandbox.run,
            Some((&state, ORIGIN)),
            REFSPEC,
            &RetryPlan::checkpoint_push(),
        )
        .await
        .expect("late mint should recover the push");

        assert_eq!(minter.calls(), 3);
        let first = &report.attempts[0];
        assert_eq!(first.refresh_error, Some(RefreshErrorKind::Mint));
        assert_eq!(first.token.unwrap().generation, 1);
        let second = &report.attempts[1];
        assert!(second.success);
        assert_eq!(second.refresh_error, None);
        assert_eq!(second.token.unwrap().generation, 2);
        assert_eq!(
            second.credential_action,
            Some(RemoteCredentialAction::Embedded),
            "the report shows the single generation transition"
        );
    }

    /// A failed `set-url` defers the embed: attempt 1 records the old
    /// generation with the refresh error, attempt 2 lands the target, and the
    /// report shows the one generation transition via `credential_action`.
    #[tokio::test(start_paused = true)]
    async fn set_url_failure_defers_the_embed_until_the_next_attempt() {
        let (state, minter) = minting_state(vec![
            MintAction::Token("ghs_gen1", chrono::Duration::minutes(5)),
            MintAction::Token("ghs_gen2", chrono::Duration::minutes(60)),
        ]);
        seed_clone_token(&state).await;
        let sandbox = ScriptedGitSandbox::with_set_url_results(
            vec![
                failed_exec("error: RPC failed; connection reset by peer"),
                ok_exec(),
            ],
            vec![failed_exec("error: could not lock config file")],
        );

        let report = git_push(
            &sandbox.run,
            Some((&state, ORIGIN)),
            REFSPEC,
            &RetryPlan::checkpoint_push(),
        )
        .await
        .expect("deferred embed should land on the retry");

        assert_eq!(
            minter.calls(),
            2,
            "the successful resolve is never repeated"
        );
        let first = &report.attempts[0];
        assert_eq!(first.refresh_error, Some(RefreshErrorKind::SetUrl));
        assert_eq!(
            first.token.unwrap().generation,
            1,
            "pin stays on the old token"
        );
        assert_eq!(
            first.credential_action,
            Some(RemoteCredentialAction::Unchanged)
        );
        let second = &report.attempts[1];
        assert_eq!(second.token.unwrap().generation, 2);
        assert_eq!(
            second.credential_action,
            Some(RemoteCredentialAction::Embedded)
        );
        assert!(second.success);
    }

    #[tokio::test(start_paused = true)]
    async fn timed_out_set_url_stops_before_push_while_it_may_still_run() {
        let (state, minter) = minting_state(vec![
            MintAction::Token("ghs_gen1", chrono::Duration::minutes(5)),
            MintAction::Token("ghs_gen2", chrono::Duration::minutes(60)),
        ]);
        seed_clone_token(&state).await;
        let sandbox = ScriptedGitSandbox::with_set_url_results(vec![], vec![timed_out_exec()]);

        let push_error = git_push(
            &sandbox.run,
            Some((&state, ORIGIN)),
            REFSPEC,
            &RetryPlan::checkpoint_push(),
        )
        .await
        .expect_err("a timed-out set-url can still rewrite origin later");

        assert_eq!(minter.calls(), 2);
        assert!(push_error.report.attempts.is_empty());
        assert_eq!(sandbox.push_count(), 0);
    }

    /// Remote drift: agent code rewrote `origin`, so the push fails on auth
    /// even though the tracked generation looks current. The first
    /// auth-shaped failure earns one forced re-embed of the pinned token.
    #[tokio::test(start_paused = true)]
    async fn remote_drift_gets_one_forced_reembed_of_the_pinned_token() {
        let (state, minter) = minting_state(vec![MintAction::Token(
            "ghs_gen1",
            chrono::Duration::minutes(60),
        )]);
        seed_clone_token(&state).await;
        let sandbox = ScriptedGitSandbox::new(vec![
            failed_exec(
                "fatal: could not read Username for 'https://github.com': No such device or address\nremote: Repository not found.",
            ),
            ok_exec(),
        ]);

        let report = git_push(
            &sandbox.run,
            Some((&state, ORIGIN)),
            REFSPEC,
            &RetryPlan::checkpoint_push(),
        )
        .await
        .expect("drift repair should restore the pinned credentials");

        assert_eq!(minter.calls(), 1, "drift repair re-embeds, never re-mints");
        assert_eq!(
            report.attempts[0].credential_action,
            Some(RemoteCredentialAction::Unchanged),
            "before the failure the tracked generation matched"
        );
        assert_eq!(
            report.attempts[1].credential_action,
            Some(RemoteCredentialAction::Embedded),
            "the retry force-re-embeds the pinned token"
        );
        let set_urls = sandbox.set_url_commands();
        assert_eq!(set_urls.len(), 1);
        assert!(set_urls[0].contains("ghs_gen1"));
    }

    #[tokio::test(start_paused = true)]
    async fn push_without_managed_credentials_reports_no_token() {
        let sandbox = ScriptedGitSandbox::new(vec![ok_exec()]);

        let report = git_push(&sandbox.run, None, REFSPEC, &RetryPlan::checkpoint_push())
            .await
            .expect("push succeeds");

        assert_eq!(report.attempts.len(), 1);
        assert_eq!(report.attempts[0].token, None);
        assert_eq!(report.attempts[0].credential_action, None);
    }

    #[tokio::test(start_paused = true)]
    async fn unauthenticated_auth_failure_is_permanent() {
        let sandbox = ScriptedGitSandbox::new(vec![failed_exec(
            "fatal: Authentication failed for 'https://github.com/fabro-testing/repo'",
        )]);

        let push_error = git_push(&sandbox.run, None, REFSPEC, &RetryPlan::publish_push())
            .await
            .expect_err("no credentials to wait on");

        assert_eq!(push_error.report.attempts.len(), 1);
        assert_eq!(push_error.report.attempts[0].retry_reason, None);
    }

    #[tokio::test(start_paused = true)]
    async fn timed_out_push_is_not_retried_while_the_remote_process_may_still_run() {
        let sandbox = ScriptedGitSandbox::new(vec![timed_out_exec()]);

        let push_error = git_push(&sandbox.run, None, REFSPEC, &RetryPlan::publish_push())
            .await
            .expect_err("an unconfirmed timeout must fail without another push");

        assert_eq!(sandbox.push_count(), 1);
        assert_eq!(push_error.report.attempts.len(), 1);
        assert_eq!(push_error.report.attempts[0].retry_reason, None);
    }

    #[tokio::test(start_paused = true)]
    async fn retry_deadline_includes_credential_lease_acquisition() {
        let source = installation_token_source("fabro-testing/repo", Arc::new(SlowMinter));
        let state = PushCredentialState::new(Some(source));
        let sandbox = ScriptedGitSandbox::new(vec![]);
        let mut plan = RetryPlan::checkpoint_push();
        plan.max_elapsed = Some(Duration::from_secs(1));

        let push_error = git_push(&sandbox.run, Some((&state, ORIGIN)), REFSPEC, &plan)
            .await
            .expect_err("credential acquisition must stop at the operation deadline");

        assert!(push_error.report.attempts.is_empty());
        assert_eq!(sandbox.push_count(), 0);
        assert!(push_error.error.to_string().contains("deadline expired"));
    }

    #[tokio::test(start_paused = true)]
    async fn expired_retry_deadline_does_not_launch_a_zero_timeout_push() {
        let sandbox = ScriptedGitSandbox::new(vec![]);
        let mut plan = RetryPlan::checkpoint_push();
        plan.max_elapsed = Some(Duration::ZERO);

        let push_error = git_push(&sandbox.run, None, REFSPEC, &plan)
            .await
            .expect_err("an expired operation must stop before exec");

        assert!(push_error.report.attempts.is_empty());
        assert_eq!(sandbox.push_count(), 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_tracing_events_do_not_log_raw_command_or_stdin_fields() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut failures = Vec::new();
        scan_for_command_tracing(&root, &mut failures);
        assert!(
            failures.is_empty(),
            "raw command/cmd/stdin tracing fields found:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn format_lines_numbered_basic() {
        let result = format_lines_numbered("hello\nworld\nfoo", None, None);
        assert_eq!(result, "1 | hello\n2 | world\n3 | foo\n");
    }

    #[test]
    fn format_lines_numbered_with_offset_limit() {
        let result = format_lines_numbered("a\nb\nc\nd\ne", Some(2), Some(2));
        assert!(result.contains("2 | b"));
        assert!(result.contains("3 | c"));
        assert!(!result.contains("1 | a"));
        assert!(!result.contains("4 | d"));
    }

    #[test]
    fn shell_quote_basic() {
        assert_eq!(shell_quote("hello"), "hello");
        assert_eq!(shell_quote("hello world"), "'hello world'");
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "unit test performs a small synchronous source scan of local Rust files"
    )]
    fn scan_for_command_tracing(path: &std::path::Path, failures: &mut Vec<String>) {
        for entry in std::fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                scan_for_command_tracing(&path, failures);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            for macro_name in [
                "tracing::trace!",
                "tracing::debug!",
                "tracing::info!",
                "tracing::warn!",
                "tracing::error!",
                "trace!",
                "debug!",
                "info!",
                "warn!",
                "error!",
            ] {
                let mut rest = source.as_str();
                while let Some(idx) = rest.find(macro_name) {
                    let start = source.len() - rest.len() + idx;
                    if start > 0 && source.as_bytes()[start - 1] == b'"' {
                        rest = &source[start + macro_name.len()..];
                        continue;
                    }
                    let Some(call) = tracing_call(&source[start..]) else {
                        break;
                    };
                    if call.contains("command,")
                        || call.contains("command =")
                        || call.contains("cmd,")
                        || call.contains("cmd =")
                        || call.contains("stdin,")
                        || call.contains("stdin =")
                    {
                        failures.push(format!(
                            "{}: {}",
                            path.display(),
                            call.lines().next().unwrap_or(call)
                        ));
                    }
                    rest = &source[start + call.len()..];
                }
            }
        }
    }

    fn tracing_call(source: &str) -> Option<&str> {
        let open = source.find('(')?;
        let mut depth = 0usize;
        for (idx, ch) in source.char_indices().skip(open) {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some(&source[..=idx]);
                    }
                }
                _ => {}
            }
        }
        None
    }
}
