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
///
/// `None` is an empty struct variant rather than a unit variant so that the
/// derived deserializer enforces `deny_unknown_fields` on `{"kind": "none"}`
/// (serde ignores sibling fields on internally tagged unit variants).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, strum::IntoStaticStr)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[strum(serialize_all = "snake_case")]
pub enum RunTarget {
    Git(GitRunTarget),
    None {},
    Folder { path: String },
}

/// A Git-backed run target.
///
/// `branch` is always the attached working branch. When present, `tag` names
/// the requested release identity. An exact `sha` is authoritative over both
/// selectors while preserving the tag in durable state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitRunTarget {
    pub repo:   String,
    pub branch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag:    Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha:    Option<String>,
}

impl RunTarget {
    /// The wire `kind` discriminator (`git`, `none`, or `folder`), for
    /// diagnostics.
    pub fn kind_name(&self) -> &'static str {
        self.into()
    }

    /// Validates and canonicalizes the target without any network resolution.
    ///
    /// Git targets include their derived operational Git projection. Targets
    /// without a repository return no projection. Folder paths require
    /// filesystem validation and canonicalization during provider admission.
    pub fn validate(self) -> Result<ValidatedRunTarget, TargetValidationError> {
        match self {
            Self::Git(GitRunTarget {
                repo,
                branch,
                tag,
                sha,
            }) => {
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
                if tag.as_deref().is_some_and(|tag| {
                    tag == "HEAD"
                        || tag.starts_with("tags/")
                        || tag.starts_with("refs/")
                        || repository::normalize_git_commit_sha(tag).is_some()
                        || !repository::is_valid_github_ref_selector(tag)
                }) {
                    return Err(TargetValidationError::Tag);
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
                Ok(ValidatedRunTarget {
                    target: Self::Git(GitRunTarget {
                        repo,
                        branch,
                        tag,
                        sha,
                    }),
                    git:    Some(git),
                })
            }
            Self::None {} => Ok(ValidatedRunTarget {
                target: Self::None {},
                git:    None,
            }),
            Self::Folder { path } => Ok(ValidatedRunTarget {
                target: Self::Folder { path },
                git:    None,
            }),
        }
    }
}

/// A [`RunTarget`] whose grammar has been validated, together with its
/// optional operational Git projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedRunTarget {
    pub target: RunTarget,
    pub git:    Option<GitContext>,
}

/// A [`RunTarget`] that failed grammar validation.
#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum TargetValidationError {
    #[error("target repository must be a valid GitHub owner/name slug")]
    Repository,
    #[error("target branch must be a non-empty branch name, not a ref or commit selector")]
    Branch,
    #[error("target tag must be a non-empty bare tag name, not a ref or commit selector")]
    Tag,
    #[error("target SHA must be exactly 40 ASCII hexadecimal characters")]
    Sha,
}
