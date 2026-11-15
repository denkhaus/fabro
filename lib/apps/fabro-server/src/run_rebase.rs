//! The checkpoint-carrying rebase for retry-on-latest (work#1320).
//!
//! A retry-on-latest successor is created at the target branch's CURRENT
//! tip (the create path's worktree preparation); the source run's
//! checkpoint work rides on top as local commits: the source run
//! branch's commits since the base the source started from, cherry-
//! picked onto the prepared worktree. Nothing is pushed — the
//! successor's own checkpoint machinery pushes its run branch at its
//! first commit.

use std::path::{Path, PathBuf};

use fabro_types::settings::run::GitAuthorSettings;
use tokio::process::Command;

/// Why the rebase could not carry the work.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RunRebaseError {
    /// The carry's source holds nothing to fetch: the run branch does
    /// not exist on the origin, or the snapshot repository does not
    /// hold the named tip. Not a failure the caller surfaces — the
    /// re-drive simply starts from the fresh tip. `origin` names the
    /// branch or repository path that came up empty.
    #[error("the carry origin {origin} holds no checkpoint work")]
    NoSourceBranch { origin: String },
    /// The cherry-pick stopped on a conflict. `files` names the
    /// unmerged paths; the successor parks-failed with this picture.
    #[error("the checkpoint work conflicts on {files:?}")]
    Conflict { files: Vec<String> },
    #[error("git {command} failed in {}", dir.display())]
    Git {
        command: String,
        dir:     PathBuf,
        #[source]
        source:  std::io::Error,
    },
    #[error("git {command} in {} said: {stderr}", dir.display())]
    GitRefused {
        command: String,
        dir:     PathBuf,
        stderr:  String,
    },
}

/// What the rebase carried.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Rebased {
    /// The commits cherry-picked onto the prepared worktree.
    pub commits:     usize,
    /// The base the source run had started from (excluded from the
    /// carry).
    pub source_base: String,
    /// The source run branch's tip that was carried.
    pub source_tip:  String,
    /// The files the carried commits touched (the conflict-free
    /// picture the caller records).
    pub touched:     Vec<String>,
}

/// The committer identity the carry's cherry-picks commit with: the
/// engine's checkpoint commits name the run's resolved author, and the
/// carry re-creates those commits on the successor's worktree — a
/// freshly prepared worktree carries no git identity of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CarryCommitter {
    pub name:  String,
    pub email: String,
}

impl CarryCommitter {
    /// The run's git author settings over the platform default — the
    /// resolution fabro-checkpoint's GitAuthor performs for the
    /// engine's own commits.
    pub(crate) fn for_run(author: Option<&GitAuthorSettings>) -> Self {
        Self {
            name:  author
                .and_then(|author| author.name.clone())
                .unwrap_or_else(|| fabro_types::GitIdentity::DEFAULT_NAME.to_string()),
            email: author
                .and_then(|author| author.email.clone())
                .unwrap_or_else(|| fabro_types::GitIdentity::DEFAULT_EMAIL.to_string()),
        }
    }
}

/// Cherry-pick the source run branch's commits since `source_base` onto
/// the prepared worktree at `worktree_dir`. Fetches the branch from the
/// origin first; a branch that does not exist is `NoSourceBranch` (the
/// caller treats it as "nothing to carry").
pub(crate) async fn carry_checkpoint_work(
    worktree_dir: &Path,
    origin_url: &str,
    source_branch: &str,
    source_base: &str,
    committer: &CarryCommitter,
) -> Result<Result<Rebased, RunRebaseError>, RunRebaseError> {
    let fetched = git(worktree_dir, &[
        "fetch",
        "--quiet",
        origin_url,
        &format!("{source_branch}:refs/retry-latest/source"),
    ])
    .await;
    match fetched {
        GitOutcome::Failed => {
            return Ok(Err(RunRebaseError::NoSourceBranch {
                origin: source_branch.to_string(),
            }));
        }
        GitOutcome::Io(err) => return Err(err),
        GitOutcome::Ran => {}
    }

    let tip = git_stdout(worktree_dir, &["rev-parse", "refs/retry-latest/source"]).await?;
    replay_range(worktree_dir, source_base, &tip, committer).await
}

/// Carry the checkpoint work out of the SOURCE's server-side snapshot
/// repository (work#1320): a failed run never pushes its run branch to
/// the origin — the checkpoints publish into the snapshot under
/// `refs/checkpoints/*` — so the retry fetches those refs and replays
/// `source_base..source_tip` onto the prepared worktree. The tip is the
/// newest checkpoint commit's sha (read from the run's checkpoint
/// records by the caller); a tip the snapshot does not hold answers
/// `NoSourceBranch`, keeping the origin form's "nothing to carry"
/// semantics.
pub(crate) async fn carry_checkpoint_work_from_snapshot(
    worktree_dir: &Path,
    snapshot_repository: &Path,
    source_tip: &str,
    source_base: &str,
    committer: &CarryCommitter,
) -> Result<Result<Rebased, RunRebaseError>, RunRebaseError> {
    let repository = snapshot_repository.display().to_string();
    let fetched = git(worktree_dir, &[
        "fetch",
        "--quiet",
        &repository,
        "+refs/checkpoints/*:refs/retry-latest/checkpoints/*",
    ])
    .await;
    match fetched {
        GitOutcome::Failed => {
            return Ok(Err(RunRebaseError::NoSourceBranch { origin: repository }));
        }
        GitOutcome::Io(err) => return Err(err),
        GitOutcome::Ran => {}
    }
    let present = git(worktree_dir, &[
        "cat-file",
        "-e",
        &format!("{source_tip}^{{commit}}"),
    ])
    .await;
    if matches!(present, GitOutcome::Failed) {
        return Ok(Err(RunRebaseError::NoSourceBranch { origin: repository }));
    }
    replay_range(worktree_dir, source_base, source_tip, committer).await
}

/// Count and cherry-pick `source_base..tip` onto the prepared worktree:
/// the shared tail of both carry sources. The picks commit as the
/// carry's committer identity (the engine's own convention for its
/// checkpoint commits): a freshly prepared worktree configures none,
/// and a bare cherry-pick dies on `Committer identity unknown`.
async fn replay_range(
    worktree_dir: &Path,
    source_base: &str,
    tip: &str,
    committer: &CarryCommitter,
) -> Result<Result<Rebased, RunRebaseError>, RunRebaseError> {
    let listed = git_stdout(worktree_dir, &[
        "rev-list",
        "--count",
        &format!("{source_base}..{tip}"),
    ])
    .await;
    let commits = match listed {
        Ok(count) => count
            .trim()
            .parse::<usize>()
            .map_err(|err| RunRebaseError::Git {
                command: format!("rev-list count parse ({count:?})"),
                dir:     worktree_dir.to_path_buf(),
                source:  std::io::Error::other(err),
            })?,
        Err(RunRebaseError::GitRefused { .. }) => {
            // The base is unknown to the worktree (a rewritten history):
            // carry the whole branch.
            git_stdout(worktree_dir, &["rev-list", "--count", &format!("..{tip}")])
                .await?
                .trim()
                .parse::<usize>()
                .map_err(|err| RunRebaseError::Git {
                    command: "rev-list count parse".to_string(),
                    dir:     worktree_dir.to_path_buf(),
                    source:  std::io::Error::other(err),
                })?
        }
        Err(other) => return Err(other),
    };
    if commits == 0 {
        return Ok(Ok(Rebased {
            commits:     0,
            source_base: source_base.to_string(),
            source_tip:  tip.to_string(),
            touched:     Vec::new(),
        }));
    }

    let touched = git_stdout_list(worktree_dir, &[
        "diff",
        "--name-only",
        &format!("{source_base}..{tip}"),
    ])
    .await?;

    // Cherry-pick the range. An empty pick (the fix already carries the
    // work) skips forward via --allow-empty; a conflict stops the pick.
    // The `-c` config outranks whatever the worktree happens to
    // configure, and gpgsign stays off — a host's signing key never
    // signs platform commits.
    let user_name = format!("user.name={}", committer.name);
    let user_email = format!("user.email={}", committer.email);
    let pick_range = format!("{source_base}..{tip}");
    let picked = run(worktree_dir, &[
        "-c",
        &user_name,
        "-c",
        &user_email,
        "-c",
        "commit.gpgsign=false",
        "cherry-pick",
        "--allow-empty",
        &pick_range,
    ])
    .await?;
    if picked.status.success() {
        Ok(Ok(Rebased {
            commits,
            source_base: source_base.to_string(),
            source_tip: tip.to_string(),
            touched,
        }))
    } else {
        let conflicted = conflicted_files(worktree_dir).await?;
        let _ = git(worktree_dir, &["cherry-pick", "--abort"]).await;
        if conflicted.is_empty() {
            // A failed pick with NO unmerged paths is not a conflict:
            // surface git's own verdict (the 2026-11-15 lab read
            // "conflicts on []" from exactly this misclassification).
            return Err(RunRebaseError::GitRefused {
                command: format!("cherry-pick {pick_range}"),
                dir:     worktree_dir.to_path_buf(),
                stderr:  String::from_utf8_lossy(&picked.stderr).trim().to_string(),
            });
        }
        Ok(Err(RunRebaseError::Conflict { files: conflicted }))
    }
}

/// The unmerged paths, for the conflict picture the park carries.
async fn conflicted_files(worktree_dir: &Path) -> Result<Vec<String>, RunRebaseError> {
    let output = run(worktree_dir, &["status", "--porcelain"]).await?;
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.starts_with("UU") || line.starts_with("AA") || line.starts_with("DU"))
        .map(|line| line[3..].trim().to_string())
        .collect())
}

enum GitOutcome {
    Ran,
    Failed,
    Io(RunRebaseError),
}

/// Run git and classify: ran (exit 0), failed (exit non-zero — often an
/// expected miss the caller interprets), or an io error.
async fn git(dir: &Path, args: &[&str]) -> GitOutcome {
    match run(dir, args).await {
        Ok(output) if output.status.success() => GitOutcome::Ran,
        Ok(_) => GitOutcome::Failed,
        Err(err) => GitOutcome::Io(err),
    }
}

async fn run(dir: &Path, args: &[&str]) -> Result<std::process::Output, RunRebaseError> {
    let command = format!("git {}", args.join(" "));
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .await
        .map_err(|source| RunRebaseError::Git {
            command,
            dir: dir.to_path_buf(),
            source,
        })
}

async fn git_stdout(dir: &Path, args: &[&str]) -> Result<String, RunRebaseError> {
    let output = run(dir, args).await?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(RunRebaseError::GitRefused {
            command: format!("git {}", args.join(" ")),
            dir:     dir.to_path_buf(),
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

async fn git_stdout_list(dir: &Path, args: &[&str]) -> Result<Vec<String>, RunRebaseError> {
    let output = run(dir, args).await?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect())
    } else {
        Err(RunRebaseError::GitRefused {
            command: format!("git {}", args.join(" ")),
            dir:     dir.to_path_buf(),
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::disallowed_methods,
        reason = "the git fixtures write and read plain files synchronously"
    )]
    use std::path::PathBuf;

    use super::*;

    /// A scratch origin plus a prepared worktree at its tip: the test
    /// fixtures build base .. work with commits on a run branch.
    struct Fixture {
        origin:   PathBuf,
        worktree: PathBuf,
        base:     String,
    }

    impl Fixture {
        async fn build(tag: &str) -> Self {
            let root = std::env::temp_dir()
                .join(format!("fabro-retry-latest-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            let origin = root.join("origin.git");
            let seed = root.join("seed");
            std::fs::create_dir_all(&seed).unwrap();
            run(&seed, &["init", "--initial-branch=main"])
                .await
                .unwrap();
            run(&seed, &["config", "user.email", "t@example.com"])
                .await
                .unwrap();
            run(&seed, &["config", "user.name", "T"]).await.unwrap();
            std::fs::write(seed.join("file.txt"), "base\n").unwrap();
            run(&seed, &["add", "."]).await.unwrap();
            run(&seed, &["commit", "-m", "base"]).await.unwrap();
            let base = git_stdout(&seed, &["rev-parse", "HEAD"]).await.unwrap();
            run(&seed, &["branch", "-m", "main"]).await.unwrap();
            run(&seed, &["clone", "--bare", ".", origin.to_str().unwrap()])
                .await
                .unwrap();

            let worktree = root.join("worktree");
            run(&root, &[
                "clone",
                origin.to_str().unwrap(),
                worktree.to_str().unwrap(),
            ])
            .await
            .unwrap();
            run(&worktree, &["config", "user.email", "t@example.com"])
                .await
                .unwrap();
            run(&worktree, &["config", "user.name", "T"]).await.unwrap();
            let _ = &root;
            Self {
                origin,
                worktree,
                base,
            }
        }

        async fn push_run_branch(&self, branch: &str, content: &str) {
            let scratch = self.origin.parent().unwrap().join("branch-scratch");
            std::fs::create_dir_all(&scratch).unwrap();
            run(scratch.parent().unwrap(), &[
                "clone",
                self.origin.to_str().unwrap(),
                scratch.to_str().unwrap(),
            ])
            .await
            .unwrap();
            run(&scratch, &["config", "user.email", "t@example.com"])
                .await
                .unwrap();
            run(&scratch, &["config", "user.name", "T"]).await.unwrap();
            run(&scratch, &["checkout", "-b", branch]).await.unwrap();
            std::fs::write(scratch.join("work.txt"), content).unwrap();
            run(&scratch, &["add", "."]).await.unwrap();
            run(&scratch, &["commit", "-m", "checkpoint work"])
                .await
                .unwrap();
            run(&scratch, &["push", "origin", branch]).await.unwrap();
            let _ = std::fs::remove_dir_all(&scratch);
        }

        /// A failed run's SNAPSHOT repository: a bare repo holding the
        /// work commit under a checkpoint ref and nothing else — the run
        /// branch never becomes a ref there. Returns the repository and
        /// the checkpoint commit's sha (the tip the records would name).
        async fn push_snapshot_checkpoint(
            &self,
            check_ref: &str,
            content: &str,
        ) -> (PathBuf, String) {
            let root = self.origin.parent().unwrap();
            let scratch = root.join("snapshot-scratch");
            let snapshot = root.join("snapshot.git");
            std::fs::create_dir_all(&scratch).unwrap();
            run(root, &[
                "clone",
                self.origin.to_str().unwrap(),
                scratch.to_str().unwrap(),
            ])
            .await
            .unwrap();
            run(&scratch, &["config", "user.email", "t@example.com"])
                .await
                .unwrap();
            run(&scratch, &["config", "user.name", "T"]).await.unwrap();
            std::fs::write(scratch.join("work.txt"), content).unwrap();
            run(&scratch, &["add", "."]).await.unwrap();
            run(&scratch, &["commit", "-m", "checkpoint work"])
                .await
                .unwrap();
            let tip = git_stdout(&scratch, &["rev-parse", "HEAD"]).await.unwrap();
            run(root, &["init", "--bare", snapshot.to_str().unwrap()])
                .await
                .unwrap();
            run(&scratch, &[
                "push",
                snapshot.to_str().unwrap(),
                &format!("HEAD:{check_ref}"),
            ])
            .await
            .unwrap();
            let _ = std::fs::remove_dir_all(&scratch);
            (snapshot, tip)
        }
    }

    #[tokio::test]
    async fn carries_the_run_branch_commits_onto_the_prepared_tip() {
        let fixture = Fixture::build("carry").await;
        fixture
            .push_run_branch("fabro/run/source", "work content\n")
            .await;
        let carried = carry_checkpoint_work(
            &fixture.worktree,
            fixture.origin.to_str().unwrap(),
            "fabro/run/source",
            &fixture.base,
            &CarryCommitter::for_run(None),
        )
        .await
        .unwrap()
        .expect("the carry succeeds");
        assert_eq!(carried.commits, 1);
        assert_eq!(carried.touched, vec!["work.txt".to_string()]);
        let content = std::fs::read_to_string(fixture.worktree.join("work.txt")).unwrap();
        assert_eq!(content, "work content\n");
        // The base file survives the carry.
        assert!(fixture.worktree.join("file.txt").exists());
    }

    #[tokio::test]
    async fn a_missing_source_branch_is_nothing_to_carry() {
        let fixture = Fixture::build("missing").await;
        let carried = carry_checkpoint_work(
            &fixture.worktree,
            fixture.origin.to_str().unwrap(),
            "fabro/run/absent",
            &fixture.base,
            &CarryCommitter::for_run(None),
        )
        .await
        .unwrap();
        assert!(matches!(
            carried,
            Err(RunRebaseError::NoSourceBranch { .. })
        ));
    }

    #[tokio::test]
    async fn a_conflicting_carry_names_the_files_and_aborts() {
        let fixture = Fixture::build("conflict").await;
        fixture
            .push_run_branch("fabro/run/source", "conflicting\n")
            .await;
        // Move the prepared tip so the same file conflicts.
        std::fs::write(fixture.worktree.join("work.txt"), "moved tip\n").unwrap();
        run(&fixture.worktree, &["add", "."]).await.unwrap();
        run(&fixture.worktree, &["commit", "-m", "tip moves the file"])
            .await
            .unwrap();
        let carried = carry_checkpoint_work(
            &fixture.worktree,
            fixture.origin.to_str().unwrap(),
            "fabro/run/source",
            &fixture.base,
            &CarryCommitter::for_run(None),
        )
        .await
        .unwrap();
        match carried {
            Err(RunRebaseError::Conflict { files }) => {
                assert_eq!(files, vec!["work.txt".to_string()]);
            }
            other => panic!("expected a conflict, got {other:?}"),
        }
        // The abort restored the worktree to the prepared tip.
        let content = std::fs::read_to_string(fixture.worktree.join("work.txt")).unwrap();
        assert_eq!(content, "moved tip\n");
    }

    #[tokio::test]
    async fn carries_the_checkpoint_refs_from_a_snapshot_repository() {
        let fixture = Fixture::build("snapshot-carry").await;
        let (snapshot, tip) = fixture
            .push_snapshot_checkpoint("refs/checkpoints/0/1/1", "snapshot work\n")
            .await;
        let committer = CarryCommitter::for_run(None);
        let carried = carry_checkpoint_work_from_snapshot(
            &fixture.worktree,
            &snapshot,
            &tip,
            &fixture.base,
            &committer,
        )
        .await
        .unwrap()
        .expect("the snapshot carry succeeds");
        assert_eq!(carried.commits, 1);
        assert_eq!(carried.source_tip, tip);
        assert_eq!(carried.touched, vec!["work.txt".to_string()]);
        let content = std::fs::read_to_string(fixture.worktree.join("work.txt")).unwrap();
        assert_eq!(content, "snapshot work\n");
    }

    #[tokio::test]
    async fn the_carry_commits_as_the_run_identity_not_the_worktrees() {
        // The prepared worktree carries no identity of its own (the
        // 2026-11-15 lab: a bare `git cherry-pick` died on `Committer
        // identity unknown`, misread as an empty conflict). The carry
        // passes the run's identity on the command line — outranking
        // whatever config the worktree happens to hold — and the
        // run's author settings win over the platform default.
        let fixture = Fixture::build("carry-identity").await;
        fixture
            .push_run_branch("fabro/run/source", "identity work\n")
            .await;
        let committer =
            CarryCommitter::for_run(Some(&fabro_types::settings::run::GitAuthorSettings {
                name:  Some("lab-bot".to_string()),
                email: Some("lab@example.com".to_string()),
            }));
        let carried = carry_checkpoint_work(
            &fixture.worktree,
            fixture.origin.to_str().unwrap(),
            "fabro/run/source",
            &fixture.base,
            &committer,
        )
        .await
        .unwrap()
        .expect("the carry succeeds");
        assert_eq!(carried.commits, 1);
        let committer = git_stdout(&fixture.worktree, &["log", "-1", "--format=%cn <%ce>"])
            .await
            .unwrap();
        assert_eq!(committer, "lab-bot <lab@example.com>");
        // The ORIGINAL authorship survives the pick.
        let author = git_stdout(&fixture.worktree, &["log", "-1", "--format=%an <%ae>"])
            .await
            .unwrap();
        assert_eq!(author, "T <t@example.com>");
    }

    #[tokio::test]
    async fn the_default_committer_is_the_platform_identity() {
        assert_eq!(CarryCommitter::for_run(None), CarryCommitter {
            name:  fabro_types::GitIdentity::DEFAULT_NAME.to_string(),
            email: fabro_types::GitIdentity::DEFAULT_EMAIL.to_string(),
        });
        assert_eq!(
            CarryCommitter::for_run(Some(&fabro_types::settings::run::GitAuthorSettings {
                name:  Some("named".to_string()),
                email: None,
            })),
            CarryCommitter {
                name:  "named".to_string(),
                email: fabro_types::GitIdentity::DEFAULT_EMAIL.to_string(),
            }
        );
    }

    #[tokio::test]
    async fn a_failed_pick_without_unmerged_paths_is_not_a_conflict() {
        // A dirty worktree file the pick would touch: git refuses with
        // "your local changes would be overwritten" — no unmerged
        // paths, no conflict picture. That failure must surface as the
        // git stderr, never as `Conflict { files: [] }` (the 2026-11-15
        // lab read exactly that phantom from a missing identity).
        let fixture = Fixture::build("phantom-conflict").await;
        fixture
            .push_run_branch("fabro/run/source", "dirty-target work\n")
            .await;
        std::fs::write(fixture.worktree.join("work.txt"), "uncommitted\n").unwrap();
        let carried = carry_checkpoint_work(
            &fixture.worktree,
            fixture.origin.to_str().unwrap(),
            "fabro/run/source",
            &fixture.base,
            &CarryCommitter::for_run(None),
        )
        .await;
        match carried {
            Err(RunRebaseError::GitRefused { stderr, .. }) => {
                assert!(
                    stderr.contains("overwritten by merge"),
                    "the refusal carries git's verdict: {stderr}"
                );
            }
            Err(RunRebaseError::Conflict { files }) => {
                panic!("a pick without unmerged paths is not a conflict: {files:?}");
            }
            other => panic!("expected a refused pick, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_snapshot_without_the_tip_answers_nothing_to_carry() {
        let fixture = Fixture::build("snapshot-miss").await;
        let (snapshot, _tip) = fixture
            .push_snapshot_checkpoint("refs/checkpoints/0/1/1", "other work\n")
            .await;
        let absent_tip = "0".repeat(40);
        let carried = carry_checkpoint_work_from_snapshot(
            &fixture.worktree,
            &snapshot,
            &absent_tip,
            &fixture.base,
            &CarryCommitter::for_run(None),
        )
        .await
        .unwrap();
        assert!(
            matches!(carried, Err(RunRebaseError::NoSourceBranch { .. })),
            "expected no source, got {carried:?}"
        );
    }
}
