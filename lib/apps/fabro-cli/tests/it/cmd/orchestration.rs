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
//! parser intact; a clap reject would say "unexpected argument". The
//! read doors additionally pin their `--json` STDOUT SHAPE against a
//! mock server — the adjudicator fast path parses that stdout
//! (`data`/`observations`/`radius`), so a shape drift is a silent
//! empty read in production, not a test failure.

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

/// Run the compiled binary with a complete argv (the caller seats
/// `--server` in each facade's grammatically correct position — the
/// flag is not global: `observation`/`work` take it at the facade
/// level, `run queue` on the verb) and parse stdout as the JSON the
/// door's consumers read.
fn json_on(argv: &[&str]) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_fabro"))
        .args(argv)
        .env("FABRO_NO_UPGRADE_CHECK", "true")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the CLI child should start")
        .wait_with_output()
        .expect("the CLI should finish");
    assert!(
        output.status.success(),
        "the door read must succeed: {}",
        stderr_of(&output)
    );
    serde_json::from_slice(&output.stdout).expect("the door prints JSON on stdout")
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

/// A blank `--workflow` is refused with the two shapes taught (door
/// symmetry: the tool twin refuses the same value — a blank filter is a
/// caller accident, never a read-everything wish).
#[test]
fn run_queue_refuses_a_blank_workflow_filter() {
    let output = fabro(&["run", "queue", "--workflow", "  "]);
    assert!(!output.status.success());
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("--workflow is blank"),
        "the refusal names the blank flag: {stderr}"
    );
    assert!(
        stderr.contains("drop"),
        "the refusal teaches the fix (drop the flag): {stderr}"
    );
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

/// The settled-verdict filter (stage E): the closing-pass argv parses
/// against the real grammar, a bogus verdict refuses naming the two
/// settled values, and pending/parked are refused as filter values —
/// they ARE the default queue, not a filter.
#[test]
fn run_queue_verdict_filter_parses_and_refuses_teaching() {
    let transport = fabro(&[
        "run",
        "queue",
        "--field",
        "closable",
        "--verdict",
        "enabled",
    ]);
    assert!(!transport.status.success());
    assert!(
        !stderr_of(&transport).contains("unexpected argument"),
        "the closing-pass argv must parse: {}",
        stderr_of(&transport)
    );

    let bogus = fabro(&["run", "queue", "--verdict", "bogus"]);
    assert!(!bogus.status.success());
    let stderr = stderr_of(&bogus);
    assert!(
        stderr.contains("enabled") && stderr.contains("refused"),
        "the refusal names the settled verdicts: {stderr}"
    );

    let pending = fabro(&["run", "queue", "--verdict", "pending"]);
    assert!(
        !pending.status.success(),
        "pending is the default queue, not a filter value"
    );
    assert!(
        stderr_of(&pending).contains("default"),
        "the refusal teaches that pending/parked are the default anti-join: {}",
        stderr_of(&pending)
    );
}

/// The other-field requirement (work#1126): the merger worklist argv
/// parses; every malformed shape refuses with a teaching message; the
/// same-field and verdict-combination arms refuse too.
#[test]
fn run_queue_requires_filter_parses_and_refuses_teaching() {
    let transport = fabro(&[
        "run",
        "queue",
        "--field",
        "closable",
        "--requires",
        "merge=enabled",
    ]);
    assert!(!transport.status.success());
    assert!(
        !stderr_of(&transport).contains("unexpected argument"),
        "the merger worklist argv must parse: {}",
        stderr_of(&transport)
    );

    let malformed = fabro(&["run", "queue", "--requires", "merge"]);
    assert!(!malformed.status.success());
    let stderr = stderr_of(&malformed);
    assert!(
        stderr.contains("field=verdict"),
        "the refusal names the pair shape: {stderr}"
    );

    let bogus_field = fabro(&["run", "queue", "--requires", "bogus=enabled"]);
    assert!(!bogus_field.status.success());
    assert!(
        stderr_of(&bogus_field).contains("orchestration field"),
        "the refusal names the fields: {}",
        stderr_of(&bogus_field)
    );

    let bogus_verdict = fabro(&["run", "queue", "--requires", "merge=bogus"]);
    assert!(!bogus_verdict.status.success());
    assert!(
        stderr_of(&bogus_verdict).contains("enabled")
            && stderr_of(&bogus_verdict).contains("refused"),
        "the refusal names the settled verdicts: {}",
        stderr_of(&bogus_verdict)
    );

    let same_field = fabro(&[
        "run",
        "queue",
        "--field",
        "closable",
        "--requires",
        "closable=enabled",
    ]);
    assert!(!same_field.status.success());
    assert!(
        stderr_of(&same_field).contains("OTHER field"),
        "the same-field refusal teaches the requirement's direction: {}",
        stderr_of(&same_field)
    );

    let combined = fabro(&[
        "run",
        "queue",
        "--field",
        "closable",
        "--verdict",
        "enabled",
        "--requires",
        "merge=enabled",
    ]);
    assert!(!combined.status.success());
    assert!(
        stderr_of(&combined).contains("anti-join"),
        "the combination refusal teaches which read owns the requirement: {}",
        stderr_of(&combined)
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

/// The comment READ verb (ADR-0039 stage D part 2,
/// denkhaus/work#1107): the answer channel's CLI twin parses and reaches
/// transport — same shape as the radius read, same server-owned
/// resolution.
#[test]
fn work_comments_reaches_transport() {
    let output = fabro(&["work", "comments", "--project", "fabro", "denkhaus/work#1"]);
    assert!(!output.status.success());
    assert!(
        !stderr_of(&output).contains("unexpected argument"),
        "the comments argv must parse: {}",
        stderr_of(&output)
    );
}

// --- The read doors' `--json` stdout shape ---------------------------------
// The adjudicator fast path (`.fabro/workflows/adjudicator/scripts/
// fast-path.nu`) parses the CLI's stdout directly: `data` off the
// queue, `observations` off the list, `radius` off the radius read —
// and the work-channel shim serves exactly these shapes to the smoke.
// A record/array drift on either side empties a production read
// WITHOUT failing anything (the fast path's parse fallback is `[]`),
// so the contract is pinned against the compiled binary here.

/// The observation list prints the `RunObservationList` RECORD, never a
/// bare array — the shape bug this test was born red against: the fast
/// path's `read-observations` reads `.observations`, and a bare array
/// silently yields zero facts (every run would look painpoint-free).
#[test]
fn observation_list_json_is_the_record_the_fast_path_parses() {
    let server = httpmock::MockServer::start();
    server.mock(|when, then| {
        when.method(httpmock::Method::GET)
            .path("/api/v1/runs/01M5844J1FDD00000000000000/observations");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"observations":[{"seq":1,"category":"note","text":"a fact"}]}"#);
    });
    let url = server.base_url();
    let stdout = json_on(&[
        "observation",
        "--server",
        &url,
        "list",
        "--run-id",
        "01M5844J1FDD00000000000000",
        "--json",
    ]);
    let observations = stdout
        .get("observations")
        .and_then(serde_json::Value::as_array)
        .unwrap_or_else(|| {
            panic!("stdout is the {{observations}} record, not a bare array: {stdout}")
        });
    assert_eq!(
        observations.len(),
        1,
        "the record carries the mock row: {stdout}"
    );
}

/// The queue prints the `RunQueueList` record (`data` + `meta`) — the
/// fast path's `entries-from` reads `.data`.
#[test]
fn run_queue_json_is_the_record_the_fast_path_parses() {
    let server = httpmock::MockServer::start();
    server.mock(|when, then| {
        when.method(httpmock::Method::GET)
            .path("/api/v1/runs/queue");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"data":[],"meta":{"has_more":false}}"#);
    });
    let url = server.base_url();
    let stdout = json_on(&["run", "queue", "--json", "--server", &url]);
    assert!(
        stdout.get("data").is_some_and(serde_json::Value::is_array),
        "stdout carries the data array the fast path reads: {stdout}"
    );
    assert!(
        stdout.get("meta").is_some(),
        "stdout carries the pagination meta: {stdout}"
    );
}

/// The radius read prints the `WorkItemRadiusResponse` record (`item` +
/// `radius`) — the fast path's `read-radius` reads `.radius`.
#[test]
fn work_radius_json_is_the_record_the_fast_path_parses() {
    let server = httpmock::MockServer::start();
    server.mock(|when, then| {
        when.method(httpmock::Method::GET)
            .path("/api/v1/projects/fabro/work-items/denkhaus%2Fwork%237/radius");
        then.status(200)
            .header("content-type", "application/json")
            .body(
                r#"{"item":{"itemKey":"denkhaus/work#7","title":"t","body":"b","state":"open","labels":[],"priority":2,"url":"https://example.invalid/i/7"},"radius":[]}"#,
            );
    });
    let url = server.base_url();
    let stdout = json_on(&[
        "work",
        "--server",
        &url,
        "radius",
        "--project",
        "fabro",
        "denkhaus/work#7",
        "--json",
    ]);
    assert!(
        stdout
            .get("radius")
            .is_some_and(serde_json::Value::is_array),
        "stdout carries the radius array the fast path reads: {stdout}"
    );
    assert_eq!(
        stdout.get("item").and_then(|item| item.get("itemKey")),
        Some(&serde_json::json!("denkhaus/work#7")),
        "stdout names the item the radius belongs to: {stdout}"
    );
}

/// The comment read prints the `WorkItemCommentList` record (`comments`)
/// — the answer channel's wire shape, what the parked-fork reader
/// consumes.
#[test]
fn work_comments_json_is_the_record_shape() {
    let server = httpmock::MockServer::start();
    server.mock(|when, then| {
        when.method(httpmock::Method::GET)
            .path("/api/v1/projects/fabro/work-items/denkhaus%2Fwork%237/comments");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"comments":[{"author":"denkhaus","body":"the answer"}]}"#);
    });
    let url = server.base_url();
    let stdout = json_on(&[
        "work",
        "--server",
        &url,
        "comments",
        "--project",
        "fabro",
        "denkhaus/work#7",
        "--json",
    ]);
    let comments = stdout
        .get("comments")
        .and_then(serde_json::Value::as_array)
        .unwrap_or_else(|| panic!("stdout is the {{comments}} record: {stdout}"));
    assert_eq!(
        comments.first().and_then(|c| c.get("author")),
        Some(&serde_json::json!("denkhaus")),
        "the record carries the comment rows: {stdout}"
    );
}
