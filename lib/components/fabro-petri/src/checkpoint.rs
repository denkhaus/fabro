//! Git snapshots of a Petri run's workspaces: the checkpoint commit and
//! what recovery does with it.
//!
//! A Fabro stage's files are committed on the run branch of its workspace
//! before the stage's finish is recorded, so a durable finish implies a
//! durable snapshot (the integration plan's F3.1). The commit message
//! carries the snapshot's identity as trailers, the run key, execution,
//! firing and attempt, so a restart reconciles a missing platform record
//! from the branch alone.
//!
//! # Where the workspace is
//!
//! Petri's host backend keeps a scope's workspace under the run directory
//! at `scopes/<workspace id>/work`, the layout `HostExecutor::workspace_for`
//! names. This module reaches it there and runs `git` on the host, which
//! is where the worker, and the server at recovery, run. A Docker or
//! Daytona workspace lives inside its sandbox, out of reach of this module:
//! the hooks record that no snapshot was taken and recovery resumes such a
//! run on the retained sandbox as it was left.
//!
//! # The snapshot repository
//!
//! Every checkpoint commit is also pushed to a bare repository beside the
//! run's workspaces, `snapshots/<workspace id>.git`, under an immutable ref
//! per checkpoint (`refs/checkpoints/<execution>/<firing>/<attempt>`). A
//! workspace that is gone at recovery is restored from it, and the refs
//! are what recovery reconciles a missing record from.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use fabro_checkpoint::author::GitAuthor;
use fabro_checkpoint::trailer::{self, Trailer};
use fabro_store::platform_records::{DecisionRef, OperationKey};
use fabro_types::settings::run::RunCheckpointSettings;
use tokio::process::Command;
use tokio::{fs, time};

/// The failure class of a stage whose checkpoint commit failed: fatal to
/// the run, and terminal for a restart.
pub const CHECKPOINT_FAILED_CLASS: &str = "checkpoint_failed";

/// The effect kind of a checkpoint in its operation identity.
pub const CHECKPOINT_EFFECT: &str = "checkpoint";

pub const RUN_TRAILER: &str = "Fabro-Run";
pub const EXECUTION_TRAILER: &str = "Fabro-Execution";
pub const FIRING_TRAILER: &str = "Fabro-Firing";
pub const ATTEMPT_TRAILER: &str = "Fabro-Attempt";

const FOOTER: &str = "\u{2692}\u{fe0f} Generated with [Fabro](https://fabro.sh)";
const REFS_PREFIX: &str = "refs/checkpoints/";

/// Directories never committed, the legacy executor's list: build output
/// and dependency caches a stage regenerates.
pub const EXCLUDE_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    ".pnpm-store",
    ".npm",
    "target",
    ".next",
    "__pycache__",
    ".venv",
    "venv",
    ".cache",
    ".tox",
    ".pytest_cache",
];

/// The identity of one snapshot: the attempt whose files it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CheckpointKey {
    pub execution: u64,
    pub firing:    u64,
    pub attempt:   u32,
}

impl CheckpointKey {
    /// The immutable ref the snapshot is published under.
    #[must_use]
    pub fn snapshot_ref(self) -> String {
        format!(
            "{REFS_PREFIX}{}/{}/{}",
            self.execution, self.firing, self.attempt
        )
    }

    /// The operation identity of the checkpoint effect: the attempt's
    /// decision in its execution, effect kind `checkpoint`.
    #[must_use]
    pub fn operation(self) -> OperationKey {
        OperationKey {
            execution: self.execution,
            decision:  DecisionRef::AttemptStart {
                firing:  self.firing,
                attempt: self.attempt,
            },
            effect:    CHECKPOINT_EFFECT.to_string(),
        }
    }

    /// The key an operation identity names, when it is a checkpoint's.
    #[must_use]
    pub fn from_operation(operation: &OperationKey) -> Option<Self> {
        match operation.decision {
            DecisionRef::AttemptStart { firing, attempt }
                if operation.effect == CHECKPOINT_EFFECT =>
            {
                Some(Self {
                    execution: operation.execution,
                    firing,
                    attempt,
                })
            }
            DecisionRef::AttemptStart { .. }
            | DecisionRef::ExecutionStart
            | DecisionRef::Route { .. } => None,
        }
    }

    fn from_ref(name: &str) -> Option<Self> {
        let mut parts = name.strip_prefix(REFS_PREFIX)?.split('/');
        let execution = parts.next()?.parse().ok()?;
        let firing = parts.next()?.parse().ok()?;
        let attempt = parts.next()?.parse().ok()?;
        parts.next().is_none().then_some(Self {
            execution,
            firing,
            attempt,
        })
    }

    /// The key a checkpoint commit's message carries in its trailers.
    #[must_use]
    pub fn from_message(message: &str) -> Option<Self> {
        Some(Self {
            execution: trailer::parse(message, EXECUTION_TRAILER)?.parse().ok()?,
            firing:    trailer::parse(message, FIRING_TRAILER)?.parse().ok()?,
            attempt:   trailer::parse(message, ATTEMPT_TRAILER)?.parse().ok()?,
        })
    }
}

/// Why a snapshot could not be taken, found or restored.
#[derive(Debug, thiserror::Error)]
pub enum CheckpointError {
    #[error("the workspace `{workspace}` does not exist at {}", path.display())]
    WorkspaceMissing {
        workspace: String,
        path:      PathBuf,
    },
    #[error("git {action} failed ({status}): {detail}")]
    Command {
        action: String,
        status: String,
        detail: String,
    },
    #[error("git {action} could not run")]
    Spawn {
        action: String,
        #[source]
        source: std::io::Error,
    },
    #[error("git {action} did not finish within {timeout:?}")]
    TimedOut { action: String, timeout: Duration },
    #[error("the workspace could not be prepared at {}", path.display())]
    Io {
        path:   PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("the restored workspace is at {actual}, not the snapshot {expected}")]
    RestoreMismatch { expected: String, actual: String },
}

/// A checkpoint commit: the commit, and whether an earlier attempt of the
/// same operation had already made it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub sha:    String,
    pub reused: bool,
}

/// One published snapshot of a workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishedSnapshot {
    pub key: CheckpointKey,
    pub sha: String,
}

/// The workspaces of one run on this host, and the Git operations Fabro
/// performs on them.
#[derive(Clone, Debug)]
pub struct RunWorkspaces {
    run_dir:       PathBuf,
    run_id:        String,
    author:        GitAuthor,
    exclude_globs: Vec<String>,
    timeout:       Duration,
}

impl RunWorkspaces {
    #[must_use]
    pub fn new(
        run_dir: PathBuf,
        run_id: String,
        author: GitAuthor,
        settings: &RunCheckpointSettings,
    ) -> Self {
        Self {
            run_dir,
            run_id,
            author,
            exclude_globs: settings.exclude_globs.clone(),
            timeout: Duration::from_millis(settings.commit_timeout_ms.max(1)),
        }
    }

    /// The run branch every workspace of the run commits on.
    #[must_use]
    pub fn run_branch(&self) -> String {
        format!("fabro/run/{}", self.run_id)
    }

    /// Where the host backend keeps the workspace: `scopes/<id>/work` under
    /// the run directory.
    #[must_use]
    pub fn workspace_path(&self, workspace: &str) -> PathBuf {
        self.run_dir.join("scopes").join(workspace).join("work")
    }

    /// The bare repository the workspace's snapshots are published to.
    #[must_use]
    pub fn snapshot_repository(&self, workspace: &str) -> PathBuf {
        self.run_dir
            .join("snapshots")
            .join(format!("{workspace}.git"))
    }

    /// Whether the workspace exists on this host.
    pub async fn workspace_exists(&self, workspace: &str) -> bool {
        fs::try_exists(self.workspace_path(workspace))
            .await
            .unwrap_or(false)
    }

    /// Commit the workspace's files on the run branch as the snapshot of
    /// `key`, and publish it. An earlier commit of the same key that the
    /// workspace still sits on, unchanged, is reused.
    pub async fn commit(
        &self,
        workspace: &str,
        key: CheckpointKey,
        node: &str,
        status: &str,
    ) -> Result<Snapshot, CheckpointError> {
        let path = self.workspace_path(workspace);
        if !self.workspace_exists(workspace).await {
            return Err(CheckpointError::WorkspaceMissing {
                workspace: workspace.to_string(),
                path,
            });
        }
        self.ensure_repository(&path).await?;
        if let Some(existing) = self.published_sha(workspace, key).await? {
            if self.head(&path).await?.as_deref() == Some(existing.as_str())
                && self.is_clean(&path).await?
            {
                return Ok(Snapshot {
                    sha:    existing,
                    reused: true,
                });
            }
        }
        let mut add = vec![
            "add".to_string(),
            "-A".to_string(),
            "--".to_string(),
            ".".to_string(),
        ];
        add.extend(
            EXCLUDE_DIRS
                .iter()
                .map(|dir| format!(":(glob,exclude)**/{dir}/**")),
        );
        add.extend(
            self.exclude_globs
                .iter()
                .map(|glob| format!(":(glob,exclude){glob}")),
        );
        self.git(&path, "add", &add).await?;
        let message = self.message(key, node, status);
        let user_name = format!("user.name={}", self.author.name);
        let user_email = format!("user.email={}", self.author.email);
        self.git(&path, "commit", &[
            "-c",
            &user_name,
            "-c",
            &user_email,
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            &message,
        ])
        .await?;
        let sha = self.git(&path, "rev-parse", &["rev-parse", "HEAD"]).await?;
        self.publish(workspace, &path, key, &sha).await?;
        Ok(Snapshot { sha, reused: false })
    }

    /// The commit of `key`, from the snapshot repository first, else from
    /// the workspace's own history by the trailers.
    pub async fn find(
        &self,
        workspace: &str,
        key: CheckpointKey,
    ) -> Result<Option<String>, CheckpointError> {
        if let Some(sha) = self.published_sha(workspace, key).await? {
            return Ok(Some(sha));
        }
        let path = self.workspace_path(workspace);
        if !self.workspace_exists(workspace).await || self.head(&path).await?.is_none() {
            return Ok(None);
        }
        let listed = self
            .git(&path, "log", &[
                "log",
                "--format=%H",
                "--extended-regexp",
                &format!("--grep=^{EXECUTION_TRAILER}: {}$", key.execution),
                &format!("--grep=^{FIRING_TRAILER}: {}$", key.firing),
                &format!("--grep=^{ATTEMPT_TRAILER}: {}$", key.attempt),
                "--all-match",
                "HEAD",
            ])
            .await?;
        Ok(listed.lines().next().map(str::to_owned))
    }

    /// Every snapshot published for the workspace.
    pub async fn published(
        &self,
        workspace: &str,
    ) -> Result<Vec<PublishedSnapshot>, CheckpointError> {
        let repository = self.snapshot_repository(workspace);
        if !fs::try_exists(&repository).await.unwrap_or(false) {
            return Ok(Vec::new());
        }
        let listed = self
            .git(&repository, "for-each-ref", &[
                "for-each-ref",
                "--format=%(refname) %(objectname)",
                REFS_PREFIX,
            ])
            .await?;
        Ok(listed
            .lines()
            .filter_map(|line| {
                let (name, sha) = line.split_once(' ')?;
                Some(PublishedSnapshot {
                    key: CheckpointKey::from_ref(name)?,
                    sha: sha.to_string(),
                })
            })
            .collect())
    }

    /// Whether `ancestor` is reachable from `descendant` in the workspace's
    /// published history.
    pub async fn is_ancestor(
        &self,
        workspace: &str,
        ancestor: &str,
        descendant: &str,
    ) -> Result<bool, CheckpointError> {
        let repository = self.snapshot_repository(workspace);
        Ok(self
            .git_status(&repository, "merge-base", &[
                "merge-base",
                "--is-ancestor",
                ancestor,
                descendant,
            ])
            .await?
            .is_some())
    }

    /// The workspace's `HEAD`, or `None` when it has no commit.
    pub async fn workspace_head(&self, workspace: &str) -> Result<Option<String>, CheckpointError> {
        let path = self.workspace_path(workspace);
        self.head(&path).await
    }

    /// Whether the workspace sits on `sha` with nothing changed since.
    pub async fn matches(&self, workspace: &str, sha: &str) -> Result<bool, CheckpointError> {
        let path = self.workspace_path(workspace);
        Ok(self.head(&path).await?.as_deref() == Some(sha) && self.is_clean(&path).await?)
    }

    /// Bring the workspace back to `sha`: tracked files reset, untracked
    /// files removed, the excluded caches left alone.
    pub async fn reset(&self, workspace: &str, sha: &str) -> Result<(), CheckpointError> {
        let path = self.workspace_path(workspace);
        self.git(&path, "reset", &["reset", "-q", "--hard", sha])
            .await?;
        let mut clean = vec!["clean".to_string(), "-fdq".to_string()];
        for dir in EXCLUDE_DIRS {
            clean.push("-e".to_string());
            clean.push((*dir).to_string());
        }
        for glob in &self.exclude_globs {
            clean.push("-e".to_string());
            clean.push(glob.clone());
        }
        self.git(&path, "clean", &clean).await?;
        Ok(())
    }

    /// Recreate a gone workspace from the published snapshot `key`, at
    /// `sha`, on the run branch.
    pub async fn restore(
        &self,
        workspace: &str,
        key: CheckpointKey,
        sha: &str,
    ) -> Result<(), CheckpointError> {
        let path = self.workspace_path(workspace);
        fs::create_dir_all(&path)
            .await
            .map_err(|source| CheckpointError::Io {
                path: path.clone(),
                source,
            })?;
        self.git(&path, "init", &["init", "-q"]).await?;
        let repository = self.snapshot_repository(workspace);
        let repository = repository.to_string_lossy().into_owned();
        self.git(&path, "fetch", &[
            "fetch",
            "-q",
            &repository,
            &key.snapshot_ref(),
        ])
        .await?;
        let branch = self.run_branch();
        self.git(&path, "checkout", &[
            "checkout",
            "-q",
            "-B",
            &branch,
            "FETCH_HEAD",
        ])
        .await?;
        let actual = self.git(&path, "rev-parse", &["rev-parse", "HEAD"]).await?;
        if actual != sha {
            return Err(CheckpointError::RestoreMismatch {
                expected: sha.to_string(),
                actual,
            });
        }
        Ok(())
    }

    /// The commit message: Fabro's subject, the footer, and the identity
    /// trailers last, so `git interpret-trailers` and
    /// [`CheckpointKey::from_message`] both read them.
    fn message(&self, key: CheckpointKey, node: &str, status: &str) -> String {
        let subject = format!("fabro({}): {node} ({status})", self.run_id);
        let execution = key.execution.to_string();
        let firing = key.firing.to_string();
        let attempt = key.attempt.to_string();
        let mut trailers = vec![
            Trailer {
                key:   RUN_TRAILER,
                value: &self.run_id,
            },
            Trailer {
                key:   EXECUTION_TRAILER,
                value: &execution,
            },
            Trailer {
                key:   FIRING_TRAILER,
                value: &firing,
            },
            Trailer {
                key:   ATTEMPT_TRAILER,
                value: &attempt,
            },
        ];
        let defaults = GitAuthor::default();
        let co_author = format!("{} <{}>", defaults.name, defaults.email);
        if !self.author.is_default() {
            trailers.push(Trailer {
                key:   "Co-Authored-By",
                value: &co_author,
            });
        }
        trailer::format_message(&subject, FOOTER, &trailers)
    }

    /// A repository on the run branch, initialised when the workspace has
    /// none.
    async fn ensure_repository(&self, path: &Path) -> Result<(), CheckpointError> {
        if self
            .git_status(path, "rev-parse", &["rev-parse", "--git-dir"])
            .await?
            .is_none()
        {
            self.git(path, "init", &["init", "-q"]).await?;
        }
        let branch = self.run_branch();
        let current = self
            .git_status(path, "symbolic-ref", &[
                "symbolic-ref",
                "-q",
                "--short",
                "HEAD",
            ])
            .await?;
        if current.as_deref() != Some(branch.as_str()) {
            self.git(path, "checkout", &["checkout", "-q", "-B", &branch])
                .await?;
        }
        Ok(())
    }

    async fn publish(
        &self,
        workspace: &str,
        path: &Path,
        key: CheckpointKey,
        sha: &str,
    ) -> Result<(), CheckpointError> {
        let repository = self.snapshot_repository(workspace);
        if !fs::try_exists(&repository).await.unwrap_or(false) {
            fs::create_dir_all(&repository)
                .await
                .map_err(|source| CheckpointError::Io {
                    path: repository.clone(),
                    source,
                })?;
            self.git(&repository, "init --bare", &["init", "-q", "--bare"])
                .await?;
        }
        let refspec = format!("{sha}:{}", key.snapshot_ref());
        let repository = repository.to_string_lossy().into_owned();
        self.git(path, "push", &[
            "push",
            "-q",
            "--force",
            &repository,
            &refspec,
        ])
        .await?;
        Ok(())
    }

    async fn published_sha(
        &self,
        workspace: &str,
        key: CheckpointKey,
    ) -> Result<Option<String>, CheckpointError> {
        let repository = self.snapshot_repository(workspace);
        if !fs::try_exists(&repository).await.unwrap_or(false) {
            return Ok(None);
        }
        self.git_status(&repository, "rev-parse", &[
            "rev-parse",
            "-q",
            "--verify",
            &key.snapshot_ref(),
        ])
        .await
    }

    async fn head(&self, path: &Path) -> Result<Option<String>, CheckpointError> {
        self.git_status(path, "rev-parse", &["rev-parse", "-q", "--verify", "HEAD"])
            .await
    }

    async fn is_clean(&self, path: &Path) -> Result<bool, CheckpointError> {
        let status = self.git(path, "status", &["status", "--porcelain"]).await?;
        Ok(status.trim().is_empty())
    }

    /// Run `git` in `cwd`; a non-zero exit is the error.
    async fn git<S: AsRef<str>>(
        &self,
        cwd: &Path,
        action: &str,
        args: &[S],
    ) -> Result<String, CheckpointError> {
        let output = self.run(cwd, action, args).await?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
        } else {
            Err(CheckpointError::Command {
                action: action.to_string(),
                status: output.status.to_string(),
                detail: detail(&output.stderr),
            })
        }
    }

    /// Run `git` in `cwd`; a non-zero exit is `None`, for the queries whose
    /// answer it is (an unborn `HEAD`, a missing ref, no repository).
    async fn git_status<S: AsRef<str>>(
        &self,
        cwd: &Path,
        action: &str,
        args: &[S],
    ) -> Result<Option<String>, CheckpointError> {
        let output = self.run(cwd, action, args).await?;
        Ok(output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned()))
    }

    async fn run<S: AsRef<str>>(
        &self,
        cwd: &Path,
        action: &str,
        args: &[S],
    ) -> Result<std::process::Output, CheckpointError> {
        let mut command = Command::new("git");
        command
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "gc.auto=0",
                "-c",
                "advice.detachedHead=false",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args.iter().map(AsRef::as_ref))
            .current_dir(cwd)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        match time::timeout(self.timeout, command.output()).await {
            Ok(Ok(output)) => Ok(output),
            Ok(Err(source)) => Err(CheckpointError::Spawn {
                action: action.to_string(),
                source,
            }),
            Err(_) => Err(CheckpointError::TimedOut {
                action:  action.to_string(),
                timeout: self.timeout,
            }),
        }
    }
}

/// The tail of git's stderr for an error message: what the run's record
/// carries about the failure, bounded.
fn detail(stderr: &[u8]) -> String {
    const LIMIT: usize = 512;
    let text = String::from_utf8_lossy(stderr);
    let text = text.trim();
    if text.is_empty() {
        return "no output".to_string();
    }
    let start = text.len().saturating_sub(LIMIT);
    let start = text
        .char_indices()
        .map(|(index, _)| index)
        .find(|index| *index >= start)
        .unwrap_or(0);
    text[start..].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspaces(dir: &Path) -> RunWorkspaces {
        RunWorkspaces::new(
            dir.to_path_buf(),
            "run-1".to_string(),
            GitAuthor::default(),
            &RunCheckpointSettings::default(),
        )
    }

    #[test]
    fn a_key_round_trips_through_its_ref_and_its_operation() {
        let key = CheckpointKey {
            execution: 3,
            firing:    17,
            attempt:   2,
        };
        assert_eq!(key.snapshot_ref(), "refs/checkpoints/3/17/2");
        assert_eq!(CheckpointKey::from_ref(&key.snapshot_ref()), Some(key));
        assert_eq!(CheckpointKey::from_ref("refs/heads/main"), None);
        assert_eq!(CheckpointKey::from_operation(&key.operation()), Some(key));
        assert_eq!(
            CheckpointKey::from_operation(&OperationKey {
                execution: 3,
                decision:  DecisionRef::Route {
                    firing:  17,
                    attempt: 2,
                },
                effect:    CHECKPOINT_EFFECT.to_string(),
            }),
            None
        );
    }

    #[test]
    fn the_message_carries_the_identity_as_trailers_last() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let key = CheckpointKey {
            execution: 0,
            firing:    4,
            attempt:   1,
        };
        let message = workspaces(dir.path()).message(key, "build", "success");
        assert!(message.starts_with("fabro(run-1): build (success)\n\n"));
        assert_eq!(CheckpointKey::from_message(&message), Some(key));
        assert_eq!(trailer::parse(&message, RUN_TRAILER), Some("run-1"));
    }

    #[tokio::test]
    async fn a_commit_is_published_found_and_restored() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let workspaces = workspaces(dir.path());
        let workspace = "invocation-0-scope-0";
        let path = workspaces.workspace_path(workspace);
        fs::create_dir_all(&path).await.expect("the workspace");
        fs::write(path.join("out.txt"), "one\n")
            .await
            .expect("a file");
        let key = CheckpointKey {
            execution: 0,
            firing:    2,
            attempt:   1,
        };

        let first = workspaces
            .commit(workspace, key, "build", "success")
            .await
            .expect("the commit");
        assert!(!first.reused);
        let again = workspaces
            .commit(workspace, key, "build", "success")
            .await
            .expect("the second commit");
        assert_eq!(again, Snapshot {
            sha:    first.sha.clone(),
            reused: true,
        });
        assert_eq!(
            workspaces.find(workspace, key).await.expect("the lookup"),
            Some(first.sha.clone())
        );
        assert_eq!(
            workspaces.published(workspace).await.expect("the listing"),
            vec![PublishedSnapshot {
                key,
                sha: first.sha.clone(),
            }]
        );
        assert!(
            workspaces
                .matches(workspace, &first.sha)
                .await
                .expect("matches")
        );

        // The stage goes on, then the workspace is lost.
        fs::write(path.join("out.txt"), "two\n")
            .await
            .expect("a change");
        fs::write(path.join("scratch.txt"), "junk\n")
            .await
            .expect("an untracked file");
        assert!(
            !workspaces
                .matches(workspace, &first.sha)
                .await
                .expect("matches")
        );
        workspaces
            .reset(workspace, &first.sha)
            .await
            .expect("the reset");
        assert_eq!(
            fs::read_to_string(path.join("out.txt"))
                .await
                .expect("the file"),
            "one\n"
        );
        assert!(
            !fs::try_exists(path.join("scratch.txt"))
                .await
                .expect("exists")
        );

        fs::remove_dir_all(&path).await.expect("the workspace goes");
        assert_eq!(
            workspaces.find(workspace, key).await.expect("the lookup"),
            Some(first.sha.clone()),
            "the snapshot repository still knows the commit"
        );
        workspaces
            .restore(workspace, key, &first.sha)
            .await
            .expect("the restore");
        assert_eq!(
            fs::read_to_string(path.join("out.txt"))
                .await
                .expect("the restored file"),
            "one\n"
        );
        assert_eq!(
            workspaces
                .workspace_head(workspace)
                .await
                .expect("the head"),
            Some(first.sha)
        );
    }

    #[tokio::test]
    async fn an_unusable_repository_fails_the_commit() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let workspaces = workspaces(dir.path());
        let workspace = "invocation-0-scope-0";
        let path = workspaces.workspace_path(workspace);
        fs::create_dir_all(&path).await.expect("the workspace");
        fs::write(path.join(".git"), "garbage\n")
            .await
            .expect("a broken gitfile");
        let key = CheckpointKey {
            execution: 0,
            firing:    2,
            attempt:   1,
        };
        let error = workspaces
            .commit(workspace, key, "build", "success")
            .await
            .expect_err("the commit fails");
        assert!(matches!(error, CheckpointError::Command { .. }), "{error}");
    }

    #[test]
    fn detail_keeps_the_tail_of_long_output() {
        let long = "x".repeat(600);
        assert_eq!(detail(long.as_bytes()).len(), 512);
        assert_eq!(detail(b""), "no output");
    }
}
