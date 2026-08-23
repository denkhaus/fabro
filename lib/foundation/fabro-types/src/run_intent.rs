use std::collections::HashMap;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunTarget {
    Git {
        repo:   String,
        branch: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha:    Option<String>,
    },
    None,
}

// Serde's derived internally tagged unit variants accept sibling fields even
// with `deny_unknown_fields`, so deserialize through strict arm-specific maps.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum RunTargetKindWire {
    Git,
    None,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GitRunTargetWire {
    kind:   RunTargetKindWire,
    repo:   String,
    branch: String,
    #[serde(default)]
    sha:    Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoneRunTargetWire {
    kind: RunTargetKindWire,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RunTargetWire {
    Git(GitRunTargetWire),
    None(NoneRunTargetWire),
}

impl<'de> Deserialize<'de> for RunTarget {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match RunTargetWire::deserialize(deserializer)? {
            RunTargetWire::Git(wire) => match wire.kind {
                RunTargetKindWire::Git => Ok(Self::Git {
                    repo:   wire.repo,
                    branch: wire.branch,
                    sha:    wire.sha,
                }),
                RunTargetKindWire::None => {
                    Err(D::Error::custom("none target must not contain Git fields"))
                }
            },
            RunTargetWire::None(wire) => match wire.kind {
                RunTargetKindWire::None => Ok(Self::None),
                RunTargetKindWire::Git => Err(D::Error::custom(
                    "git target requires repository and branch fields",
                )),
            },
        }
    }
}

impl RunTarget {
    /// Validates and canonicalizes the target without any network resolution.
    ///
    /// Git targets include their derived operational Git projection. Targets
    /// without a repository return no projection.
    pub fn validate(self) -> Result<ValidatedRunTarget, TargetValidationError> {
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
                Ok(ValidatedRunTarget {
                    target: Self::Git { repo, branch, sha },
                    git:    Some(git),
                })
            }
            Self::None => Ok(ValidatedRunTarget {
                target: Self::None,
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
    #[error("target SHA must be exactly 40 ASCII hexadecimal characters")]
    Sha,
}
