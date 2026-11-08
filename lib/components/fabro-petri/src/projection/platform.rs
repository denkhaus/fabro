//! The platform records folded into the view: the run's creation, its
//! lifecycle, its title and parent, the branch and identity the first
//! checkpoint recorded, every checkpoint and artifact, the run's diff, and
//! the pull request (VIEWS.md "Run", "Checkpoints", "Artifacts", "Pull
//! request").

use chrono::{DateTime, Utc};
use fabro_store::platform_records::{
    PlatformRecord, RunLifecycleKind, RunLifecycleRecord, StoredPlatformRecord,
};
use fabro_types::{
    CheckpointRecord as ViewCheckpoint, Conclusion, FailureCategory, FailureDetail, FailureReason,
    OrchestrationField, PullRequestCreation, PullRequestCreationStatus, PullRequestLink,
    PullRequestState, RunApproval, RunApprovalState, RunArtifact, RunControlAction, RunDiff,
    RunFailure, RunProjection, RunSandbox, RunStatus, StageObservation, StageOutcome,
    format_blob_ref,
};
use tracing::debug;

use super::sandbox::sandbox_plan;
use super::{FiringKey, RunView, apply_status, millis, settle_control, touch};

impl RunView {
    pub(super) fn fold_platform(&mut self, stored: &StoredPlatformRecord, stream_seq: u64) {
        let at = millis(stored.recorded_at);
        if let PlatformRecord::RunCreated(created) = &stored.record {
            let title = created
                .title
                .clone()
                .unwrap_or_else(|| fabro_types::infer_run_title(created.spec.graph.goal()));
            let mut projection = RunProjection::new(title, created.spec.clone(), at);
            projection.parent_id = created.parent_id;
            projection.retried_from = created.retried_from;
            projection.web_url.clone_from(&created.web_url);
            projection.sandbox = Some(RunSandbox::planned(sandbox_plan(
                &projection.spec.settings.run.environment,
            )));
            self.projection = Some(projection);
            return;
        }
        let Some(projection) = self.projection.as_mut() else {
            debug!(
                seq = stored.seq,
                kind = %stored.record.kind(),
                "platform record before run.created; not folded"
            );
            return;
        };
        touch(projection, at);
        match &stored.record {
            PlatformRecord::RunLifecycle(record) => fold_lifecycle(projection, record, at),
            PlatformRecord::RunTitle(record) => projection.title.clone_from(&record.title),
            PlatformRecord::RunParent(record) => projection.parent_id = record.parent_id,
            PlatformRecord::RunArchived => projection.archived_at = Some(at),
            PlatformRecord::RunUnarchived => projection.archived_at = None,
            PlatformRecord::RunSuperseded(record) => {
                projection.superseded_by = Some(record.new_run_id);
            }
            PlatformRecord::RunCreated(_)
            | PlatformRecord::RunNotice(_)
            | PlatformRecord::InterviewAnswered(_)
            | PlatformRecord::NotificationSent(_)
            | PlatformRecord::RunPaired(_)
            | PlatformRecord::RunBranchPublished(_) => {}
            PlatformRecord::WorkClaimed(record) => {
                projection.current_work_item = Some(record.item_key.clone());
            }
            PlatformRecord::WorkReleased(_) => {
                projection.current_work_item = None;
            }
            PlatformRecord::WorkLanded(_) => {
                // Merge-complete also ends the run's claim (work#1128):
                // the land folds like the release — the item leaves the
                // run's current-work-item slot.
                projection.current_work_item = None;
            }
            PlatformRecord::RunBranch(record) => {
                self.state.run_branch.clone_from(&record.run_branch);
                self.state.base_sha.clone_from(&record.base_sha);
                if let Some(start) = projection.start.as_mut() {
                    start.run_branch.clone_from(&record.run_branch);
                    start.base_sha.clone_from(&record.base_sha);
                }
            }
            PlatformRecord::GitIdentity(record) => {
                projection.git_identity = Some(record.identity.clone());
            }
            PlatformRecord::Checkpoint(record) => {
                self.state.checkpoints = self.state.checkpoints.saturating_add(1);
                let stage = self
                    .state
                    .stages
                    .get(&FiringKey::new(record.execution, record.firing));
                let current_node = stage.map_or_else(String::new, |stage| stage.node_name.clone());
                let stage_id = stage
                    .filter(|stage| stage.shown)
                    .map(|stage| stage.stage_id.clone());
                let checkpoint = fabro_types::Checkpoint {
                    timestamp:      at,
                    current_node:   current_node.clone(),
                    git_commit_sha: record.git_commit_sha.clone(),
                };
                // The patch stays in the blob table; the view carries its
                // reference for a reader to resolve.
                let patch = record.patch_blob.as_ref().map(format_blob_ref);
                if let Some(stage) = stage_id.and_then(|stage_id| projection.stage_mut(&stage_id)) {
                    if patch.is_some() {
                        stage.diff.clone_from(&patch);
                    }
                }
                projection.checkpoints.push(ViewCheckpoint {
                    seq: u32::try_from(stream_seq).unwrap_or(u32::MAX),
                    checkpoint,
                    diff: RunDiff {
                        patch,
                        summary: record.diff_summary,
                    },
                });
            }
            PlatformRecord::ArtifactCollected(record) => {
                let stage = self
                    .state
                    .stages
                    .get(&FiringKey::new(record.execution, record.firing));
                let Some(stage_id) = stage.map(|stage| stage.stage_id.clone()) else {
                    debug!(
                        seq = stored.seq,
                        path = record.path,
                        "artifact record for an unknown firing; not folded"
                    );
                    return;
                };
                projection.artifacts.push(RunArtifact {
                    stage_id,
                    retry: record.attempt,
                    relative_path: record.path.clone(),
                    size: record.bytes,
                    blob: record.blob,
                });
            }
            PlatformRecord::ObservationRecorded(record) => {
                // ADR-0038: the entry folds onto the stage its recording
                // session ran under; `seq` is the identity the link seam
                // (stage 3) references. An unknown firing keeps the record
                // in the stream but out of the view, like an artifact.
                let stage = self
                    .state
                    .stages
                    .get(&FiringKey::new(record.execution, record.firing));
                let Some(stage_id) = stage.map(|stage| stage.stage_id.clone()) else {
                    debug!(
                        seq = stored.seq,
                        "observation record for an unknown firing; not folded"
                    );
                    return;
                };
                if let Some(stage) = projection.stage_mut(&stage_id) {
                    stage.observations.push(StageObservation {
                        seq:      stored.seq,
                        category: record.category,
                        text:     record.text.clone(),
                    });
                }
            }
            PlatformRecord::RunOrchestration(record) => {
                // ADR-0039 orchestration field: one writer per field —
                // the adjudicator names `merge`, the merger names
                // `closable`. The stream is `seq`-ordered, so a later
                // record for the same field transitions the run
                // (parked → enabled) by simply overwriting the fold.
                match record.field {
                    OrchestrationField::Merge => projection.merge = Some(record.verdict),
                    OrchestrationField::Closable => projection.closable = Some(record.verdict),
                }
            }
            PlatformRecord::RunDiff(record) => {
                let diff = RunDiff {
                    patch:   record.patch_blob.as_ref().map(format_blob_ref),
                    summary: record.diff_summary,
                };
                if let Some(conclusion) = projection.conclusion.as_mut() {
                    conclusion.diff = diff.clone();
                }
                self.state.run_diff = Some(diff);
            }
            PlatformRecord::PullRequestRequested(record) => {
                projection.pull_request_creation = Some(PullRequestCreation {
                    id:           record.creation_id,
                    status:       PullRequestCreationStatus::Pending,
                    model:        record.model.clone(),
                    force:        record.force,
                    requested_at: at,
                    updated_at:   at,
                    pull_request: None,
                    error:        None,
                });
            }
            PlatformRecord::PullRequestCreated(record) => {
                let link = PullRequestLink {
                    owner:  record.owner.clone(),
                    repo:   record.repo.clone(),
                    number: record.number,
                    // A fresh PR's outcome is unknown to the platform;
                    // the merged/closed records name it when it settles
                    // (denkhaus/work#1173).
                    state:  None,
                };
                projection.pull_request = Some(link.clone());
                if let Some(creation) = projection
                    .pull_request_creation
                    .as_mut()
                    .filter(|creation| creation.is_pending())
                {
                    creation.succeed(link, at);
                }
            }
            PlatformRecord::PullRequestFailed(record) => {
                if let Some(creation) =
                    projection
                        .pull_request_creation
                        .as_mut()
                        .filter(|creation| {
                            creation.is_pending()
                                && record
                                    .creation_id
                                    .is_none_or(|creation_id| creation_id == creation.id)
                        })
                {
                    creation.fail(record.error.clone(), at);
                    // Fork seam (fabro-6655): a publish failure trailing the
                    // conclusion re-classifies the green run as
                    // PublishBlocked (records can arrive after the finish).
                    super::fork_taxonomy::reclassify_publish_blocked(projection, at);
                }
            }
            PlatformRecord::PullRequestLinked(record) => {
                let link = record.link();
                projection.pull_request = Some(link.clone());
                if let Some(creation) = projection
                    .pull_request_creation
                    .as_mut()
                    .filter(|creation| creation.is_pending())
                {
                    creation.succeed(link, at);
                }
            }
            PlatformRecord::PullRequestUnlinked(_) => {
                projection.pull_request = None;
                projection.pull_request_creation = None;
            }
            // The PR outcome settles on the stored link
            // (denkhaus/work#1173): the run list projection carries it
            // for the board's PR state chip. A state record ahead of any
            // link stays unfolded — there is nothing to colour.
            PlatformRecord::PullRequestMerged => {
                if let Some(link) = projection.pull_request.as_mut() {
                    link.state = Some(PullRequestState::Merged);
                }
            }
            PlatformRecord::PullRequestClosed => {
                if let Some(link) = projection.pull_request.as_mut() {
                    link.state = Some(PullRequestState::Closed);
                }
            }
        }
    }
}

fn fold_lifecycle(projection: &mut RunProjection, record: &RunLifecycleRecord, at: DateTime<Utc>) {
    use RunLifecycleKind as Kind;
    match record.transition {
        Kind::Submitted => apply_status(projection, RunStatus::Submitted, at),
        Kind::StartRequested => {}
        Kind::Unpaused => {
            apply_status(projection, projection.status.unpaused(), at);
            settle_control(projection, RunControlAction::Unpause);
        }
        Kind::Pending => {
            if let Some(status) = record.status {
                apply_status(projection, status, at);
            }
            projection.approval = Some(RunApproval {
                state:         RunApprovalState::Pending,
                requested_at:  at,
                decided_at:    None,
                denial_reason: None,
            });
        }
        Kind::Approved => {
            if let Some(approval) = projection.approval.as_mut() {
                approval.state = RunApprovalState::Approved;
                approval.decided_at = Some(at);
            }
        }
        Kind::Denied => {
            if let Some(approval) = projection.approval.as_mut() {
                approval.state = RunApprovalState::Denied;
                approval.decided_at = Some(at);
                approval.denial_reason.clone_from(&record.reason);
            }
            apply_status(
                projection,
                RunStatus::Failed {
                    reason: FailureReason::ApprovalDenied,
                },
                at,
            );
        }
        Kind::Runnable => {
            // A run left in flight by a restart goes back to the queue: the
            // resume's `runnable` steps back from wherever the run stood.
            let in_flight = matches!(
                projection.status,
                RunStatus::Starting
                    | RunStatus::Running
                    | RunStatus::Blocked { .. }
                    | RunStatus::Paused { .. }
            );
            if in_flight && record.status == Some(RunStatus::Runnable) {
                projection.status = RunStatus::Runnable;
                projection.status_updated_at = at;
            } else if let Some(status) = record.status {
                apply_status(projection, status, at);
            }
        }
        Kind::Blocked => {
            // A block that lands while the run is paused waits behind the
            // pause: the unpause restores it.
            match (projection.status, record.status) {
                (RunStatus::Paused { .. }, Some(RunStatus::Blocked { blocked_reason })) => {
                    apply_status(
                        projection,
                        RunStatus::Paused {
                            prior_block: Some(blocked_reason),
                        },
                        at,
                    );
                }
                (_, Some(status)) => apply_status(projection, status, at),
                (_, None) => {}
            }
        }
        Kind::Unblocked => {
            let status = match projection.status {
                RunStatus::Paused { .. } => RunStatus::Paused { prior_block: None },
                _ => record.status.unwrap_or(RunStatus::Running),
            };
            apply_status(projection, status, at);
        }
        Kind::Starting | Kind::Running | Kind::Removing | Kind::Dead => {
            if let Some(status) = record.status {
                apply_status(projection, status, at);
            }
        }
        Kind::Paused => {
            apply_status(projection, projection.status.paused(), at);
            settle_control(projection, RunControlAction::Pause);
        }
        Kind::Succeeded | Kind::Failed => {
            if let Some(status) = record.status {
                apply_status(projection, status, at);
            }
            projection.pending_control = None;
            if projection.conclusion.is_none() {
                let (outcome, failure) = match record.status {
                    Some(RunStatus::Failed { reason }) => (
                        StageOutcome::Failed {
                            retry_requested: false,
                        },
                        Some(RunFailure {
                            reason,
                            detail: FailureDetail::new(
                                record
                                    .reason
                                    .clone()
                                    .unwrap_or_else(|| "the run failed".to_string()),
                                FailureCategory::Deterministic,
                            ),
                        }),
                    ),
                    _ => (StageOutcome::Succeeded, None),
                };
                projection.conclusion = Some(Conclusion::outcome_only(at, outcome, failure));
            }
        }
        Kind::CancelRequested => projection.pending_control = Some(RunControlAction::Cancel),
        Kind::PauseRequested => projection.pending_control = Some(RunControlAction::Pause),
        Kind::UnpauseRequested => projection.pending_control = Some(RunControlAction::Unpause),
    }
}

#[cfg(test)]
mod tests {
    use fabro_store::platform_records::{
        PullRequestCreatedRecord, PullRequestLinkedRecord, RunCreatedRecord,
    };
    use fabro_types::test_support as types_support;

    use super::*;

    fn stored(seq: u64, record: PlatformRecord) -> StoredPlatformRecord {
        StoredPlatformRecord {
            seq,
            recorded_at: seq * 1_000,
            record,
            position: None,
        }
    }

    /// A view whose run was created and now carries PR #7.
    fn view_with_pull_request() -> RunView {
        let mut view = RunView::new();
        view.fold_platform(
            &stored(
                1,
                PlatformRecord::RunCreated(RunCreatedRecord {
                    spec:         types_support::test_run_spec(),
                    title:        None,
                    parent_id:    None,
                    retried_from: None,
                    web_url:      None,
                }),
            ),
            1,
        );
        view.fold_platform(
            &stored(
                2,
                PlatformRecord::PullRequestCreated(PullRequestCreatedRecord {
                    number:    7,
                    owner:     "acme".to_string(),
                    repo:      "widgets".to_string(),
                    html_url:  "https://github.com/acme/widgets/pull/7".to_string(),
                    head_sha:  None,
                    draft:     false,
                    operation: None,
                }),
            ),
            2,
        );
        view
    }

    #[test]
    fn merged_and_closed_records_settle_the_link_state() {
        let mut view = view_with_pull_request();
        view.fold_platform(&stored(3, PlatformRecord::PullRequestMerged), 3);
        assert_eq!(
            view.projection()
                .unwrap()
                .pull_request
                .as_ref()
                .unwrap()
                .state,
            Some(PullRequestState::Merged)
        );
        view.fold_platform(&stored(4, PlatformRecord::PullRequestClosed), 4);
        assert_eq!(
            view.projection()
                .unwrap()
                .pull_request
                .as_ref()
                .unwrap()
                .state,
            Some(PullRequestState::Closed)
        );
    }

    #[test]
    fn a_created_link_starts_without_a_state() {
        let view = view_with_pull_request();
        assert_eq!(
            view.projection()
                .unwrap()
                .pull_request
                .as_ref()
                .unwrap()
                .state,
            None
        );
    }

    #[test]
    fn a_state_record_ahead_of_any_link_is_not_folded() {
        let mut view = RunView::new();
        view.fold_platform(
            &stored(
                1,
                PlatformRecord::RunCreated(RunCreatedRecord {
                    spec:         types_support::test_run_spec(),
                    title:        None,
                    parent_id:    None,
                    retried_from: None,
                    web_url:      None,
                }),
            ),
            1,
        );
        view.fold_platform(&stored(2, PlatformRecord::PullRequestMerged), 2);
        assert!(view.projection().unwrap().pull_request.is_none());
    }

    #[test]
    fn a_relink_resets_the_state_to_unknown() {
        let mut view = view_with_pull_request();
        view.fold_platform(&stored(3, PlatformRecord::PullRequestMerged), 3);
        view.fold_platform(
            &stored(
                4,
                PlatformRecord::PullRequestLinked(PullRequestLinkedRecord {
                    owner:  "acme".to_string(),
                    repo:   "widgets".to_string(),
                    number: 8,
                }),
            ),
            4,
        );
        let link = view.projection().unwrap().pull_request.as_ref().unwrap();
        assert_eq!(link.number, 8);
        assert_eq!(link.state, None);
    }
}
