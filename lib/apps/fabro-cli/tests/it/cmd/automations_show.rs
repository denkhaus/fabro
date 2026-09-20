use fabro_test::{fabro_snapshot, test_context};
use httpmock::MockServer;

fn automation_json(revision: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "conductor-fabro",
        "revision": revision,
        "name": "Conductor",
        "description": "Keeps the loop moving",
        "environment_id": "toolchain",
        "last_error": "scheduler failed to queue run",
        "target": { "kind": "git", "repo": "fabro-sh/fabro", "branch": "main" },
        "workflow": "conductor",
        "on_overlap": "skip",
        "triggers": [
            { "id": "manual", "type": "api", "enabled": true },
            {
                "id": "every-30",
                "type": "schedule",
                "enabled": true,
                "expression": "*/30 * * * *",
                "breaker": {
                    "signature": "api_transient|zai|rate_limited",
                    "consecutive_count": 2,
                    "last_run_id": "01M2ZSYQR4ZM8N6HD67TT3AT3X",
                    "paused_at": null
                }
            }
        ]
    })
}

#[test]
fn help() {
    let context = test_context!();
    let mut cmd = context.command();
    cmd.args(["automations", "show", "--help"]);
    fabro_snapshot!(context.filters(), cmd, @"
    success: true
    exit_code: 0
    ----- stdout -----
    Show one server automation

    Usage: fabro automations show [OPTIONS] <ID>

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
fn show_renders_triggers_breaker_and_next_fire() {
    let context = test_context!();
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method("GET")
            .path("/api/v1/automations/conductor-fabro");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(
                automation_json("1111111111111111111111111111111111111111111111111111111111111111")
                    .to_string(),
            );
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["automations", "show", "conductor-fabro"])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "conductor-fabro",
        "111111111111",
        "type=api enabled=true",
        "type=schedule enabled=true expression=*/30 * * * *",
        "next fire",
        "breaker",
        "count=2",
        "api_transient|zai|rate_limited",
        "01M2ZSYQR4ZM8N6HD67TT3AT3X",
        "scheduler failed to queue run",
    ] {
        assert!(
            stdout.contains(expected),
            "automations show should render {expected}: {stdout}"
        );
    }
    mock.assert();
}

#[test]
fn show_json_returns_the_definition() {
    let context = test_context!();
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method("GET")
            .path("/api/v1/automations/conductor-fabro");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(
                automation_json("1111111111111111111111111111111111111111111111111111111111111111")
                    .to_string(),
            );
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["--json", "automations", "show", "conductor-fabro"])
        .output()
        .expect("command should run");

    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json output");
    assert_eq!(value["id"], "conductor-fabro");
    assert_eq!(
        value["triggers"][1]["breaker"]["consecutive_count"],
        serde_json::json!(2)
    );
    mock.assert();
}
