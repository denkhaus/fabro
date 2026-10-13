//! Fork-only pin (fabro-71e6, sprint-35 arch finding N1): the CLI's
//! replace request drops the server-owned `model_providers` field a fire
//! learns (fabro-0611). The 2026-10-11 fix's only pin lived inline in the
//! upstream automations module; this file drives the PUBLIC command
//! surface (pause) against a fired automation, the exact production
//! break: every pause/unpause/breaker-reset was refused with
//! "unknown field `model_providers`" (observed on loop-fabro, 2026-10-11,
//! run 01M4NSBKV6ZBPAFFEFND65HEP9's window pause).

use fabro_test::test_context;
use httpmock::MockServer;

const REVISION_1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const REVISION_2: &str = "2222222222222222222222222222222222222222222222222222222222222222";

/// The automation as the server serves it AFTER a fired pass: carrying the
/// learned `model_providers` (fabro-0611) plus a stale `last_error`.
fn fired_automation() -> serde_json::Value {
    serde_json::json!({
        "id": "loop-fabro",
        "revision": REVISION_1,
        "name": "Loop",
        "description": null,
        "environment_id": "toolchain",
        "last_error": "old error",
        "model_providers": [{"provider": "zai", "model": "glm-5.3"}],
        "target": { "kind": "git", "repo": "denkhaus/fabro", "branch": "denkhaus" },
        "workflow": "loop",
        "on_overlap": "skip",
        "triggers": [
            { "id": "manual", "type": "api", "enabled": true },
            { "id": "every-30", "type": "schedule", "enabled": true, "expression": "*/30 * * * *" }
        ]
    })
}

#[test]
fn pausing_a_fired_automation_drops_the_learned_model_providers() {
    let context = test_context!();
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method("GET").path("/api/v1/automations/loop-fabro");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(fired_automation().to_string());
    });
    // The exact-body mock IS the pin: a request still carrying
    // `model_providers` (or `last_error`) does not match and the command
    // fails — the 2026-10-11 production break, from the public surface.
    let put = server.mock(|when, then| {
        when.method("PUT")
            .path("/api/v1/automations/loop-fabro")
            .header("if-match", REVISION_1)
            .json_body(serde_json::json!({
                "name": "Loop",
                "environment_id": "toolchain",
                "target": { "kind": "git", "repo": "denkhaus/fabro", "branch": "denkhaus" },
                "workflow": "loop",
                "on_overlap": "skip",
                "triggers": [
                    { "id": "manual", "type": "api", "enabled": true },
                    { "id": "every-30", "type": "schedule", "enabled": false, "expression": "*/30 * * * *" }
                ]
            }));
        then.status(200)
            .header("Content-Type", "application/json")
            .body(
                {
                    let mut value = fired_automation();
                    value["revision"] = serde_json::json!(REVISION_2);
                    let triggers = value["triggers"].as_array_mut().unwrap();
                    triggers[1]["enabled"] = serde_json::json!(false);
                    value
                }
                .to_string(),
            );
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["automations", "pause", "loop-fabro"])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The exact-body PUT really fired — the pin's contract.
    put.assert();
}
