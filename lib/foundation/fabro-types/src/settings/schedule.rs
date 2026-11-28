//! Schedule domain.
//!
//! `[schedule]` is the workflow definition's declaration about its own
//! scheduled fires. The engine's scheduler reads it before a scheduled
//! fire; nothing else acts on it. The engine knows no workflow by name
//! (ADR-0052): whether a lap is idle is this declaration, never a table
//! in the engine.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::run_projection::{OrchestrationField, OrchestrationVerdict};

/// The `[schedule]` view of a resolved workflow definition.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScheduleNamespace {
    /// The conditions under which a scheduled fire is SKIPPED: when
    /// every listed worklist is empty. An empty list means the workflow
    /// fires at every slot.
    pub when_empty: Vec<WhenEmptyCondition>,
}

/// One declared worklist of `[[schedule.when_empty]]`. The kinds are the
/// engine's generic worklists; no kind names a workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WhenEmptyCondition {
    /// Terminal runs whose latest verdict on `field` is unsettled, or —
    /// with `verdict` — exactly that verdict. `requires` narrows to runs
    /// settled on another field, `workflow` to one workflow's runs.
    RunQueue {
        field:    OrchestrationField,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        verdict:  Option<OrchestrationVerdict>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        requires: Option<VerdictRequirement>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workflow: Option<String>,
    },
    /// Observation records of `category` in the derived triage state
    /// `triage` (default `untriaged`).
    ObservationFeed {
        category: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        triage:   Option<String>,
    },
    /// Free work items of the automation's bound work project. Absent
    /// `lanes`: every lane the project declares.
    ClaimPool {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lanes: Option<Vec<String>>,
    },
    /// Open pull requests carrying `label` on the automation's git target.
    LabeledPullRequests { label: String },
}

impl WhenEmptyCondition {
    /// The stable name of this condition's worklist, for logs and skip
    /// records.
    #[must_use]
    pub fn worklist_name(&self) -> &'static str {
        match self {
            Self::RunQueue { .. } => "run_queue",
            Self::ObservationFeed { .. } => "observation_feed",
            Self::ClaimPool { .. } => "claim_pool",
            Self::LabeledPullRequests { .. } => "labeled_pull_requests",
        }
    }
}

/// A settled verdict on another field, written `<field>=<verdict>` in
/// the definition (for example `merge=enabled`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VerdictRequirement {
    pub field:   OrchestrationField,
    pub verdict: OrchestrationVerdict,
}

impl VerdictRequirement {
    const SEPARATOR: char = '=';

    /// The definition's spelling: `<field>=<verdict>`.
    #[must_use]
    pub fn to_definition_string(self) -> String {
        format!("{}{}{}", self.field, Self::SEPARATOR, self.verdict)
    }

    /// Parse the definition's spelling.
    pub fn parse(text: &str) -> Result<Self, String> {
        let (field, verdict) = text.split_once(Self::SEPARATOR).ok_or_else(|| {
            format!("a verdict requirement is written `<field>=<verdict>`, got {text:?}")
        })?;
        let field = field
            .trim()
            .parse::<OrchestrationField>()
            .map_err(|error| format!("unknown orchestration field in {text:?}: {error}"))?;
        let verdict = verdict
            .trim()
            .parse::<OrchestrationVerdict>()
            .map_err(|error| format!("unknown orchestration verdict in {text:?}: {error}"))?;
        Ok(Self { field, verdict })
    }
}

impl std::str::FromStr for VerdictRequirement {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl std::fmt::Display for VerdictRequirement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_definition_string())
    }
}

impl Serialize for VerdictRequirement {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_definition_string())
    }
}

impl<'de> Deserialize<'de> for VerdictRequirement {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_verdict_requirement_round_trips_its_definition_spelling() {
        let parsed: VerdictRequirement = serde_json::from_str("\"merge=enabled\"").expect("parses");
        assert_eq!(parsed, VerdictRequirement {
            field:   OrchestrationField::Merge,
            verdict: OrchestrationVerdict::Enabled,
        });
        assert_eq!(
            serde_json::to_string(&parsed).expect("serializes"),
            "\"merge=enabled\""
        );
    }

    #[test]
    fn a_malformed_verdict_requirement_is_refused() {
        for text in ["merge", "merge=", "=enabled", "nope=enabled", "merge=maybe"] {
            assert!(
                VerdictRequirement::parse(text).is_err(),
                "{text:?} must not parse"
            );
        }
    }

    #[test]
    fn a_condition_carries_its_kind_and_an_unknown_key_is_refused() {
        let condition: WhenEmptyCondition = serde_json::from_str(
            r#"{"kind":"run_queue","field":"closable","requires":"merge=enabled"}"#,
        )
        .expect("parses");
        assert_eq!(condition.worklist_name(), "run_queue");
        assert!(
            serde_json::from_str::<WhenEmptyCondition>(
                r#"{"kind":"run_queue","field":"closable","nope":true}"#
            )
            .is_err(),
            "an unknown key is refused"
        );
        assert!(
            serde_json::from_str::<WhenEmptyCondition>(r#"{"kind":"merger"}"#).is_err(),
            "an unknown kind is refused"
        );
    }

    #[test]
    fn a_schedule_without_conditions_fires_at_every_slot() {
        let schedule: ScheduleNamespace = serde_json::from_str("{}").expect("parses");
        assert!(schedule.when_empty.is_empty());
    }
}
