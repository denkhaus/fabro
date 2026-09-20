use fabro_test::{fabro_snapshot, test_context};
use httpmock::MockServer;

use super::support::remote_run_summary_json;

#[test]
fn help() {
    let context = test_context!();
    let mut cmd = context.command();
    cmd.args(["automations", "runs", "--help"]);
    fabro_snapshot!(context.filters(), cmd, @"
    success: true
    exit_code: 0
    ----- stdout -----
    List the runs an automation created

    Usage: fabro automations runs [OPTIONS] <ID>

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
fn runs_renders_fire_history() {
    let context = test_context!();
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method("GET")
            .path("/api/v1/automations/conductor-fabro/runs");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(
                serde_json::json!({
                    "data": [
                        remote_run_summary_json(
                            "01M2ZSYQR4ZM8N6HD67TT3AT3X",
                            "Conductor",
                            "conductor",
                            "Keep the loop moving",
                            &serde_json::json!({ "kind": "succeeded", "reason": "completed" }),
                            "2026-09-20T03:00:00Z"
                        ),
                        remote_run_summary_json(
                            "01M2ZHYDC6JB00N6HD67TT3AT3",
                            "Conductor",
                            "conductor",
                            "Keep the loop moving",
                            &serde_json::json!({ "kind": "failed", "reason": "workflow_error" }),
                            "2026-09-19T03:00:00Z"
                        )
                    ],
                    "meta": { "has_more": false }
                })
                .to_string(),
            );
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["automations", "runs", "conductor-fabro"])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in ["RUN", "STATUS", "CREATED", "01M2ZSYQR4ZM8N6HD67TT3AT3X"] {
        assert!(
            stdout.contains(expected),
            "automations runs should render {expected}: {stdout}"
        );
    }
    mock.assert();
}

#[test]
fn runs_json_returns_the_history() {
    let context = test_context!();
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method("GET")
            .path("/api/v1/automations/conductor-fabro/runs");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(
                serde_json::json!({
                    "data": [remote_run_summary_json(
                        "01M2ZSYQR4ZM8N6HD67TT3AT3X",
                        "Conductor",
                        "conductor",
                        "Keep the loop moving",
                        &serde_json::json!({ "kind": "succeeded", "reason": "completed" }),
                        "2026-09-20T03:00:00Z"
                    )],
                    "meta": { "has_more": false }
                })
                .to_string(),
            );
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["--json", "automations", "runs", "conductor-fabro"])
        .output()
        .expect("command should run");

    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json output");
    assert_eq!(value[0]["id"], "01M2ZSYQR4ZM8N6HD67TT3AT3X");
    mock.assert();
}
