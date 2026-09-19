//! The Docker provider for the workflow scenarios: an environment on
//! [`DOCKER_IMAGE`], on an isolated server.
//!
//! Petri serves every provider through a sandbox-driver plugin executable it
//! finds on `PATH` (`sandbox-driver-docker` here); CI installs the
//! executables at the rev the workspace pins, and a developer installs them
//! with
//! `cargo install --locked --git https://github.com/lithoscomputer/sandbox-driver --rev <rev> sandbox-driver-host sandbox-driver-docker`.
//! A scenario configured here runs against its own server so the environment
//! it creates never leaks into the shared session server.

#![expect(
    clippy::disallowed_methods,
    reason = "test setup reads the process environment for its opt-in gate and probes Docker synchronously"
)]
#![expect(
    clippy::print_stderr,
    reason = "a skipped scenario says why on the test's stderr"
)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use fabro_test::{TestContext, expect_reqwest_status};
use serde_json::json;

use crate::cmd::support::server_endpoint;

/// Set in CI so a missing executable or daemon fails the test instead of
/// skipping it.
const REQUIRE_ENV: &str = "FABRO_REQUIRE_SANDBOX_PLUGINS";
const DOCKER_IMAGE: &str = "buildpack-deps:noble";
const DOCKER_PLUGIN: &str = "sandbox-driver-docker";
/// The environment id the scenario selects with `--environment`.
pub(crate) const ENVIRONMENT: &str = "docker";

/// Point `context` at an isolated server with a Docker environment on
/// [`DOCKER_IMAGE`]. Returns the environment id, or `None` when the
/// prerequisites are missing and the test should skip.
pub(crate) fn configure(context: &mut TestContext) -> Option<&'static str> {
    let required = std::env::var_os(REQUIRE_ENV).is_some();
    if plugin_executable().is_none() {
        assert!(
            !required,
            "{REQUIRE_ENV} is set but {DOCKER_PLUGIN} is not on PATH"
        );
        eprintln!(
            "skipping: {DOCKER_PLUGIN} is not on PATH; install the sandbox-driver executables at \
             the rev Cargo.toml pins"
        );
        return None;
    }
    if !docker_image_available() {
        assert!(
            !required,
            "{REQUIRE_ENV} is set but no Docker daemon with {DOCKER_IMAGE} is available"
        );
        eprintln!("skipping: no Docker daemon with {DOCKER_IMAGE}");
        return None;
    }

    let storage_dir = context.temp_dir.join("docker-server-storage");
    let settings = format!(
        r#"[server.storage]
root = "{storage}"

[server.auth]
methods = ["dev-token"]
"#,
        storage = toml_path(&storage_dir),
    );
    context.write_home(".fabro/settings.toml", settings);
    context.isolated_server();
    create_environment(&context.storage_dir);
    Some(ENVIRONMENT)
}

/// The Docker plugin executable on `PATH`, when installed.
fn plugin_executable() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(DOCKER_PLUGIN))
        .find(|candidate| candidate.is_file())
}

fn docker_image_available() -> bool {
    Command::new("docker")
        .args(["image", "inspect", DOCKER_IMAGE])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn toml_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

fn create_environment(storage_dir: &Path) {
    let body = json!({
        "id": ENVIRONMENT,
        "provider": "docker",
        "image": { "docker": DOCKER_IMAGE, "dockerfile": null },
        "resources": { "cpu": null, "memory": null, "disk": null },
        "network": { "mode": "allow_all", "allow": [] },
        "lifecycle": { "preserve": false, "stop_on_terminal": true, "auto_stop": null },
        "labels": {},
        "env": {}
    });
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build")
        .block_on(async {
            let (client, base_url) =
                server_endpoint(storage_dir).expect("isolated server endpoint should exist");
            let response = client
                .post(format!("{base_url}/api/v1/environments"))
                .json(&body)
                .send()
                .await
                .expect("environment create request should send");
            if response.status() != fabro_http::StatusCode::CREATED {
                eprintln!("server log tail:\n{}", server_log_tail(storage_dir));
            }
            expect_reqwest_status(
                response,
                fabro_http::StatusCode::CREATED,
                "POST /api/v1/environments",
            )
            .await;
        });
}

/// Run a scenario; when it fails, print the isolated server's log first, since
/// the worker's stderr (and so a plugin's launch failure) lands only there
/// and the server root is removed when the context drops.
pub(crate) fn run_with_server_log(context: &TestContext, scenario: impl FnOnce()) {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(scenario));
    if let Err(panic) = outcome {
        eprintln!(
            "server log tail:\n{}",
            server_log_tail(&context.storage_dir)
        );
        std::panic::resume_unwind(panic);
    }
}

/// The last lines of the isolated server's log, for a failure message.
pub(crate) fn server_log_tail(storage_dir: &Path) -> String {
    let path = fabro_config::Storage::new(storage_dir)
        .runtime_directory()
        .log_path();
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return format!("(no server log at {})", path.display());
    };
    let lines: Vec<&str> = contents.lines().collect();
    let start = lines.len().saturating_sub(60);
    lines[start..].join("\n")
}
