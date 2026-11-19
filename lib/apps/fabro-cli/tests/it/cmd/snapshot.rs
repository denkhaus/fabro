//! `fabro snapshot <run>` — the CLI read door over a run's snapshot
//! repository: the exported bundle's commits must be recoverable through
//! `git fetch` alone, the salvage flow the command exists for.

#![expect(
    clippy::disallowed_methods,
    reason = "integration tests stage git fixtures with sync std::process and std::fs"
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use fabro_config::Storage;
use fabro_test::{fabro_snapshot, test_context};
use fabro_types::RunId;

use super::support::{run_state, setup_completed_dry_run};

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

#[test]
fn help() {
    let context = test_context!();
    let mut cmd = context.command();
    cmd.args(["snapshot", "--help"]);
    fabro_snapshot!(context.filters(), cmd, @"
    success: true
    exit_code: 0
    ----- stdout -----
    Export a run's snapshot repository as a git bundle

    Usage: fabro snapshot [OPTIONS] <RUN>

    Arguments:
      <RUN>  Run ID prefix or workflow name

    Options:
          --json              Output as JSON [env: FABRO_JSON=]
          --server <SERVER>   Fabro server target: http(s) URL or absolute Unix socket path [env: FABRO_SERVER=]
          --debug             Enable DEBUG-level logging (default is INFO) [env: FABRO_DEBUG=]
      -o, --output <OUTPUT>   Output file for the git bundle (default: fabro-snapshot-<run>.bundle in the current directory) [aliases: --out]
          --no-upgrade-check  Disable automatic upgrade check [env: FABRO_NO_UPGRADE_CHECK=true]
          --quiet             Suppress non-essential output [env: FABRO_QUIET=]
          --verbose           Enable verbose output [env: FABRO_VERBOSE=]
      -h, --help              Print help
    ----- stderr -----
    ");
}

/// The one snapshot repository a single-workspace run publishes, from the
/// server's scratch for the run.
fn snapshot_repository(storage_dir: &Path, run_id: &str) -> PathBuf {
    let run_id: RunId = run_id.parse().expect("the run id parses");
    let snapshots = Storage::new(storage_dir)
        .run_scratch(&run_id)
        .root()
        .join("petri")
        .join("snapshots");
    let mut repositories: Vec<PathBuf> = std::fs::read_dir(&snapshots)
        .expect("the snapshots directory lists")
        .map(|entry| entry.expect("an entry reads").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "git"))
        .collect();
    assert_eq!(repositories.len(), 1, "one workspace: {repositories:?}");
    repositories.remove(0)
}

#[test]
fn snapshot_exports_a_local_runs_checkpoint_work_for_git_fetch() {
    let context = test_context!();
    let workflow = context.temp_dir.join("salvage.fabro");
    std::fs::write(
        &workflow,
        r#"digraph Salvage {
    graph [goal="Leave checkpoint work behind"]
    start [shape=Mdiamond]
    exit  [shape=Msquare]
    work  [shape=parallelogram, script="printf salvaged > work.txt"]
    start -> work -> exit
}
"#,
    )
    .expect("the workflow writes");

    let mut run_cmd = context.run_cmd();
    run_cmd.current_dir(&context.temp_dir);
    run_cmd.timeout(Duration::from_secs(30));
    run_cmd.args(["--environment", "local"]);
    run_cmd.arg(&workflow);
    let run_output = run_cmd.output().expect("the run executes");
    assert!(
        run_output.status.success(),
        "workflow run failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run_output.stdout),
        String::from_utf8_lossy(&run_output.stderr)
    );
    let run_id = run_state(&context.single_run_dir()).spec.run_id.to_string();

    let repository = snapshot_repository(&context.storage_dir, &run_id);
    // The snapshot store keeps the checkpoints chain under
    // refs/checkpoints/, not the workspace's run branch.
    let expected_subjects = git(&repository, &["log", "--all", "--format=%s"]);

    let bundle = context.temp_dir.join("salvage.bundle");
    let mut cmd = context.command();
    cmd.args(["snapshot", "--out", bundle.to_str().unwrap(), &run_id]);
    let output = cmd.output().expect("snapshot executes");
    assert!(
        output.status.success(),
        "snapshot export failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(bundle.is_file(), "the bundle file exists at the --out path");

    // The salvage flow: everything the checkpoint hooks published comes
    // back through `git fetch` on the exported bundle alone.
    let salvage_repo = context.temp_dir.join("salvage-repo");
    std::fs::create_dir_all(&salvage_repo).expect("the salvage repo dir creates");
    git(&salvage_repo, &["init", "-q"]);
    git(&salvage_repo, &[
        "fetch",
        "-q",
        bundle.to_str().unwrap(),
        "refs/*:refs/remotes/run/*",
    ]);
    let recovered_subjects = git(&salvage_repo, &["log", "--all", "--format=%s"]);
    assert_eq!(
        recovered_subjects, expected_subjects,
        "the checkpoint commits round-trip through the bundle"
    );
    assert!(
        recovered_subjects
            .lines()
            .any(|subject| subject.starts_with(&format!("fabro({run_id}): "))),
        "the recovered commits are the run's own checkpoint work: {recovered_subjects}"
    );
}

#[test]
fn snapshot_refuses_an_existing_output_path() {
    let context = test_context!();
    let run = setup_completed_dry_run(&context);
    let bundle = context.temp_dir.join("occupied.bundle");
    std::fs::write(&bundle, "occupied").expect("the occupied path writes");

    let mut cmd = context.command();
    cmd.args(["snapshot", "--out", bundle.to_str().unwrap(), &run.run_id]);
    let output = cmd.output().expect("snapshot executes");
    assert!(
        !output.status.success(),
        "snapshot must refuse an existing output path"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("already exists"),
        "the refusal names the conflict:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&bundle).unwrap(),
        "occupied",
        "the refused export leaves the existing file untouched"
    );
}
