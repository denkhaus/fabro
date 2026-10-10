//! Host-side staging of hook-declared workflow files.
//!
//! A `[[run.hooks]] files = [...]` declaration rides the workflow closure
//! (see `fabro-manifest`'s hook-file collection). The server holds the
//! closure bytes in the workflow-version store, but a host-side hook
//! (`sandbox = false`) executes in the worker process where no repository
//! checkout exists — the Docker provider's sandboxes do not share the host
//! filesystem. Before launching the worker, the server stages every
//! hook-declared file of the run's root workflow version under
//! `<run_dir>/hook-assets/`, and the worker exports that directory as
//! `FABRO_HOOK_ASSETS`, so hook scripts execute the staged bytes by path
//! instead of embedding them in the hook command.

use std::path::{Path, PathBuf};

use fabro_types::{RunId, WorkflowPath, WorkflowVersionId};
use fabro_workflow_version::WorkflowVersionStore;
use tokio::fs;

/// Name of the hook-assets directory inside a run directory.
const HOOK_ASSETS_DIR_NAME: &str = "hook-assets";

/// Stage the run's hook-declared files under `<run_dir>/hook-assets/` and
/// return that directory, or `None` when the run's version declares none —
/// in that case the variable stays unset and hook scripts fall back to
/// workspace-relative paths.
///
/// A version absent from the store stages nothing: the run fails on its own
/// version-loading path, and staging must not mask that failure mode.
pub(crate) async fn stage(
    store: &WorkflowVersionStore,
    run_id: RunId,
    root_version: WorkflowVersionId,
    run_dir: &Path,
) -> anyhow::Result<Option<PathBuf>> {
    let Some(version) = store.get(&root_version).await? else {
        return Ok(None);
    };
    stage_contents(run_id, run_dir, &version.hook_file_contents()).await
}

/// Write the resolved hook-file contents into the run's hook-assets
/// directory. Package-root-relative keys map onto the same shape below the
/// directory, so a hook script addresses a staged file as
/// `$FABRO_HOOK_ASSETS/<key>`.
async fn stage_contents(
    run_id: RunId,
    run_dir: &Path,
    contents: &[(WorkflowPath, &str)],
) -> anyhow::Result<Option<PathBuf>> {
    if contents.is_empty() {
        return Ok(None);
    }
    let assets_dir = run_dir.join(HOOK_ASSETS_DIR_NAME);
    for (path, content) in contents {
        let target = assets_dir.join(path.as_str());
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(&target, content).await?;
    }
    tracing::debug!(
        run_id = %run_id,
        files = contents.len(),
        dir = %assets_dir.display(),
        "Staged hook assets"
    );
    Ok(Some(assets_dir))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use fabro_store::test_support;
    use fabro_types::{RunId, WorkflowVersion};

    use super::*;

    fn config_with_hook_files(files: &str) -> String {
        format!(
            "_version = 1\n[[run.hooks]]\nname = \"shadow\"\nevent = \"stage_complete\"\nsandbox = false\nfiles = {files}\nscript = \"true\"\n"
        )
    }

    fn version_with_hooks(
        hook_files: &[(&str, &str)],
    ) -> fabro_workflow_version::ValidatedWorkflowVersion {
        let mut map = BTreeMap::new();
        map.insert(
            "workflow.fabro".parse().unwrap(),
            "digraph W { start [shape=Mdiamond] exit [shape=Msquare] start -> exit }".to_owned(),
        );
        map.insert("workflow.toml".parse().unwrap(), {
            let list = hook_files
                .iter()
                .map(|(reference, _)| format!("\"{reference}\""))
                .collect::<Vec<_>>()
                .join(", ");
            let mut config = config_with_hook_files(&format!("[{list}]"));
            config.push('\n');
            config
        });
        for (path, content) in hook_files {
            map.insert((*path).parse().unwrap(), (*content).to_owned());
        }
        fabro_workflow_version::ValidatedWorkflowVersion::new(
            WorkflowVersion::new("workflow.fabro".parse().unwrap(), map, BTreeMap::new()).unwrap(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn stages_hook_declared_files_under_the_run_dir() {
        let database = test_support::test_database();
        let store = fabro_workflow_version::WorkflowVersionStore::new(database.blobs());
        let version = version_with_hooks(&[
            ("scripts/shadow.nu", "def main [] { 0 }"),
            ("scripts/shared/util.nu", "export def helper [] { 1 }"),
        ]);
        let id = store.put(&version).await.unwrap();
        let run_dir = tempfile::tempdir().unwrap();

        let staged = stage(&store, RunId::new(), id, run_dir.path())
            .await
            .unwrap()
            .expect("a version with hook files must stage a directory");

        assert_eq!(
            fs::read_to_string(staged.join("scripts/shadow.nu"))
                .await
                .unwrap(),
            "def main [] { 0 }"
        );
        assert_eq!(
            fs::read_to_string(staged.join("scripts/shared/util.nu"))
                .await
                .unwrap(),
            "export def helper [] { 1 }"
        );
    }

    #[tokio::test]
    async fn stages_nothing_without_hook_files_or_version() {
        let database = test_support::test_database();
        let store = fabro_workflow_version::WorkflowVersionStore::new(database.blobs());
        let plain = fabro_workflow_version::ValidatedWorkflowVersion::new(
            WorkflowVersion::new(
                "workflow.fabro".parse().unwrap(),
                BTreeMap::from([("workflow.fabro".parse().unwrap(), "digraph W {}".to_owned())]),
                BTreeMap::new(),
            )
            .unwrap(),
        )
        .unwrap();
        let id = store.put(&plain).await.unwrap();
        let run_dir = tempfile::tempdir().unwrap();

        assert!(
            stage(&store, RunId::new(), id, run_dir.path())
                .await
                .unwrap()
                .is_none()
        );
        assert!(!run_dir.path().join("hook-assets").exists());

        let missing = fabro_types::WorkflowVersionId::from(fabro_types::BlobHash::new(b"missing"));
        assert!(
            stage(&store, RunId::new(), missing, run_dir.path())
                .await
                .unwrap()
                .is_none()
        );
    }
}
