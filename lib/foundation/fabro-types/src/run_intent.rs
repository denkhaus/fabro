use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{DirtyStatus, GitContext, GitHubRepositorySlug, RunId, WorkflowVersionId, repository};

/// A request to create a run from an immutable workflow version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunIntent {
    pub workflow_version_id: WorkflowVersionId,
    pub target:              RunTarget,
    pub args:                RunIntentArgs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_id:      Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id:           Option<RunId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title:               Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal:                Option<String>,
}

/// Structured run-setting overrides accepted by [`RunIntent`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunIntentArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model:    Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub inputs:   HashMap<String, Value>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub labels:   HashMap<String, String>,
}

/// Requested workspace content, independent of sandbox placement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunTarget {
    Git {
        repo:   String,
        branch: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha:    Option<String>,
    },
}

impl RunTarget {
    /// Validates the target's grammar without any network resolution and
    /// derives its operational Git projection.
    ///
    /// This is the single owner of the Git-target grammar: admission uses it
    /// to reject invalid targets, and sandbox start re-derives the clone
    /// source from the persisted target through the same rules.
    pub fn validate(self) -> Result<ValidatedGitTarget, TargetValidationError> {
        match self {
            Self::Git { repo, branch, sha } => {
                let slug = GitHubRepositorySlug::try_new(&repo)
                    .ok_or(TargetValidationError::Repository)?;
                // The selector grammar is checked on the bare branch name so
                // its leading-character rules apply to the branch itself, not
                // to a `heads/`-prefixed selector that would mask them.
                if branch == "HEAD"
                    || branch.starts_with("heads/")
                    || branch.starts_with("tags/")
                    || branch.starts_with("refs/")
                    || repository::normalize_git_commit_sha(&branch).is_some()
                    || !repository::is_valid_github_ref_selector(&branch)
                {
                    return Err(TargetValidationError::Branch);
                }
                let sha = sha
                    .map(|sha| {
                        repository::normalize_git_commit_sha(&sha).ok_or(TargetValidationError::Sha)
                    })
                    .transpose()?;
                let git = GitContext {
                    origin_url: slug.https_url(),
                    branch:     branch.clone(),
                    sha:        sha.clone(),
                    dirty:      DirtyStatus::Clean,
                };
                Ok(ValidatedGitTarget {
                    target: Self::Git { repo, branch, sha },
                    git,
                })
            }
        }
    }
}

/// A [`RunTarget`] whose grammar has been validated, together with the
/// operational Git projection derived from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedGitTarget {
    pub target: RunTarget,
    pub git:    GitContext,
}

/// A [`RunTarget`] that failed grammar validation.
#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum TargetValidationError {
    #[error("target repository must be a valid GitHub owner/name slug")]
    Repository,
    #[error("target branch must be a non-empty branch name, not a ref or commit selector")]
    Branch,
    #[error("target SHA must be exactly 40 ASCII hexadecimal characters")]
    Sha,
}
