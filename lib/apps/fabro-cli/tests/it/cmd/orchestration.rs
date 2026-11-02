//! CLI contract tests for the orchestration doors (ADR-0039 stage D,
//! denkhaus/work#1107): the `fabro run mark` / `fabro run queue` verb
//! grammar, the workflow-positional guard the subcommands forced into a
//! runtime check, and the read verbs' refusal teaching. The server-side
//! behavior (authorization matrix, queue anti-join) is pinned in
//! fabro-server's api::orchestration ITs; the channel end-to-end drive
//! lands with the adjudicator fast-path probe (stage D part 2).
//!
//! Each arm drives the COMPILED binary against a dead server (the
//! probe-28 discipline): a connect refusal proves the argv left the
//! parser intact; a clap reject would say "unexpected argument".

//! refusal proves the request leaves with the right shape (a connect
//! refusal, never a clap parse error).

#![expect(
    clippy::disallowed_methods,
    reason = "These CLI contract tests intentionally drive the compiled fabro binary as a synchronous child process."
)]

use std::process::{Command, Output, Stdio};

fn fabro(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fabro"))
        .args(args)
        .env("FABRO_NO_UPGRADE_CHECK", "true")
        .env("FABRO_SERVER", "http://127.0.0.1:1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the CLI child should start")
        .wait_with_output()
        .expect("the CLI should finish")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The verbs parse and reach TRANSPORT against a dead server — a clap
/// reject would print "unexpected argument" or "usage", a connect
/// refusal proves the argv left the parser intact (the probe-28 class).
#[test]
fn run_mark_reaches_transport_not_a_parse_error() {
    let output = fabro(&["run", "mark", "01M5844J1FDD", "merge=enabled"]);
    assert!(
        !stderr_of(&output).contains("unexpected argument"),
        "the mark argv must parse: {}",
        stderr_of(&output)
    );
    assert!(
        !output.status.success(),
        "a dead server must fail the mark call"
    );
    assert!(
        stderr_of(&output).contains("error"),
        "the transport refusal is visible: {}",
        stderr_of(&output)
    );
}

/// The `<field>=<verdict>` grammar refuses unknown shapes with the
/// accepted forms named.
#[test]
fn run_mark_refusals_teach_the_grammar() {
    for (argv, needle) in [
        (
            &["run", "mark", "01M5844J1FDD", "merge"][..],
            "merge=enabled",
        ),
        (
            &["run", "mark", "01M5844J1FDD", "title=enabled"][..],
            "closable",
        ),
        (
            &["run", "mark", "01M5844J1FDD", "merge=pending"][..],
            "never a write",
        ),
    ] {
        let output = fabro(argv);
        assert!(
            !output.status.success(),
            "the malformed shape `{}` must fail",
            argv.last().expect("the shape argument")
        );
        assert!(
            stderr_of(&output).contains(needle),
            "the refusal names `{needle}`: {}",
            stderr_of(&output)
        );
    }
}

/// The workflow positional became a runtime guard when the verbs moved
/// under `fabro run`: `fabro run` with neither workflow nor verb
/// teaches both shapes.
#[test]
fn run_without_workflow_or_verb_teaches_both_shapes() {
    let output = fabro(&["run"]);
    assert!(!output.status.success());
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("the workflow is required"),
        "the guard names the workflow: {stderr}"
    );
    assert!(
        stderr.contains("`fabro run mark`"),
        "the guard names the verb subcommands: {stderr}"
    );
}

/// The queue's field flag refuses unknown fields with the two fields
/// named; the happy argv reaches transport.
#[test]
fn run_queue_field_refusal_and_transport() {
    let refused = fabro(&["run", "queue", "--field", "bogus"]);
    assert!(!refused.status.success());
    assert!(
        stderr_of(&refused).contains("merge"),
        "the refusal names the fields: {}",
        stderr_of(&refused)
    );

    let transport = fabro(&["run", "queue"]);
    assert!(!transport.status.success());
    assert!(
        !stderr_of(&transport).contains("unexpected argument"),
        "the queue argv must parse: {}",
        stderr_of(&transport)
    );
}

/// The observation read door: a non-run-id refuses before transport;
/// the happy argv reaches transport.
#[test]
fn observation_list_validates_the_run_id() {
    let refused = fabro(&["observation", "list", "--run-id", "not-a-run-id"]);
    assert!(!refused.status.success());
    assert!(
        stderr_of(&refused).contains("is not a run id"),
        "the refusal names the bad id: {}",
        stderr_of(&refused)
    );

    let transport = fabro(&["observation", "list", "--run-id", "01M5844J1FDD"]);
    assert!(!transport.status.success());
}

/// The radius verb requires the project and validates nothing else
/// client-side — the server owns item resolution.
#[test]
fn work_radius_reaches_transport() {
    let output = fabro(&["work", "radius", "--project", "fabro", "denkhaus/work#1"]);
    assert!(!output.status.success());
    assert!(
        !stderr_of(&output).contains("unexpected argument"),
        "the radius argv must parse: {}",
        stderr_of(&output)
    );
}
