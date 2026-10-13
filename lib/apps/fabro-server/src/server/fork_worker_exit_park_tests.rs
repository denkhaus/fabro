//! Fork-only pin (fabro-71e6, sprint-35 arch finding N1): the worker-exit
//! settle keeps a RESUMABLE QUOTA PARK (fabro-2c4a/2e7b) — upstream's
//! inline pins for the 2026-10-11 fix (acb3abffc) live in server/tests.rs,
//! which an upstream merge rewrites freely; this file survives it.
//!
//! Production shape (run 01M4NV2HNFF0KGWZ8BF23J375C): the conclusion
//! carried the 5h-window park (soft_stop + transient_infra naming the
//! window) while the worker-exit path read `Blocked` as unsettled and
//! clobbered it with `Failed { Terminated }` — the parked run read as a
//! termination and the line lost its resumable pass.

use fabro_types::{BlockedReason, RunStatus};

/// A park the run is standing in survives its worker's exit — successful or
/// not — and wins over a stale non-park store status. The settle treats a
/// resumable park like a terminal status: the park owns the run.
#[test]
fn a_resumable_quota_park_is_settled_at_worker_exit() {
    let parked = RunStatus::Blocked {
        blocked_reason: BlockedReason::QuotaRateLimit,
    };
    assert_eq!(
        super::status_after_worker_exit(parked, parked, false),
        parked,
        "an unsuccessful exit keeps the park (01M4NV2HNFF0)"
    );
    assert_eq!(
        super::status_after_worker_exit(parked, parked, true),
        parked,
        "a successful exit keeps the park"
    );
    assert_eq!(
        super::status_after_worker_exit(parked, RunStatus::Running, false),
        parked,
        "the live park wins over a stale running store status"
    );
}
