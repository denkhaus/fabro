use fabro_types::{ParallelBranchId, StageId};

/// Stage-level scope threaded through event emission to populate
/// `stage_id` / `parallel_group_id` / `parallel_branch_id` on events
/// that happen inside a concrete stage execution.
///
/// `visit` is the 1-based stage execution ordinal — the numeric component of
/// the external `StageId`.
#[derive(Clone, Debug)]
pub struct StageScope {
    pub node_id:            String,
    pub visit:              u32,
    pub parallel_group_id:  Option<StageId>,
    pub parallel_branch_id: Option<ParallelBranchId>,
}

impl StageScope {
    #[must_use]
    pub fn stage_id(&self) -> StageId {
        StageId::new(&self.node_id, self.visit)
    }
}
