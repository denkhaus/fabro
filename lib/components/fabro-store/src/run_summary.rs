//! The run summary (`Run`) a projection stands for: the row the run list,
//! the board and the scheduler read, derived from the projection.

use fabro_types::{
    AskFabro, RepositoryRef, Run, RunError, RunId, RunLifecycle, RunLinks, RunModel, RunOrigin,
    RunProjection, RunSize, RunTimestamps, WorkflowRef, sum_usage,
};
use lithos_llm::types::Usage;

/// The run summary (`Run`) a projection stands for: what the run list, the
/// board and the scheduler read.
#[must_use]
pub fn build_summary(state: &RunProjection, run_id: &RunId) -> Run {
    let goal = state.spec.graph.goal().to_string();
    let diff_summary = state
        .conclusion
        .as_ref()
        .and_then(|conclusion| conclusion.diff.summary)
        .or_else(|| {
            state
                .checkpoints
                .iter()
                .rev()
                .find_map(|checkpoint| checkpoint.diff.summary)
        });

    let current_question = state
        .pending_interviews
        .iter()
        .min_by(|(left_id, left), (right_id, right)| {
            left.started_at
                .cmp(&right.started_at)
                .then_with(|| left_id.cmp(right_id))
        })
        .map(|(_, record)| record.question.clone());
    let models = run_models(state);
    let created_by = state.spec.provenance.subject.clone();
    let source_directory = state.spec.source_directory.clone();
    let repo_origin_url = state.spec.git.as_ref().map(|git| git.origin_url.clone());
    let start_time = state.start.as_ref().map(|start| start.start_time);
    let completed_at = state
        .conclusion
        .as_ref()
        .map(|conclusion| conclusion.timestamp);
    let run_timing = state
        .conclusion
        .as_ref()
        .map(|conclusion| conclusion.timing);
    let usage = projected_usage(state);

    Run {
        id: *run_id,
        parent_id: state.parent_id,
        children_count: 0,
        title: state.title().into_owned(),
        claimed_seed_id: state.claimed_seed_id.clone(),
        goal,
        workflow: WorkflowRef {
            slug:       state.spec.workflow_slug.clone(),
            name:       state.spec.workflow_name().map(ToOwned::to_owned),
            graph_name: state.spec.graph_name().map(ToOwned::to_owned),
            node_count: i64::try_from(state.spec.graph.nodes.len())
                .expect("graph node count should fit in i64"),
            edge_count: i64::try_from(state.spec.graph.edges.len())
                .expect("graph edge count should fit in i64"),
        },
        automation: state.spec.automation.clone(),
        repository: Some(RepositoryRef::from_origin_and_source(
            repo_origin_url,
            source_directory.as_deref(),
        )),
        created_by,
        origin: RunOrigin::default(),
        labels: state.spec.labels.clone(),
        lifecycle: RunLifecycle {
            status:             state.status,
            approval:           state.approval.clone(),
            pending_control:    state.pending_control,
            queue_position:     None,
            // work#1418: the UI verdict bar reads `lifecycle.error.message`
            // ("No reason was recorded" when absent) — the conclusion's
            // failure detail IS the recorded why, so it must not stay
            // behind when the row leaves the projection.
            error:              state.conclusion.as_ref().and_then(|conclusion| {
                conclusion.failure.as_ref().map(|failure| RunError {
                    message: failure.detail.message.clone(),
                })
            }),
            conclusion_failure: state.conclusion.as_ref().and_then(|c| c.failure.clone()),
            archived:           state.archived_at.is_some(),
            archived_at:        state.archived_at,
        },
        sandbox: state.sandbox.clone(),
        workflow_version_id: state.spec.workflow_version_id,
        models,
        source_directory,
        timestamps: RunTimestamps {
            created_at: run_id.created_at(),
            started_at: start_time,
            last_event_at: Some(state.last_event_at),
            completed_at,
        },
        timing: run_timing,
        usage,
        size: RunSize::from_cost(usage.cost),
        ask_fabro: AskFabro::default(),
        diff: diff_summary,
        pull_request: state.pull_request.clone(),
        current_question,
        superseded_by: state.superseded_by,
        retried_from: state.retried_from,
        links: RunLinks {
            web: state.web_url.clone(),
        },
    }
}

/// The run's usage: the conclusion's total once the run ended, else the sum
/// of every non-boundary stage's usage so far.
#[must_use]
pub fn projected_usage(state: &RunProjection) -> Usage {
    if let Some(usage) = state
        .conclusion
        .as_ref()
        .and_then(|conclusion| conclusion.usage)
    {
        return usage;
    }

    sum_usage(
        state
            .iter_stages()
            .filter(|(stage_id, _)| !state.is_boundary_stage(stage_id.node_id()))
            .map(|(_, stage)| stage.usage),
    )
}

fn run_models(state: &RunProjection) -> Vec<RunModel> {
    let mut models = state
        .iter_stages()
        .filter_map(|(_, stage)| stage.model.as_ref())
        .map(|model| RunModel {
            provider: Some(model.provider.to_string()),
            name:     model.model_id.to_string(),
        })
        .collect::<Vec<_>>();
    models.sort_by(|left, right| {
        left.provider
            .cmp(&right.provider)
            .then_with(|| left.name.cmp(&right.name))
    });
    models.dedup_by(|left, right| left.provider == right.provider && left.name == right.name);
    models
}

#[cfg(test)]
mod tests {
    use fabro_types::outcome::StageOutcome;
    use fabro_types::{
        Conclusion, FailureCategory, FailureDetail, FailureReason, RunFailure, RunStatus,
        test_support as types_support,
    };

    use super::build_summary;

    fn concluded_projection(failure: Option<RunFailure>) -> fabro_types::RunProjection {
        let mut projection = fabro_types::RunProjection::new(
            "summary".to_string(),
            types_support::test_run_spec(),
            chrono::Utc::now(),
        );
        projection.status = RunStatus::Failed {
            reason: FailureReason::SoftStop,
        };
        projection.conclusion = Some(Conclusion::outcome_only(
            chrono::Utc::now(),
            StageOutcome::Failed {
                retry_requested: false,
            },
            failure,
        ));
        projection
    }

    /// work#1418: the verdict bar reads `lifecycle.error.message` and
    /// renders "No reason was recorded" when absent — the conclusion's
    /// failure detail must lift into the summary row's error slot.
    #[test]
    fn a_concluded_failure_populates_the_lifecycle_error_message() {
        let projection = concluded_projection(Some(RunFailure {
            reason: FailureReason::SoftStop,
            detail: FailureDetail::new(
                "Reviewer failed (next run re-enters)",
                FailureCategory::TransientInfra,
            ),
        }));
        let summary = build_summary(&projection, &fabro_types::RunId::new());
        assert_eq!(
            summary
                .lifecycle
                .error
                .as_ref()
                .map(|error| error.message.as_str()),
            Some("Reviewer failed (next run re-enters)"),
            "the exit-edge prose reaches the verdict bar's read slot"
        );
    }

    /// The control: a concluded run WITHOUT a failure carries no error —
    /// successes and reasonless conclusions stay quiet.
    #[test]
    fn a_failureless_conclusion_carries_no_lifecycle_error() {
        let projection = concluded_projection(None);
        let summary = build_summary(&projection, &fabro_types::RunId::new());
        assert!(
            summary.lifecycle.error.is_none(),
            "no failure recorded, no error prose invented"
        );
    }
}
