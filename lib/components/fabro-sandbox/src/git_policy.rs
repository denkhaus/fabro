//! Fabro's retry budget for git operations against GitHub.
//!
//! The driver owns the retry loop and the decision
//! ([`sandbox_driver::retry_git`]): a remote that cannot be reached is retried,
//! a rejected credential is retried only while the token is fresh enough to
//! still be replicating to GitHub's git endpoints, a static credential fails
//! fast, and a command whose outcome is unknown is never replayed. Fabro keeps
//! what is policy: how many attempts the host-side repository probe gets,
//! how it paces them, and when the credential it runs with was minted.
//!
//! Retries reuse the same token on purpose. Replication of a given token
//! only makes progress, so each attempt strictly improves the odds, while
//! re-minting would restart the replication clock.

use std::future::Future;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use fabro_github::token_source::TokenSnapshot;
use sandbox_driver::{GitBackoff, GitCredentials, GitFailure, GitFailureKind, GitRetryPolicy};

/// The username GitHub expects with an installation token or PAT.
const GITHUB_TOKEN_USERNAME: &str = "x-access-token";

/// Backoff between attempts: 3s, then 9s.
///
/// GitHub's guidance for token replication is to wait a few seconds and
/// retry with the same token. Sub-second delays land inside the same
/// replication window and spend an attempt for nothing.
fn replication_backoff() -> GitBackoff {
    GitBackoff::new(Duration::from_secs(3), 3.0, Duration::from_secs(10))
}

/// Host-side repository probes get 3 attempts at replication pacing, with
/// no deadline of their own.
#[must_use]
pub fn repository_probe_policy() -> GitRetryPolicy {
    GitRetryPolicy::new(3, replication_backoff())
}

/// Credentials carrying only the token's mint time, which is all the
/// driver's decision reads for git that ran outside a sandbox. The token
/// itself never leaves its snapshot.
fn credential_age(snapshot: Option<&TokenSnapshot>) -> Option<GitCredentials> {
    let snapshot = snapshot?;
    let credentials = GitCredentials::new(GITHUB_TOKEN_USERNAME, "");
    Some(match snapshot.minted_at() {
        Some(minted_at) => credentials.minted_at(SystemTime::from(minted_at)),
        None => credentials,
    })
}

/// The driver's failure for a rendered git message, so git that ran
/// outside a sandbox (the host-side repository probe, the metadata push)
/// is classified the same way as git the driver ran.
fn classified_failure(operation: &str, message: &str) -> sandbox_driver::Error {
    sandbox_driver::Error::Git(GitFailure::classified(
        operation,
        GitFailureKind::from_message(message),
        None,
    ))
}

/// Runs a host-side git operation that reports failures as rendered
/// messages under `policy`, retrying while the driver's decision says the
/// message is transient for the token behind `snapshot`. The final failure
/// comes back as the operation's own message.
pub async fn retry_git_messages<F, Fut>(
    policy: &GitRetryPolicy,
    snapshot: Option<&TokenSnapshot>,
    operation: &str,
    mut run: F,
) -> Result<(), String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    let credentials = credential_age(snapshot);
    // The operation's own message is kept beside the classified failure the
    // driver decides on, so the caller reads the message it knows.
    let last_message = Mutex::new(None);
    let result = sandbox_driver::retry_git(
        policy,
        credentials.as_ref(),
        operation,
        |_attempt, _timeout| {
            let attempt = run();
            let last_message = &last_message;
            async move {
                attempt.await.map_err(|message| {
                    let error = classified_failure(operation, &message);
                    *last_message.lock().unwrap_or_else(PoisonError::into_inner) = Some(message);
                    error
                })
            }
        },
    )
    .await;
    match result {
        Ok(_) => Ok(()),
        Err(failure) => Err(last_message
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner)
            .unwrap_or_else(|| failure.error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use fabro_github::token_source::TokenProvenance;

    use super::*;

    fn snapshot(age: Duration) -> TokenSnapshot {
        let now = Utc::now();
        TokenSnapshot {
            generation: 1,
            provenance: TokenProvenance::Minted {
                minted_at:  now - chrono::Duration::from_std(age).unwrap(),
                expires_at: now + chrono::Duration::hours(1),
            },
        }
    }

    fn static_snapshot() -> TokenSnapshot {
        TokenSnapshot {
            generation: 0,
            provenance: TokenProvenance::Static,
        }
    }

    #[test]
    fn probe_backoff_paces_at_replication_intervals() {
        let backoff = repository_probe_policy().backoff;
        assert_eq!(backoff.delay_after(1), Duration::from_secs(3));
        assert_eq!(backoff.delay_after(2), Duration::from_secs(9));
    }

    #[tokio::test(start_paused = true)]
    async fn host_side_retries_keep_the_operations_own_message() {
        let calls = Mutex::new(0_u32);
        let result = retry_git_messages(
            &repository_probe_policy(),
            Some(&snapshot(Duration::from_secs(1))),
            "repository probe",
            || {
                let attempt = {
                    let mut calls = calls.lock().unwrap();
                    *calls += 1;
                    *calls
                };
                async move {
                    if attempt < 3 {
                        Err(format!("remote: Repository not found. (attempt {attempt})"))
                    } else {
                        Ok(())
                    }
                }
            },
        )
        .await;
        assert_eq!(result, Ok(()));
        assert_eq!(*calls.lock().unwrap(), 3);

        let permanent = retry_git_messages(
            &repository_probe_policy(),
            Some(&static_snapshot()),
            "repository probe",
            || async { Err("remote: Repository not found.".to_owned()) },
        )
        .await;
        assert_eq!(permanent, Err("remote: Repository not found.".to_owned()));
    }
}
