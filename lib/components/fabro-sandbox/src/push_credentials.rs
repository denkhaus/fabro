//! Shared push-credential state for clone-based sandbox providers.
//!
//! Docker and Daytona embed GitHub credentials into the cloned repository's
//! `origin` remote and refresh them before pushes. Both providers hold this
//! state so the compare → `set-url` → record sequence, the generation
//! tracking, and the refresh-error logging behave identically across
//! providers. The token cache itself sits below the providers, in
//! [`fabro_github::token_source::InstallationTokenSource`].

use std::future::Future;
use std::sync::Arc;

use fabro_github::GitHubCredentials;
use fabro_github::token_source::{InstallationTokenSource, ResolvedToken};
use fabro_redact::DisplaySafeUrl;
use tokio::sync::Mutex;

use crate::sandbox::{RefreshOutcome, RemoteCredentialAction};

/// Build the shared installation-token source for a clone-based sandbox.
///
/// Returns `None` when there are no managed credentials or no GitHub origin
/// to scope them to. Minted tokens carry the same `contents: write`
/// permission the clone token uses.
pub(crate) fn build_token_source(
    github_app: Option<&GitHubCredentials>,
    clone_origin_url: Option<&str>,
) -> crate::Result<Option<Arc<InstallationTokenSource>>> {
    let Some(creds) = github_app else {
        return Ok(None);
    };
    let Some(origin_url) = clone_origin_url.filter(|url| !url.trim().is_empty()) else {
        return Ok(None);
    };
    let normalized = fabro_github::normalize_repo_origin_url(origin_url);
    if fabro_github::parse_github_owner_repo(&normalized).is_err() {
        // Non-GitHub origins never clone in these providers, so there is no
        // remote to keep credentials fresh for.
        return Ok(None);
    }
    InstallationTokenSource::for_origin(
        creds,
        &normalized,
        serde_json::json!({ "contents": "write" }),
    )
    .map(Some)
    .map_err(|err| crate::Error::message(format!("Failed to build GitHub token source: {err:#}")))
}

/// Push-credential state one provider instance tracks for its `origin`
/// remote.
pub(crate) struct PushCredentialState {
    source:   Option<Arc<InstallationTokenSource>>,
    /// Serializes compare → `set-url` → record. The token source's
    /// single-flight ends before the sandbox exec, so without this lock a
    /// refresh-ahead tick and a push could both see the old embedded
    /// generation and race on `.git/config.lock`. Holds the last
    /// successfully embedded token: its secret is already in the remote URL
    /// inside the sandbox, so retaining it adds no exposure, and it is what
    /// a push falls back to when a refresh fails. The tracked value is local
    /// belief, not ground truth — agent code inside the sandbox can rewrite
    /// `origin`.
    embedded: Mutex<Option<ResolvedToken>>,
}

impl PushCredentialState {
    pub(crate) fn new(source: Option<Arc<InstallationTokenSource>>) -> Self {
        Self {
            source,
            embedded: Mutex::new(None),
        }
    }

    pub(crate) fn source(&self) -> Option<&Arc<InstallationTokenSource>> {
        self.source.as_ref()
    }

    /// Record the token embedded in `origin` outside the refresh path — the
    /// clone is the first operation to embed a token, and it seeds this
    /// state so the first refresh compares against the clone token instead
    /// of believing nothing was ever embedded.
    pub(crate) async fn record_embedded(&self, token: ResolvedToken) {
        *self.embedded.lock().await = Some(token);
    }

    /// Refresh the credentials embedded in `origin`.
    ///
    /// Resolves through the shared source, skips the `set-url` exec when the
    /// resolved generation is already embedded, and records the new
    /// generation only after `set_url` succeeds. `set_url` receives the
    /// authenticated URL to embed and runs under the embed lock.
    pub(crate) async fn refresh<F, Fut>(
        &self,
        origin_url: &str,
        set_url: F,
    ) -> crate::Result<RefreshOutcome>
    where
        F: FnOnce(DisplaySafeUrl) -> Fut,
        Fut: Future<Output = crate::Result<()>>,
    {
        let Some(source) = &self.source else {
            return Ok(RefreshOutcome::none());
        };
        let mut embedded = self.embedded.lock().await;
        let resolved = match source.resolve().await {
            Ok(resolved) => resolved,
            Err(err) => {
                // The refresh-error path is defined, not incidental: the push
                // proceeds with the last embedded token, so log which one
                // that is instead of losing the credential state.
                if let Some(prev) = embedded.as_ref() {
                    tracing::warn!(
                        error = %format!("{err:#}"),
                        generation = prev.snapshot.generation,
                        provenance = %prev.snapshot.provenance,
                        token_age_ms = prev.snapshot.age_ms(),
                        "GitHub token refresh failed; origin keeps the last embedded credentials"
                    );
                } else {
                    tracing::warn!(
                        error = %format!("{err:#}"),
                        "GitHub token refresh failed and no credentials were ever embedded"
                    );
                }
                return Err(crate::Error::message(
                    "Failed to refresh push credentials: token_mint_failed",
                ));
            }
        };
        if embedded
            .as_ref()
            .is_some_and(|prev| prev.snapshot.generation == resolved.snapshot.generation)
        {
            return Ok(RefreshOutcome {
                action: RemoteCredentialAction::Unchanged,
                token:  Some(resolved.snapshot),
            });
        }
        let auth_url = fabro_github::embed_token_in_url(origin_url, resolved.token.expose())
            .map_err(|err| {
                crate::Error::message(format!("Failed to build authenticated origin URL: {err:#}"))
            })?;
        set_url(auth_url).await?;
        let snapshot = resolved.snapshot;
        *embedded = Some(resolved);
        Ok(RefreshOutcome {
            action: RemoteCredentialAction::Embedded,
            token:  Some(snapshot),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use chrono::Utc;
    use fabro_github::InstallationToken;
    use fabro_github::token_source::InstallationTokenMinter;

    use super::*;

    struct FixedMinter {
        calls: AtomicUsize,
        ttl:   chrono::Duration,
    }

    #[async_trait::async_trait]
    impl InstallationTokenMinter for FixedMinter {
        async fn mint(&self) -> anyhow::Result<InstallationToken> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(InstallationToken {
                token:      format!("ghs_gen{call}"),
                expires_at: Utc::now() + self.ttl,
            })
        }
    }

    struct FailingMinter;

    #[async_trait::async_trait]
    impl InstallationTokenMinter for FailingMinter {
        async fn mint(&self) -> anyhow::Result<InstallationToken> {
            Err(anyhow::anyhow!("mint failed"))
        }
    }

    fn minting_state(ttl: chrono::Duration) -> PushCredentialState {
        PushCredentialState::new(Some(InstallationTokenSource::with_minter(
            "owner/repo".to_string(),
            Box::new(FixedMinter {
                calls: AtomicUsize::new(0),
                ttl,
            }),
        )))
    }

    const ORIGIN: &str = "https://github.com/owner/repo";

    #[tokio::test]
    async fn refresh_without_managed_credentials_reports_none() {
        let state = PushCredentialState::new(None);
        let outcome = state
            .refresh(ORIGIN, |_| async { panic!("set-url must not run") })
            .await
            .unwrap();
        assert_eq!(outcome.action, RemoteCredentialAction::None);
        assert_eq!(outcome.token, None);
    }

    #[tokio::test]
    async fn refresh_embeds_a_new_generation_and_skips_matching_ones() {
        let state = minting_state(chrono::Duration::minutes(60));
        let set_url_calls = AtomicUsize::new(0);

        let first = state
            .refresh(ORIGIN, |auth_url| {
                set_url_calls.fetch_add(1, Ordering::SeqCst);
                assert!(auth_url.as_raw_url().as_str().contains("ghs_gen1"));
                async { Ok(()) }
            })
            .await
            .unwrap();
        assert_eq!(first.action, RemoteCredentialAction::Embedded);
        assert_eq!(first.token.unwrap().generation, 1);

        // The cached token is fresh, so the second refresh must skip set-url.
        let second = state
            .refresh(ORIGIN, |_| {
                set_url_calls.fetch_add(1, Ordering::SeqCst);
                async { Ok(()) }
            })
            .await
            .unwrap();
        assert_eq!(second.action, RemoteCredentialAction::Unchanged);
        assert_eq!(second.token.unwrap().generation, 1);
        assert_eq!(set_url_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn refresh_embeds_again_when_the_source_mints_a_new_generation() {
        // Tokens expire inside the margin, so every resolve re-mints.
        let state = minting_state(chrono::Duration::minutes(5));
        let set_url_calls = AtomicUsize::new(0);

        let first = state
            .refresh(ORIGIN, |_| {
                set_url_calls.fetch_add(1, Ordering::SeqCst);
                async { Ok(()) }
            })
            .await
            .unwrap();
        let second = state
            .refresh(ORIGIN, |_| {
                set_url_calls.fetch_add(1, Ordering::SeqCst);
                async { Ok(()) }
            })
            .await
            .unwrap();

        assert_eq!(first.token.unwrap().generation, 1);
        assert_eq!(second.action, RemoteCredentialAction::Embedded);
        assert_eq!(second.token.unwrap().generation, 2);
        assert_eq!(set_url_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn clone_seed_makes_the_first_refresh_a_no_op() {
        let state = minting_state(chrono::Duration::minutes(60));
        let clone_token = state.source().unwrap().mint_for_clone().await.unwrap();
        state.record_embedded(clone_token).await;

        let outcome = state
            .refresh(ORIGIN, |_| async { panic!("set-url must not run") })
            .await
            .unwrap();
        assert_eq!(outcome.action, RemoteCredentialAction::Unchanged);
        assert_eq!(outcome.token.unwrap().generation, 1);
    }

    #[tokio::test]
    async fn failed_set_url_does_not_record_the_new_generation() {
        let state = minting_state(chrono::Duration::minutes(60));

        let err = state
            .refresh(ORIGIN, |_| async {
                Err(crate::Error::message("set-url failed"))
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("set-url failed"));

        // The generation was not recorded, so the retry embeds again instead
        // of wrongly skipping.
        let retried = state.refresh(ORIGIN, |_| async { Ok(()) }).await.unwrap();
        assert_eq!(retried.action, RemoteCredentialAction::Embedded);
        assert_eq!(retried.token.unwrap().generation, 1);
    }

    #[tokio::test]
    async fn static_credentials_seeded_at_clone_skip_set_url() {
        let source = InstallationTokenSource::for_origin(
            &GitHubCredentials::Pat("ghp_pat".to_string()),
            ORIGIN,
            serde_json::json!({ "contents": "write" }),
        )
        .unwrap();
        let state = PushCredentialState::new(Some(source));
        let clone_token = state.source().unwrap().mint_for_clone().await.unwrap();
        state.record_embedded(clone_token).await;

        let outcome = state
            .refresh(ORIGIN, |_| async { panic!("set-url must not run") })
            .await
            .unwrap();
        assert_eq!(outcome.action, RemoteCredentialAction::Unchanged);
        assert!(outcome.token.unwrap().is_static());
    }

    #[tokio::test]
    async fn mint_failure_maps_to_the_token_mint_failed_error() {
        let state = PushCredentialState::new(Some(InstallationTokenSource::with_minter(
            "owner/repo".to_string(),
            Box::new(FailingMinter),
        )));

        let err = state
            .refresh(ORIGIN, |_| async { panic!("set-url must not run") })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("token_mint_failed"), "{err}");
    }

    #[test]
    fn token_source_requires_managed_credentials_and_a_github_origin() {
        assert!(build_token_source(None, Some(ORIGIN)).unwrap().is_none());
        let pat = GitHubCredentials::Pat("ghp_pat".to_string());
        assert!(build_token_source(Some(&pat), None).unwrap().is_none());
        assert!(
            build_token_source(Some(&pat), Some("https://gitlab.com/owner/repo"))
                .unwrap()
                .is_none()
        );
        assert!(
            build_token_source(Some(&pat), Some(ORIGIN))
                .unwrap()
                .is_some()
        );
    }
}
