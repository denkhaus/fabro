use std::borrow::Cow;
use std::fmt::Write as _;
use std::sync::Arc;

use fabro_types::ExecOutputTail;
use fabro_types::settings::run::PullRequestSettings;

use super::pull_request::{AutoMergeOptions, OpenPullRequestRequest, open_pull_request};
use super::types::{Concluded, PublishOptions, PublishOutcome, Published};
use crate::error::{Error, FailureCategory, classify_failure_reason};
use crate::event::Event;
use crate::lifecycle::git::push_run_branch;

/// PUBLISH phase: push the final run commit and, when configured, open a pull
/// request.
///
/// Publish is always present in the pipeline. It becomes a no-op when the run
/// did not succeed, is a dry run, or has no remote branch configured.
pub async fn publish(concluded: Concluded, options: &PublishOptions) -> Published {
    let mut publish_outcome = PublishOutcome::default();
    let publish_error = concluded.publish(options, &mut publish_outcome).await.err();

    let Concluded {
        outcome,
        conclusion,
        artifact_count,
        graph: _,
        run_options,
        services,
    } = concluded;

    Published {
        execution_outcome: outcome,
        publish_outcome,
        publish_error,
        conclusion,
        artifact_count,
        run_options,
        services,
    }
}

/// Build the terminal publish error from a failed push operation.
///
/// Retries exhausted on transient classifications stay `TransientInfra`: a
/// mature-token 404 is not proof of permanent access loss — a service-side
/// failure presents the same surface — so `Deterministic` would need
/// independent evidence this path does not gather. Each attempt becomes one
/// bounded cause line in the failure detail; git output stays inside the
/// exec output tail.
fn publish_push_error(
    run_branch: &str,
    push_error: fabro_sandbox::Error,
    exec_output_tail: Option<ExecOutputTail>,
    attempts: &[fabro_sandbox::PushAttempt],
    last_successful_push_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Error {
    let message = match last_successful_push_at {
        Some(at) => format!(
            "failed to push run branch '{run_branch}' (last successful push at {})",
            at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        ),
        None => format!("failed to push run branch '{run_branch}'"),
    };
    let failure_class = match attempts.last().and_then(|attempt| attempt.retry_reason) {
        Some(_) => FailureCategory::TransientInfra,
        None => classify_failure_reason(&format!(
            "{message}: {}",
            fabro_sandbox::display_for_log(&push_error)
        )),
    };
    let causes = attempts.iter().map(push_attempt_cause).collect();
    Error::publish_with_source_and_class(
        message,
        push_error,
        failure_class,
        exec_output_tail,
        causes,
    )
}

/// One bounded line per push attempt for the failure detail.
fn push_attempt_cause(attempt: &fabro_sandbox::PushAttempt) -> String {
    let outcome = if attempt.success {
        "succeeded".to_string()
    } else {
        attempt
            .retry_reason
            .map_or_else(|| "unclassified".to_string(), |reason| reason.to_string())
    };
    let mut line = format!(
        "push attempt {} at {}: {outcome}",
        attempt.attempt,
        attempt
            .started_at
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    );
    if let Some(age_ms) = attempt
        .token
        .and_then(|token| token.age_at(attempt.started_at))
        .map(|age| u64::try_from(age.as_millis()).unwrap_or(u64::MAX))
    {
        let _ = write!(line, " (token age {age_ms}ms)");
    }
    line
}

impl Concluded {
    /// Run the publish steps, recording each one into `outcome` as it lands.
    ///
    /// `outcome` accumulates what actually happened, so a branch that reached
    /// the remote is still reported when pull request creation later fails.
    async fn publish(
        &self,
        options: &PublishOptions,
        outcome: &mut PublishOutcome,
    ) -> Result<(), Error> {
        // A run that did not succeed, or that never intended to touch the
        // remote, has nothing to publish — even when a pull request was asked
        // for. Only a run that got far enough to publish can fail publishing.
        if !self
            .outcome
            .as_ref()
            .is_ok_and(|o| o.status.is_successful())
            || self.run_options.dry_run_enabled()
        {
            return Ok(());
        }

        // Soft-exit downgrade (fabro-18a5): a run routed through a
        // kind="deadlock" or kind="soft" exit is heading to a
        // failed(Deadlock|SoftStop) terminal — publish is not eligible, so
        // neither the final push nor a pull request runs. The exit stage
        // itself usually succeeded, which is exactly what let publish run
        // before. Work preservation does not need the terminal push:
        // checkpoint pushes during execution already carried the run branch.
        if matches!(self.conclusion.exit_kind.as_str(), "deadlock" | "soft") {
            return Ok(());
        }

        let pull_request_requested = options.pr_config.is_some();
        let (origin_url, run_branch) = match self.publish_target(options) {
            Ok(target) => target,
            Err(_) if !pull_request_requested => return Ok(()),
            Err(reason) => return Err(self.pull_request_error(reason)),
        };

        self.push_final_commit(run_branch).await?;
        outcome.pushed_branch = Some(run_branch.to_string());

        let Some(pr_config) = options.pr_config.as_ref() else {
            return Ok(());
        };
        let diff = self.conclusion.diff.patch.as_deref().unwrap_or_default();
        if diff.trim().is_empty() || diff_is_journal_only(diff) {
            return Ok(());
        }

        // Only pull request creation needs the SHA, to check that the remote
        // branch really carries this run's work. Pushing does not: the refspec
        // sends whatever the branch points at.
        let final_sha = self
            .conclusion
            .final_git_commit_sha
            .as_deref()
            .ok_or_else(|| {
                self.pull_request_error("pull request creation requires the run's final commit SHA")
            })?;

        let github_base_url = fabro_github::github_api_base_url();
        let request = self.pull_request_request(
            options,
            pr_config,
            origin_url,
            run_branch,
            final_sha,
            &github_base_url,
        )?;

        let created = open_pull_request(request).await.map_err(|error| {
            self.services.emitter.emit(&Event::PullRequestFailed {
                creation_id: None,
                error:       error.clone(),
            });
            Error::publish_with_source("failed to create pull request", anyhow::anyhow!(error))
        })?;

        self.services
            .emitter
            .emit(&Event::pull_request_created_with_auto_merge(
                &created.link,
                &created.base_branch,
                &created.head_branch,
                final_sha,
                &created.title,
                pr_config.draft,
                created.auto_merge.clone(),
            ));
        outcome.pr_url = Some(created.link.html_url());

        Ok(())
    }

    /// The pull-request open request for this run, every field wired from
    /// its declared source.
    ///
    /// The PR content model is `options.pr_resolved_model` — the dedicated
    /// `[run.pull_request]` model when it resolved against the catalog, else
    /// the run model. `options.pr_model` is only the run-model fallback
    /// input to that resolution, never the content model itself.
    fn pull_request_request<'a>(
        &'a self,
        options: &'a PublishOptions,
        pr_config: &'a PullRequestSettings,
        origin_url: &'a str,
        run_branch: &'a str,
        final_sha: &'a str,
        github_base_url: &'a str,
    ) -> Result<OpenPullRequestRequest<'a>, Error> {
        let base_branch = self.run_options.base_branch.as_deref().ok_or_else(|| {
            self.pull_request_error("pull request creation requires a base branch")
        })?;
        let credentials = options.github_app.as_ref().ok_or_else(|| {
            self.pull_request_error("pull request creation requires GitHub credentials")
        })?;
        Ok(OpenPullRequestRequest {
            github: fabro_github::GitHubContext::new(credentials, github_base_url),
            origin_url,
            base_branch,
            head_branch: run_branch,
            expected_head_sha: final_sha,
            goal: self.graph.goal(),
            diff: self.conclusion.diff.patch.as_deref().unwrap_or_default(),
            model: &options.pr_resolved_model,
            reasoning_effort: options.pr_reasoning_effort,
            draft: pr_config.draft,
            auto_merge: pr_config.auto_merge.then_some(AutoMergeOptions {
                merge_strategy: pr_config.merge_strategy,
            }),
            run_store: &self.services.run_store,
            llm_source: Arc::clone(&self.services.llm_source),
            catalog: Arc::clone(&self.services.catalog),
            conclusion: Some(&self.conclusion),
            run_state: None,
        })
    }

    /// The origin and run branch to publish to.
    ///
    /// `Err` carries why there is no target. That is only a failure when a
    /// pull request was requested; otherwise publish just has nothing to do.
    fn publish_target<'a>(
        &'a self,
        options: &'a PublishOptions,
    ) -> Result<(&'a str, &'a str), &'static str> {
        let origin_url = options
            .origin_url
            .as_deref()
            .filter(|origin| !origin.trim().is_empty())
            .ok_or("pull request creation requires a GitHub origin URL")?;
        let run_branch = self
            .run_options
            .run_branch()
            .ok_or("pull request creation requires a run branch")?;
        if !self.run_options.settings.run.run_branch.push {
            return Err("pull request creation requires run branch pushing");
        }
        Ok((origin_url, run_branch))
    }

    async fn push_final_commit(&self, run_branch: &str) -> Result<(), Error> {
        // The terminal push guards the whole run's value, so it gets a real
        // retry budget; attempts are nearly free at this point.
        let policy = fabro_sandbox::publish_push_policy();
        match push_run_branch(self.services.sandbox.as_ref(), run_branch, &policy).await {
            Ok(report) => {
                self.services.sandbox_git.record_successful_push();
                self.services.emitter.emit(&Event::GitPush {
                    branch:           run_branch.to_string(),
                    success:          true,
                    exec_output_tail: None,
                    attempts:         report.attempts,
                });
                Ok(())
            }
            Err(push_error) => {
                let fabro_sandbox::PushError { report, error } = push_error;
                let exec_output_tail = fabro_sandbox::default_redacted_output_tail(&error);
                let attempts = report.attempts;
                self.services.emitter.emit(&Event::GitPush {
                    branch:           run_branch.to_string(),
                    success:          false,
                    exec_output_tail: exec_output_tail.clone(),
                    attempts:         attempts.clone(),
                });
                Err(publish_push_error(
                    run_branch,
                    error,
                    exec_output_tail,
                    &attempts,
                    self.services.sandbox_git.last_successful_push_at(),
                ))
            }
        }
    }

    fn pull_request_error(&self, message: &str) -> Error {
        self.services.emitter.emit(&Event::PullRequestFailed {
            creation_id: None,
            error:       message.to_string(),
        });
        Error::publish(message)
    }
}

/// True when every changed path in the patch is under `.fabro/journal/`.
///
/// A journal-only run carries no reviewable work, so it skips pull request
/// creation (fabro-9f97): its branch is still pushed — meta and checkpoint
/// machinery depend on that — but the PR previously flipped a successful
/// no-op run to failed(publish_failed) via a deterministic auto-merge 403
/// (run 01M0SFEYVC9TD6MP816RHEBFQY). Malformed headers fail closed: any
/// `diff --git` line whose `b/` path cannot be parsed counts as non-journal,
/// and a patch with no parseable path at all never skips.
fn diff_is_journal_only(diff: &str) -> bool {
    let mut saw_path = false;
    for line in diff.lines().filter(|line| line.starts_with("diff --git ")) {
        let Some(b_path) = diff_header_b_path(line) else {
            return false;
        };
        saw_path = true;
        if !b_path.starts_with(".fabro/journal/") {
            return false;
        }
    }
    saw_path
}

/// The changed (`b/`-side) path of one `diff --git a/… b/…` header line.
///
/// Git C-quotes any path containing special bytes, wrapping it in `"` with
/// backslash escapes and octal `\nnn` sequences — so a quoted path may carry
/// a literal ` b/` that a plain `rsplit_once(" b/")` would mis-split
/// (fabro-c0a2). A quoted a-side therefore locates the b-side by its closing
/// unescaped quote instead of splitting; plain headers keep the borrowed
/// fast path, quoted ones decode into an owned `Cow` only when escapes are
/// present. Malformed quoting (unterminated quote, dangling backslash) is
/// `None`, so `diff_is_journal_only` fails closed.
fn diff_header_b_path(line: &str) -> Option<Cow<'_, str>> {
    let rest = line.strip_prefix("diff --git ")?;
    let b_side = if rest.starts_with('"') {
        let a_body = rest.get(1..)?;
        let close = closing_quote_index(a_body)?;
        let after = a_body.get(close + '"'.len_utf8()..)?;
        let b_quoted = after.strip_prefix(" \"")?;
        let b_body = b_quoted.strip_prefix("b/")?.strip_suffix('"')?;
        b_body
    } else {
        rest.rsplit_once(" b/")?.1
    };
    decode_c_quoted_path(b_side)
}

/// Byte index of the closing `"` in `body` (opening quote already stripped),
/// skipping backslash-escaped characters. `None` when the quote never closes.
fn closing_quote_index(body: &str) -> Option<usize> {
    let mut escaped = false;
    for (index, character) in body.char_indices() {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '"' {
            return Some(index);
        }
    }
    None
}

/// Decode a git C-quoted path body (quotes and `b/` prefix stripped) when it
/// carries escapes, otherwise borrow it unchanged. Escape handling follows
/// git's `quote_c_style` set (`\a \b \f \n \r \t \v \" \\` plus octal `\nnn`,
/// which is how git writes an embedded space as `\040`). Documented choice
/// for an unrecognized escape: keep the backslash literally rather than
/// fail — the surrounding quoting already fixed the path boundaries, so
/// only the byte value is uncertain, never the split.
fn decode_c_quoted_path(body: &str) -> Option<Cow<'_, str>> {
    if !body.contains('\\') {
        return Some(Cow::Borrowed(body));
    }
    let mut decoded = String::with_capacity(body.len());
    let mut characters = body.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            decoded.push(character);
            continue;
        }
        match characters.next()? {
            'a' => decoded.push('\u{7}'),
            'b' => decoded.push('\u{8}'),
            'f' => decoded.push('\u{c}'),
            'n' => decoded.push('\n'),
            'r' => decoded.push('\r'),
            't' => decoded.push('\t'),
            'v' => decoded.push('\u{b}'),
            '"' => decoded.push('"'),
            '\\' => decoded.push('\\'),
            digit @ '0'..='7' => {
                let mut value = digit.to_digit(8)?;
                for _ in 0..2 {
                    match characters.clone().next() {
                        Some(next @ '0'..='7') => {
                            value = value * 8 + next.to_digit(8)?;
                            characters.next();
                        }
                        _ => break,
                    }
                }
                decoded.push(char::from_u32(value)?);
            }
            unknown => {
                decoded.push('\\');
                decoded.push(unknown);
            }
        }
    }
    Some(Cow::Owned(decoded))
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::error::FailureCategory;

    /// A journal-only diff skips PR creation entirely (fabro-9f97): publish
    /// returns before ever reaching the pull request call.
    #[test]
    fn journal_only_diff_skips_pull_request_creation() {
        let diff = "diff --git a/.fabro/journal/run.jsonl b/.fabro/journal/run.jsonl\n\
                    index 1111111..2222222 100644\n\
                    --- a/.fabro/journal/run.jsonl\n\
                    +++ b/.fabro/journal/run.jsonl\n\
                    @@ -1 +1,2 @@\n\
                    +{}\n";
        assert!(diff_is_journal_only(diff));
    }

    /// A mixed diff (journal plus real code) still publishes: the gate must
    /// not skip, so PR creation proceeds exactly as before.
    #[test]
    fn mixed_diff_still_publishes() {
        let diff = "diff --git a/.fabro/journal/run.jsonl b/.fabro/journal/run.jsonl\n\
                    @@ -1 +1,2 @@\n\
                    +{}\n\
                    diff --git a/src/lib.rs b/src/lib.rs\n\
                    @@ -1 +1,2 @@\n\
                    +fn new_feature() {}\n";
        assert!(!diff_is_journal_only(diff));
    }

    /// An empty diff never reaches the journal check — the existing
    /// `diff.trim().is_empty()` early return pins that behavior, and a patch
    /// with no parseable `diff --git` path also fails closed (no skip).
    #[test]
    fn empty_or_unparseable_diff_does_not_skip_via_journal_gate() {
        assert!(!diff_is_journal_only(""));
        assert!(!diff_is_journal_only("   \n\t"));
        assert!(!diff_is_journal_only("not a git diff at all"));
    }

    /// Table-driven coverage for quote-aware `diff_header_b_path` parsing
    /// (fabro-c0a2): plain headers keep their old result, git C-quoted
    /// headers decode correctly even when the quoted path contains a literal
    /// ` b/` that the old `rsplit_once(" b/")` mis-split, and escape
    /// sequences (space via `\040`, backslash via `\\`) decode per git's
    /// `quote_c_style`. Malformed quoting fails closed to `None`.
    #[test]
    fn diff_header_b_path_parses_plain_and_quoted_paths() {
        let cases: &[(&str, Option<&str>)] = &[
            ("diff --git a/x b/x", Some("x")),
            (
                "diff --git a/.fabro/journal/run.jsonl b/.fabro/journal/run.jsonl",
                Some(".fabro/journal/run.jsonl"),
            ),
            // Space-bearing quoted path containing a literal ` b/`.
            ("diff --git \"a/p b/ one\" \"b/p b/ one\"", Some("p b/ one")),
            // Embedded space encoded the way git actually emits it.
            ("diff --git \"a/p\\040q\" \"b/p\\040q\"", Some("p q")),
            // Backslash escape: git writes a literal backslash as `\\`.
            ("diff --git \"a/p\\\\ q\" \"b/p\\\\ q\"", Some("p\\ q")),
            // Unterminated quoting never yields a mis-split fragment.
            ("diff --git \"a/p b/ one\" \"b/p b/ one", None),
        ];
        for (line, expected) in cases {
            assert_eq!(
                diff_header_b_path(line).as_deref(),
                *expected,
                "line: {line}"
            );
        }
    }

    /// The PR content model must be `pr_resolved_model`, never the run-model
    /// fallback `pr_model` (regression a1e27c9bf: the dedicated model was
    /// resolved, stored, and dropped — every PR used the run model, and no
    /// test noticed because both options fields carried the same fixture
    /// value).
    #[test]
    fn pull_request_request_wires_the_resolved_pr_model() {
        use std::collections::HashMap;

        use fabro_graphviz::graph::Graph;
        use fabro_types::settings::run::MergeStrategy;
        use fabro_types::{Conclusion, RunDiff, RunId, RunTiming, StageOutcome, WorkflowSettings};
        use tokio_util::sync::CancellationToken;

        use crate::outcome::Outcome;
        use crate::run_options::RunOptions;

        let services = Arc::new(crate::services::EngineServices::test_default().run);
        let concluded = Concluded {
            outcome:        Ok(Outcome::success()),
            conclusion:     Conclusion {
                timestamp:            Utc::now(),
                status:               StageOutcome::Succeeded,
                timing:               RunTiming::default(),
                failure:              None,
                final_git_commit_sha: Some("final-sha".to_string()),
                stages:               Vec::new(),
                usage:                None,
                total_retries:        0,
                diff:                 RunDiff {
                    patch:   Some("diff --git a/src/lib.rs b/src/lib.rs".to_string()),
                    summary: None,
                },
                exit_kind:            "natural".to_string(),
            },
            artifact_count: 0,
            graph:          Graph::new("test"),
            run_options:    RunOptions {
                settings:         WorkflowSettings::default(),
                run_dir:          std::path::PathBuf::new(),
                cancel_token:     CancellationToken::new(),
                git_identity:     None,
                run_id:           RunId::new(),
                labels:           HashMap::new(),
                workflow_slug:    Some("test".to_string()),
                github_app:       None,
                pre_run_git:      None,
                fork_source_ref:  None,
                base_branch:      Some("main".to_string()),
                display_base_sha: None,
                git:              None,
            },
            services:       Arc::clone(&services),
        };
        let options = PublishOptions {
            pr_config:           Some(PullRequestSettings {
                enabled:          true,
                draft:            true,
                auto_merge:       true,
                merge_strategy:   MergeStrategy::Squash,
                model:            None,
                reasoning_effort: None,
            }),
            github_app:          Some(fabro_github::GitHubCredentials::Pat(
                "test-token".to_string(),
            )),
            origin_url:          Some("https://github.com/owner/repo.git".to_string()),
            pr_model:            "run-model-sentinel".to_string(),
            pr_resolved_model:   "pr-model-sentinel".to_string(),
            pr_reasoning_effort: None,
        };
        let pr_config = options.pr_config.as_ref().unwrap();

        let request = concluded
            .pull_request_request(
                &options,
                pr_config,
                "https://github.com/owner/repo.git",
                "fabro/run/123",
                "final-sha",
                "https://api.github.example.test",
            )
            .expect("wiring should not fail with base branch and credentials set");

        assert_eq!(request.model, "pr-model-sentinel");
        assert_eq!(request.base_branch, "main");
        assert_eq!(request.head_branch, "fabro/run/123");
        assert_eq!(request.expected_head_sha, "final-sha");
        assert!(request.draft);
        assert_eq!(
            request.auto_merge.map(|merge| merge.merge_strategy),
            Some(MergeStrategy::Squash)
        );
    }

    fn push_attempt(
        attempt: u32,
        retry_reason: Option<fabro_sandbox::GitRetryReason>,
        token_age_ms: Option<u64>,
    ) -> fabro_sandbox::PushAttempt {
        let started_at = Utc::now();
        fabro_sandbox::PushAttempt {
            attempt,
            started_at,
            success: false,
            retry_reason,
            exec_output_tail: None,
            token: token_age_ms.map(|age_ms| fabro_sandbox::TokenSnapshot {
                generation: 14,
                provenance: fabro_sandbox::TokenProvenance::Minted {
                    minted_at:  started_at
                        - chrono::Duration::milliseconds(i64::try_from(age_ms).unwrap()),
                    expires_at: started_at + chrono::Duration::hours(1),
                },
            }),
        }
    }

    fn push_attempts_with_reasons(
        reasons: &[Option<fabro_sandbox::GitRetryReason>],
    ) -> Vec<fabro_sandbox::PushAttempt> {
        reasons
            .iter()
            .enumerate()
            .map(|(index, reason)| fabro_sandbox::PushAttempt {
                attempt:          u32::try_from(index).unwrap() + 1,
                started_at:       Utc::now(),
                success:          false,
                retry_reason:     *reason,
                exec_output_tail: None,
                token:            None,
            })
            .collect()
    }

    fn push_source_error() -> fabro_sandbox::Error {
        fabro_sandbox::Error::message("remote: Repository not found.")
    }

    /// Exhausted retries on a retryable classification are transient
    /// infrastructure, not deterministic: the same push succeeded manually an
    /// hour after run 01M0DH033P2XSTHAGVBHG6922F failed, with no
    /// configuration change.
    #[test]
    fn exhausted_transient_retries_classify_as_transient_infra() {
        let attempts = push_attempts_with_reasons(&[
            Some(fabro_sandbox::GitRetryReason::TokenReplication),
            Some(fabro_sandbox::GitRetryReason::TokenReplication),
        ]);
        let error =
            publish_push_error("fabro/run/test", push_source_error(), None, &attempts, None);
        assert_eq!(error.failure_category(), FailureCategory::TransientInfra);
    }

    #[test]
    fn permanently_classified_push_falls_back_to_message_sniffing() {
        let attempts = push_attempts_with_reasons(&[None]);
        let error =
            publish_push_error("fabro/run/test", push_source_error(), None, &attempts, None);
        // "Repository not found." carries no transient hint for the
        // heuristic, so the fallback stays deterministic.
        assert_eq!(error.failure_category(), FailureCategory::Deterministic);
    }

    #[test]
    fn failure_detail_renders_one_cause_line_per_attempt() {
        let attempts = vec![
            push_attempt(
                1,
                Some(fabro_sandbox::GitRetryReason::TokenReplication),
                Some(180),
            ),
            push_attempt(
                2,
                Some(fabro_sandbox::GitRetryReason::TokenReplication),
                Some(3320),
            ),
        ];
        let last_push = Utc::now() - chrono::Duration::seconds(67);
        let error = publish_push_error(
            "fabro/run/test",
            push_source_error(),
            None,
            &attempts,
            Some(last_push),
        );

        let detail = error.to_failure_detail();
        assert!(
            detail.message.contains("last successful push at"),
            "{}",
            detail.message
        );
        let attempt_lines: Vec<&String> = detail
            .causes
            .iter()
            .filter(|cause| cause.starts_with("push attempt"))
            .collect();
        assert_eq!(attempt_lines.len(), 2);
        assert!(
            attempt_lines[0].contains("token_replication"),
            "{attempt_lines:?}"
        );
        assert!(
            attempt_lines[0].contains("(token age 180ms)"),
            "{attempt_lines:?}"
        );
        assert!(
            attempt_lines[1].contains("(token age 3320ms)"),
            "{attempt_lines:?}"
        );
        assert_eq!(
            detail
                .causes
                .iter()
                .filter(|cause| cause.as_str() == "remote: Repository not found.")
                .count(),
            1,
            "the source chain must not repeat the inner push error"
        );
    }
}
