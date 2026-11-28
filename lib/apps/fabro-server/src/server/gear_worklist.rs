//! The verdict-writer allowlist carrier (ADR-0039 decision 6, stage D):
//! which workflow's runs may write which orchestration field — exactly one
//! writer per field, and the writer is the workflow that sweeps that
//! field's queue. Consumed by the doors that authorize on it
//! (`handler::orchestration::owned_field_for_writer`): the verdict write
//! door, the merge door, and the resolve push door.
//!
//! DEBT, named (ADR-0052, denkhaus/work#1716 step 3): this table still keys
//! on the fabro project's workflow names, which the engine must not know.
//! It is left over from the retired gear admission gate (work#1436), which
//! dispatched on the same slugs. The successor is a declaration in the
//! workflow definition (`[run.grants]`); until it lands, every entry here
//! is named debt, never a pattern to extend.

use fabro_types::OrchestrationField;

pub(crate) const MERGER: &str = "merger";
pub(crate) const ADJUDICATOR: &str = "adjudicator";
pub(crate) const TRIAGER: &str = "triager";

/// One allowlisted workflow: its slug and the field its runs may write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GearWorklist {
    /// The workflow slug — the run summary's `workflow.slug`.
    pub slug:        &'static str,
    /// The orchestration field this workflow's runs own. `None` for a
    /// workflow that is allowlisted for a door without a verdict field
    /// (the triager writes observation triage, not a field).
    pub owned_field: Option<OrchestrationField>,
}

/// The allowlist itself.
pub(crate) const GEARS: &[GearWorklist] = &[
    GearWorklist {
        slug:        MERGER,
        owned_field: Some(OrchestrationField::Closable),
    },
    GearWorklist {
        slug:        ADJUDICATOR,
        owned_field: Some(OrchestrationField::Merge),
    },
    GearWorklist {
        slug:        TRIAGER,
        owned_field: None,
    },
];

/// The allowlisted workflow whose slug names it, `None` for every other
/// workflow.
pub(crate) fn by_slug(slug: &str) -> Option<&'static GearWorklist> {
    GEARS.iter().find(|gear| gear.slug == slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ADR-0039 decision 6: each orchestration field's one writer is the
    /// workflow that sweeps its queue. The pairing is pinned so a table
    /// edit cannot silently move a write allowlist.
    #[test]
    fn each_orchestration_fields_writer_is_the_workflow_that_sweeps_its_queue() {
        let merge_writer = GEARS
            .iter()
            .find(|gear| gear.owned_field == Some(OrchestrationField::Merge))
            .expect("exactly one workflow owns the merge field");
        let closable_writer = GEARS
            .iter()
            .find(|gear| gear.owned_field == Some(OrchestrationField::Closable))
            .expect("exactly one workflow owns the closable field");
        assert_eq!(
            merge_writer.slug, ADJUDICATOR,
            "the adjudicator owns the merge field (ADR-0039 stage D)"
        );
        assert_eq!(
            closable_writer.slug, MERGER,
            "the merger owns the closable field (ADR-0039 stage E)"
        );
    }
}
