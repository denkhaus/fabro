use fabro_test::test_context;
use httpmock::MockServer;

const REVISION_1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const REVISION_2: &str = "2222222222222222222222222222222222222222222222222222222222222222";

fn automation_json(schedule_enabled: bool) -> serde_json::Value {
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
                "enabled": schedule_enabled,
                "expression": "*/30 * * * *"
            }
        ]
    })
}

fn replaced_automation(schedule_enabled: bool) -> serde_json::Value {
    let mut value = automation_json(schedule_enabled);
    value["revision"] = serde_json::json!(REVISION_2);
    value
}

#[test]
fn pause_disables_the_schedule_trigger_and_keeps_the_api_trigger() {
    let context = test_context!();
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method("GET")
            .path("/api/v1/automations/conductor-fabro");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(automation_json(true).to_string());
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
                        "enabled": false,
                        "expression": "*/30 * * * *"
                    }
                ]
            }));
        then.status(200)
            .header("Content-Type", "application/json")
            .body(replaced_automation(false).to_string());
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["automations", "pause", "conductor-fabro"])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Paused automation conductor-fabro"),
        "pause should report the change: {stderr}"
    );
    put.assert();
}

#[test]
fn unpause_round_trips_the_trigger_without_dropping_it() {
    let context = test_context!();
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method("GET")
            .path("/api/v1/automations/conductor-fabro");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(automation_json(false).to_string());
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
                        "expression": "*/30 * * * *"
                    }
                ]
            }));
        then.status(200)
            .header("Content-Type", "application/json")
            .body(replaced_automation(true).to_string());
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["automations", "unpause", "conductor-fabro"])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Unpaused automation conductor-fabro"),
        "unpause should report the change: {stderr}"
    );
    put.assert();
}

#[test]
fn pause_is_a_noop_when_already_paused() {
    let context = test_context!();
    let server = MockServer::start();
    let get = server.mock(|when, then| {
        when.method("GET")
            .path("/api/v1/automations/conductor-fabro");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(automation_json(false).to_string());
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["automations", "pause", "conductor-fabro"])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("already paused"),
        "pause should report the no-op: {stderr}"
    );
    get.assert();
}
