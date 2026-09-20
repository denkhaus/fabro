use fabro_test::test_context;
use httpmock::MockServer;

const REVISION_1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const REVISION_2: &str = "2222222222222222222222222222222222222222222222222222222222222222";

fn automation_json(expression: &str) -> serde_json::Value {
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
                "expression": expression
            }
        ]
    })
}

#[test]
fn set_schedule_puts_new_expression_with_if_match_and_keeps_other_triggers() {
    let context = test_context!();
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method("GET")
            .path("/api/v1/automations/conductor-fabro");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(automation_json("*/30 * * * *").to_string());
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
                        "expression": "0 */2 * * *"
                    }
                ]
            }));
        then.status(200)
            .header("Content-Type", "application/json")
            .body(automation_json("0 */2 * * *").to_string());
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args([
            "automations",
            "set-schedule",
            "conductor-fabro",
            "--cron",
            "0 */2 * * *",
        ])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    put.assert();
}

#[test]
fn set_schedule_response_carries_new_revision() {
    let context = test_context!();
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method("GET")
            .path("/api/v1/automations/conductor-fabro");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(automation_json("*/30 * * * *").to_string());
    });
    let put = server.mock(|when, then| {
        when.method("PUT")
            .path("/api/v1/automations/conductor-fabro");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(
                serde_json::json!({
                    "id": "conductor-fabro",
                    "revision": REVISION_2,
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
                            "expression": "0 */2 * * *"
                        }
                    ]
                })
                .to_string(),
            );
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args([
            "automations",
            "set-schedule",
            "conductor-fabro",
            "--cron",
            "0 */2 * * *",
        ])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("111111111111 -> 222222222222"),
        "set-schedule should print old -> new revision: {stderr}"
    );
    put.assert();
}

#[test]
fn set_schedule_rejects_invalid_cron_before_any_write() {
    let context = test_context!();
    let server = MockServer::start();
    // No mocks registered: any request would fail. A valid client-side
    // rejection makes zero HTTP calls.
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args([
            "automations",
            "set-schedule",
            "conductor-fabro",
            "--cron",
            "not-a-cron",
        ])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid cron expression"),
        "set-schedule should reject the invalid expression client-side: {stderr}"
    );
}

#[test]
fn set_schedule_is_a_noop_when_expression_matches() {
    let context = test_context!();
    let server = MockServer::start();
    let get = server.mock(|when, then| {
        when.method("GET")
            .path("/api/v1/automations/conductor-fabro");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(automation_json("*/30 * * * *").to_string());
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args([
            "automations",
            "set-schedule",
            "conductor-fabro",
            "--cron",
            "*/30 * * * *",
        ])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("already set"),
        "set-schedule should report the no-op: {stderr}"
    );
    get.assert();
}
