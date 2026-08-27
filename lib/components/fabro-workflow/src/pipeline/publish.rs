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
    if let Some(refresh_error) = attempt.refresh_error {
        let _ = write!(line, ", refresh error: {refresh_error}");
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
        if diff.trim().is_empty() {
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

        let created = open_pull_request(request)
        .await
        .map_err(|error| {
            self.services.emitter.emit(&Event::PullRequestFailed {
                creation_id: None,
                error:       error.clone(),
            });
            Error::publish_with_source("failed to create pull request", anyhow::anyhow!(error))
        })?;

        self.services.emitter.emit(&Event::pull_request_created(
            &created.link,
            &created.base_branch,
            &created.head_branch,
            final_sha,
            &created.title,
            pr_config.draft,
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
            llm_source: self.services.llm_source.as_ref(),
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
        let plan = fabro_sandbox::RetryPlan::publish_push();
        match push_run_branch(self.services.sandbox.as_ref(), run_branch, &plan).await {
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

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::error::FailureCategory;


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
                billing:              None,
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
        refresh_error: Option<fabro_sandbox::RefreshErrorKind>,
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
            credential_action: Some(fabro_sandbox::RemoteCredentialAction::Unchanged),
            refresh_error,
        }
    }

    fn push_attempts_with_reasons(
        reasons: &[Option<fabro_sandbox::GitRetryReason>],
    ) -> Vec<fabro_sandbox::PushAttempt> {
        reasons
            .iter()
            .enumerate()
            .map(|(index, reason)| fabro_sandbox::PushAttempt {
                attempt:           u32::try_from(index).unwrap() + 1,
                started_at:        Utc::now(),
                success:           false,
                retry_reason:      *reason,
                exec_output_tail:  None,
                token:             None,
                credential_action: None,
                refresh_error:     None,
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
                None,
            ),
            push_attempt(
                2,
                Some(fabro_sandbox::GitRetryReason::TokenReplication),
                Some(3320),
                Some(fabro_sandbox::RefreshErrorKind::SetUrl),
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
            attempt_lines[1].contains("refresh error: set_url"),
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
