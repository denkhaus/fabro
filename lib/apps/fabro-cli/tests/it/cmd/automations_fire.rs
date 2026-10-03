use fabro_test::{fabro_snapshot, test_context};
use httpmock::MockServer;

use super::support::remote_run_summary_json;

#[test]
fn help() {
    let context = test_context!();
    let mut cmd = context.command();
    cmd.args(["automations", "fire", "--help"]);
    fabro_snapshot!(context.filters(), cmd, @"
    success: true
    exit_code: 0
    ----- stdout -----
    Fire an automation through its API trigger

    Usage: fabro automations fire [OPTIONS] <ID>

    Arguments:
      <ID>  Automation id

    Options:
          --json              Output as JSON [env: FABRO_JSON=]
          --debug             Enable DEBUG-level logging (default is INFO) [env: FABRO_DEBUG=]
          --no-upgrade-check  Disable automatic upgrade check [env: FABRO_NO_UPGRADE_CHECK=true]
          --quiet             Suppress non-essential output [env: FABRO_QUIET=]
          --verbose           Enable verbose output [env: FABRO_VERBOSE=]
      -h, --help              Print help
    ----- stderr -----
    ");
}

#[test]
fn fire_creates_a_run_through_the_api_trigger() {
    let context = test_context!();
    let server = MockServer::start();
    let post = server.mock(|when, then| {
        when.method("POST")
            .path("/api/v1/automations/conductor-fabro/runs");
        then.status(201)
            .header("Content-Type", "application/json")
            .body(
                remote_run_summary_json(
                    "01M2ZSYQR4ZM8N6HD67TT3AT3X",
                    "Conductor",
                    "conductor",
                    "Keep the loop moving",
                    &serde_json::json!({ "kind": "pending", "reason": "approval_required" }),
                    "2026-09-20T12:00:00Z",
                )
                .to_string(),
            );
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["automations", "fire", "conductor-fabro"])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Fired automation conductor-fabro -> run 01M2ZSYQR4ZM8N6HD67TT3AT3X"),
        "fire should report the created run: {stderr}"
    );
    post.assert();
}

#[test]
fn fire_json_returns_the_created_run() {
    let context = test_context!();
    let server = MockServer::start();
    let post = server.mock(|when, then| {
        when.method("POST")
            .path("/api/v1/automations/conductor-fabro/runs");
        then.status(201)
            .header("Content-Type", "application/json")
            .body(
                remote_run_summary_json(
                    "01M2ZSYQR4ZM8N6HD67TT3AT3X",
                    "Conductor",
                    "conductor",
                    "Keep the loop moving",
                    &serde_json::json!({ "kind": "pending", "reason": "approval_required" }),
                    "2026-09-20T12:00:00Z",
                )
                .to_string(),
            );
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["--json", "automations", "fire", "conductor-fabro"])
        .output()
        .expect("command should run");

    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json output");
    assert_eq!(value["id"], "01M2ZSYQR4ZM8N6HD67TT3AT3X");
    post.assert();
}
