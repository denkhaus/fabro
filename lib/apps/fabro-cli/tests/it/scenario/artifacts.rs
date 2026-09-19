//! `fabro artifact list` and `fabro artifact cp` over a run whose artifacts
//! the engine's hooks collected: every file under `[run.artifacts] include`
//! in a stage's workspace, once per content, into the blob table.

use std::path::PathBuf;
use std::time::Duration;

use fabro_test::{fabro_snapshot, test_context};

use super::petri::{RunningServer, host_plugin, run_detached, wait_for_success};
use crate::cmd::support::{read_text, text_tree};

/// Three command stages that leave files under `assets/`. The second and
/// third write different contents to the same path, so the path names an
/// artifact of each; the third also writes a `summary.txt` that collides
/// by filename with the first stage's.
#[expect(
    clippy::disallowed_methods,
    reason = "the fixture files are written before the run starts"
)]
fn artifact_workspace(context: &fabro_test::TestContext) -> PathBuf {
    let workspace = context.temp_dir.join("artifact-workspace");
    std::fs::create_dir_all(&workspace).expect("the workspace creates");
    std::fs::write(
        workspace.join("workflow.fabro"),
        "digraph ArtifactRun {\n  graph [goal=\"Exercise artifact commands\", \
         default_max_retries=0]\n  start [shape=Mdiamond]\n  exit [shape=Msquare]\n  \
         create_assets [shape=parallelogram, script=\"mkdir -p assets/node_a assets/shared && \
         printf alpha > assets/node_a/summary.txt && printf one > \
         assets/shared/report.txt\"]\n  update_assets [shape=parallelogram, script=\"mkdir -p \
         assets/retry && printf second > assets/retry/report.txt\"]\n  create_colliding \
         [shape=parallelogram, script=\"mkdir -p assets/other && printf beta > \
         assets/other/summary.txt && printf third > assets/retry/report.txt\"]\n  start -> \
         create_assets -> update_assets -> create_colliding -> exit\n}\n",
    )
    .expect("the workflow writes");
    std::fs::write(
        workspace.join("workflow.toml"),
        "_version = 1\n\n[workflow]\ngraph = \"workflow.fabro\"\n\n[run]\ngoal = \"Exercise \
         artifact commands\"\n\n[run.artifacts]\ninclude = [\"assets/**\"]\n",
    )
    .expect("the settings write");
    workspace
}

#[tokio::test(flavor = "multi_thread")]
async fn artifact_commands_read_the_artifacts_the_hooks_collected() {
    if host_plugin().is_none() {
        return;
    }
    let context = test_context!();
    let server = RunningServer::start().await;
    let workspace = artifact_workspace(&context);
    let run_id = run_detached(&context, &server, &workspace);
    wait_for_success(&server, &run_id).await;
    let target = server.target();
    let filters = context.filters();

    let mut list_json = context.command();
    list_json.args(["artifact", "list", &run_id, "--json", "--server", &target]);
    fabro_snapshot!(filters.clone(), list_json, @r#"
    success: true
    exit_code: 0
    ----- stdout -----
    [
      {
        "stage_id": "create_assets@1",
        "node_slug": "create_assets",
        "retry": 1,
        "relative_path": "assets/node_a/summary.txt",
        "size": 5
      },
      {
        "stage_id": "create_assets@1",
        "node_slug": "create_assets",
        "retry": 1,
        "relative_path": "assets/shared/report.txt",
        "size": 3
      },
      {
        "stage_id": "create_colliding@1",
        "node_slug": "create_colliding",
        "retry": 1,
        "relative_path": "assets/other/summary.txt",
        "size": 4
      },
      {
        "stage_id": "create_colliding@1",
        "node_slug": "create_colliding",
        "retry": 1,
        "relative_path": "assets/retry/report.txt",
        "size": 5
      },
      {
        "stage_id": "update_assets@1",
        "node_slug": "update_assets",
        "retry": 1,
        "relative_path": "assets/retry/report.txt",
        "size": 6
      }
    ]
    ----- stderr -----
    "#);

    let mut list_filtered = context.command();
    list_filtered.args([
        "artifact",
        "list",
        &run_id,
        "--node",
        "update_assets",
        "--retry",
        "1",
        "--json",
        "--server",
        &target,
    ]);
    fabro_snapshot!(filters.clone(), list_filtered, @r#"
    success: true
    exit_code: 0
    ----- stdout -----
    [
      {
        "stage_id": "update_assets@1",
        "node_slug": "update_assets",
        "retry": 1,
        "relative_path": "assets/retry/report.txt",
        "size": 6
      }
    ]
    ----- stderr -----
    "#);

    let mut list_stage_filtered = context.command();
    list_stage_filtered.args([
        "artifact",
        "list",
        &run_id,
        "--stage",
        "create_colliding@1",
        "--json",
        "--server",
        &target,
    ]);
    fabro_snapshot!(filters.clone(), list_stage_filtered, @r#"
    success: true
    exit_code: 0
    ----- stdout -----
    [
      {
        "stage_id": "create_colliding@1",
        "node_slug": "create_colliding",
        "retry": 1,
        "relative_path": "assets/other/summary.txt",
        "size": 4
      },
      {
        "stage_id": "create_colliding@1",
        "node_slug": "create_colliding",
        "retry": 1,
        "relative_path": "assets/retry/report.txt",
        "size": 5
      }
    ]
    ----- stderr -----
    "#);

    let single_dest = context.temp_dir.join("artifact-one");
    let mut cp_single = context.command();
    cp_single.args([
        "artifact",
        "cp",
        &format!("{run_id}:assets/retry/report.txt"),
        single_dest.to_str().unwrap(),
        "--stage",
        "create_colliding@1",
        "--server",
        &target,
    ]);
    fabro_snapshot!(filters.clone(), cp_single, @"
    success: true
    exit_code: 0
    ----- stdout -----
    Copied assets/retry/report.txt to [TEMP_DIR]/artifact-one/report.txt
    ----- stderr -----
    ");
    assert_eq!(read_text(&single_dest.join("report.txt")), "third");

    let node_dest = context.temp_dir.join("artifact-node");
    let mut cp_node = context.command();
    cp_node.args([
        "artifact",
        "cp",
        &format!("{run_id}:assets/retry/report.txt"),
        node_dest.to_str().unwrap(),
        "--node",
        "update_assets",
        "--server",
        &target,
    ]);
    fabro_snapshot!(filters.clone(), cp_node, @"
    success: true
    exit_code: 0
    ----- stdout -----
    Copied assets/retry/report.txt to [TEMP_DIR]/artifact-node/report.txt
    ----- stderr -----
    ");
    assert_eq!(read_text(&node_dest.join("report.txt")), "second");

    let stage_tree_dest = context.temp_dir.join("artifact-stage-tree");
    let mut cp_stage_tree = context.command();
    cp_stage_tree.args([
        "artifact",
        "cp",
        &run_id,
        stage_tree_dest.to_str().unwrap(),
        "--stage",
        "create_colliding@1",
        "--tree",
        "--server",
        &target,
    ]);
    fabro_snapshot!(filters.clone(), cp_stage_tree, @"
    success: true
    exit_code: 0
    ----- stdout -----
    Copied 2 artifact(s) to [TEMP_DIR]/artifact-stage-tree
    ----- stderr -----
    ");
    insta::assert_snapshot!(
        text_tree(&stage_tree_dest).join("\n"),
        @r"
        create_colliding/retry_1/assets/other/summary.txt = beta
        create_colliding/retry_1/assets/retry/report.txt = third
        "
    );

    let tree_dest = context.temp_dir.join("artifact-tree");
    let mut cp_tree = context.command();
    cp_tree.args([
        "artifact",
        "cp",
        &run_id,
        tree_dest.to_str().unwrap(),
        "--tree",
        "--server",
        &target,
    ]);
    cp_tree.timeout(Duration::from_secs(30));
    fabro_snapshot!(filters.clone(), cp_tree, @"
    success: true
    exit_code: 0
    ----- stdout -----
    Copied 5 artifact(s) to [TEMP_DIR]/artifact-tree
    ----- stderr -----
    ");
    insta::assert_snapshot!(
        text_tree(&tree_dest).join("\n"),
        @r"
        create_assets/retry_1/assets/node_a/summary.txt = alpha
        create_assets/retry_1/assets/shared/report.txt = one
        create_colliding/retry_1/assets/other/summary.txt = beta
        create_colliding/retry_1/assets/retry/report.txt = third
        update_assets/retry_1/assets/retry/report.txt = second
        "
    );

    let ambiguous_dest = context.temp_dir.join("artifact-ambiguous");
    let mut cp_ambiguous = context.command();
    cp_ambiguous.args([
        "artifact",
        "cp",
        &format!("{run_id}:assets/retry/report.txt"),
        ambiguous_dest.to_str().unwrap(),
        "--server",
        &target,
    ]);
    fabro_snapshot!(filters.clone(), cp_ambiguous, @"
    success: false
    exit_code: 1
    ----- stdout -----
    ----- stderr -----
      × Path 'assets/retry/report.txt' matches multiple artifacts: create_colliding@1:retry_1, update_assets@1:retry_1. Use --stage and/or --retry to disambiguate.
    ");

    let flat_dest = context.temp_dir.join("artifact-flat");
    let mut cp_flat = context.command();
    cp_flat.args([
        "artifact",
        "cp",
        &run_id,
        flat_dest.to_str().unwrap(),
        "--server",
        &target,
    ]);
    fabro_snapshot!(filters, cp_flat, @"
    success: false
    exit_code: 1
    ----- stdout -----
    ----- stderr -----
      × Filename collision: 'summary.txt' exists in both create_assets@1:retry_1 and create_colliding@1:retry_1. Use --tree to preserve directory structure, or --stage and/or --retry to filter.
    ");
    server.shutdown();
}
