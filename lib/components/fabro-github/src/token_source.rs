//! Cached GitHub installation-token source.
//!
//! One [`InstallationTokenSource`] serves every GitHub-token consumer for an
//! origin repository — the clone-based sandbox providers and the run-metadata
//! writer share a single source, so "reuse a token until near expiry" is the
//! default behavior instead of a per-call-site special case. Reusing mature
//! tokens keeps consumers out of GitHub's token-replication lag window, where
//! a token minted milliseconds earlier is rejected with 404 "Repository not
//! found" or an authentication failure.
//!
//! The source also reports *provenance*: when it minted the token it returned,
//! and which mint generation it belongs to. Retry classification, logging, and
//! failure reports all read that one fact instead of threading booleans
//! through call stacks.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use tokio::sync::Mutex;

use crate::{GitHubAppCredentials, GitHubCredentials, InstallationToken};

/// How long before expiry a cached installation token stops being reused.
///
/// Must comfortably exceed the longest git operation that pins a resolved
/// token, so a token handed out just above the margin still outlives the
/// operation. GitHub App installation tokens live 60 minutes.
pub const REFRESH_MARGIN: Duration = Duration::from_mins(10);

/// Where the token a resolve returned came from.
///
/// Time metadata exists only for tokens this source minted. Static
/// credentials (a PAT, or a pre-minted installation token) carry no
/// `minted_at`, so token age is undefined for them and they are never
/// treated as freshly minted.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, strum::Display,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum TokenProvenance {
    /// This resolve minted the token.
    Minted {
        minted_at:  DateTime<Utc>,
        expires_at: DateTime<Utc>,
    },
    /// This resolve returned a token minted by an earlier resolve.
    Reused {
        minted_at:  DateTime<Utc>,
        expires_at: DateTime<Utc>,
    },
    /// A fixed credential the source cannot re-mint.
    Static,
}

/// Non-secret description of the token a resolve returned. Shared by the
/// source, refresh outcomes, logs, and events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TokenSnapshot {
    /// Increments per mint; 0 for `Static`.
    pub generation: u64,
    pub provenance: TokenProvenance,
}

impl TokenSnapshot {
    #[must_use]
    pub fn minted_at(&self) -> Option<DateTime<Utc>> {
        match self.provenance {
            TokenProvenance::Minted { minted_at, .. }
            | TokenProvenance::Reused { minted_at, .. } => Some(minted_at),
            TokenProvenance::Static => None,
        }
    }

    #[must_use]
    pub fn expires_at(&self) -> Option<DateTime<Utc>> {
        match self.provenance {
            TokenProvenance::Minted { expires_at, .. }
            | TokenProvenance::Reused { expires_at, .. } => Some(expires_at),
            TokenProvenance::Static => None,
        }
    }

    /// Age of the token at `now`. `None` for static credentials, whose age is
    /// undefined.
    #[must_use]
    pub fn age_at(&self, now: DateTime<Utc>) -> Option<Duration> {
        let minted_at = self.minted_at()?;
        Some((now - minted_at).to_std().unwrap_or(Duration::ZERO))
    }

    /// Age of the token in milliseconds, measured now.
    #[must_use]
    pub fn age_ms(&self) -> Option<u64> {
        self.age_at(Utc::now())
            .map(|age| u64::try_from(age.as_millis()).unwrap_or(u64::MAX))
    }

    #[must_use]
    pub fn is_static(&self) -> bool {
        matches!(self.provenance, TokenProvenance::Static)
    }
}

/// A token secret that never appears in `Debug` output. Call
/// [`SecretString::expose`] at the point of use (URL embedding, git
/// credentials) — never in a log line.
#[derive(Clone)]
pub struct SecretString(String);

impl SecretString {
    #[must_use]
    pub fn new(secret: String) -> Self {
        Self(secret)
    }

    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretString(<redacted>)")
    }
}

/// A token handed out by [`InstallationTokenSource::resolve`]: the secret plus
/// its non-secret snapshot. Only the snapshot may cross logging or event
/// boundaries.
#[derive(Debug, Clone)]
pub struct ResolvedToken {
    pub token:    SecretString,
    pub snapshot: TokenSnapshot,
}

/// Mints installation tokens for [`InstallationTokenSource`]. Abstracted so
/// tests can script mint results without HTTP.
#[async_trait::async_trait]
pub trait InstallationTokenMinter: Send + Sync {
    async fn mint(&self) -> anyhow::Result<InstallationToken>;
}

/// Real minter backed by GitHub App credentials.
struct AppTokenMinter {
    creds:       GitHubAppCredentials,
    http:        fabro_http::HttpClient,
    owner:       String,
    repo:        String,
    base_url:    String,
    permissions: serde_json::Value,
}

#[async_trait::async_trait]
impl InstallationTokenMinter for AppTokenMinter {
    async fn mint(&self) -> anyhow::Result<InstallationToken> {
        self.creds
            .mint_installation_token(
                &self.http,
                &self.owner,
                &self.repo,
                &self.base_url,
                self.permissions.clone(),
                None,
            )
            .await
    }
}

/// A minted token plus the metadata the cache tracks for it.
struct CachedToken {
    token:      InstallationToken,
    minted_at:  DateTime<Utc>,
    generation: u64,
}

impl CachedToken {
    fn resolved(&self, provenance: TokenProvenance) -> ResolvedToken {
        ResolvedToken {
            token:    SecretString::new(self.token.token.clone()),
            snapshot: TokenSnapshot {
                generation: self.generation,
                provenance,
            },
        }
    }
}

enum SourceState {
    /// A fixed personal access token — no expiry metadata.
    Pat(SecretString),
    /// A pre-minted installation token — fixed, rejected client-side once
    /// expired.
    Installation(InstallationToken),
    /// GitHub App credentials that mint installation tokens on demand.
    ///
    /// The async lock is held across the mint, making `resolve()`
    /// single-flight: concurrent near-expiry callers wait and receive the
    /// same generation instead of racing to mint.
    App {
        minter: Box<dyn InstallationTokenMinter>,
        cache:  Mutex<Option<CachedToken>>,
    },
}

/// Cached installation-token source for one origin repository.
///
/// Static credentials pass through unchanged. App credentials mint through
/// the shared cache: a resolve reuses the cached token until it is within
/// [`REFRESH_MARGIN`] of expiry, then mints a new generation.
pub struct InstallationTokenSource {
    /// `owner/repo`, for logs only.
    repo:  String,
    state: SourceState,
}

impl InstallationTokenSource {
    /// Build a source for `creds` against the repository in `origin_url`.
    ///
    /// `permissions` scopes minted installation tokens; static credentials
    /// pass through and ignore it.
    pub fn for_origin(
        creds: &GitHubCredentials,
        origin_url: &str,
        permissions: serde_json::Value,
    ) -> anyhow::Result<Arc<Self>> {
        let normalized = crate::normalize_repo_origin_url(origin_url);
        let (owner, repo) = crate::parse_github_owner_repo(&normalized)
            .context("parsing GitHub origin for token source")?;
        let repo_display = format!("{owner}/{repo}");
        let state = match creds {
            GitHubCredentials::Pat(token) => SourceState::Pat(SecretString::new(token.clone())),
            GitHubCredentials::Installation(token) => SourceState::Installation(token.clone()),
            GitHubCredentials::App(app) => {
                let http = fabro_http::http_client()
                    .map_err(anyhow::Error::new)
                    .context("building HTTP client for token source")?;
                SourceState::App {
                    minter: Box::new(AppTokenMinter {
                        creds: app.clone(),
                        http,
                        owner,
                        repo,
                        base_url: crate::github_api_base_url(),
                        permissions,
                    }),
                    cache:  Mutex::new(None),
                }
            }
        };
        Ok(Arc::new(Self {
            repo: repo_display,
            state,
        }))
    }

    /// Build a minting source over a custom minter. For tests.
    #[must_use]
    pub fn with_minter(repo: String, minter: Box<dyn InstallationTokenMinter>) -> Arc<Self> {
        Arc::new(Self {
            repo,
            state: SourceState::App {
                minter,
                cache: Mutex::new(None),
            },
        })
    }

    /// Whether this source can mint new tokens (GitHub App credentials).
    #[must_use]
    pub fn mints_installation_tokens(&self) -> bool {
        matches!(self.state, SourceState::App { .. })
    }

    /// Resolve a token, reusing the cached one until it nears expiry.
    pub async fn resolve(&self) -> anyhow::Result<ResolvedToken> {
        match &self.state {
            SourceState::Pat(_) | SourceState::Installation(_) => self.resolve_static(),
            SourceState::App { minter, cache } => {
                let mut cache = cache.lock().await;
                // Re-check under the lock: a waiter queued behind a minter
                // finds the fresh token here instead of minting again.
                if let Some(cached) = cache.as_ref() {
                    if !cached.token.near_expiry(REFRESH_MARGIN) {
                        let resolved = cached.resolved(TokenProvenance::Reused {
                            minted_at:  cached.minted_at,
                            expires_at: cached.token.expires_at,
                        });
                        tracing::debug!(
                            repo = %self.repo,
                            generation = cached.generation,
                            expires_at = %cached.token.expires_at,
                            "Reusing cached GitHub installation token"
                        );
                        return Ok(resolved);
                    }
                }
                self.mint_locked(minter.as_ref(), &mut cache).await
            }
        }
    }

    /// Mint a fresh token for the first repository clone and seed the cache
    /// with it.
    ///
    /// The clone deliberately never reuses a warm cache: retrying a clone with
    /// the token minted for it is the established replication-lag recovery,
    /// and reuse of older tokens for clones is a separate follow-up. Seeding
    /// makes the clone token generation 1, so later refreshes reuse it until
    /// it nears expiry.
    pub async fn mint_for_clone(&self) -> anyhow::Result<ResolvedToken> {
        match &self.state {
            SourceState::Pat(_) | SourceState::Installation(_) => self.resolve_static(),
            SourceState::App { minter, cache } => {
                let mut cache = cache.lock().await;
                self.mint_locked(minter.as_ref(), &mut cache).await
            }
        }
    }

    fn resolve_static(&self) -> anyhow::Result<ResolvedToken> {
        let secret = match &self.state {
            SourceState::Pat(token) => token.clone(),
            SourceState::Installation(token) => SecretString::new(token.valid_token()?.to_owned()),
            SourceState::App { .. } => unreachable!("resolve_static called for App credentials"),
        };
        Ok(ResolvedToken {
            token:    secret,
            snapshot: TokenSnapshot {
                generation: 0,
                provenance: TokenProvenance::Static,
            },
        })
    }

    async fn mint_locked(
        &self,
        minter: &dyn InstallationTokenMinter,
        cache: &mut Option<CachedToken>,
    ) -> anyhow::Result<ResolvedToken> {
        let token = minter
            .mint()
            .await
            .context("minting GitHub installation access token")?;
        let generation = cache.as_ref().map_or(0, |cached| cached.generation) + 1;
        let minted_at = Utc::now();
        tracing::info!(
            repo = %self.repo,
            generation,
            expires_at = %token.expires_at,
            "Minted GitHub installation token"
        );
        let cached = CachedToken {
            token,
            minted_at,
            generation,
        };
        let resolved = cached.resolved(TokenProvenance::Minted {
            minted_at,
            expires_at: cached.token.expires_at,
        });
        *cache = Some(cached);
        Ok(resolved)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use anyhow::anyhow;

    use super::*;

    enum MintAction {
        Token(&'static str, DateTime<Utc>),
        Error(&'static str),
    }

    struct MockMinter {
        calls:  AtomicUsize,
        script: Mutex<VecDeque<MintAction>>,
    }

    impl MockMinter {
        fn new(script: Vec<MintAction>) -> Self {
            Self {
                calls:  AtomicUsize::new(0),
                script: Mutex::new(script.into()),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl InstallationTokenMinter for MockMinter {
        async fn mint(&self) -> anyhow::Result<InstallationToken> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.script.lock().await.pop_front().expect("mint script") {
                MintAction::Token(token, expires_at) => Ok(InstallationToken {
                    token: token.to_string(),
                    expires_at,
                }),
                MintAction::Error(message) => Err(anyhow!(message)),
            }
        }
    }

    struct SharedMinter(Arc<MockMinter>);

    #[async_trait::async_trait]
    impl InstallationTokenMinter for SharedMinter {
        async fn mint(&self) -> anyhow::Result<InstallationToken> {
            self.0.mint().await
        }
    }

    fn mintable(script: Vec<MintAction>) -> (Arc<InstallationTokenSource>, Arc<MockMinter>) {
        let minter = Arc::new(MockMinter::new(script));
        let source = InstallationTokenSource::with_minter(
            "owner/repo".to_string(),
            Box::new(SharedMinter(Arc::clone(&minter))),
        );
        (source, minter)
    }

    #[tokio::test]
    async fn pat_resolves_as_static_generation_zero() {
        let source = InstallationTokenSource::for_origin(
            &GitHubCredentials::Pat("ghp_pat".to_string()),
            "https://github.com/owner/repo.git",
            serde_json::json!({ "contents": "write" }),
        )
        .unwrap();

        let resolved = source.resolve().await.unwrap();
        assert_eq!(resolved.token.expose(), "ghp_pat");
        assert_eq!(resolved.snapshot.generation, 0);
        assert!(resolved.snapshot.is_static());
        assert!(!source.mints_installation_tokens());
    }

    #[tokio::test]
    async fn static_installation_token_resolves_until_expiry() {
        let valid = InstallationTokenSource::for_origin(
            &GitHubCredentials::Installation(InstallationToken {
                token:      "ghs_static".to_string(),
                expires_at: Utc::now() + chrono::Duration::minutes(30),
            }),
            "https://github.com/owner/repo.git",
            serde_json::json!({}),
        )
        .unwrap();
        let resolved = valid.resolve().await.unwrap();
        assert_eq!(resolved.token.expose(), "ghs_static");
        assert!(resolved.snapshot.is_static());

        let expired = InstallationTokenSource::for_origin(
            &GitHubCredentials::Installation(InstallationToken {
                token:      "ghs_expired".to_string(),
                expires_at: Utc::now() - chrono::Duration::seconds(1),
            }),
            "https://github.com/owner/repo.git",
            serde_json::json!({}),
        )
        .unwrap();
        assert!(expired.resolve().await.is_err());
    }

    #[tokio::test]
    async fn resolve_reuses_cached_token_before_the_margin() {
        let (source, minter) = mintable(vec![MintAction::Token(
            "ghs_gen1",
            Utc::now() + chrono::Duration::minutes(30),
        )]);

        let first = source.resolve().await.unwrap();
        let second = source.resolve().await.unwrap();

        assert_eq!(minter.calls(), 1);
        assert_eq!(first.snapshot.generation, 1);
        assert_eq!(second.snapshot.generation, 1);
        assert!(matches!(
            first.snapshot.provenance,
            TokenProvenance::Minted { .. }
        ));
        assert!(matches!(
            second.snapshot.provenance,
            TokenProvenance::Reused { .. }
        ));
        assert_eq!(second.token.expose(), "ghs_gen1");
    }

    #[tokio::test]
    async fn resolve_mints_a_new_generation_inside_the_margin() {
        let (source, minter) = mintable(vec![
            // Expires inside REFRESH_MARGIN, so the second resolve re-mints.
            MintAction::Token("ghs_gen1", Utc::now() + chrono::Duration::minutes(5)),
            MintAction::Token("ghs_gen2", Utc::now() + chrono::Duration::minutes(60)),
        ]);

        let first = source.resolve().await.unwrap();
        let second = source.resolve().await.unwrap();

        assert_eq!(minter.calls(), 2);
        assert_eq!(first.snapshot.generation, 1);
        assert_eq!(second.snapshot.generation, 2);
        assert!(matches!(
            second.snapshot.provenance,
            TokenProvenance::Minted { .. }
        ));
        assert_eq!(second.token.expose(), "ghs_gen2");
    }

    #[tokio::test]
    async fn concurrent_resolves_share_one_generation() {
        // Single mint in the script: a second mint would panic on an empty
        // script, so success proves single-flight.
        let (source, minter) = mintable(vec![MintAction::Token(
            "ghs_gen1",
            Utc::now() + chrono::Duration::minutes(60),
        )]);

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let source = Arc::clone(&source);
                tokio::spawn(async move { source.resolve().await })
            })
            .collect();

        for handle in handles {
            let resolved = handle.await.unwrap().unwrap();
            assert_eq!(resolved.snapshot.generation, 1);
            assert_eq!(resolved.token.expose(), "ghs_gen1");
        }
        assert_eq!(minter.calls(), 1);
    }

    #[tokio::test]
    async fn mint_for_clone_always_mints_and_seeds_the_cache() {
        let (source, minter) = mintable(vec![MintAction::Token(
            "ghs_clone",
            Utc::now() + chrono::Duration::minutes(60),
        )]);

        let clone_token = source.mint_for_clone().await.unwrap();
        assert_eq!(clone_token.snapshot.generation, 1);
        assert!(matches!(
            clone_token.snapshot.provenance,
            TokenProvenance::Minted { .. }
        ));

        // A later resolve reuses the clone token instead of minting again.
        let refreshed = source.resolve().await.unwrap();
        assert_eq!(refreshed.snapshot.generation, 1);
        assert_eq!(refreshed.token.expose(), "ghs_clone");
        assert!(matches!(
            refreshed.snapshot.provenance,
            TokenProvenance::Reused { .. }
        ));
        assert_eq!(minter.calls(), 1);
    }

    #[tokio::test]
    async fn mint_failure_surfaces_with_context() {
        let (source, _minter) = mintable(vec![MintAction::Error("mint failed")]);

        let err = format!("{:#}", source.resolve().await.unwrap_err());
        assert!(err.contains("mint failed"), "got: {err}");
        assert!(
            err.contains("minting GitHub installation access token"),
            "got: {err}"
        );
    }

    #[test]
    fn secret_string_debug_never_prints_the_secret() {
        let secret = SecretString::new("ghs_super_secret".to_string());
        let rendered = format!("{secret:?}");
        assert!(!rendered.contains("ghs_super_secret"), "{rendered}");
    }

    #[test]
    fn snapshot_age_is_defined_only_for_minted_tokens() {
        let now = Utc::now();
        let minted = TokenSnapshot {
            generation: 3,
            provenance: TokenProvenance::Minted {
                minted_at:  now - chrono::Duration::seconds(42),
                expires_at: now + chrono::Duration::minutes(60),
            },
        };
        assert_eq!(minted.age_at(now), Some(Duration::from_secs(42)));

        let fixed = TokenSnapshot {
            generation: 0,
            provenance: TokenProvenance::Static,
        };
        assert_eq!(fixed.age_at(now), None);
        assert_eq!(fixed.expires_at(), None);
    }
}
