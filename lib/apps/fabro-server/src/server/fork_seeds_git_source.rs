//! The production seeds source (fabro-3488, fork decision B): a
//! [`GitRepoCache`] mirror of the line repository, refreshed via
//! `git fetch`, serving the `.seeds/` store of the configured branch.
//!
//! Fork-only file: upstream has no seeds read API; a merge cannot
//! overwrite this module (the presence pins live in
//! `fork_seeds_read_api_tests.rs`).
//!
//! Snapshot invalidation contract (operator requirement, 2026-09-24): a
//! cached snapshot must never survive a served-ref change. The cache key
//! is the served ref identity — `(branch, commit)`. Every refresh
//! re-resolves the branch tip; when either component moved, the store is
//! re-read from the new worktree before anything is served. Within the
//! refresh window the cached snapshot is served only while its key still
//! names the configured branch.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use fabro_static::EnvVars;
use fabro_types::GitHubRepositorySlug;
use fabro_types::settings::server::SeedsMirrorSettings;
use seeds::Store;
use tokio::sync::Mutex;

use super::seeds_source::{SeedsSnapshot, SeedsSource, SeedsSourceError};
use crate::git_checkout::{
    GitAuthConfig, GitCheckoutSelector, GitRepoCache, WorktreeDepth, WorktreePrepareInput,
};
use crate::interp::process_env_var;

/// How long a refreshed snapshot is served without re-fetching the mirror.
const SEEDS_MIRROR_REFRESH_WINDOW: Duration = Duration::from_mins(1);

/// The line-repo mirror backing the seeds read API.
pub(crate) struct GitSeedsSource {
    cache:          GitRepoCache,
    slug:           GitHubRepositorySlug,
    origin:         String,
    branch:         String,
    auth:           Option<GitAuthConfig>,
    refresh_window: Duration,
    state:          Mutex<Option<CachedSnapshot>>,
}

/// A snapshot plus the served-ref identity it was taken at.
struct CachedSnapshot {
    branch:       String,
    commit:       String,
    snapshot:     Arc<SeedsSnapshot>,
    refreshed_at: Instant,
}

impl GitSeedsSource {
    /// Build the source from the resolved `[server.seeds.mirror]` settings.
    ///
    /// `cache_root` is the mirror cache directory (the `cache_dir` override
    /// or the storage cache root's `seeds-repos` subdirectory). Private
    /// GitHub mirrors authenticate with the ambient `GITHUB_TOKEN`, when
    /// one is present; public and file:// origins need none.
    pub(crate) fn new(mirror: &SeedsMirrorSettings, cache_root: PathBuf) -> Self {
        let auth = process_env_var(EnvVars::GITHUB_TOKEN)
            .map(|token| GitAuthConfig::from_parts("x-access-token", &token));
        Self {
            cache: GitRepoCache::new(cache_root),
            slug: mirror_slug(&mirror.origin),
            origin: mirror.origin.clone(),
            branch: mirror.branch.clone(),
            auth,
            refresh_window: SEEDS_MIRROR_REFRESH_WINDOW,
            state: Mutex::new(None),
        }
    }

    /// Shrink the refresh window (tests observe invalidation without
    /// waiting out the production window).
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn with_refresh_window(mut self, window: Duration) -> Self {
        self.refresh_window = window;
        self
    }
}

#[async_trait::async_trait]
impl SeedsSource for GitSeedsSource {
    async fn snapshot(&self) -> Result<SeedsSnapshot, SeedsSourceError> {
        let mut cached = self.state.lock().await;
        if let Some(entry) = cached.as_ref() {
            // Serve the cache only while the served ref is unchanged AND
            // the refresh window holds; a branch move always invalidates.
            if entry.branch == self.branch && entry.refreshed_at.elapsed() < self.refresh_window {
                return Ok(shared(&entry.snapshot));
            }
        }

        // Refresh: fetch the configured branch into the mirror and check
        // out a short-lived worktree (dropped after the store is read).
        let worktree = tempfile::tempdir().map_err(|err| {
            SeedsSourceError::Unavailable(format!("creating seeds mirror worktree: {err}"))
        })?;
        let commit = self
            .cache
            .prepare_worktree(
                WorktreePrepareInput {
                    repo:         &self.slug,
                    selector:     GitCheckoutSelector::Branch(&self.branch),
                    auth:         self.auth.as_ref(),
                    worktree_dir: worktree.path(),
                    depth:        WorktreeDepth::Shallow,
                },
                &self.origin,
            )
            .await
            .map_err(|err| {
                SeedsSourceError::Unavailable(format!("refreshing seeds mirror: {err}"))
            })?;

        // The served ref did not move: keep the parsed store, renew the
        // window only.
        if cached
            .as_ref()
            .is_some_and(|entry| entry.branch == self.branch && entry.commit == commit)
        {
            let entry = cached.as_mut().expect("entry checked above");
            entry.refreshed_at = Instant::now();
            return Ok(shared(&Arc::clone(&entry.snapshot)));
        }

        // The served ref moved (branch switch or upstream commit): the
        // cached snapshot never survives; re-read the store.
        let store = Store::open(worktree.path().join(".seeds")).map_err(|err| {
            SeedsSourceError::Unavailable(format!(
                "reading .seeds store from branch {}: {err}",
                self.branch
            ))
        })?;
        let snapshot = Arc::new(SeedsSnapshot {
            store:  Arc::new(store),
            commit: Some(commit.clone()),
        });
        *cached = Some(CachedSnapshot {
            branch: self.branch.clone(),
            commit,
            snapshot: Arc::clone(&snapshot),
            refreshed_at: Instant::now(),
        });
        Ok(shared(&snapshot))
    }
}

/// A cheap share of a snapshot: the parsed store moves by `Arc`, the
/// commit string by clone.
fn shared(snapshot: &SeedsSnapshot) -> SeedsSnapshot {
    SeedsSnapshot {
        store:  Arc::clone(&snapshot.store),
        commit: snapshot.commit.clone(),
    }
}

/// The mirror's cache key: the GitHub slug for GitHub origins, a stable
/// synthesized slug for any other origin (local paths, other forges).
fn mirror_slug(origin: &str) -> GitHubRepositorySlug {
    if let Some(slug) = github_slug_from_url(origin) {
        return slug;
    }
    let mut hasher = DefaultHasher::new();
    origin.hash(&mut hasher);
    let digest = format!("mirror-{:016x}", hasher.finish());
    GitHubRepositorySlug::try_new(&format!("seeds-mirror/{digest}"))
        .expect("synthesized seeds mirror slug should parse")
}

fn github_slug_from_url(origin: &str) -> Option<GitHubRepositorySlug> {
    let rest = origin
        .strip_prefix("https://github.com/")
        .or_else(|| origin.strip_prefix("git@github.com:"))?;
    GitHubRepositorySlug::try_new(rest.trim_end_matches(".git"))
}
