//! Fork-only duplicate-child guard (fabro-8ee1, re-landed 2026-09-14;
//! petri port W2-1 2026-09-21).
//!
//! A parent run must not gain a second non-terminal child of the same
//! workflow while a sibling is still running. Prompt prose alone proved
//! violable twice (2026-09-05 recreate after approval_required start
//! error; 2026-09-10 recreate after wait-timeout), so the create call
//! itself fails with the existing child named. The only legal
//! continuation is waiting on that child with `fabro_run_wait`.
//!
//! Under the intent contract, workflow identity is the
//! `WorkflowVersionId`. Petri's run SUMMARIES no longer carry it (the
//! projection's spec does), so the guard reads each non-terminal child's
//! projection — children counts are orchestration-small, and exact
//! version identity (not slug identity) is what lets a fresh pass with a
//! fresh version proceed while a stale child is stuck.
//!
//! This guard was dropped once by the #832 salvage merge (00ffd60f6
//! incident). It lives in this fork-only file with its presence tests in
//! `fork_duplicate_child_guard_tests.rs` and a touchpoints row — merges
//! cannot conflict it away, and a dropped seam call reds the gate.

use fabro_types::{RunId, WorkflowVersionId};

use crate::common::{FabroToolBackend, ToolError, ToolResult};

/// Reject the create when `parent_run_id` already has a non-terminal child
/// of the same workflow version. Terminal siblings and different versions
/// stay allowed.
pub(crate) async fn reject_duplicate_active_child(
    backend: &dyn FabroToolBackend,
    parent_run_id: RunId,
    workflow_version_id: WorkflowVersionId,
) -> ToolResult<()> {
    let children = backend
        .list_store_runs_by_parent(parent_run_id)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))?;
    for child in &children {
        if !child.lifecycle.status.is_terminal() {
            let version = backend
                .get_run_state(&child.id)
                .await
                .ok()
                .and_then(|projection| projection.spec.workflow_version_id);
            if version == Some(workflow_version_id) {
                return Err(ToolError::message(format!(
                    "duplicate child rejected (fabro-8ee1): parent run {parent_run_id} already \
                     has a non-terminal child {} of workflow version {workflow_version_id} — \
                     wait on it with fabro_run_wait instead of creating a sibling",
                    child.id
                )));
            }
        }
    }
    Ok(())
}
