use fabro_test::{fabro_snapshot, test_context};
use httpmock::MockServer;

fn automation_json(id: &str, revision: &str, last_error: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "revision": revision,
        "name": "Nightly dependency update",
        "description": null,
        "environment_id": "toolchain",
        "last_error": last_error,
        "target": { "kind": "git", "repo": "fabro-sh/fabro", "branch": "main" },
        "workflow": "dependency-update",
        "on_overlap": "skip",
        "triggers": [
            { "id": "manual", "type": "api", "enabled": true },
            {
                "id": "nightly",
                "type": "schedule",
                "enabled": true,
                "expression": "0 3 * * *",
                "breaker": {
                    "signature": "park|dependency-update|boundary",
                    "consecutive_count": 1,
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
    cmd.args(["automations", "list", "--help"]);
    fabro_snapshot!(context.filters(), cmd, @"
    success: true
    exit_code: 0
    ----- stdout -----
    List server automations

    Usage: fabro automations list [OPTIONS]

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
fn list_renders_automations_table() {
    let context = test_context!();
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method("GET").path("/api/v1/automations");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(
                serde_json::json!({
                    "data": [
                        automation_json(
                            "nightly-deps",
                            "1111111111111111111111111111111111111111111111111111111111111111",
                            None
                        ),
                        automation_json(
                            "architect",
                            "2222222222222222222222222222222222222222222222222222222222222222",
                            Some("scheduler failed to queue run")
                        )
                    ],
                    "meta": { "total": 2 }
                })
                .to_string(),
            );
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["automations", "list"])
        .output()
        .expect("command should run");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "ID",
        "NAME",
        "SCHEDULE",
        "ENABLED",
        "BREAKER",
        "LAST ERROR",
        "nightly-deps",
        "architect",
        "0 3 * * *",
        "1",
        "scheduler failed to queue run",
    ] {
        assert!(
            stdout.contains(expected),
            "automations list should show {expected}: {stdout}"
        );
    }
    mock.assert();
}

#[test]
fn list_json_returns_the_definitions() {
    let context = test_context!();
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method("GET").path("/api/v1/automations");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(
                serde_json::json!({
                    "data": [automation_json(
                        "nightly-deps",
                        "1111111111111111111111111111111111111111111111111111111111111111",
                        None
                    )],
                    "meta": { "total": 1 }
                })
                .to_string(),
            );
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["--json", "automations", "list"])
        .output()
        .expect("command should run");

    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json output");
    assert_eq!(value[0]["id"], "nightly-deps");
    assert_eq!(value[0]["triggers"][1]["expression"], "0 3 * * *");
    mock.assert();
}

#[test]
fn auto_alias_reaches_the_family() {
    let context = test_context!();
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method("GET").path("/api/v1/automations");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(
                serde_json::json!({
                    "data": [],
                    "meta": { "total": 0 }
                })
                .to_string(),
            );
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["auto", "list"])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    mock.assert();
}
