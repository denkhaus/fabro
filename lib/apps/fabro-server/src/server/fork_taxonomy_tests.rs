//! Fork-only terminal-taxonomy tests (extracted per ADR-0021 D7 in the
//! v0.357 merge; user directive 2026-09-15): the publish-blocked /
//! boundary taxonomy (fabro-67e5, fabro-08b4) must survive upstream
//! merges — the tests ARE the presence pin.

use fabro_types::{FailureCategory, FailureDetail, RunStatus, SuccessReason, fixtures};
use httpmock::MockServer;

use super::tests::{
    append_slack_notification_event, create_slack_notification_run, mock_slack_post,
    slack_lifecycle_service, workflow_settings_with_run_notifications,
};
use super::*;
use crate::test_support::*;

/// fabro-67e5: a publish-blocked completion routes through run.completed
/// (not run.failed) and the Slack result carries the remediation.
#[tokio::test]
async fn slack_lifecycle_publish_blocked_completion_posts_remediation() {
    let server = MockServer::start_async().await;
    let post = mock_slack_post(
        &server,
        vec![
            r##""channel":"#deploys""##.to_string(),
            "Fabro run completed".to_string(),
            "succeeded — publish_blocked".to_string(),
            "the run branch 'fabro/run/1' was pushed".to_string(),
            "1m 5s".to_string(),
        ],
        "100.2",
    )
    .await;
    let state = test_app_state();
    let service = slack_lifecycle_service(server.base_url(), None);
    let run_id = fixtures::RUN_1;
    let settings = workflow_settings_with_run_notifications(
        r##"
[run.notifications.deploys]
enabled = true
provider = "slack"
events = ["run.completed"]

[run.notifications.deploys.slack]
channel = "#deploys"
"##,
        Some("Deploy workflow"),
    );
    let run_store = create_slack_notification_run(&state, run_id, settings, "deploy", None).await;
    workflow_event::append_event(&run_store, &run_id, &workflow_event::Event::RunRunnable {
        source: fabro_types::RunRunnableSource::StartRequested,
        actor:  None,
    })
    .await
    .unwrap();
    workflow_event::append_event(&run_store, &run_id, &workflow_event::Event::RunStarting)
        .await
        .unwrap();
    workflow_event::append_event(&run_store, &run_id, &workflow_event::Event::RunRunning)
        .await
        .unwrap();
    let envelope = append_slack_notification_event(
        &run_store,
        run_id,
        &workflow_event::Event::WorkflowRunCompleted {
            timing:               fabro_types::RunTiming::wall_only(65_432),
            artifact_count:       0,
            status:               "succeeded".to_string(),
            reason:               SuccessReason::PublishBlocked,
            failure:              Some(fabro_types::RunFailure {
                reason: fabro_types::FailureReason::PublishFailed,
                detail: FailureDetail::new(
                    "Work done, publish blocked — the run branch 'fabro/run/1' was pushed",
                    FailureCategory::Deterministic,
                ),
            }),
            final_git_commit_sha: None,
            final_patch:          None,
            diff_summary:         None,
            usage:                None,
        },
    )
    .await;

    service.handle_event(state.as_ref(), &envelope, None).await;

    post.assert_async().await;
}

/// fabro-67e5: a publish-blocked completion keeps the run green
/// (`Succeeded { PublishBlocked }`) while surfacing the delivery blocker as
/// the run error, so list and detail views render the remediation.
#[tokio::test]
async fn publish_blocked_completion_stays_succeeded_with_error_detail() {
    let state = test_app_state();
    let run_id = fixtures::RUN_1;
    let temp_dir = tempfile::tempdir().unwrap();
    {
        let mut runs = state.runs.lock().expect("runs lock poisoned");
        runs.insert(
            run_id,
            managed_run(
                String::new(),
                RunStatus::Running,
                chrono::Utc::now(),
                temp_dir.path().join(run_id.to_string()),
                RunExecutionMode::Start,
            ),
        );
    }

    let completed =
        workflow_event::to_run_event(&run_id, &workflow_event::Event::WorkflowRunCompleted {
            timing:               fabro_types::RunTiming::wall_only(65_432),
            artifact_count:       0,
            status:               "succeeded".to_string(),
            reason:               SuccessReason::PublishBlocked,
            failure:              Some(fabro_types::RunFailure {
                reason: fabro_types::FailureReason::PublishFailed,
                detail: FailureDetail::new(
                    "failed to create pull request Work done, publish blocked — the run \
                     branch 'fabro/run/1' was pushed",
                    FailureCategory::Deterministic,
                ),
            }),
            final_git_commit_sha: None,
            final_patch:          None,
            diff_summary:         None,
            usage:                None,
        });
    update_live_run_from_event(&state, run_id, &completed);

    let runs = state.runs.lock().expect("runs lock poisoned");
    let run = runs.get(&run_id).expect("run present");
    assert_eq!(run.status, RunStatus::Succeeded {
        reason: SuccessReason::PublishBlocked,
    });
    let error = run.error.as_deref().expect("publish blocker as error");
    assert!(error.contains("publish blocked"), "{error}");
}
