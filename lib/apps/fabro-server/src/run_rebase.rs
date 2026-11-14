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

use tokio::process::Command;

/// Why the rebase could not carry the work.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RunRebaseError {
    /// The source run branch does not exist on the origin: the source
    /// never pushed a checkpoint. Not a failure the caller surfaces —
    /// the re-drive simply starts from the fresh tip.
    #[error("the source run branch {branch} does not exist on the origin")]
    NoSourceBranch { branch: String },
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

/// Cherry-pick the source run branch's commits since `source_base` onto
/// the prepared worktree at `worktree_dir`. Fetches the branch from the
/// origin first; a branch that does not exist is `NoSourceBranch` (the
/// caller treats it as "nothing to carry").
pub(crate) async fn carry_checkpoint_work(
    worktree_dir: &Path,
    origin_url: &str,
    source_branch: &str,
    source_base: &str,
) -> Result<Result<Rebased, RunRebaseError>, RunRebaseError> {
    let fetched = git(worktree_dir, &[
        "fetch",
        "--quiet",
        origin_url,
        &format!("{source_branch}:refs/retry-latest/source"),
    ])
    .await;
    if matches!(fetched, GitOutcome::Failed) {
        return Ok(Err(RunRebaseError::NoSourceBranch {
            branch: source_branch.to_string(),
        }));
    }

    let tip = git_stdout(worktree_dir, &["rev-parse", "refs/retry-latest/source"]).await?;
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
            source_tip:  tip,
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
    let picked = git(worktree_dir, &[
        "cherry-pick",
        "--allow-empty",
        &format!("{source_base}..{tip}"),
    ])
    .await;
    match picked {
        GitOutcome::Ran => Ok(Ok(Rebased {
            commits,
            source_base: source_base.to_string(),
            source_tip: tip,
            touched,
        })),
        GitOutcome::Failed => {
            let conflicted = conflicted_files(worktree_dir).await?;
            let _ = git(worktree_dir, &["cherry-pick", "--abort"]).await;
            Ok(Err(RunRebaseError::Conflict { files: conflicted }))
        }
        GitOutcome::Io(err) => Err(err),
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
}
