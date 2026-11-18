//! The claim-hold seam (denkhaus/work#1457): the one home answering "does
//! this claim hold, and what releases it?".
//!
//! Five surfaces used to re-derive that answer independently — the
//! reconciler's verdict-driven release (ADR-0039 stage E), the grant
//! window's ledger uniqueness ("while non-terminal", the GLOSSARY wording
//! ADR-0045 supersedes), push-gate.nu, terminal_status.nu, and the
//! adjudicator fast-path park arms. This module carries the rule once;
//! the reconciler and the grant path (`claim_next`'s pool policy) consume
//! it instead of re-deriving.
//!
//! Arms, in evaluation order:
//!
//! 1. Terminal claim states (`Landed` / `Released`) hold nothing.
//! 2. An `AwaitingMerge` claim holds while its pull request is open; a refused
//!    merge verdict releases it (ADR-0039 stage E).
//! 3. A `granted`/`active` claim on a live (non-terminal) run holds by the
//!    ledger's uniqueness — the ordinary case.
//! 4. A `granted`/`active` claim on a terminal run: a refused merge verdict
//!    releases (stage E, verbatim); the park pin (work#1419, work#1448) holds a
//!    parked (`human_needed`) run's claim against everything but its operator's
//!    disposition; every other terminal state holds until the adjudicator
//!    verdicts.
//!
//! The orphan grace is NOT an arm here: a claim whose run record is gone
//! releases through the reconciler's grace window, which stays where it
//! is (work_reconciler.rs). The ledger stays the authority (ADR-0032
//! decision 2) — this module is vocabulary over a claim row, never a
//! second ledger.

use fabro_types::{OrchestrationVerdict, RunStatus};

use crate::store::{ClaimState, WorkClaim};

/// Why a live claim holds its work item. `None` from [`hold_reason`]
/// means the claim no longer holds and the item is free.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, strum::Display, strum::EnumString, strum::IntoStaticStr,
)]
#[strum(serialize_all = "snake_case")]
pub enum HoldReason {
    /// The owning run is still live: the ledger's uniqueness holds the
    /// item until the run ends and releases (or hands the claim on).
    RunActive,
    /// The owning run is parked (`human_needed`, ADR-0045): a waiting
    /// state, not a terminal one. The claim yields to nothing but the
    /// operator's disposition — never to a fresh lap's grant (the
    /// production burn on denkhaus/work#1312, runs 01M7NWYS…/01M7QB9Y…).
    ParkPinned,
    /// The owning run ended without a pull request and has no releasing
    /// verdict yet: the claim holds until the adjudicator verdicts
    /// (ADR-0039 stage E — the lock span is claim to merge-complete).
    AdjudicationPending,
    /// The claim sits at `awaiting_merge`: an open pull request is not
    /// an available item. The merger and the closing pass land it.
    AwaitingMerge,
}

/// Whether this claim holds its work item, and under what name. `None`
/// releases: the item is free for a fresh grant.
#[must_use]
pub fn hold_reason(
    claim: &WorkClaim,
    run_status: RunStatus,
    merge_verdict: Option<OrchestrationVerdict>,
) -> Option<HoldReason> {
    match claim.state {
        ClaimState::Landed | ClaimState::Released => None,
        ClaimState::AwaitingMerge => {
            if merge_verdict == Some(OrchestrationVerdict::Refused) {
                // ADR-0039 stage E: a PR the adjudicator refused never
                // merges — the verdict releases the claim.
                None
            } else {
                Some(HoldReason::AwaitingMerge)
            }
        }
        ClaimState::Granted | ClaimState::Active => {
            if !run_status.is_terminal() {
                // The owning run is live: the ledger's uniqueness holds
                // the item (the owner park, work#1345, included — a
                // non-terminal park holds through ordinary uniqueness).
                Some(HoldReason::RunActive)
            } else if merge_verdict == Some(OrchestrationVerdict::Refused) {
                // ADR-0039 stage E, verbatim: the refused verdict
                // releases; the adjudicator rejected the run.
                None
            } else if matches!(run_status, RunStatus::HumanNeeded) {
                // The park pin (work#1419, work#1448, ADR-0045 P1): a
                // parked decision-request run is WAITING, not done — its
                // claim yields to nothing but the operator's disposition,
                // never to a fresh lap's grant.
                Some(HoldReason::ParkPinned)
            } else {
                // ADR-0039 stage E: any other terminal state holds until
                // the adjudicator verdicts (the lock span is claim to
                // merge-complete).
                Some(HoldReason::AdjudicationPending)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use fabro_types::{OrchestrationVerdict, RunStatus};

    use super::{HoldReason, hold_reason};
    use crate::store::{ClaimState, WorkClaim};

    fn claim(state: ClaimState) -> WorkClaim {
        WorkClaim {
            claim_id: "claim-1".to_string(),
            project_id: "fabro".to_string(),
            item_key: "denkhaus/work#1".to_string(),
            run_id: "01M7NWYS000000000000000000".parse().unwrap(),
            lane: "loop".to_string(),
            state,
            granted_at: Utc::now(),
            updated_at: Utc::now(),
            released_at: None,
            release_reason: None,
            pr_url: None,
        }
    }

    fn running() -> RunStatus {
        RunStatus::Running
    }

    /// The decision-request park (ADR-0045): terminal today, a WAITING
    /// state to the hold seam.
    fn parked() -> RunStatus {
        RunStatus::HumanNeeded
    }

    fn failed() -> RunStatus {
        RunStatus::Failed {
            reason: fabro_types::FailureReason::WorkflowError,
        }
    }

    /// THE PARK PIN (work#1419, work#1448; the production burn on
    /// denkhaus/work#1312): a live claim on a parked `human_needed` run
    /// HOLDS. The grant window must see this arm — a fresh lap's grant
    /// attempt against the parked run's item is refused, and the item
    /// never re-claims while the park stands.
    #[test]
    fn a_parked_runs_live_claim_holds_park_pinned() {
        let held = claim(ClaimState::Active);
        assert_eq!(
            hold_reason(&held, parked(), None),
            Some(HoldReason::ParkPinned),
            "the park pin: a parked run's live claim holds"
        );
        assert_eq!(
            hold_reason(&held, parked(), Some(OrchestrationVerdict::Parked)),
            Some(HoldReason::ParkPinned),
            "the parked verdict is the park's own echo, still a hold"
        );
        let granted = claim(ClaimState::Granted);
        assert_eq!(
            hold_reason(&granted, parked(), None),
            Some(HoldReason::ParkPinned),
            "a not-yet-activated claim pins the same way"
        );
    }

    /// ADR-0039 stage E, verbatim: a terminal run WITHOUT a pull request
    /// whose merge verdict is refused RELEASES its claim; any other
    /// verdict holds.
    #[test]
    fn a_refused_verdict_releases_and_every_other_verdict_holds() {
        let held = claim(ClaimState::Active);
        assert_eq!(
            hold_reason(&held, failed(), Some(OrchestrationVerdict::Refused)),
            None,
            "stage E: the refused verdict releases"
        );
        assert_eq!(
            hold_reason(&held, failed(), None),
            Some(HoldReason::AdjudicationPending),
            "stage E: no verdict yet holds until the adjudicator verdicts"
        );
        assert_eq!(
            hold_reason(&held, failed(), Some(OrchestrationVerdict::Parked)),
            Some(HoldReason::AdjudicationPending),
            "stage E: an open operator escalation holds"
        );
        assert_eq!(
            hold_reason(&held, failed(), Some(OrchestrationVerdict::Enabled)),
            Some(HoldReason::AdjudicationPending),
            "stage E: an approved-but-unlanded verdict holds"
        );
        let succeeded = RunStatus::Succeeded {
            reason: fabro_types::SuccessReason::Completed,
        };
        assert_eq!(
            hold_reason(&held, succeeded, Some(OrchestrationVerdict::Refused)),
            None,
            "the release rule is verdict-driven, not failure-driven"
        );
    }

    /// A live run's claim holds by the ledger's uniqueness — the ordinary
    /// case the grant window's "while non-terminal" used to name.
    #[test]
    fn a_live_runs_claim_holds_by_uniqueness() {
        let held = claim(ClaimState::Granted);
        assert_eq!(
            hold_reason(&held, running(), None),
            Some(HoldReason::RunActive)
        );
        let blocked = RunStatus::Blocked {
            blocked_reason: fabro_types::BlockedReason::OwnerPending,
        };
        assert_eq!(
            hold_reason(&held, blocked, None),
            Some(HoldReason::RunActive),
            "the owner park (work#1345) is non-terminal: ordinary uniqueness holds"
        );
    }

    /// An `awaiting_merge` claim holds while the PR is open; the refused
    /// verdict releases it (the reconciler's AwaitingMerge arm, stage E).
    #[test]
    fn an_awaiting_merge_claim_holds_until_the_verdict_refuses() {
        let mut held = claim(ClaimState::AwaitingMerge);
        held.pr_url = Some("https://github.com/denkhaus/fabro/pull/1".to_string());
        assert_eq!(
            hold_reason(&held, failed(), None),
            Some(HoldReason::AwaitingMerge)
        );
        assert_eq!(
            hold_reason(&held, failed(), Some(OrchestrationVerdict::Refused)),
            None,
            "a PR the adjudicator refused never merges: the verdict releases"
        );
    }

    /// Terminal claim states hold nothing — the item is free.
    #[test]
    fn terminal_claim_states_hold_nothing() {
        for state in [ClaimState::Landed, ClaimState::Released] {
            assert_eq!(hold_reason(&claim(state), failed(), None), None);
        }
    }
}
