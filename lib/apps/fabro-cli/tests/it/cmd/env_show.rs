use fabro_test::test_context;
use httpmock::MockServer;

fn environment_json(id: &str, docker: &str, revision: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "revision": revision,
        "provider": "docker",
        "image": { "docker": docker, "dockerfile": null },
        "resources": { "cpu": 4, "memory": "4GiB", "disk": "20GiB" },
        "network": { "mode": "allow_all", "allow": [] },
        "lifecycle": {
            "preserve": false,
            "stop_on_terminal": true,
            "auto_stop": "30m"
        },
        "labels": { "tier": "lab" },
        "env": {}
    })
}

#[test]
fn show_renders_environment_detail() {
    let context = test_context!();
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method("GET").path("/api/v1/environments/toolchain");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(
                environment_json(
                    "toolchain",
                    "ghcr.io/denkhaus/fabro-toolchain:0123456789ab",
                    "1111111111111111111111111111111111111111111111111111111111111111",
                )
                .to_string(),
            );
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["env", "show", "toolchain"])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "toolchain",
        "ghcr.io/denkhaus/fabro-toolchain:0123456789ab",
        "cpu=4",
        "preserve=false",
        "tier=lab",
    ] {
        assert!(
            stdout.contains(expected),
            "env show should render {expected}: {stdout}"
        );
    }
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("revision 111111111111"),
        "env show should print the short revision"
    );
    mock.assert();
}

#[test]
fn show_json_returns_the_environment() {
    let context = test_context!();
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method("GET").path("/api/v1/environments/toolchain");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(
                environment_json(
                    "toolchain",
                    "ghcr.io/denkhaus/fabro-toolchain:0123456789ab",
                    "1111111111111111111111111111111111111111111111111111111111111111",
                )
                .to_string(),
            );
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["--json", "env", "show", "toolchain"])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json output");
    assert_eq!(value["id"], "toolchain");
    assert_eq!(
        value["revision"],
        "1111111111111111111111111111111111111111111111111111111111111111"
    );
    mock.assert();
}

#[test]
fn show_missing_environment_fails() {
    let context = test_context!();
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method("GET").path("/api/v1/environments/missing");
        then.status(404)
            .header("Content-Type", "application/json")
            .body(
                serde_json::json!({
                    "errors": [{ "detail": "environment not found: missing" }]
                })
                .to_string(),
            );
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["env", "show", "missing"])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("environment not found"),
        "env show 404 should surface the server error: {stderr}"
    );
}
