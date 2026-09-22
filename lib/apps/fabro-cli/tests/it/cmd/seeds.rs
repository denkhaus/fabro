//! `fabro seeds` skeleton behavior (fabro-088b): the parity surface is
//! visible in help, unwired subcommands refuse with the pending-binding
//! error, and the pinned format core round-trips a fixture store.
#![expect(
    clippy::disallowed_methods,
    reason = "fixture staging uses sync std::fs; test infrastructure, not a Tokio path"
)]

use std::fs;

use fabro_test::{fabro_snapshot, test_context};

#[test]
fn seeds_help_lists_the_sd_parity_surface() {
    let context = test_context!();
    let mut cmd = context.command();
    cmd.args(["seeds", "--help"]);
    fabro_snapshot!(context.filters(), cmd, @"success: true
exit_code: 0
----- stdout -----
Operate on the repository's seeds issue tracker (.seeds/, sd-compatible)

Usage: fabro seeds [OPTIONS] <COMMAND>

Commands:
  create  Create a new seed
  show    Show one seed by id
  list    List seeds
  ready   List seeds with resolved dependencies
  update  Update seed fields
  close   Close a seed
  dep     Manage seed dependencies
  prime   Print a priming prompt from open seeds
  search  Search seeds by keyword
  help    Print this message or the help of the given subcommand(s)

Options:
      --json              Output as JSON [env: FABRO_JSON=]
      --debug             Enable DEBUG-level logging (default is INFO) [env: FABRO_DEBUG=]
      --no-upgrade-check  Disable automatic upgrade check [env: FABRO_NO_UPGRADE_CHECK=true]
      --quiet             Suppress non-essential output [env: FABRO_QUIET=]
      --verbose           Enable verbose output [env: FABRO_VERBOSE=]
  -h, --help              Print help
----- stderr -----");
}

#[test]
fn unwired_seeds_list_refuses_with_pending_binding_error() {
    let context = test_context!();
    let mut cmd = context.command();
    cmd.args(["seeds", "list"]);
    fabro_snapshot!(context.filters(), cmd, @"success: false
exit_code: 1
----- stdout -----
----- stderr -----
  × `fabro seeds list` is not wired yet: the seeds command-layer binding is pending (fabro-088b); the tracker format core is already pinned in the workspace");
}

#[test]
fn pinned_seeds_core_round_trips_a_fixture_store() {
    let root = std::env::temp_dir().join(format!("fabro-seeds-fixture-{}", std::process::id()));
    let seeds_dir = root.join(".seeds");
    fs::create_dir_all(&seeds_dir).expect("fixture dir");
    fs::write(seeds_dir.join("config.yaml"), "project: fabro\n").expect("config fixture");
    fs::write(
        seeds_dir.join("issues.jsonl"),
        concat!(
            "{\"id\":\"fabro-0001\",\"title\":\"fixture\",\"status\":\"open\",",
            "\"type\":\"task\",\"priority\":2,\"labels\":[],\"blockedBy\":[],",
            "\"assignee\":null,\"createdAt\":\"2026-09-22T00:00:00.000Z\",",
            "\"updatedAt\":\"2026-09-22T00:00:00.000Z\",",
            "\"description\":\"fixture body\",\"custom_field\":\"must survive\"}\n",
        ),
    )
    .expect("issues fixture");

    let mut store = seeds::Store::open(&seeds_dir).expect("store opens fixture");
    assert_eq!(
        store.issue("fabro-0001").expect("fixture record").title(),
        "fixture"
    );
    store
        .issue_mut("fabro-0001")
        .expect("mutable record")
        .set_title("changed");
    store.save().expect("store saves");

    let rewritten = fs::read_to_string(seeds_dir.join("issues.jsonl")).expect("rewritten issues");
    assert!(
        rewritten.contains("\"title\":\"changed\""),
        "rewrite visible: {rewritten}"
    );
    assert!(
        rewritten.contains("\"custom_field\":\"must survive\""),
        "unknown fields survive the pinned core write: {rewritten}"
    );

    fs::remove_dir_all(&root).expect("cleanup");
}
