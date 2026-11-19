//! The gear worklist table (work#1488): ONE named spelling of "should
//! this gear fire". Before this table the answer lived in three
//! vocabularies that had already drifted — the admission gate's local
//! consts (which mirrored the scripts' `WORKFLOW` const and
//! `claim_next`'s hardcoded page), the gear scripts' own consts, and
//! the `run queue` door's `field=verdict` argv grammar — and the drift
//! was visible: the gate's develop claim-pool read answered `All`
//! where `claim_next` reads `Open`, the same pool under two filters.
//!
//! The table binds each gated gear's workflow slug to its worklist
//! spec: the run-queue reads (field, workflow, requirement, closing
//! verdict), the observation feed spec, or the claim-pool read. Its
//! consumers:
//!
//! - the admission gate ([`super::automation_admission`]) dispatches on the
//!   slug and consults the spec — it holds no gear vocabulary of its own;
//! - the orchestration doors ([`super::handler::orchestration`]) take the gear
//!   slugs from here, and the verdict-writer allowlist is the table itself:
//!   ADR-0039 decision 6 gives each field exactly one writer, and that writer
//!   is the gear whose worklist sweeps the queue;
//! - the claim door ([`super::handler::work_items`] `claim_next`) reads the
//!   claim-pool spec, so the gate's consult and the lap's own claim walk the
//!   same page and the same state filter by construction.
//!
//! Placement: the table lives in `fabro-server` beside its only
//! consumers. `fabro-types` is the shared-vocabulary crate, but the
//! spec binds `OrchestrationField`/`OrchestrationVerdict` (fabro-types)
//! with `ObservationTriageState` (fabro-store) and
//! `WorkItemStateFilter` (fabro-work) — the foundation crate cannot
//! depend on the two higher crates, and no other crate consumes the
//! table. If a second consumer crate appears, the slug/queue half can
//! move down to `fabro-types` then.
//!
//! The gear scripts (`.fabro/workflows/{merger,adjudicator,triager}/
//! scripts/*.nu`) keep their own consts for now — wiring them to this
//! table means an engine-side render seam, deliberately out of
//! work#1488's scope (scope discipline over completeness).

use fabro_store::platform_records::ObservationTriageState;
use fabro_types::{OrchestrationField, OrchestrationVerdict};
use fabro_work::WorkItemStateFilter;

pub(crate) const MERGER: &str = "merger";
pub(crate) const ADJUDICATOR: &str = "adjudicator";
pub(crate) const TRIAGER: &str = "triager";
pub(crate) const DEVELOP: &str = "develop";

/// The workflow slug the run-queue gears sweep (the scripts' `WORKFLOW`
/// const: the adjudicator's fast path and closing pass and the merger's
/// fast path all read `--workflow develop`).
pub(crate) const SWEPT_WORKFLOW: &str = "develop";

/// The observation category the triager sweeps (`--category painpoint`,
/// `.fabro/workflows/triager/scripts/sweep.nu`) — the store matches the
/// serde fragment, so the wire name rides here verbatim.
pub(crate) const PAINPOINT_CATEGORY: &str = "painpoint";

/// The claim pool spec: the bounded lane read the claim door serves
/// (`handler::work_items::claim_next` — limit 50, page 1) and the
/// admission gate's consult re-reads, so the two never disagree.
/// `Open` is the claimable reality (work#1488): the pool answers "is
/// there work a lap could claim", not "is there any item" — the gate's
/// former `All` read counted closed items as work no lap could claim.
pub(crate) const CLAIM_POOL: ClaimPoolSpec = ClaimPoolSpec {
    page:  50,
    state: WorkItemStateFilter::Open,
};

/// The develop gear's claim-pool read parameters, shared by the claim
/// door and the admission gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ClaimPoolSpec {
    /// The lane read's page size (claim_next's `limit`).
    pub page:  usize,
    /// Which item states the pool enumerates.
    pub state: WorkItemStateFilter,
}

/// One gear's worklist spec: what the admission gate consults before it
/// lets a scheduled fire through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GearWorklistSpec {
    /// The merger's worklist (ADR-0039 stage E): one unsettled
    /// anti-join read — terminal runs of the swept workflow whose
    /// latest verdict on `field` is not settled — narrowed by the
    /// settled requirement on the OTHER field
    /// (`run queue --field closable --requires merge=enabled`).
    Queue {
        workflow: &'static str,
        field:    OrchestrationField,
        requires: (OrchestrationField, OrchestrationVerdict),
    },
    /// The adjudicator's two worklists (ADR-0039 stage D): the
    /// unsettled merge queue read AND the closing pass's settled
    /// verdict-flipped read — BOTH must drain before the gate skips,
    /// so pass-one adjudication never starves beside a fresh batch of
    /// terminal runs.
    QueueAndClosing {
        workflow:    &'static str,
        queue_field: OrchestrationField,
        closing:     (OrchestrationField, OrchestrationVerdict),
    },
    /// The triager's worklist (ADR-0044): the observation feed's
    /// untriaged count of one category — the count, not the page, is
    /// the worklist.
    Feed {
        category: &'static str,
        triage:   ObservationTriageState,
    },
    /// The develop gear's claim pool (ADR-0032): every declared lane
    /// must drain of claimable items before the gate skips.
    ClaimPool(ClaimPoolSpec),
}

impl GearWorklistSpec {
    /// The run-queue field this gear's fast path sweeps, when its
    /// worklist is a run queue — the orchestration field the gear owns
    /// on the write doors (ADR-0039 decision 6: one writer per field,
    /// and the writer is the gear that sweeps the queue). `None` for
    /// gears whose worklist is not a run queue.
    pub(crate) fn queue_field(&self) -> Option<OrchestrationField> {
        match self {
            Self::Queue { field, .. } => Some(*field),
            Self::QueueAndClosing { queue_field, .. } => Some(*queue_field),
            Self::Feed { .. } | Self::ClaimPool(_) => None,
        }
    }
}

/// One gated gear: its workflow slug and the worklist the gate consults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GearWorklist {
    /// The gear's workflow slug — the scheduler's `workflow` name, the
    /// admission gate's dispatch key, the orchestration doors' writer
    /// allowlist entry.
    pub slug: &'static str,
    /// The worklist the admission gate consults.
    pub spec: GearWorklistSpec,
}

/// The table itself. Exactly the four worklist-driven gears of
/// denkhaus/work#1436: the watchdog is deliberately absent (its
/// heartbeat sweeps its own systems and fires ungated), and every
/// workflow outside the table fires ungated too.
pub(crate) const GEARS: &[GearWorklist] = &[
    GearWorklist {
        slug: MERGER,
        spec: GearWorklistSpec::Queue {
            workflow: SWEPT_WORKFLOW,
            field:    OrchestrationField::Closable,
            requires: (OrchestrationField::Merge, OrchestrationVerdict::Enabled),
        },
    },
    GearWorklist {
        slug: ADJUDICATOR,
        spec: GearWorklistSpec::QueueAndClosing {
            workflow:    SWEPT_WORKFLOW,
            queue_field: OrchestrationField::Merge,
            closing:     (OrchestrationField::Closable, OrchestrationVerdict::Enabled),
        },
    },
    GearWorklist {
        slug: TRIAGER,
        spec: GearWorklistSpec::Feed {
            category: PAINPOINT_CATEGORY,
            triage:   ObservationTriageState::Untriaged,
        },
    },
    GearWorklist {
        slug: DEVELOP,
        spec: GearWorklistSpec::ClaimPool(CLAIM_POOL),
    },
];

/// The gear whose workflow slug names it, `None` for every ungated
/// workflow.
pub(crate) fn by_slug(slug: &str) -> Option<&'static GearWorklist> {
    GEARS.iter().find(|gear| gear.slug == slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The verdict-writer allowlist (ADR-0039 decision 6) rides the
    /// table: each orchestration field's one writer is the gear that
    /// sweeps its queue. The orchestration doors consume this through
    /// `owned_field_for_writer` — the test pins the pairing so a table
    /// edit cannot silently move a write allowlist.
    #[test]
    fn each_orchestration_fields_writer_is_the_gear_that_sweeps_its_queue() {
        let merge_writer = GEARS
            .iter()
            .find(|gear| gear.spec.queue_field() == Some(OrchestrationField::Merge))
            .expect("exactly one gear sweeps the merge queue");
        let closable_writer = GEARS
            .iter()
            .find(|gear| gear.spec.queue_field() == Some(OrchestrationField::Closable))
            .expect("exactly one gear sweeps the closable queue");
        assert_eq!(
            merge_writer.slug, ADJUDICATOR,
            "the adjudicator owns the merge field (ADR-0039 stage D)"
        );
        assert_eq!(
            closable_writer.slug, MERGER,
            "the merger owns the closable field (ADR-0039 stage E)"
        );
    }

    /// The claim-pool parity (work#1488): the pool spec is the claimable
    /// reality — `Open`, claim_next's page — so the admission gate's
    /// consult and the lap's own claim read the same pool by
    /// construction.
    #[test]
    fn the_claim_pool_spec_is_the_claimable_reality() {
        assert_eq!(
            CLAIM_POOL,
            ClaimPoolSpec {
                page:  50,
                state: WorkItemStateFilter::Open,
            },
            "the pool reads OPEN items on the claim door's bounded page"
        );
        let develop = by_slug(DEVELOP).expect("the develop gear is in the table");
        assert_eq!(
            develop.spec,
            GearWorklistSpec::ClaimPool(CLAIM_POOL),
            "the develop entry consults the shared claim-pool spec"
        );
    }

    /// The gate's dispatch shape: the four gears resolve, the watchdog
    /// (and every other workflow) stays ungated.
    #[test]
    fn the_table_names_exactly_the_four_gated_gears() {
        for slug in [MERGER, ADJUDICATOR, TRIAGER, DEVELOP] {
            assert_eq!(by_slug(slug).expect("gated gear").slug, slug);
        }
        assert!(
            by_slug("watchdog").is_none(),
            "the watchdog is not a worklist-driven gear — it fires ungated"
        );
        assert!(by_slug("workflow.fabro").is_none());
    }
}
