use chrono::{Duration, Utc};
use fabro_test::test_context;
use httpmock::MockServer;

use super::support::remote_run_summary_json;

const REVISION_1: &str = "1111111111111111111111111111111111111111111111111111111111111111";

fn automation_json(
    expression: &str,
    last_error: Option<&str>,
    breaker_count: u64,
) -> serde_json::Value {
    serde_json::json!({
        "id": "conductor-fabro",
        "revision": REVISION_1,
        "name": "Conductor",
        "description": null,
        "environment_id": "toolchain",
        "last_error": last_error,
        "target": { "kind": "git", "repo": "fabro-sh/fabro", "branch": "main" },
        "workflow": "conductor",
        "on_overlap": "skip",
        "triggers": [
            { "id": "manual", "type": "api", "enabled": true },
            {
                "id": "tick",
                "type": "schedule",
                "enabled": true,
                "expression": expression,
                "breaker": (breaker_count > 0).then(|| serde_json::json!({
                    "signature": "api_transient|zai|rate_limited",
                    "consecutive_count": breaker_count,
                    "last_run_id": "01M2ZSYQR4ZM8N6HD67TT3AT3X",
                    "paused_at": null
                }))
            }
        ]
    })
}

fn runs_response(last_success: Option<&str>) -> serde_json::Value {
    let data = last_success
        .map(|created_at| {
            vec![remote_run_summary_json(
                "01M2ZSYQR4ZM8N6HD67TT3AT3X",
                "Conductor",
                "conductor",
                "Keep the loop moving",
                &serde_json::json!({ "kind": "succeeded", "reason": "completed" }),
                created_at,
            )]
        })
        .unwrap_or_default();
    serde_json::json!({ "data": data, "meta": { "has_more": false } })
}

#[test]
fn status_flags_fire_drift_breaker_and_last_error() {
    let context = test_context!();
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method("GET").path("/api/v1/automations");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(
                serde_json::json!({
                    "data": [automation_json("*/5 * * * *", Some("scheduler failed"), 2)],
                    "meta": { "total": 1 }
                })
                .to_string(),
            );
    });
    // Five-minute cron with the last success in 2026 is always older than
    // the 2x-interval (10-minute) threshold, so the drift rule stays
    // deterministic regardless of when the test runs.
    server.mock(|when, then| {
        when.method("GET")
            .path("/api/v1/automations/conductor-fabro/runs");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(runs_response(Some("2026-01-01T00:00:00Z")).to_string());
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["automations", "status"])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("fire drift"),
        "status should flag the stale schedule: {stdout}"
    );
    assert!(
        stdout.contains("breaker count 2"),
        "status should flag the breaker: {stdout}"
    );
    assert!(
        stdout.contains("last_error: scheduler failed"),
        "status should flag last_error: {stdout}"
    );
}

#[test]
fn status_reports_healthy_automations_as_ok() {
    let context = test_context!();
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method("GET").path("/api/v1/automations");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(
                serde_json::json!({
                    "data": [automation_json("0 3 * * *", None, 0)],
                    "meta": { "total": 1 }
                })
                .to_string(),
            );
    });
    // Daily cron with a success one hour ago is well within the 2-day
    // threshold; computed relative to the test's real clock.
    let recent = (Utc::now() - Duration::hours(1)).to_rfc3339();
    server.mock(|when, then| {
        when.method("GET")
            .path("/api/v1/automations/conductor-fabro/runs");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(runs_response(Some(&recent)).to_string());
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["automations", "status"])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("ok"),
        "status should render the healthy automation as ok: {stdout}"
    );
    assert!(
        !stdout.contains("fire drift"),
        "a recent success must not flag drift: {stdout}"
    );
}
