//! The acceptance fold (denkhaus/work#1555): the one derived reader for
//! an item's done-state, across tracker state, claim ledger, run
//! verdicts and PR projection — the acceptance twin of the claim-hold
//! seam ([`crate::hold_reason`], denkhaus/work#1457).
//!
//! An item's completion state used to be smeared across five stores with
//! no derived reader; every surface re-derived its own slice (the
//! architecture review's B1 finding, 2026-11-21). This module carries
//! the mapping ONCE: one pure function over store rows to named
//! [`AcceptanceState`] values. NEVER STORED — every reader re-derives,
//! exactly like the triage-state precedent (work#1379).
//!
//! Inputs (all local rows; no tracker I/O, no network):
//!
//! - the item's tracker-visible open/closed state as projected locally (the
//!   view store's row; a missing row reads open — the safe side for every
//!   consumer),
//! - the item's claim history ([`WorkClaim`] rows, oldest or newest — order is
//!   irrelevant to the fold),
//! - per claim: the claim's run's latest `merge` and `closable` orchestration
//!   verdicts (absence = pending),
//! - per claim: the PR state projection of the claim's pull request (`None` =
//!   unobserved).
//!
//! Consumers: the reconciler's worklist (its refused arm reads
//! [`AcceptanceState::MergeRefused`] instead of re-deriving the verdict
//! inline), the queue view's per-item `acceptance` field, and — in the
//! follow-up carrying work#1552's task-list door — the close door's
//! precondition: refuse or explain on
//! [`AcceptanceState::ClosedUnlanded`] (item closed while a claim still
//! holds: the two-phase commit's open window) and
//! [`AcceptanceState::ClosableLanded`] (landed-but-open: the closing
//! pass's own input), never a half-commit. This change exposes the
//! fold; the close-door wiring rides that follow-up.
//!
//! The orphan grace is NOT an arm here (the hold seam's posture,
//! verbatim): a claim whose run record is gone releases through the
//! reconciler's grace window, which stays where it is. The ledger stays
//! the authority (ADR-0032 decision 2) — this module is vocabulary over
//! store rows, never a second ledger.

use fabro_types::{OrchestrationVerdict, PullRequestState};
use serde::{Deserialize, Serialize};

use crate::store::{ClaimState, WorkClaim};

/// Where an item stands in the acceptance chain. Both production
/// failure classes are named states: [`AcceptanceState::ClosedUnlanded`]
/// (closed-with-unlanded-halves, work#1463) and
/// [`AcceptanceState::ClosableLanded`] (landed-but-open, the
/// work#1533/#1469/#1535 audit finds).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum AcceptanceState {
    /// Open, and the chain holds nothing for it: never claimed, or
    /// every claim released back — the claim pool treats both alike.
    NoClaimOpenItem,
    /// Closed with no claim history at all — a plain human/triage
    /// close; the chain never entered. Healthy, and distinct from
    /// [`AcceptanceState::ClosedUnlanded`] (which asserts unlanded
    /// work).
    NoClaimClosedItem,
    /// A claim holds the item and its run carries no settled `merge`
    /// verdict yet (absence, or an open `parked` escalation — the
    /// operator's answer is the pending verdict).
    WaitingForMergeVerdict,
    /// The adjudicator refused the run: the claim releases (the
    /// reconciler's refused arm reads exactly this state) and the item
    /// returns to the pool.
    MergeRefused,
    /// The merge verdict is `enabled` — the PR is approved — and the
    /// merger has not integrated yet.
    WaitingForClosable,
    /// Integration happened (the `closable` verdict is `enabled`, or
    /// the PR is observed merged) and the item is not closed — the
    /// ordinary pre-close state, and the landed-but-open failure class
    /// when nothing closes it.
    ClosableLanded,
    /// The item CLOSED while no claim ever landed — and for a live
    /// claim, while one still holds the item. The #1463 failure class:
    /// closed-with-unlanded-halves. The claims' release reasons tell
    /// which lap failed; the fold maps, it does not judge.
    ClosedUnlanded,
    /// A claim landed AND the item closed — the healthy terminal (the
    /// ADR-0040 happy path: the PR merged, then the close landed the
    /// chain).
    LandedClosed,
}

/// One claim's rows as the fold reads them: the claim row plus the
/// run-scoped evidence the claim store does not carry. A plain-data
/// borrow — the resolver (the server layer, which owns both stores)
/// fills it; the fold never reaches for I/O.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimFold<'a> {
    pub claim:            &'a WorkClaim,
    /// The claim run's latest `merge` verdict; `None` is pending.
    pub merge_verdict:    Option<OrchestrationVerdict>,
    /// The claim run's latest `closable` verdict; `None` is pending.
    pub closable_verdict: Option<OrchestrationVerdict>,
    /// The PR state projection of the claim's pull request; `None` is
    /// unobserved by the platform.
    pub pr_state:         Option<PullRequestState>,
}

/// The acceptance fold: map one item's rows to its named
/// [`AcceptanceState`]. Pure — same rows in, same state out, every
/// reader re-derives.
///
/// Arms, in evaluation order:
///
/// 1. No claims: the chain never entered — [`AcceptanceState::NoClaimOpenItem`]
///    / [`AcceptanceState::NoClaimClosedItem`].
/// 2. A live claim (`granted`/`active`/`awaiting_merge` — the ledger's
///    uniqueness keeps at most one): the live lap owns the chain position, so a
///    landed claim from an earlier lap cannot shadow it. A closed item with a
///    live claim is the #1463 stuck window
///    ([`AcceptanceState::ClosedUnlanded`]); an open item climbs the verdict
///    ladder — integration evidence first (a `closable` `enabled` verdict or an
///    observed-merged PR outranks even a stale refusal: the work IS in the
///    target), then the refused verdict, then the approved merge, then the
///    wait.
/// 3. No live claim: the terminal history decides — landed plus closed is
///    [`AcceptanceState::LandedClosed`]; landed without closed is
///    [`AcceptanceState::ClosableLanded`] (landed-but-open); closed without
///    landed is [`AcceptanceState::ClosedUnlanded`]; open without landed
///    returns to [`AcceptanceState::NoClaimOpenItem`] (the pool).
#[must_use]
pub fn acceptance_fold(item_open: bool, claims: &[ClaimFold<'_>]) -> AcceptanceState {
    // Arm 1: the chain never entered.
    if claims.is_empty() {
        return if item_open {
            AcceptanceState::NoClaimOpenItem
        } else {
            AcceptanceState::NoClaimClosedItem
        };
    }
    let Some(live) = claims.iter().find(|row| row.claim.state.holds_item()) else {
        let landed = claims
            .iter()
            .any(|row| row.claim.state == ClaimState::Landed);
        return match (item_open, landed) {
            (true, false) => AcceptanceState::NoClaimOpenItem,
            (true, true) => AcceptanceState::ClosableLanded,
            (false, true) => AcceptanceState::LandedClosed,
            (false, false) => AcceptanceState::ClosedUnlanded,
        };
    };
    // Arm 2: the live lap owns the chain position. A closed item with a
    // live claim is the two-phase commit's open window — the item state
    // outranks any verdict.
    if !item_open {
        return AcceptanceState::ClosedUnlanded;
    }
    // Integration evidence outranks a stale refusal: a `closable`
    // enabled verdict or an observed-merged PR means the work IS in the
    // target, whatever the merge verdict once said.
    if live.closable_verdict == Some(OrchestrationVerdict::Enabled)
        || live.pr_state == Some(PullRequestState::Merged)
    {
        return AcceptanceState::ClosableLanded;
    }
    if live.merge_verdict == Some(OrchestrationVerdict::Refused) {
        return AcceptanceState::MergeRefused;
    }
    if live.merge_verdict == Some(OrchestrationVerdict::Enabled) {
        return AcceptanceState::WaitingForClosable;
    }
    // Absence and an open `parked` escalation are both an unsettled
    // verdict: the chain waits for the adjudicator (or the operator).
    AcceptanceState::WaitingForMergeVerdict
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use fabro_types::{OrchestrationVerdict, PullRequestState, RunId};

    use super::{AcceptanceState, ClaimFold, acceptance_fold};
    use crate::store::{ClaimState, WorkClaim};

    fn run_id(seed: &str) -> RunId {
        format!("01M7NWYS0000000000000000{seed}")
            .parse()
            .expect("seeded run id parses")
    }

    fn claim(state: ClaimState, run_seed: &str) -> WorkClaim {
        WorkClaim {
            claim_id: format!("claim-{run_seed}"),
            project_id: "fabro".to_string(),
            item_key: "denkhaus/work#1555".to_string(),
            run_id: run_id(run_seed),
            lane: "loop".to_string(),
            state,
            granted_at: Utc::now(),
            updated_at: Utc::now(),
            released_at: state.is_terminal().then_some(Utc::now()),
            release_reason: (state == ClaimState::Released).then(|| "released".to_string()),
            pr_url: None,
        }
    }

    fn fold(claim: &WorkClaim) -> ClaimFold<'_> {
        ClaimFold {
            claim,
            merge_verdict: None,
            closable_verdict: None,
            pr_state: None,
        }
    }

    fn fold_with(
        claim: &WorkClaim,
        merge: Option<OrchestrationVerdict>,
        closable: Option<OrchestrationVerdict>,
        pr: Option<PullRequestState>,
    ) -> ClaimFold<'_> {
        ClaimFold {
            merge_verdict: merge,
            closable_verdict: closable,
            pr_state: pr,
            ..fold(claim)
        }
    }

    fn fold_state(item_open: bool, claims: &[ClaimFold<'_>]) -> AcceptanceState {
        acceptance_fold(item_open, claims)
    }

    // ---- Arm 1: no claims — the chain never entered -----------------

    #[test]
    fn an_open_item_without_claims_is_no_claim_open_item() {
        assert_eq!(fold_state(true, &[]), AcceptanceState::NoClaimOpenItem);
    }

    #[test]
    fn a_closed_item_without_claims_is_no_claim_closed_item() {
        assert_eq!(fold_state(false, &[]), AcceptanceState::NoClaimClosedItem);
    }

    // ---- Arm 2: a live claim owns the chain position ---------------

    #[test]
    fn a_closed_item_with_a_live_claim_is_the_stuck_window() {
        // The #1463 window: the tracker closed while a claim still
        // holds — every live state reads the same failure class.
        for state in [
            ClaimState::Granted,
            ClaimState::Active,
            ClaimState::AwaitingMerge,
        ] {
            let row = claim(state, "01");
            assert_eq!(
                fold_state(false, &[fold(&row)]),
                AcceptanceState::ClosedUnlanded,
                "{state} on a closed item is the stuck window"
            );
        }
    }

    #[test]
    fn integration_evidence_outranks_a_stale_refusal() {
        let row = claim(ClaimState::AwaitingMerge, "02");
        assert_eq!(
            fold_state(true, &[fold_with(
                &row,
                Some(OrchestrationVerdict::Refused),
                Some(OrchestrationVerdict::Enabled),
                Some(PullRequestState::Open),
            )],),
            AcceptanceState::ClosableLanded,
            "the merger integrated after the refusal: the work is in the target"
        );
    }

    #[test]
    fn an_observed_merged_pr_is_integration_evidence() {
        let row = claim(ClaimState::AwaitingMerge, "03");
        assert_eq!(
            fold_state(true, &[fold_with(
                &row,
                None,
                None,
                Some(PullRequestState::Merged),
            )],),
            AcceptanceState::ClosableLanded,
            "the platform observed the merge: integrated, not closed"
        );
        assert_eq!(
            fold_state(true, &[fold_with(
                &row,
                Some(OrchestrationVerdict::Refused),
                None,
                Some(PullRequestState::Merged),
            )],),
            AcceptanceState::ClosableLanded,
            "an external merge outranks the stale refusal too"
        );
    }

    #[test]
    fn a_refused_merge_verdict_names_merge_refused() {
        let row = claim(ClaimState::Active, "04");
        assert_eq!(
            fold_state(true, &[fold_with(
                &row,
                Some(OrchestrationVerdict::Refused),
                None,
                Some(PullRequestState::Open),
            )],),
            AcceptanceState::MergeRefused,
        );
        assert_eq!(
            fold_state(true, &[fold_with(
                &row,
                Some(OrchestrationVerdict::Refused),
                None,
                None
            )]),
            AcceptanceState::MergeRefused,
            "the refusal fires without PR evidence too"
        );
    }

    #[test]
    fn an_enabled_merge_verdict_waits_for_the_merger() {
        let row = claim(ClaimState::AwaitingMerge, "05");
        assert_eq!(
            fold_state(true, &[fold_with(
                &row,
                Some(OrchestrationVerdict::Enabled),
                None,
                Some(PullRequestState::Open),
            )],),
            AcceptanceState::WaitingForClosable,
        );
        assert_eq!(
            fold_state(true, &[fold_with(
                &row,
                Some(OrchestrationVerdict::Enabled),
                Some(OrchestrationVerdict::Refused),
                Some(PullRequestState::Open),
            )],),
            AcceptanceState::WaitingForClosable,
            "a refused closable verdict is NOT integration: still waiting"
        );
    }

    #[test]
    fn pending_and_parked_merges_wait_for_the_verdict() {
        let row = claim(ClaimState::Active, "06");
        assert_eq!(
            fold_state(true, &[fold(&row)]),
            AcceptanceState::WaitingForMergeVerdict,
            "no verdict at all: waiting"
        );
        assert_eq!(
            fold_state(true, &[fold_with(
                &row,
                Some(OrchestrationVerdict::Parked),
                None,
                None
            )],),
            AcceptanceState::WaitingForMergeVerdict,
            "an open escalation is an unsettled verdict: waiting"
        );
        assert_eq!(
            fold_state(true, &[fold_with(
                &row,
                None,
                None,
                Some(PullRequestState::Open)
            )],),
            AcceptanceState::WaitingForMergeVerdict,
            "an open PR without a verdict is still the adjudicator's to judge"
        );
        assert_eq!(
            fold_state(true, &[fold_with(
                &row,
                None,
                None,
                Some(PullRequestState::Closed)
            )],),
            AcceptanceState::WaitingForMergeVerdict,
            "a dead PR without a verdict is literally waiting for one — \
             the adjudicator's queue still holds the run"
        );
    }

    #[test]
    fn a_landed_claim_from_an_earlier_lap_cannot_shadow_a_live_one() {
        let landed = claim(ClaimState::Landed, "07");
        let live = claim(ClaimState::Granted, "08");
        assert_eq!(
            fold_state(true, &[fold(&landed), fold(&live)]),
            AcceptanceState::WaitingForMergeVerdict,
            "the re-claim's lap owns the chain position, not the old land"
        );
    }

    // ---- Arm 3: no live claim — the terminal history decides --------

    #[test]
    fn landed_and_closed_is_the_healthy_terminal() {
        let landed = claim(ClaimState::Landed, "09");
        assert_eq!(
            fold_state(false, &[fold(&landed)]),
            AcceptanceState::LandedClosed,
        );
    }

    #[test]
    fn a_landed_claim_on_an_open_item_is_landed_but_open() {
        // The #1533/#1469/#1535 audit class: integration done, the
        // closing pass never ran.
        let landed = claim(ClaimState::Landed, "10");
        assert_eq!(
            fold_state(true, &[fold(&landed)]),
            AcceptanceState::ClosableLanded,
        );
    }

    #[test]
    fn closed_without_a_landed_claim_is_the_unlanded_class() {
        // The #1463 class, aftermath shape: closed, every claim
        // released unlanded.
        let released = claim(ClaimState::Released, "11");
        assert_eq!(
            fold_state(false, &[fold(&released)]),
            AcceptanceState::ClosedUnlanded,
        );
    }

    #[test]
    fn an_open_item_with_only_released_claims_returns_to_the_pool() {
        let released = claim(ClaimState::Released, "12");
        assert_eq!(
            fold_state(true, &[fold(&released)]),
            AcceptanceState::NoClaimOpenItem,
            "no live claim, nothing landed: the claim pool's ordinary input"
        );
    }

    // ---- The table: every (claim states × item state) combination ---
    // lands in exactly one named state, and distinct inputs never merge.

    #[test]
    fn the_full_input_table_maps_every_row_to_exactly_one_state() {
        let no_claim: Vec<ClaimFold<'_>> = Vec::new();
        let released = claim(ClaimState::Released, "21");
        let landed = claim(ClaimState::Landed, "22");
        let granted = claim(ClaimState::Granted, "23");
        let active = claim(ClaimState::Active, "24");
        let awaiting = claim(ClaimState::AwaitingMerge, "25");
        let cases: Vec<(bool, Vec<ClaimFold<'_>>, AcceptanceState)> = vec![
            (true, no_claim.clone(), AcceptanceState::NoClaimOpenItem),
            (false, no_claim.clone(), AcceptanceState::NoClaimClosedItem),
            (
                true,
                vec![fold(&released)],
                AcceptanceState::NoClaimOpenItem,
            ),
            (
                false,
                vec![fold(&released)],
                AcceptanceState::ClosedUnlanded,
            ),
            (true, vec![fold(&landed)], AcceptanceState::ClosableLanded),
            (false, vec![fold(&landed)], AcceptanceState::LandedClosed),
            (
                true,
                vec![fold(&granted)],
                AcceptanceState::WaitingForMergeVerdict,
            ),
            (false, vec![fold(&granted)], AcceptanceState::ClosedUnlanded),
            (
                true,
                vec![fold(&active)],
                AcceptanceState::WaitingForMergeVerdict,
            ),
            (false, vec![fold(&active)], AcceptanceState::ClosedUnlanded),
            (
                true,
                vec![fold(&awaiting)],
                AcceptanceState::WaitingForMergeVerdict,
            ),
            (
                false,
                vec![fold(&awaiting)],
                AcceptanceState::ClosedUnlanded,
            ),
            // Landed history + a later released claim, item closed: the
            // healthy terminal still reads through the release.
            (
                false,
                vec![fold(&landed), fold(&released)],
                AcceptanceState::LandedClosed,
            ),
            // Released history + a refused verdict record: the release
            // already happened; the pool state stands (the verdict
            // ladder only runs for LIVE claims).
            (
                true,
                vec![fold_with(
                    &released,
                    Some(OrchestrationVerdict::Refused),
                    None,
                    None,
                )],
                AcceptanceState::NoClaimOpenItem,
            ),
            (
                false,
                vec![fold_with(
                    &released,
                    Some(OrchestrationVerdict::Refused),
                    None,
                    None,
                )],
                AcceptanceState::ClosedUnlanded,
            ),
        ];
        for (item_open, claims, expected) in cases {
            assert_eq!(
                acceptance_fold(item_open, &claims),
                expected,
                "item_open={item_open}, claims={} must fold to {expected}",
                claims
                    .iter()
                    .map(|row| row.claim.state.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
    }

    /// The wire spelling is the canonical snake_case of the state
    /// names — the queue view's enum and any later close-door exposure
    /// spell them identically.
    #[test]
    fn the_state_names_spell_snake_case() {
        assert_eq!(
            AcceptanceState::NoClaimOpenItem.to_string(),
            "no_claim_open_item"
        );
        assert_eq!(
            AcceptanceState::NoClaimClosedItem.to_string(),
            "no_claim_closed_item"
        );
        assert_eq!(
            AcceptanceState::WaitingForMergeVerdict.to_string(),
            "waiting_for_merge_verdict"
        );
        assert_eq!(AcceptanceState::MergeRefused.to_string(), "merge_refused");
        assert_eq!(
            AcceptanceState::WaitingForClosable.to_string(),
            "waiting_for_closable"
        );
        assert_eq!(
            AcceptanceState::ClosableLanded.to_string(),
            "closable_landed"
        );
        assert_eq!(
            AcceptanceState::ClosedUnlanded.to_string(),
            "closed_unlanded"
        );
        assert_eq!(AcceptanceState::LandedClosed.to_string(), "landed_closed");
        for state in [
            AcceptanceState::NoClaimOpenItem,
            AcceptanceState::NoClaimClosedItem,
            AcceptanceState::WaitingForMergeVerdict,
            AcceptanceState::MergeRefused,
            AcceptanceState::WaitingForClosable,
            AcceptanceState::ClosableLanded,
            AcceptanceState::ClosedUnlanded,
            AcceptanceState::LandedClosed,
        ] {
            assert_eq!(
                state.to_string().parse::<AcceptanceState>(),
                Ok(state),
                "{state} round-trips through the wire spelling"
            );
        }
    }
}
