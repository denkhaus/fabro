//! Fork-only slack lifecycle interview-notification tests (extracted from
//! `server/tests.rs` per fabro-ab8e, user directive 2026-09-18): routing
//! `interview.started` notifications to Slack channels is fork-added
//! behavior. Keeping the tests in a fork-owned file means an upstream merge
//! can never silently drop them, and the upstream-owned `tests.rs` monolith
//! stays structurally upstream-identical.

use fabro_types::fixtures;
use httpmock::MockServer;

use super::tests::{
    append_slack_notification_event, create_slack_notification_run, mock_slack_post,
    slack_lifecycle_service, workflow_settings_with_run_notifications,
};
use super::*;
use crate::test_support::*;

#[tokio::test]
async fn slack_lifecycle_interview_started_posts_question_to_route() {
    let server = MockServer::start_async().await;
    let post = mock_slack_post(
        &server,
        vec![
            r##""channel":"#gates""##.to_string(),
            "Should we ship the fix?".to_string(),
            "times out in ~5m".to_string(),
            "Open in Fabro".to_string(),
        ],
        "100.3",
    )
    .await;
    let state = test_app_state();
    let service = slack_lifecycle_service(server.base_url(), None);
    let run_id = fixtures::RUN_1;
    let settings = workflow_settings_with_run_notifications(
        r##"
[run.notifications.gates]
enabled = true
provider = "slack"
events = ["interview.started"]

[run.notifications.gates.slack]
channel = "#gates"
"##,
        Some("Revisor"),
    );
    let run_store =
        create_slack_notification_run(&state, run_id, settings, "revisor", Some("revisor")).await;
    let envelope = append_slack_notification_event(
        &run_store,
        run_id,
        &workflow_event::Event::InterviewStarted {
            question_id:     "q-1".to_string(),
            question:        "Should we ship the fix?".to_string(),
            stage:           "review".to_string(),
            question_type:   "yes_no".to_string(),
            options:         vec![],
            allow_freeform:  false,
            timeout_seconds: Some(300.0),
            context_display: None,
            review_target:   None,
        },
    )
    .await;

    service
        .handle_event(
            state.as_ref(),
            &envelope,
            Some("https://fabro.example/runs/run-1"),
        )
        .await;

    post.assert_async().await;
    assert_eq!(post.calls_async().await, 1, "must dispatch exactly once");
    assert!(
        service
            .posted_messages
            .lock()
            .expect("posted messages lock poisoned")
            .is_empty(),
        "route posts must not use interview bot message state"
    );
}

#[tokio::test]
async fn slack_lifecycle_interview_started_with_bot_and_route_posts_both_channels() {
    // A default channel AND a matching route both fire for the same
    // question: the bot owns its channel, the route owns its own —
    // including the (documented) case where both channels differ.
    let server = MockServer::start_async().await;
    let route_post = mock_slack_post(
        &server,
        vec![
            r##""channel":"#gates""##.to_string(),
            "Ship it?".to_string(),
        ],
        "100.5",
    )
    .await;
    let bot_post = mock_slack_post(
        &server,
        vec![
            r#""channel":"C-default""#.to_string(),
            "Ship it?".to_string(),
        ],
        "100.6",
    )
    .await;
    let state = test_app_state();
    // The bot posts to the default channel id resolved from the mock's
    // response; slack_lifecycle_service(base_url, default_channel).
    let service = slack_lifecycle_service(server.base_url(), Some("C-default"));
    let run_id = fixtures::RUN_1;
    let settings = workflow_settings_with_run_notifications(
        r##"
[run.notifications.gates]
enabled = true
provider = "slack"
events = ["interview.started"]

[run.notifications.gates.slack]
channel = "#gates"
"##,
        Some("Revisor"),
    );
    let run_store =
        create_slack_notification_run(&state, run_id, settings, "revisor", Some("revisor")).await;
    let envelope = append_slack_notification_event(
        &run_store,
        run_id,
        &workflow_event::Event::InterviewStarted {
            question_id:     "q-1".to_string(),
            question:        "Ship it?".to_string(),
            stage:           "review".to_string(),
            question_type:   "yes_no".to_string(),
            options:         vec![],
            allow_freeform:  false,
            timeout_seconds: None,
            context_display: None,
            review_target:   None,
        },
    )
    .await;

    service.handle_event(state.as_ref(), &envelope, None).await;

    route_post.assert_async().await;
    bot_post.assert_async().await;
    assert_eq!(route_post.calls_async().await, 1);
    assert_eq!(bot_post.calls_async().await, 1);
    // The bot, not the route, tracked its message for later updates.
    assert_eq!(
        service
            .posted_messages
            .lock()
            .expect("posted messages lock poisoned")
            .len(),
        1
    );
}

#[tokio::test]
async fn slack_lifecycle_interview_started_without_matching_route_does_not_post() {
    let server = MockServer::start_async().await;
    let post = mock_slack_post(
        &server,
        vec!["Should we ship the fix?".to_string()],
        "100.4",
    )
    .await;
    let state = test_app_state();
    let service = slack_lifecycle_service(server.base_url(), None);
    let run_id = fixtures::RUN_1;
    // Route subscribes only to terminal events: the pending question
    // must not reach it.
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
    let run_store =
        create_slack_notification_run(&state, run_id, settings, "deploy", Some("deploy")).await;
    let envelope = append_slack_notification_event(
        &run_store,
        run_id,
        &workflow_event::Event::InterviewStarted {
            question_id:     "q-1".to_string(),
            question:        "Should we ship the fix?".to_string(),
            stage:           "review".to_string(),
            question_type:   "yes_no".to_string(),
            options:         vec![],
            allow_freeform:  false,
            timeout_seconds: None,
            context_display: None,
            review_target:   None,
        },
    )
    .await;

    service
        .handle_event(
            state.as_ref(),
            &envelope,
            Some("https://fabro.example/runs/run-1"),
        )
        .await;

    assert_eq!(
        post.calls_async().await,
        0,
        "no route matched interview.started, nothing may post"
    );
}
