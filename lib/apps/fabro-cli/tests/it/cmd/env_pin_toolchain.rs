use fabro_test::test_context;
use httpmock::MockServer;

const REVISION_1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const REVISION_2: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const TAG_SHA12: &str = "0123456789ab";
const SERVER_SHA: &str = "0123456789abcdef0123456789abcdef01234567";
const PINNED_IMAGE: &str = "ghcr.io/denkhaus/fabro-toolchain:0123456789ab";
const OLD_IMAGE: &str = "ghcr.io/denkhaus/fabro-toolchain:fedcba987654";

fn environment_json(docker: &str, revision: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "toolchain",
        "revision": revision,
        "provider": "docker",
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

fn mock_system_info<'a>(server: &'a MockServer, git_sha: Option<&'a str>) -> httpmock::Mock<'a> {
    server.mock(|when, then| {
        when.method("GET").path("/api/v1/system/info");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(
                serde_json::json!({ "git_sha": git_sha, "version": "0.361.0-nightly.0" })
                    .to_string(),
            );
    })
}

fn mock_get_environment<'a>(
    server: &'a MockServer,
    docker: &'a str,
    revision: &'a str,
) -> httpmock::Mock<'a> {
    server.mock(|when, then| {
        when.method("GET").path("/api/v1/environments/toolchain");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(environment_json(docker, revision).to_string());
    })
}

#[test]
fn pin_toolchain_pins_after_parity_verification() {
    let context = test_context!();
    let server = MockServer::start();
    mock_system_info(&server, Some(SERVER_SHA));
    mock_get_environment(&server, OLD_IMAGE, REVISION_1);
    let put = server.mock(|when, then| {
        when.method("PUT")
            .path("/api/v1/environments/toolchain")
            .header("if-match", REVISION_1)
            .json_body(serde_json::json!({
                "provider": "docker",
                "image": { "docker": PINNED_IMAGE, "dockerfile": null },
                "resources": { "cpu": null, "memory": null, "disk": null },
                "network": { "mode": "allow_all", "allow": [] },
                "lifecycle": {
                    "preserve": false,
                    "stop_on_terminal": true,
                    "auto_stop": null
                },
                "labels": {},
                "env": {}
            }));
        then.status(200)
            .header("Content-Type", "application/json")
            .body(environment_json(PINNED_IMAGE, REVISION_2).to_string());
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["env", "pin-toolchain", "--tag", TAG_SHA12])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&format!("{OLD_IMAGE} -> {PINNED_IMAGE}")),
        "pin should print old -> new image: {stderr}"
    );
    put.assert();
}

#[test]
fn pin_toolchain_is_idempotent_when_already_pinned() {
    let context = test_context!();
    let server = MockServer::start();
    mock_system_info(&server, Some(SERVER_SHA));
    mock_get_environment(&server, PINNED_IMAGE, REVISION_1);
    let put = server.mock(|when, then| {
        when.method("PUT").path("/api/v1/environments/toolchain");
        then.status(500);
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["env", "pin-toolchain", "--tag", TAG_SHA12])
        .output()
        .expect("command should run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("already pinned"),
        "already-pinned run should say so: {stderr}"
    );
    put.assert_calls(0);
}

#[test]
fn pin_toolchain_aborts_on_deployed_server_tag_mismatch() {
    let context = test_context!();
    let server = MockServer::start();
    mock_system_info(&server, Some("fedcba9876543210fedcba9876543210fedcba98"));
    mock_get_environment(&server, OLD_IMAGE, REVISION_1);
    let put = server.mock(|when, then| {
        when.method("PUT").path("/api/v1/environments/toolchain");
        then.status(200);
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["env", "pin-toolchain", "--tag", TAG_SHA12])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("deployed-server/tag mismatch"),
        "mismatch should name the failure: {stderr}"
    );
    assert!(
        stderr.contains(TAG_SHA12) && stderr.contains("fedcba987654"),
        "mismatch should name both the tag and the deployed server sha: {stderr}"
    );
    put.assert_calls(0);
}

#[test]
fn pin_toolchain_aborts_when_server_reports_no_build_sha() {
    let context = test_context!();
    let server = MockServer::start();
    mock_system_info(&server, None);
    let put = server.mock(|when, then| {
        when.method("PUT").path("/api/v1/environments/toolchain");
        then.status(200);
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["env", "pin-toolchain", "--tag", TAG_SHA12])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("did not report a build sha"),
        "missing build sha should fail closed: {stderr}"
    );
    put.assert_calls(0);
}

#[test]
fn pin_toolchain_rejects_non_hex_tags() {
    let context = test_context!();
    let server = MockServer::start();
    let put = server.mock(|when, then| {
        when.method("PUT").path("/api/v1/environments/toolchain");
        then.status(200);
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["env", "pin-toolchain", "--tag", "not-a-sha!"])
        .output()
        .expect("command should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("12-character lowercase hex"),
        "invalid tag should be rejected with a clear error: {stderr}"
    );
    put.assert_calls(0);
}

#[test]
fn pin_toolchain_requires_exactly_one_tag_source() {
    let context = test_context!();
    let neither = context
        .command()
        .args(["env", "pin-toolchain"])
        .output()
        .expect("command should run");
    assert!(!neither.status.success());

    let both = context
        .command()
        .args([
            "env",
            "pin-toolchain",
            "--tag",
            TAG_SHA12,
            "--from-run-images",
        ])
        .output()
        .expect("command should run");
    assert!(!both.status.success());
}
