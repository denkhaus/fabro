use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use fabro_config::parse::SettingsSource;
use fabro_config::{RunGoalLayer, SettingsLayer};
use fabro_environment::{EnvironmentId, EnvironmentValidationError};
use fabro_types::settings::InterpString;
use fabro_types::{
    ManifestPath, SandboxProviderKind, TargetValidationError, WorkflowPath, WorkflowVersionId,
};
use fabro_workflow::workflow_bundle::{BundledWorkflow, ParsedWorkflowConfig, WorkflowBundle};
use fabro_workflow_version::LoadedWorkflowVersionClosure;
use thiserror::Error;

use crate::run_compiler::{RunCompilerError, settings_layer_with_resolved_dockerfiles};

#[derive(Debug, Error)]
pub(crate) enum RunIntentAdmissionError {
    #[error("workflow-version storage could not be opened")]
    StoreOpen {
        #[source]
        source: fabro_store::Error,
    },
    #[error("workflow-version closure could not be loaded")]
    VersionStore {
        #[source]
        source: fabro_workflow_version::WorkflowVersionStoreError,
    },
    #[error(transparent)]
    Lowering(#[from] WorkflowClosureLoweringError),
    #[error(transparent)]
    Target(#[from] TargetValidationError),
    #[error(transparent)]
    Environment(#[from] EnvironmentSelectionError),
    #[error(transparent)]
    Compiler(#[from] RunCompilerError),
    #[error("run variables could not be loaded")]
    VariableSnapshot {
        #[source]
        source: fabro_variable::Error,
    },
}

#[derive(Debug, Error)]
pub(crate) enum EnvironmentSelectionError {
    #[error("invalid environment ID `{value}`")]
    InvalidId {
        value:  String,
        #[source]
        source: EnvironmentValidationError,
    },
    #[error("environment `{id}` not found")]
    NotFound { id: EnvironmentId },
    #[error("Git targets require a compatible clone-enabled Docker or Daytona environment")]
    TargetUnsupported,
    #[error("{detail}")]
    ProviderDisabled {
        provider: SandboxProviderKind,
        detail:   String,
    },
    #[error("{name} is not configured for sandbox provider `{provider}`")]
    MissingCredential {
        provider: SandboxProviderKind,
        name:     &'static str,
    },
    #[error("failed to read sandbox credential `{name}`")]
    CredentialStore {
        name:   &'static str,
        #[source]
        source: fabro_vault::SecretStoreError,
    },
}

#[derive(Debug)]
pub(crate) struct LoweredWorkflowClosure {
    pub(crate) workflow_bundle: WorkflowBundle,
    pub(crate) entrypoint:      ManifestPath,
    pub(crate) workflow_layer:  Option<SettingsLayer>,
}

#[derive(Debug, Error)]
pub(crate) enum WorkflowClosureLoweringError {
    #[error("workflow version `{id}` is missing from the loaded closure")]
    MissingVersion { id: WorkflowVersionId },
    #[error("workflow version `{id}` has no entrypoint content")]
    MissingEntrypoint { id: WorkflowVersionId },
    #[error("workflow path `{path}` cannot be mounted at `{mount}`")]
    InvalidMount {
        path:  WorkflowPath,
        mount: ManifestPath,
    },
    #[error("workflow mount `{path}` resolves to two different workflow versions")]
    ConflictingMount { path: ManifestPath },
    #[error("workflow config goal file `{reference}` cannot be resolved")]
    InvalidGoalReference { reference: String },
    #[error("workflow config goal file `{path}` is missing from the version")]
    MissingGoalFile { path: ManifestPath },
    #[error("workflow-version settings are unusable")]
    Settings {
        #[source]
        source: Box<RunCompilerError>,
    },
}

pub(crate) fn lower_workflow_closure(
    closure: &LoadedWorkflowVersionClosure,
) -> Result<LoweredWorkflowClosure, WorkflowClosureLoweringError> {
    let entrypoint = manifest_path(closure.root().entrypoint(), closure.root().entrypoint())?;
    let mut mounts = HashMap::new();
    let mut workflows = HashMap::new();
    mount_version(
        closure,
        closure.root_id(),
        entrypoint.clone(),
        &mut mounts,
        &mut workflows,
    )?;

    let root_workflow = workflows
        .get(&entrypoint)
        .expect("root workflow should be mounted");
    let workflow_layer = root_workflow
        .config
        .as_ref()
        .map(|config| {
            settings_layer_with_resolved_dockerfiles(
                &config.source,
                &config.path,
                &root_workflow.files,
                SettingsSource::Workflow,
            )
            .map_err(|source| WorkflowClosureLoweringError::Settings {
                source: Box::new(source),
            })
            .and_then(|mut layer| {
                inline_goal_file(&mut layer, &config.path, &root_workflow.files)?;
                Ok(layer)
            })
        })
        .transpose()?;

    Ok(LoweredWorkflowClosure {
        workflow_bundle: WorkflowBundle::new(workflows),
        entrypoint,
        workflow_layer,
    })
}

pub(crate) fn pin_workflow_environment_authority(layer: &mut SettingsLayer, environment_id: &str) {
    if let Some(environment) = layer.environments.get_mut(environment_id) {
        environment.provider = None;
        environment.cwd = None;
        environment.image = None;
    }
    if let Some(environment) = layer.run.as_mut().and_then(|run| run.environment.as_mut()) {
        environment.image = None;
    }
}

fn mount_version(
    closure: &LoadedWorkflowVersionClosure,
    id: WorkflowVersionId,
    mounted_entrypoint: ManifestPath,
    mounts: &mut HashMap<ManifestPath, WorkflowVersionId>,
    workflows: &mut HashMap<ManifestPath, BundledWorkflow>,
) -> Result<(), WorkflowClosureLoweringError> {
    if let Some(existing) = mounts.get(&mounted_entrypoint) {
        return if *existing == id {
            Ok(())
        } else {
            Err(WorkflowClosureLoweringError::ConflictingMount {
                path: mounted_entrypoint,
            })
        };
    }
    mounts.insert(mounted_entrypoint.clone(), id);

    let version = closure
        .get(&id)
        .ok_or(WorkflowClosureLoweringError::MissingVersion { id })?;
    let mut files = HashMap::new();
    for (path, content) in version.files() {
        files.insert(
            manifest_path(version.entrypoint(), path).and_then(|local| {
                rebase_path(version.entrypoint(), &mounted_entrypoint, &local, path)
            })?,
            content.clone(),
        );
    }
    let source = version
        .files()
        .get(version.entrypoint())
        .cloned()
        .ok_or(WorkflowClosureLoweringError::MissingEntrypoint { id })?;
    let config_local = WorkflowPath::new("workflow.toml")
        .expect("the static workflow config path should be valid");
    let config_path = version.files().get(&config_local).map(|source| {
        rebase_path(
            version.entrypoint(),
            &mounted_entrypoint,
            &ManifestPath::from_wire(config_local.as_str())
                .expect("validated workflow path should be a manifest path"),
            &config_local,
        )
        .map(|path| ParsedWorkflowConfig {
            path,
            source: source.clone(),
        })
    });
    let config = config_path.transpose()?;

    workflows.insert(mounted_entrypoint.clone(), BundledWorkflow {
        path: mounted_entrypoint.clone(),
        source,
        config,
        files,
    });

    for (binding, dependency_id) in version.workflow_dependencies() {
        let local = ManifestPath::from_wire(binding.as_str())
            .expect("validated workflow path should be a manifest path");
        let dependency_mount =
            rebase_path(version.entrypoint(), &mounted_entrypoint, &local, binding)?;
        mount_version(closure, *dependency_id, dependency_mount, mounts, workflows)?;
    }
    Ok(())
}

fn manifest_path(
    entrypoint: &WorkflowPath,
    path: &WorkflowPath,
) -> Result<ManifestPath, WorkflowClosureLoweringError> {
    ManifestPath::from_wire(path.as_str()).ok_or_else(|| {
        WorkflowClosureLoweringError::InvalidMount {
            path:  path.clone(),
            mount: ManifestPath::from_wire(entrypoint.as_str())
                .expect("validated entrypoint should be a manifest path"),
        }
    })
}

fn rebase_path(
    local_entrypoint: &WorkflowPath,
    mounted_entrypoint: &ManifestPath,
    local_path: &ManifestPath,
    workflow_path: &WorkflowPath,
) -> Result<ManifestPath, WorkflowClosureLoweringError> {
    let relative = relative_path(
        local_entrypoint_parent(local_entrypoint),
        local_path.as_path(),
    );
    let mapped = ManifestPath::from_reference(
        mounted_entrypoint.parent_or_dot(),
        &relative.to_string_lossy(),
    )
    .filter(|path| !path.as_path().starts_with(".."))
    .ok_or_else(|| WorkflowClosureLoweringError::InvalidMount {
        path:  workflow_path.clone(),
        mount: mounted_entrypoint.clone(),
    })?;
    Ok(mapped)
}

fn local_entrypoint_parent(entrypoint: &WorkflowPath) -> &Path {
    Path::new(entrypoint.as_str())
        .parent()
        .unwrap_or_else(|| Path::new("."))
}

fn relative_path(base: &Path, path: &Path) -> PathBuf {
    let base = base
        .components()
        .filter_map(normal_component)
        .collect::<Vec<_>>();
    let path = path
        .components()
        .filter_map(normal_component)
        .collect::<Vec<_>>();
    let common = base
        .iter()
        .zip(&path)
        .take_while(|(left, right)| left == right)
        .count();
    let mut relative = PathBuf::new();
    for _ in &base[common..] {
        relative.push("..");
    }
    for component in &path[common..] {
        relative.push(component);
    }
    relative
}

fn normal_component(component: Component<'_>) -> Option<&std::ffi::OsStr> {
    match component {
        Component::Normal(value) => Some(value),
        Component::CurDir | Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
            None
        }
    }
}

fn inline_goal_file(
    layer: &mut SettingsLayer,
    config_path: &ManifestPath,
    files: &HashMap<ManifestPath, String>,
) -> Result<(), WorkflowClosureLoweringError> {
    let Some(RunGoalLayer::File { file }) = layer.run.as_mut().and_then(|run| run.goal.as_mut())
    else {
        return Ok(());
    };
    let reference = unresolved_source(file);
    let path =
        ManifestPath::from_reference(config_path.parent_or_dot(), &reference).ok_or_else(|| {
            WorkflowClosureLoweringError::InvalidGoalReference {
                reference: reference.clone(),
            }
        })?;
    let content = files
        .get(&path)
        .ok_or_else(|| WorkflowClosureLoweringError::MissingGoalFile { path: path.clone() })?;
    *layer
        .run
        .as_mut()
        .and_then(|run| run.goal.as_mut())
        .expect("goal file should still be present") =
        RunGoalLayer::Inline(InterpString::parse(content));
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "workflow-version lowering preserves authored goal-file references for validated lookup"
)]
fn unresolved_source(value: &InterpString) -> String {
    value.as_source()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use fabro_types::{WorkflowVersion, WorkflowVersionId};
    use fabro_workflow_version::{ValidatedWorkflowVersion, WorkflowVersionStore};

    use super::*;

    fn workflow_path(value: &str) -> WorkflowPath {
        WorkflowPath::new(value).unwrap()
    }

    fn version(
        entrypoint: &str,
        files: impl IntoIterator<Item = (&'static str, &'static str)>,
        dependencies: impl IntoIterator<Item = (&'static str, WorkflowVersionId)>,
    ) -> ValidatedWorkflowVersion {
        let files = files
            .into_iter()
            .map(|(path, source)| (workflow_path(path), source.to_string()))
            .collect();
        let dependencies = dependencies
            .into_iter()
            .map(|(path, id)| (workflow_path(path), id))
            .collect::<BTreeMap<_, _>>();
        ValidatedWorkflowVersion::new(
            WorkflowVersion::new(workflow_path(entrypoint), files, dependencies).unwrap(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn lowers_nested_entrypoints_and_inlines_goal_files() {
        let (database, _) = crate::test_support::test_store_bundle();
        let blobs = database.blobs().await.unwrap();
        let store = WorkflowVersionStore::new(blobs);
        let grandchild = version(
            "deep/leaf.fabro",
            [("deep/leaf.fabro", "digraph Leaf {}")],
            [],
        );
        let grandchild_id = store.put(&grandchild).await.unwrap();
        let child = version(
            "pkg/child.fabro",
            [(
                "pkg/child.fabro",
                "digraph Child { leaf [stack.child_workflow=\"../nested/leaf.fabro\"] }",
            )],
            [("nested/leaf.fabro", grandchild_id)],
        );
        let child_id = store.put(&child).await.unwrap();
        let root = version(
            "flows/root.fabro",
            [
                (
                    "flows/root.fabro",
                    "digraph Root { child [stack.child_workflow=\"../deps/run.fabro\"] }",
                ),
                (
                    "workflow.toml",
                    "_version = 1\n[run.goal]\nfile = \"goal.md\"\n",
                ),
                ("goal.md", "Ship {{ vars.owner }}"),
            ],
            [("deps/run.fabro", child_id)],
        );
        let root_id = store.put(&root).await.unwrap();
        let closure = store.get_closure(&root_id).await.unwrap().unwrap();

        let lowered = lower_workflow_closure(&closure).unwrap();

        assert!(
            lowered
                .workflow_bundle
                .workflow(&lowered.entrypoint)
                .is_some()
        );
        assert!(
            lowered
                .workflow_bundle
                .workflow(&ManifestPath::from_wire("deps/run.fabro").unwrap())
                .is_some()
        );
        assert!(
            lowered
                .workflow_bundle
                .workflow(&ManifestPath::from_wire("nested/leaf.fabro").unwrap())
                .is_some()
        );
        assert!(matches!(
            lowered
                .workflow_layer
                .as_ref()
                .and_then(|layer| layer.run.as_ref())
                .and_then(|run| run.goal.as_ref()),
            Some(RunGoalLayer::Inline(_))
        ));
    }

    #[tokio::test]
    async fn lowers_same_version_at_distinct_mount_paths() {
        let (database, _) = crate::test_support::test_store_bundle();
        let blobs = database.blobs().await.unwrap();
        let store = WorkflowVersionStore::new(blobs);
        let child = version(
            "pkg/child.fabro",
            [("pkg/child.fabro", "digraph Child {}")],
            [],
        );
        let child_id = store.put(&child).await.unwrap();
        let root = version(
            "flows/root.fabro",
            [(
                "flows/root.fabro",
                "digraph Root { one [stack.child_workflow=\"../children/one.fabro\"] two [stack.child_workflow=\"../children/two.fabro\"] }",
            )],
            [
                ("children/one.fabro", child_id),
                ("children/two.fabro", child_id),
            ],
        );
        let root_id = store.put(&root).await.unwrap();
        let closure = store.get_closure(&root_id).await.unwrap().unwrap();

        let lowered = lower_workflow_closure(&closure).unwrap();

        assert!(
            lowered
                .workflow_bundle
                .workflow(&ManifestPath::from_wire("children/one.fabro").unwrap())
                .is_some()
        );
        assert!(
            lowered
                .workflow_bundle
                .workflow(&ManifestPath::from_wire("children/two.fabro").unwrap())
                .is_some()
        );
    }

    #[tokio::test]
    async fn rejects_distinct_versions_that_converge_on_one_mount_path() {
        let (database, _) = crate::test_support::test_store_bundle();
        let blobs = database.blobs().await.unwrap();
        let store = WorkflowVersionStore::new(blobs);
        let first_leaf = version(
            "leaf/first.fabro",
            [("leaf/first.fabro", "digraph FirstLeaf {}")],
            [],
        );
        let first_leaf_id = store.put(&first_leaf).await.unwrap();
        let second_leaf = version(
            "leaf/second.fabro",
            [("leaf/second.fabro", "digraph SecondLeaf {}")],
            [],
        );
        let second_leaf_id = store.put(&second_leaf).await.unwrap();
        let first_parent = version(
            "a/first.fabro",
            [(
                "a/first.fabro",
                "digraph FirstParent { child [stack.child_workflow=\"../shared/collision.fabro\"] }",
            )],
            [("shared/collision.fabro", first_leaf_id)],
        );
        let first_parent_id = store.put(&first_parent).await.unwrap();
        let second_parent = version(
            "b/second.fabro",
            [(
                "b/second.fabro",
                "digraph SecondParent { child [stack.child_workflow=\"../shared/collision.fabro\"] }",
            )],
            [("shared/collision.fabro", second_leaf_id)],
        );
        let second_parent_id = store.put(&second_parent).await.unwrap();
        let root = version(
            "flows/root.fabro",
            [(
                "flows/root.fabro",
                "digraph Root { first [stack.child_workflow=\"../left/first.fabro\"] second [stack.child_workflow=\"../right/second.fabro\"] }",
            )],
            [
                ("left/first.fabro", first_parent_id),
                ("right/second.fabro", second_parent_id),
            ],
        );
        let root_id = store.put(&root).await.unwrap();
        let closure = store.get_closure(&root_id).await.unwrap().unwrap();

        let error = lower_workflow_closure(&closure).unwrap_err();

        assert!(matches!(
            error,
            WorkflowClosureLoweringError::ConflictingMount { .. }
        ));
    }

    #[tokio::test]
    async fn rejects_rebased_files_that_escape_the_runtime_root() {
        let (database, _) = crate::test_support::test_store_bundle();
        let blobs = database.blobs().await.unwrap();
        let store = WorkflowVersionStore::new(blobs);
        let child = version(
            "nested/child.fabro",
            [
                ("nested/child.fabro", "digraph Child {}"),
                ("workflow.toml", "_version = 1"),
            ],
            [],
        );
        let child_id = store.put(&child).await.unwrap();
        let root = version(
            "root.fabro",
            [(
                "root.fabro",
                "digraph Root { child [stack.child_workflow=\"child.fabro\"] }",
            )],
            [("child.fabro", child_id)],
        );
        let root_id = store.put(&root).await.unwrap();
        let closure = store.get_closure(&root_id).await.unwrap().unwrap();

        let error = lower_workflow_closure(&closure).unwrap_err();

        assert!(matches!(
            error,
            WorkflowClosureLoweringError::InvalidMount { .. }
        ));
    }
}
