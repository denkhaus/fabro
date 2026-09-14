//! Fork-only duplicate-child guard (fabro-8ee1, re-landed 2026-09-14).
//!
//! A parent run must not gain a second non-terminal child of the same
//! workflow while a sibling is still running. Prompt prose alone proved
//! violable twice (2026-09-05: recreate after an approval_required start
//! error; 2026-09-10: recreate after a wait-timeout produced a duplicate
//! develop child and a failed pass — the conductor prompt documents the
//! rejection message verbatim), so the create call itself fails with the
//! existing child named. The only legal continuation is waiting on that
//! child with `fabro_run_wait`.
//!
//! Under the intent contract (`RunIntent`), workflow identity is the
//! `WorkflowVersionId` — required on every create, and fixed within one
//! parent pass (the conductor pre-registers once and passes the id,
//! fabro-978d), so version-id equality covers both recorded incident
//! classes. Cross-pass orphans carry a fresh version id; that residual
//! gap needs workflow-name identity at admission and is tracked with the
//! chain-terminus work in fabro-b4ed.
//!
//! This guard was dropped once by the #832 salvage merge (00ffd60f6
//! incident: the guard AND its inline tests were removed in one conflict
//! resolution). It now lives in this fork-only file with its presence
//! test in `fork_duplicate_child_guard_tests.rs` and a touchpoints row —
//! upstream merges cannot conflict it away, and a dropped seam call reds
//! the gate.

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
        if child.workflow.workflow_version_id == Some(workflow_version_id)
            && !child.lifecycle.status.is_terminal()
        {
            return Err(ToolError::message(format!(
                "duplicate child rejected (fabro-8ee1): parent run {parent_run_id} already has a \
                 non-terminal child {} of workflow version {workflow_version_id} — wait on it with \
                 fabro_run_wait instead of creating a sibling",
                child.id
            )));
        }
    }
    Ok(())
}
