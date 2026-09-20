use fabro_test::{fabro_snapshot, test_context};
use httpmock::MockServer;

fn environment_json(id: &str, provider: &str, docker: &str, revision: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "revision": revision,
        "provider": provider,
        "image": { "docker": docker, "dockerfile": null },
        "resources": { "cpu": null, "memory": null, "disk": null },
        "network": { "mode": "allow_all", "allow": [] },
        "lifecycle": {
            "preserve": false,
            "stop_on_terminal": true,
            "auto_stop": null
        },
        "labels": {},
        "env": {}
    })
}

#[test]
fn help() {
    let context = test_context!();
    let mut cmd = context.command();
    cmd.args(["env", "list", "--help"]);
    fabro_snapshot!(context.filters(), cmd, @"
    success: true
    exit_code: 0
    ----- stdout -----
    List server environments

    Usage: fabro env list [OPTIONS]

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
fn list_renders_environments_table() {
    let context = test_context!();
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method("GET").path("/api/v1/environments");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(
                serde_json::json!({
                    "data": [
                        environment_json(
                            "toolchain",
                            "docker",
                            "ghcr.io/denkhaus/fabro-toolchain:0123456789ab",
                            "1111111111111111111111111111111111111111111111111111111111111111"
                        ),
                        environment_json(
                            "local",
                            "local",
                            "-",
                            "2222222222222222222222222222222222222222222222222222222222222222"
                        ),
                    ],
                    "meta": { "total": 2 }
                })
                .to_string(),
            );
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["env", "list"])
        .output()
        .expect("command should run");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "ID",
        "PROVIDER",
        "IMAGE",
        "REVISION",
        "toolchain",
        "docker",
        "local",
    ] {
        assert!(
            stdout.contains(expected),
            "env list should show {expected}: {stdout}"
        );
    }
    assert!(
        stdout.contains("0123456789ab"),
        "env list should render the short revision: {stdout}"
    );
    mock.assert();
}

#[test]
fn list_json_returns_the_catalog() {
    let context = test_context!();
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method("GET").path("/api/v1/environments");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(
                serde_json::json!({
                    "data": [environment_json(
                        "toolchain",
                        "docker",
                        "ghcr.io/denkhaus/fabro-toolchain:0123456789ab",
                        "1111111111111111111111111111111111111111111111111111111111111111"
                    )],
                    "meta": { "total": 1 }
                })
                .to_string(),
            );
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["--json", "env", "list"])
        .output()
        .expect("command should run");

    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json output");
    assert_eq!(value[0]["id"], "toolchain");
    assert_eq!(
        value[0]["image"]["docker"],
        "ghcr.io/denkhaus/fabro-toolchain:0123456789ab"
    );
    mock.assert();
}
