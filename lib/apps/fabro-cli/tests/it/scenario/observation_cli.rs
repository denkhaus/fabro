//! The CLI door of the observation surface (denkhaus/work#1107 stage C,
//! ADR-0039): `fabro observation record` is the script-stage twin of the
//! `fabro_observation_record` agent tool — one endpoint, one
//! authorization (the run channel's worker principal), the tool's
//! validation verbatim. A script stage knows its stage identity because
//! the ENGINE binds it (FABRO_RUN_ID, FABRO_EXECUTION, FABRO_FIRING);
//! the CLI defaults from that environment, flags override, and a caller
//! with neither is taught the contract instead of guessing.
//!
//! The server half runs in this test process (test-support AppState,
//! production `spawn_for_run`), the CLI half in the compiled child —
//! the same two-process shape the work-channel acceptance pinned.

#![expect(
    clippy::disallowed_methods,
    reason = "This real CLI channel IT intentionally drives the compiled fabro binary as a child process."
)]

use std::process::{Command, Output, Stdio};

use fabro_server::test_support::{
    TestAppStateBuilder, create_test_run_with_target, spawn_for_run_for_test,
};
use fabro_types::RunId;

/// Run the compiled CLI against the channel socket — the sandbox stage
/// environment: the socket path as the server target, the run's
/// handshake token, the upgrade-check suppression, and the engine-bound
/// stage identity a script stage carries. Each identity member is its
/// own option so an arm can name a PARTIAL identity (the refusals' pin).
fn cli_over_channel(
    socket: &std::path::Path,
    token: &str,
    args: &[&str],
    run_id: Option<&str>,
    execution: Option<&str>,
    firing: Option<&str>,
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fabro"));
    command
        .args(args)
        .env("FABRO_SERVER", socket)
        .env(fabro_static::EnvVars::FABRO_RUN_CHANNEL_TOKEN, token)
        .env("FABRO_NO_UPGRADE_CHECK", "true")
        .env_remove("FABRO_RUN_CHANNEL_SOCK")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in [
        ("FABRO_RUN_ID", run_id),
        ("FABRO_EXECUTION", execution),
        ("FABRO_FIRING", firing),
    ] {
        match value {
            Some(value) => {
                command.env(name, value);
            }
            None => {
                command.env_remove(name);
            }
        }
    }
    command
        .spawn()
        .expect("the CLI child should start")
        .wait_with_output()
        .expect("the CLI should finish")
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The acceptance run: record through the channel with the engine-bound
/// identity, record with explicit flags, and every refusal taught —
/// missing identity, partial position, blank text, over-long text, and
/// a foreign run the run-scoped authorization refuses.
///
/// multi_thread: the channel's serve task lives in THIS runtime while
/// the CLI child blocks a worker with its synchronous wait.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_observation_cli_records_through_the_run_channel() {
    let state = TestAppStateBuilder::new().build();
    let run_id = create_test_run_with_target(&state, "denkhaus/fabro", "denkhaus").await;
    let (socket, token) = spawn_for_run_for_test(&state, run_id)
        .await
        .expect("the run's channel spawns");
    let run = run_id.to_string();

    // the happy path: the engine-bound identity from the environment
    let recorded = cli_over_channel(
        &socket,
        &token,
        &[
            "observation",
            "record",
            "--category",
            "note",
            "--text",
            "nothing durable to report",
        ],
        Some(run.as_str()),
        Some("0"),
        Some("3"),
    );
    assert!(
        recorded.status.success(),
        "observation record through the channel: {}",
        stderr_of(&recorded)
    );
    assert!(
        stdout_of(&recorded).contains("recorded at seq"),
        "the result names the record's seq: {}",
        stdout_of(&recorded)
    );

    // a painpoint through explicit flags: the flags override the
    // environment and the category reaches the far end
    let painpoint = cli_over_channel(
        &socket,
        &token,
        &[
            "observation",
            "record",
            "--category",
            "painpoint",
            "--text",
            "the diff touches Dockerfile paths",
            "--run-id",
            &run_id.to_string(),
            "--execution",
            "0",
            "--firing",
            "7",
        ],
        None,
        None,
        None,
    );
    assert!(
        painpoint.status.success(),
        "flags carry the identity when the environment cannot: {}",
        stderr_of(&painpoint)
    );
    assert!(
        stdout_of(&painpoint).contains("painpoint recorded at seq"),
        "the result names the category: {}",
        stdout_of(&painpoint)
    );

    // no identity from either source: the teaching error names both ways
    let no_identity = cli_over_channel(
        &socket,
        &token,
        &["observation", "record", "--category", "note", "--text", "x"],
        None,
        None,
        None,
    );
    assert!(
        !no_identity.status.success(),
        "a caller with no identity is refused"
    );
    assert!(
        stderr_of(&no_identity).contains("FABRO_RUN_ID"),
        "the refusal names the engine binding: {}",
        stderr_of(&no_identity)
    );

    // a partial position is refused naming both halves
    let partial = cli_over_channel(
        &socket,
        &token,
        &["observation", "record", "--category", "note", "--text", "x"],
        Some(run.as_str()),
        Some("0"),
        None,
    );
    assert!(
        !partial.status.success(),
        "a position of one half is refused"
    );
    assert!(
        stderr_of(&partial).contains("--firing"),
        "the refusal names the missing half: {}",
        stderr_of(&partial)
    );

    // blank text: the tool's empty-entry teaching, verbatim
    let blank = cli_over_channel(
        &socket,
        &token,
        &[
            "observation",
            "record",
            "--category",
            "note",
            "--text",
            "   ",
        ],
        Some(run.as_str()),
        Some("0"),
        Some("3"),
    );
    assert!(
        !blank.status.success(),
        "blank text is refused before any transport"
    );
    assert!(
        stderr_of(&blank).contains("nothing durable"),
        "the refusal teaches the explicit empty entry: {}",
        stderr_of(&blank)
    );

    // over-long text: the refusal names the limit and the actual length
    let long_text = "x".repeat(2001);
    let over_long = cli_over_channel(
        &socket,
        &token,
        &[
            "observation",
            "record",
            "--category",
            "note",
            "--text",
            &long_text,
        ],
        Some(run.as_str()),
        Some("0"),
        Some("3"),
    );
    assert!(
        !over_long.status.success(),
        "over-long text is refused before any transport"
    );
    assert!(
        stderr_of(&over_long).contains("2000"),
        "the refusal names the limit: {}",
        stderr_of(&over_long)
    );

    // a foreign run: the run channel's worker principal is scoped to
    // its own run, so the authorization refuses the append
    let foreign = RunId::new();
    let foreign_run = cli_over_channel(
        &socket,
        &token,
        &[
            "observation",
            "record",
            "--category",
            "note",
            "--text",
            "not this run's record",
            "--run-id",
            &foreign.to_string(),
            "--execution",
            "0",
            "--firing",
            "1",
        ],
        None,
        None,
        None,
    );
    assert!(
        !foreign_run.status.success(),
        "the channel cannot append to a foreign run"
    );
}
