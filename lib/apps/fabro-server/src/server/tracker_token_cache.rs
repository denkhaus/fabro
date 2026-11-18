//! Cached installation-token sources for work-project trackers (work#1384).
//!
//! `AppState::work_tracker_for_project` used to construct a fresh
//! [`InstallationTokenSource`] on every call. The source carries its own
//! token cache (a `Mutex<Option<CachedToken>>`, reused until `REFRESH_MARGIN`
//! — 10 minutes of a token's 60-minute life), but with a fresh source per
//! call that cache was dead weight: every resolution paid one JWT sign plus
//! one `POST /app/installations/{id}/access_tokens` mint. At the work
//! reconciler's 30-second cadence alone that is ~2,880 mints per day and
//! project, and on 2026-11-17 production's tracker started refusing every
//! read AND write with GitHub 403 rate-limit exhaustion under normal call
//! volume — the per-call minting had drained the installation's
//! token-creation budget.
//!
//! This cache holds the SOURCE per mint scope, not the tracker: a
//! `GitHubTracker` pins one token string for its whole lifetime
//! (`bearer_auth(&self.token)` per call), so a cached tracker would serve a
//! dead token after 60 minutes. A cached source hands out a reused token
//! until `REFRESH_MARGIN` to expiry, and the cheap `GitHubTracker` struct is
//! rebuilt around each resolved token.
//!
//! The cache key carries a fingerprint of the resolved credentials, so any
//! credential change misses the cache without an invalidation hook. A
//! `replace_runtime_settings` hook was considered and rejected: the private
//! key does not flow through settings — it lives in the vault and rotates
//! with `fabro secret set`, independent of a settings reload — so a reload
//! hook would miss exactly the credential changes this fingerprint catches.
//!
//! Locking: the map lock guards only get/insert of the `Arc`; source
//! construction and the caller's async `resolve().await` both happen
//! outside it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use fabro_github::GitHubCredentials;
use fabro_github::token_source::InstallationTokenSource;
use sha2::{Digest, Sha256};

/// The permission scope every tracker token is minted with.
pub(crate) fn tracker_token_permissions() -> serde_json::Value {
    serde_json::json!({"issues": "write", "metadata": "read"})
}

/// Everything that determines a source's mint behavior. Two resolutions
/// with equal keys share one source; any difference builds a fresh one,
/// which is how a credential change self-invalidates without a reload hook.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TrackerTokenSourceKey {
    tracker_repo:            String,
    /// The permission scope is a fixed literal built identically per call,
    /// so its serialization (this workspace's serde_json preserves
    /// insertion order) is stable across resolutions.
    permissions:             String,
    /// The API base URL the source bakes into its minter at construction
    /// (the env-derived default, read again per lookup).
    mint_base_url:           String,
    /// SHA-256 over the credential identity (strategy tag, app id, secret
    /// material) — never the secret itself.
    credentials_fingerprint: String,
}

impl TrackerTokenSourceKey {
    fn new(tracker_repo: &str, credentials: &GitHubCredentials) -> Self {
        Self {
            tracker_repo:            tracker_repo.to_owned(),
            permissions:             serde_json::to_string(&tracker_token_permissions())
                .expect("tracker permissions should serialize"),
            mint_base_url:           fabro_github::github_api_base_url(),
            credentials_fingerprint: credentials_fingerprint(credentials),
        }
    }
}

/// A non-reversible fingerprint of the credential identity: a strategy tag
/// plus SHA-256 over the identifying and secret material. Two resolutions
/// share a cached source only when their credentials are byte-identical.
fn credentials_fingerprint(credentials: &GitHubCredentials) -> String {
    let mut hasher = Sha256::new();
    match credentials {
        GitHubCredentials::Pat(token) => {
            hasher.update(b"pat\x00");
            hasher.update(token.as_bytes());
        }
        GitHubCredentials::Installation(token) => {
            hasher.update(b"installation\x00");
            hasher.update(token.token.as_bytes());
            hasher.update(b"\x00");
            hasher.update(token.expires_at.to_rfc3339().as_bytes());
        }
        GitHubCredentials::App(app) => {
            hasher.update(b"app\x00");
            hasher.update(app.app_id.as_bytes());
            hasher.update(b"\x00");
            hasher.update(app.private_key_pem.as_bytes());
            hasher.update(b"\x00");
            hasher.update(app.slug.as_deref().unwrap_or_default().as_bytes());
        }
    }
    hex::encode(hasher.finalize())
}

/// Cached installation-token sources per tracker mint scope (work#1384):
/// one [`InstallationTokenSource`] per (tracker repo, permission scope,
/// credentials) key, so its per-instance token cache stays warm across
/// calls instead of dying with the resolution that built it.
#[derive(Default)]
pub(crate) struct TrackerTokenSourceCache {
    sources: Mutex<HashMap<TrackerTokenSourceKey, Arc<InstallationTokenSource>>>,
}

impl TrackerTokenSourceCache {
    /// The cached source for one project's tracker mint scope. A cache miss
    /// builds the source OUTSIDE the lock (construction builds an HTTP
    /// client) and publishes it under the lock; the caller's async
    /// `resolve().await` also happens outside the lock.
    pub(crate) fn source_for(
        &self,
        tracker_repo: &str,
        credentials: &GitHubCredentials,
    ) -> anyhow::Result<Arc<InstallationTokenSource>> {
        let key = TrackerTokenSourceKey::new(tracker_repo, credentials);
        if let Some(source) = self
            .sources
            .lock()
            .expect("tracker token source cache lock poisoned")
            .get(&key)
        {
            return Ok(Arc::clone(source));
        }
        let source = build_tracker_token_source(tracker_repo, credentials)?;
        let mut sources = self
            .sources
            .lock()
            .expect("tracker token source cache lock poisoned");
        // A concurrent caller may have published a source for the same key
        // while this one built; either serves the same mint scope.
        let published = sources.entry(key).or_insert(source);
        Ok(Arc::clone(published))
    }
}

fn build_tracker_token_source(
    tracker_repo: &str,
    credentials: &GitHubCredentials,
) -> anyhow::Result<Arc<InstallationTokenSource>> {
    let (owner, name) = tracker_repo
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("tracker repository must be owner/name: {tracker_repo}"))?;
    InstallationTokenSource::for_repository(
        credentials,
        owner.to_owned(),
        name.to_owned(),
        tracker_token_permissions(),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fabro_config::{RunLayer, ServerSettingsBuilder};
    use fabro_github::{GitHubAppCredentials, GitHubCredentials};
    use fabro_static::EnvVars;
    use fabro_types::ServerSettings;
    use fabro_vault::SecretType;
    use fabro_work::WorkProject;
    use httpmock::MockServer;

    use crate::test_support::{TestAppStateBuilder, test_work_project};

    /// The installation id the mock resolves every repository to.
    const INSTALLATION_ID: u64 = 42;

    /// An expiry far beyond any test run, so a cached token is always
    /// reusable (`near_expiry` never fires inside the test).
    const FAR_FUTURE_EXPIRY: &str = "2030-01-01T00:00:00Z";

    /// Throwaway RSA test keys, mirrored by reference — NOT copied here, so
    /// the betterleaks gate never sees new key material. Both files are
    /// already in the audited `.betterleaks-baseline.json` (the twin crate
    /// and fabro-github commit them as static testdata). The mock never
    /// verifies the JWT; `sign_app_jwt` only needs a parseable PEM, and the
    /// two keys must differ for the credentials-change pin.
    const RSA_KEY_A: &str =
        include_str!("../../../../components/fabro-github/src/testdata/rsa_private.pem");
    const RSA_KEY_B: &str =
        include_str!("../../../../../test/twin/github/src/testdata/rsa_private.pem");

    fn github_app_settings(app_id: &str) -> ServerSettings {
        ServerSettingsBuilder::from_toml(&format!(
            r#"_version = 1

[server.auth]
methods = ["dev-token"]

[server.integrations.github]
enabled = true
strategy = "app"
app_id = "{app_id}"
"#,
        ))
        .expect("github app settings fixture should resolve")
    }

    /// An AppState whose work tracker mints through the local mock: the
    /// installation-token source bakes the GitHub base URL from the
    /// `GITHUB_BASE_URL` env at construction, so it is set here, before the
    /// first resolution (nextest runs every test in its own process).
    #[expect(
        unsafe_code,
        reason = "test-only: set GITHUB_BASE_URL to the local mock so the mint hits httpmock; \
                  nextest isolates each test in its own process, so the value cannot leak"
    )]
    #[expect(
        clippy::disallowed_methods,
        reason = "test-only: the production seam (InstallationTokenSource::with_app_scope) \
                  bakes the base URL from the env at construction — there is no injection \
                  seam without changing fabro-github"
    )]
    fn app_state_minting_through(
        server: &MockServer,
        private_key_pem: &str,
    ) -> Arc<crate::server::AppState> {
        // SAFETY: test process, single-threaded construction before any
        // other env reader in this test's process.
        unsafe {
            std::env::set_var(EnvVars::GITHUB_BASE_URL, server.base_url());
        }
        TestAppStateBuilder::new()
            .runtime_settings(github_app_settings("1234"), RunLayer::default())
            .vault_entries([(EnvVars::GITHUB_APP_PRIVATE_KEY, private_key_pem)])
            .github_api_base_url(server.base_url())
            .build()
    }

    fn project_with_tracker_repo(project_id: &str, tracker_repo: &str) -> WorkProject {
        WorkProject {
            tracker_repo: tracker_repo.to_owned(),
            ..test_work_project(project_id)
        }
    }

    async fn mock_installation_lookup<'a>(
        server: &'a MockServer,
        owner: &str,
        repo: &str,
    ) -> httpmock::Mock<'a> {
        server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path(format!("/repos/{owner}/{repo}/installation"));
                then.status(200).json_body(serde_json::json!({
                    "id": INSTALLATION_ID,
                }));
            })
            .await
    }

    async fn mock_token_mint(server: &MockServer) -> httpmock::Mock<'_> {
        server
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST).path(format!(
                    "/app/installations/{INSTALLATION_ID}/access_tokens"
                ));
                then.status(201).json_body(serde_json::json!({
                    "token": "ghs_tracker_test",
                    "expires_at": FAR_FUTURE_EXPIRY,
                }));
            })
            .await
    }

    #[test]
    fn cache_key_tracks_repo_permissions_and_credentials() {
        use super::{TrackerTokenSourceKey, credentials_fingerprint};

        let app = |pem: &str| {
            GitHubCredentials::App(GitHubAppCredentials {
                app_id:          "1234".to_owned(),
                private_key_pem: pem.to_owned(),
                slug:            None,
            })
        };
        let key = |repo: &str, creds: &GitHubCredentials| TrackerTokenSourceKey::new(repo, creds);

        // Same repo + credentials -> equal keys share a cache entry.
        assert_eq!(
            key("acme/alpha", &app(RSA_KEY_A)),
            key("acme/alpha", &app(RSA_KEY_A))
        );
        // A rotated private key must miss the cache.
        assert_ne!(
            key("acme/alpha", &app(RSA_KEY_A)),
            key("acme/alpha", &app(RSA_KEY_B))
        );
        // A changed app id must miss the cache even with the same key material.
        let other_app_id = GitHubCredentials::App(GitHubAppCredentials {
            app_id:          "9999".to_owned(),
            private_key_pem: RSA_KEY_A.to_owned(),
            slug:            None,
        });
        assert_ne!(
            key("acme/alpha", &app(RSA_KEY_A)),
            key("acme/alpha", &other_app_id)
        );
        // A different tracker repo is a different mint scope.
        assert_ne!(
            key("acme/alpha", &app(RSA_KEY_A)),
            key("acme/beta", &app(RSA_KEY_A))
        );
        // The fingerprint never carries the secret bytes.
        assert!(!credentials_fingerprint(&app(RSA_KEY_A)).contains("PRIVATE"));
    }

    /// THE budget pin (work#1384): two consecutive tracker resolutions for
    /// the same project must mint ONCE — the second resolution reuses the
    /// cached token instead of paying another JWT sign + mint round trip.
    /// Against the per-call source construction this observed TWO mints,
    /// which drained production's installation into GitHub 403
    /// rate-limit exhaustion on 2026-11-17.
    #[tokio::test]
    async fn two_consecutive_tracker_resolutions_mint_once() {
        let server = MockServer::start_async().await;
        let installation = mock_installation_lookup(&server, "denkhaus", "work").await;
        let mint = mock_token_mint(&server).await;
        let state = app_state_minting_through(&server, RSA_KEY_A);
        let project = test_work_project("budget-pin");

        let _first = state.work_tracker_for_project(&project).await.unwrap();
        let _second = state.work_tracker_for_project(&project).await.unwrap();

        assert_eq!(
            installation.calls(),
            1,
            "installation lookup must happen once"
        );
        assert_eq!(
            mint.calls(),
            1,
            "the second resolution must reuse the cached token, not mint again"
        );
    }

    /// The credentials-change pin: a changed private key must miss the cache
    /// and mint a fresh token — the cached source for the old credential
    /// must never serve the new one.
    #[tokio::test]
    async fn changed_credentials_mint_again_instead_of_serving_the_old_source() {
        let server = MockServer::start_async().await;
        let installation = mock_installation_lookup(&server, "denkhaus", "work").await;
        let mint = mock_token_mint(&server).await;
        let state = app_state_minting_through(&server, RSA_KEY_A);
        let project = test_work_project("credentials-pin");

        let _first = state.work_tracker_for_project(&project).await.unwrap();
        let _warm = state.work_tracker_for_project(&project).await.unwrap();
        assert_eq!(mint.calls(), 1, "the cache must be warm before the change");

        state
            .stores
            .vault
            .set(
                EnvVars::GITHUB_APP_PRIVATE_KEY,
                RSA_KEY_B,
                SecretType::Token,
                None,
            )
            .await
            .expect("rotating the test private key should persist");

        let _after = state.work_tracker_for_project(&project).await.unwrap();
        assert_eq!(
            mint.calls(),
            2,
            "changed credentials must produce a fresh source that mints again"
        );
        assert_eq!(installation.calls(), 2);
    }

    /// The distinct-projects pin: two different tracker repos mint
    /// separately (each repo's token is scoped to its repository), while a
    /// repeat resolution of the same repo reuses its own cached token.
    #[tokio::test]
    async fn distinct_tracker_repos_mint_separately() {
        let server = MockServer::start_async().await;
        let alpha_installation = mock_installation_lookup(&server, "acme", "alpha").await;
        let beta_installation = mock_installation_lookup(&server, "acme", "beta").await;
        let mint = mock_token_mint(&server).await;
        let state = app_state_minting_through(&server, RSA_KEY_A);
        let alpha = project_with_tracker_repo("alpha-pin", "acme/alpha");
        let beta = project_with_tracker_repo("beta-pin", "acme/beta");

        let _alpha_first = state.work_tracker_for_project(&alpha).await.unwrap();
        let _alpha_warm = state.work_tracker_for_project(&alpha).await.unwrap();
        let _beta_first = state.work_tracker_for_project(&beta).await.unwrap();

        assert_eq!(alpha_installation.calls(), 1);
        assert_eq!(
            beta_installation.calls(),
            1,
            "the second repo must resolve its own installation"
        );
        assert_eq!(
            mint.calls(),
            2,
            "two repos mint separately; the repeat alpha resolution reuses"
        );
    }
}
