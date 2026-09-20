use fabro_test::{fabro_snapshot, test_context};
use httpmock::MockServer;

const REVISION_1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
/// Automation whose breaker tripped: the schedule trigger is disabled and
/// carries breaker facts with `paused_at` set.
fn tripped_automation_json() -> serde_json::Value {
    serde_json::json!({
        "id": "conductor-fabro",
        "revision": REVISION_1,
        "name": "Conductor",
        "description": "Keeps the loop moving",
        "environment_id": "toolchain",
        "last_error": null,
        "target": { "kind": "git", "repo": "fabro-sh/fabro", "branch": "main" },
        "workflow": "conductor",
        "on_overlap": "skip",
        "triggers": [
            { "id": "manual", "type": "api", "enabled": true },
            {
                "id": "every-30",
                "type": "schedule",
                "enabled": false,
                "expression": "*/30 * * * *",
                "breaker": {
                    "signature": "api_transient|zai|rate_limited",
                    "consecutive_count": 3,
                    "last_run_id": "01M2ZSYQR4ZM8N6HD67TT3AT3X",
                    "paused_at": "2026-09-20T03:00:00Z"
                }
            }
        ]
    })
}

/// Automation with an enabled schedule trigger and no breaker facts.
fn clean_automation_json() -> serde_json::Value {
    serde_json::json!({
        "id": "conductor-fabro",
        "revision": REVISION_1,
        "name": "Conductor",
        "description": "Keeps the loop moving",
        "environment_id": "toolchain",
        "last_error": null,
        "target": { "kind": "git", "repo": "fabro-sh/fabro", "branch": "main" },
        "workflow": "conductor",
        "on_overlap": "skip",
        "triggers": [
            { "id": "manual", "type": "api", "enabled": true },
            {
                "id": "every-30",
                "type": "schedule",
                "enabled": true,
                "expression": "*/30 * * * *"
            }
        ]
    })
}

#[test]
fn help() {
    let context = test_context!();
    let mut cmd = context.command();
    cmd.args(["automations", "breaker", "reset", "--help"]);
    fabro_snapshot!(context.filters(), cmd, @"
    success: true
    exit_code: 0
    ----- stdout -----
    Unlatch a tripped breaker (no-op when clean)

    Usage: fabro automations breaker reset [OPTIONS] <ID>

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
fn reset_re_enables_the_paused_trigger_with_if_match() {
    let context = test_context!();
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method("GET")
            .path("/api/v1/automations/conductor-fabro");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(tripped_automation_json().to_string());
    });
    let put = server.mock(|when, then| {
        when.method("PUT")
            .path("/api/v1/automations/conductor-fabro")
            .header("if-match", REVISION_1)
            .json_body(serde_json::json!({
                "name": "Conductor",
                "description": "Keeps the loop moving",
                "environment_id": "toolchain",
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
                            "consecutive_count": 3,
                            "last_run_id": "01M2ZSYQR4ZM8N6HD67TT3AT3X",
                            "paused_at": "2026-09-20T03:00:00Z"
                        }
                    }
                ]
            }));
        then.status(200)
            .header("Content-Type", "application/json")
            .body(clean_automation_json().to_string());
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["automations", "breaker", "reset", "conductor-fabro"])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Reset breaker for automation conductor-fabro"),
        "breaker reset should report the unlatch: {stderr}"
    );
    put.assert();
}

#[test]
fn reset_is_a_clean_success_noop_when_breaker_is_clean() {
    let context = test_context!();
    let server = MockServer::start();
    let get = server.mock(|when, then| {
        when.method("GET")
            .path("/api/v1/automations/conductor-fabro");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(clean_automation_json().to_string());
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["automations", "breaker", "reset", "conductor-fabro"])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "clean breaker reset must be a clean success path: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("already clean"),
        "breaker reset should report the no-op: {stderr}"
    );
    get.assert();
}
