//! Which engine runs a workflow, and what Petri admitted for a run.
//!
//! A run goes to Petri when its workflow version says so (`engine = "petri"`
//! in the `[workflow]` table of `workflow.toml`) or when the server's
//! `[server.execution] engine` default says so. The choice is recorded on
//! the run's spec as [`RunEngine`], so every later reader (the executor, the
//! projection, the API) sees the same answer without re-reading settings.
//!
//! A Petri run carries the graph Petri lowered and admitted at create time:
//! [`PetriAdmission`] names the root graph and its pre-lowered children by
//! blob and digest. The run executes and resumes from that graph, never from
//! a fresh lowering, so admission-time decisions such as the pinned model
//! routes hold for the run's whole life.

use serde::{Deserialize, Serialize};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

use crate::BlobHash;

/// The engine a workflow version or a server names.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    IntoStaticStr,
    VariantArray,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum Engine {
    /// Fabro's own executor in `fabro-workflow`.
    #[default]
    Legacy,
    /// The Petri workflow engine, reached through `fabro-petri`.
    Petri,
}

/// One lowered graph in the blob store: its bytes by hash, and Petri's own
/// content digest of it, which is how a nested-workflow step names its child.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PetriGraphRef {
    pub blob:   BlobHash,
    pub digest: String,
}

/// What Petri admitted for a run at create time: the lowered root graph and
/// the pre-lowered child graphs, every one persisted before the run exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PetriAdmission {
    pub graph:    PetriGraphRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<PetriGraphRef>,
}

/// The engine a run was created for, with what that engine admitted.
///
/// Defaults to the legacy executor when absent, so specs serialized before
/// the field existed still decode.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum RunEngine {
    #[default]
    Legacy,
    Petri(PetriAdmission),
}

impl RunEngine {
    #[must_use]
    pub fn engine(&self) -> Engine {
        match self {
            Self::Legacy => Engine::Legacy,
            Self::Petri(_) => Engine::Petri,
        }
    }

    #[must_use]
    pub fn is_legacy(&self) -> bool {
        matches!(self, Self::Legacy)
    }

    #[must_use]
    pub fn is_petri(&self) -> bool {
        matches!(self, Self::Petri(_))
    }

    /// What Petri admitted, for a Petri run.
    #[must_use]
    pub fn petri(&self) -> Option<&PetriAdmission> {
        match self {
            Self::Legacy => None,
            Self::Petri(admission) => Some(admission),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_names_are_lowercase_in_both_directions() {
        assert_eq!(Engine::Petri.to_string(), "petri");
        assert_eq!("petri".parse::<Engine>(), Ok(Engine::Petri));
        assert_eq!("legacy".parse::<Engine>(), Ok(Engine::Legacy));
        assert_eq!(
            serde_json::to_value(Engine::Petri).expect("engine serializes"),
            serde_json::json!("petri")
        );
        assert_eq!(Engine::default(), Engine::Legacy);
    }

    #[test]
    fn run_engine_defaults_to_legacy_and_tags_petri() {
        assert_eq!(RunEngine::default(), RunEngine::Legacy);
        let petri = RunEngine::Petri(PetriAdmission {
            graph:    PetriGraphRef {
                blob:   BlobHash::new(b"graph"),
                digest: "abc".to_string(),
            },
            children: Vec::new(),
        });
        let value = serde_json::to_value(&petri).expect("run engine serializes");
        assert_eq!(value["kind"], "petri");
        assert!(value.get("children").is_none());
        let decoded: RunEngine = serde_json::from_value(value).expect("run engine decodes");
        assert_eq!(decoded, petri);
        assert_eq!(decoded.engine(), Engine::Petri);
        assert!(decoded.petri().is_some());
    }
}
