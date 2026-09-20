use std::path::Path;
use std::process::Command;

use fabro_redact::DisplaySafeUrl;
use fabro_types::{DirtyStatus, GitContext};

use crate::error::{Error, Result};

/// A local checkout could not be inspected without changing it.
#[derive(Debug, thiserror::Error)]
pub enum GitObservationError {
    #[error("failed to discover the local Git repository")]
    Discover {
        #[source]
        source: git2::Error,
    },
    #[error("failed to read the local Git repository HEAD")]
    Head {
        #[source]
        source: git2::Error,
    },
    #[error("failed to read the local Git repository origin")]
    Origin {
        #[source]
        source: git2::Error,
    },
    #[error("failed to read the local Git repository status")]
    Status {
        #[source]
        source: git2::Error,
    },
}

/// Observe the current branch, commit, origin, and dirty state of a local
/// checkout without invoking Git commands, contacting a remote, or mutating
/// the repository. Non-repositories, unborn repositories, and detached HEADs
/// have no usable [`GitContext`] and return `Ok(None)`.
pub fn observe_git_context(
    path: &Path,
) -> std::result::Result<Option<GitContext>, GitObservationError> {
    let repo = match git2::Repository::discover(path) {
        Ok(repo) => repo,
        Err(source) if source.code() == git2::ErrorCode::NotFound => return Ok(None),
        Err(source) => return Err(GitObservationError::Discover { source }),
    };
    let head = match repo.head() {
        Ok(head) => head,
        Err(source)
            if matches!(
                source.code(),
                git2::ErrorCode::NotFound | git2::ErrorCode::UnbornBranch
            ) =>
        {
            return Ok(None);
        }
        Err(source) => return Err(GitObservationError::Head { source }),
    };
    if !head.is_branch() {
        return Ok(None);
    }
    let Some(branch) = head.shorthand().filter(|branch| !branch.is_empty()) else {
        return Ok(None);
    };
    let branch = branch.to_string();
    let sha = head.target().map(|oid| oid.to_string());
    drop(head);

    let origin_url = match repo.find_remote("origin") {
        Ok(remote) => remote.url().map(sanitized_origin_url).unwrap_or_default(),
        Err(source) if source.code() == git2::ErrorCode::NotFound => String::new(),
        Err(source) => return Err(GitObservationError::Origin { source }),
    };
    let mut status_options = git2::StatusOptions::new();
    status_options
        .include_untracked(true)
        .no_refresh(true)
        .update_index(false);
    let statuses = repo
        .statuses(Some(&mut status_options))
        .map_err(|source| GitObservationError::Status { source })?;
    let dirty = if statuses
        .iter()
        .any(|entry| entry.status() != git2::Status::CURRENT)
    {
        DirtyStatus::Dirty
    } else {
        DirtyStatus::Clean
    };

    Ok(Some(GitContext {
        origin_url,
        branch,
        sha,
        dirty,
    }))
}

fn sanitized_origin_url(value: &str) -> String {
    let normalized = fabro_github::normalize_repo_origin_url(value);
    let Ok(url) = DisplaySafeUrl::parse(&normalized) else {
        return String::new();
    };
    let mut url = url.without_credentials().into_owned();
    url.set_query(None);
    url.set_fragment(None);
    fabro_github::normalize_repo_origin_url(url.as_str())
}

fn git_error(msg: impl Into<String>) -> Error {
    Error::engine(msg.into())
}

/// Return a pre-configured `git` command with auto-maintenance disabled.
#[expect(
    clippy::disallowed_methods,
    reason = "This shared synchronous git helper layer is used by sync code; async callers must wrap it in spawn_blocking."
)]
fn git_cmd(dir: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.args(["-c", "maintenance.auto=0", "-c", "gc.auto=0"])
        .current_dir(dir);
    cmd
}

/// Whether the working directory is a git repo with no uncommitted changes.
fn working_tree_is_clean(repo: &Path) -> bool {
    git_cmd(repo)
        .args(["status", "--porcelain"])
        .output()
        .is_ok_and(|output| output.status.success() && output.stdout.trim_ascii().is_empty())
}

/// Return the SHA of HEAD.
pub fn head_sha(repo: &Path) -> Result<String> {
    let output = git_cmd(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|e| Error::engine_with_source("git rev-parse failed", e))?;

    if !output.status.success() {
        return Err(git_error("git rev-parse HEAD failed"));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Push a local branch to the named remote without allowing Git to prompt.
pub fn push_branch_noninteractive(repo: &Path, remote: &str, branch: &str) -> Result<()> {
    tracing::info!(
        repo_dir = %repo.display(),
        remote,
        branch,
        "Pushing branch to remote without terminal prompts"
    );
    let output = git_cmd(repo)
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(["push", remote, branch])
        .output()
        .map_err(|e| Error::engine_with_source("git push failed", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(git_error(format!("git push failed: {stderr}")));
    }
    Ok(())
}

/// Read the exact commit currently advertised for a remote branch without
/// allowing Git to prompt for credentials.
///
/// This queries the remote itself rather than trusting the checkout's local
/// remote-tracking ref, which may be stale or may have been rewritten locally.
pub fn remote_branch_sha_noninteractive(
    repo: &Path,
    remote: &str,
    branch: &str,
) -> Result<Option<String>> {
    let branch_ref = format!("refs/heads/{branch}");
    let output = git_cmd(repo)
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(["ls-remote", "--refs", remote, &branch_ref])
        .output()
        .map_err(|e| Error::engine_with_source("git ls-remote failed", e))?;
    if !output.status.success() {
        return Err(git_error("git ls-remote failed"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let mut fields = line.split_whitespace();
        let (Some(sha), Some(observed_ref), None) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if observed_ref == branch_ref {
            return Ok(Some(sha.to_owned()));
        }
    }
    Ok(None)
}

/// Returns true if the local branch has commits not yet on the remote.
/// On any git error (no remote ref, detached HEAD, etc.), returns true
/// so the caller falls back to pushing.
pub fn branch_needs_push(repo: &Path, remote: &str, branch: &str) -> bool {
    let local = git_cmd(repo)
        .args(["rev-parse", &format!("refs/heads/{branch}")])
        .output();
    let remote_ref = git_cmd(repo)
        .args(["rev-parse", &format!("refs/remotes/{remote}/{branch}")])
        .output();
    match (local, remote_ref) {
        (Ok(l), Ok(r)) if l.status.success() && r.status.success() => l.stdout != r.stdout,
        _ => true,
    }
}

/// Tri-state summary of the local repository's readiness for a workflow run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitSyncStatus {
    /// Working tree is clean and the branch is pushed to the remote.
    Synced,
    /// Working tree is clean but the branch has unpushed commits
    /// (or push status could not be verified, e.g. detached HEAD).
    Unsynced,
    /// Working tree has uncommitted changes.
    Dirty,
}

impl GitSyncStatus {
    /// Whether the working tree has no uncommitted changes.
    pub fn is_clean(&self) -> bool {
        matches!(self, Self::Synced | Self::Unsynced)
    }
}

impl std::fmt::Display for GitSyncStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Synced => write!(f, "synced"),
            Self::Unsynced => write!(f, "unsynced (unpushed commits)"),
            Self::Dirty => write!(f, "dirty (uncommitted changes)"),
        }
    }
}

/// Determine the sync status of the repository relative to a remote.
pub fn sync_status(repo: &Path, remote: &str, branch: Option<&str>) -> GitSyncStatus {
    if !working_tree_is_clean(repo) {
        return GitSyncStatus::Dirty;
    }
    match branch {
        Some(b) if !branch_needs_push(repo, remote, b) => GitSyncStatus::Synced,
        _ => GitSyncStatus::Unsynced,
    }
}

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    reason = "tests write git state fixtures to disk"
)]
mod tests {
    use std::fs;

    use super::*;

    /// Create a temporary git repo with an initial commit.
    #[expect(
        clippy::disallowed_methods,
        reason = "This synchronous test helper shells out to git while constructing fixture repositories."
    )]
    fn init_repo(dir: &Path) {
        Command::new("git")
            .args(["init"])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args([
                "-c",
                "user.name=test",
                "-c",
                "user.email=test@test",
                "commit",
                "--allow-empty",
                "-m",
                "init",
            ])
            .current_dir(dir)
            .output()
            .unwrap();
    }

    #[test]
    fn observe_git_context_is_read_only_and_reports_local_state() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let repo = git2::Repository::open(dir.path()).unwrap();
        repo.remote("origin", "git@github.com:fabro-sh/fabro.git")
            .unwrap();
        drop(repo);
        let git_dir = dir.path().join(".git");
        let head_before = fs::read(git_dir.join("HEAD")).unwrap();
        let index_before = fs::read(git_dir.join("index")).unwrap();
        let config_before = fs::read(git_dir.join("config")).unwrap();

        let observed = observe_git_context(dir.path()).unwrap().unwrap();
        assert!(!observed.branch.is_empty());
        assert_eq!(observed.origin_url, "https://github.com/fabro-sh/fabro");
        assert_eq!(observed.sha.as_deref().map(str::len), Some(40));
        assert_eq!(observed.dirty, DirtyStatus::Clean);
        assert_eq!(fs::read(git_dir.join("HEAD")).unwrap(), head_before);
        assert_eq!(fs::read(git_dir.join("index")).unwrap(), index_before);
        assert_eq!(fs::read(git_dir.join("config")).unwrap(), config_before);
        assert!(!git_dir.join("HEAD.lock").exists());
        assert!(!git_dir.join("index.lock").exists());
        assert!(!git_dir.join("config.lock").exists());

        fs::write(dir.path().join("untracked.txt"), "changed").unwrap();
        let observed = observe_git_context(dir.path()).unwrap().unwrap();
        assert_eq!(observed.dirty, DirtyStatus::Dirty);
        assert_eq!(fs::read(git_dir.join("HEAD")).unwrap(), head_before);
        assert_eq!(fs::read(git_dir.join("index")).unwrap(), index_before);
        assert_eq!(fs::read(git_dir.join("config")).unwrap(), config_before);
    }

    #[test]
    fn observe_git_context_never_persists_remote_credentials() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let repo = git2::Repository::open(dir.path()).unwrap();
        repo.remote(
            "origin",
            "http://run-user:secret@example.com/acme/widgets.git?token=secret#fragment",
        )
        .unwrap();
        drop(repo);

        let observed = observe_git_context(dir.path()).unwrap().unwrap();

        assert_eq!(observed.origin_url, "http://example.com/acme/widgets");
        assert!(!observed.origin_url.contains("secret"));
        assert!(!observed.origin_url.contains("token"));
    }

    #[test]
    fn observe_git_context_accepts_a_non_repository() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(observe_git_context(dir.path()).unwrap(), None);
    }

    #[test]
    fn observe_git_context_handles_unborn_detached_and_nested_checkouts() {
        let unborn = tempfile::tempdir().unwrap();
        git2::Repository::init(unborn.path()).unwrap();
        assert_eq!(observe_git_context(unborn.path()).unwrap(), None);

        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let nested = dir.path().join("nested");
        fs::create_dir(&nested).unwrap();
        let observed = observe_git_context(&nested).unwrap().unwrap();
        assert!(observed.origin_url.is_empty());
        assert!(!observed.branch.is_empty());

        let repo = git2::Repository::open(dir.path()).unwrap();
        let head = repo.head().unwrap().target().unwrap();
        repo.set_head_detached(head).unwrap();
        assert_eq!(observe_git_context(dir.path()).unwrap(), None);
    }

    #[test]
    fn sync_status_is_dirty_with_uncommitted_changes() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        assert_ne!(
            sync_status(dir.path(), "origin", None),
            GitSyncStatus::Dirty
        );
        fs::write(dir.path().join("dirty.txt"), "hello").unwrap();
        assert_eq!(
            sync_status(dir.path(), "origin", None),
            GitSyncStatus::Dirty
        );
    }

    #[test]
    fn sync_status_is_dirty_on_non_repo() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            sync_status(dir.path(), "origin", None),
            GitSyncStatus::Dirty
        );
    }

    #[test]
    fn head_sha_returns_40_char_hex() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let sha = head_sha(dir.path()).unwrap();
        assert_eq!(sha.len(), 40);
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn push_branch_noninteractive_fails_for_nonexistent_remote() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let result = push_branch_noninteractive(dir.path(), "nonexistent", "main");
        assert!(result.is_err());
    }

    #[test]
    fn branch_needs_push_when_no_remote_ref() {
        let dir = tempfile::tempdir().unwrap();
        let repo_dir = dir.path();

        init_repo(repo_dir);

        // No remote at all — should return true (safe default)
        assert!(branch_needs_push(repo_dir, "origin", "main"));
    }
}
