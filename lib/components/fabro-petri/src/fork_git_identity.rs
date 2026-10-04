//! The run's Git identity in its sandboxes (fabro-19f9): every process a
//! run's sandbox spawns carries `GIT_AUTHOR_NAME`/`GIT_AUTHOR_EMAIL` and
//! `GIT_COMMITTER_NAME`/`GIT_COMMITTER_EMAIL` set to the identity the run
//! resolved, so a stage that commits inside the sandbox — a revisor or
//! implementer agent running `git commit` — never meets `Author identity
//! unknown` and never invents an identity of its own.
//!
//! The identity is the run's checkpoint identity ([`GitIdentity`], what
//! `HooksSpec` carries and `fabro-checkpoint` authors the engine's commits
//! with), so a stage's commit and the engine's own commits name the same
//! author.
//!
//! Why the environment rather than a repository-local `git config`: the
//! workspace's repository arrives with the run's checkout, after the scope
//! is acquired, and the engine's checkpoint commits already pass
//! `-c user.name=... -c user.email=...` on the command line. Git reads the
//! four variables with precedence over configuration, so the identity
//! reaches every commit a stage makes, in every repository the workspace
//! holds, without writing to the tree the checkpoint then commits.
//!
//! Precedence follows the stage-environment seam (fabro-6e7f): the
//! engine's value wins over a caller-supplied one, so a stage cannot
//! author a commit as somebody else. The variables also outrank the
//! `-c user.name=...` the checkpoint commits pass on the command line;
//! both carry the run's identity, so a commit names the run either way.
//!
//! This file is fork-owned (new file, `fork_` prefix): an upstream merge
//! cannot silently absorb it. The seam it feeds is the exec facet in
//! `fork_stage_env.rs`, which applies it beside the dispatched stage;
//! `runtime.rs` builds the cell from the run's hooks spec.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use fabro_types::GitIdentity;

/// The author name Git reads for a commit; the engine's value wins over a
/// caller-supplied one and over configuration.
pub const AUTHOR_NAME: &str = "GIT_AUTHOR_NAME";

/// The author email Git reads for a commit.
pub const AUTHOR_EMAIL: &str = "GIT_AUTHOR_EMAIL";

/// The committer name Git reads for a commit.
pub const COMMITTER_NAME: &str = "GIT_COMMITTER_NAME";

/// The committer email Git reads for a commit.
pub const COMMITTER_EMAIL: &str = "GIT_COMMITTER_EMAIL";

/// The run's Git identity, shared between the run's runtime (which sets it
/// once, from the run's hooks spec) and the sandbox exec facet that
/// injects it into every process a stage spawns.
///
/// The cell is one-shot: a run has one identity, fixed before any sandbox
/// connects, and a second write is ignored rather than a race to lose (a
/// lock would also make a poisoned cell read as "no identity", the very
/// degradation this seam removes).
#[derive(Debug, Default)]
pub struct RunGitIdentity {
    identity: OnceLock<GitIdentity>,
}

impl RunGitIdentity {
    /// A shared cell for one run's runtime.
    #[must_use]
    pub fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Record the run's identity: the one its checkpoints commit with. The
    /// first write wins; the runtime writes it once, before any sandbox is
    /// acquired.
    pub fn set(&self, identity: GitIdentity) {
        let _ = self.identity.set(identity);
    }

    /// The run's identity, when one is recorded.
    #[must_use]
    pub fn identity(&self) -> Option<GitIdentity> {
        self.identity.get().cloned()
    }

    /// Apply the identity to a process environment: the four variables Git
    /// reads for its author and committer, each over any value that
    /// arrived. A run without a recorded identity leaves the environment
    /// alone rather than guessing one.
    pub(crate) fn apply(&self, env: &mut BTreeMap<String, String>) {
        let Some(identity) = self.identity() else {
            return;
        };
        env.insert(AUTHOR_NAME.to_owned(), identity.name.clone());
        env.insert(AUTHOR_EMAIL.to_owned(), identity.email.clone());
        env.insert(COMMITTER_NAME.to_owned(), identity.name);
        env.insert(COMMITTER_EMAIL.to_owned(), identity.email);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> GitIdentity {
        GitIdentity {
            name:   "fabro-bot[bot]".to_string(),
            email:  "1+fabro-bot[bot]@users.noreply.github.com".to_string(),
            source: fabro_types::GitIdentitySource::GithubApp,
        }
    }

    #[test]
    fn a_run_without_an_identity_leaves_the_environment_alone() {
        let cell = RunGitIdentity::shared();
        let mut env = BTreeMap::from([(AUTHOR_NAME.to_string(), "someone".to_string())]);

        cell.apply(&mut env);

        assert_eq!(env.get(AUTHOR_NAME).map(String::as_str), Some("someone"));
        assert_eq!(cell.identity(), None);
    }

    #[test]
    fn the_runs_identity_reaches_every_git_process() {
        let cell = RunGitIdentity::shared();
        cell.set(identity());

        let mut env = BTreeMap::new();
        cell.apply(&mut env);

        assert_eq!(
            env.get(AUTHOR_NAME).map(String::as_str),
            Some("fabro-bot[bot]")
        );
        assert_eq!(
            env.get(AUTHOR_EMAIL).map(String::as_str),
            Some("1+fabro-bot[bot]@users.noreply.github.com")
        );
        assert_eq!(
            env.get(COMMITTER_NAME).map(String::as_str),
            Some("fabro-bot[bot]")
        );
        assert_eq!(
            env.get(COMMITTER_EMAIL).map(String::as_str),
            Some("1+fabro-bot[bot]@users.noreply.github.com")
        );
    }

    #[test]
    fn the_engines_identity_wins_over_a_supplied_one() {
        let cell = RunGitIdentity::shared();
        cell.set(identity());

        let mut env = BTreeMap::from([
            (AUTHOR_NAME.to_string(), "spoofed".to_string()),
            (
                COMMITTER_EMAIL.to_string(),
                "spoofed@example.test".to_string(),
            ),
        ]);
        cell.apply(&mut env);

        assert_eq!(
            env.get(AUTHOR_NAME).map(String::as_str),
            Some("fabro-bot[bot]")
        );
        assert_eq!(
            env.get(COMMITTER_EMAIL).map(String::as_str),
            Some("1+fabro-bot[bot]@users.noreply.github.com")
        );
    }
}
