//! The run's sandbox as the view shows it: the plan its settings give, and
//! the instance Petri's scope records name (VIEWS.md "Sandbox").

use fabro_types::settings::run::RunEnvironmentSettings;
use fabro_types::{
    RunProjection, RunSandboxInstance, RunSandboxPlan, RunSandboxRuntime, SandboxProviderKind,
};
use petri_runtime::ir::SandboxInstance;

pub(super) fn sandbox_plan(settings: &RunEnvironmentSettings) -> RunSandboxPlan {
    RunSandboxPlan {
        provider: settings.provider.clone(),
        image:    (settings.provider == SandboxProviderKind::DOCKER)
            .then(|| settings.image.docker.clone())
            .flatten()
            .filter(|image| !image.is_empty()),
        snapshot: None,
    }
}

/// The plan the projection's sandbox carries, or the one its environment
/// settings give when no sandbox was projected yet.
pub(super) fn sandbox_plan_of(projection: &RunProjection) -> RunSandboxPlan {
    projection.sandbox.as_ref().map_or_else(
        || sandbox_plan(&projection.spec.settings.run.environment),
        |sandbox| sandbox.plan().clone(),
    )
}

/// Fabro's name for the provider Petri's `scope.acquired` names: Petri's
/// `host` is Fabro's `local`; every other kind is spelled the same. `None`
/// for a name that is no provider kind.
pub(super) fn provider_kind(provider: &str) -> Option<SandboxProviderKind> {
    if provider == "host" {
        return Some(SandboxProviderKind::LOCAL);
    }
    SandboxProviderKind::try_new(provider).ok()
}

/// The run's sandbox instance from Petri's record of the scope's
/// acquisition: the provider, the provider's id for the sandbox (what a
/// reconnect attaches by), its image and snapshot when the provider knows
/// them, the working directory, and how long the acquisition took. The
/// clone fields stay unset: Petri's checkout copies the bound repository
/// into the workspace and is not a clone Fabro made, and the workspace
/// roots are the provider's own layout, read live. `retained` waits for
/// the scope's release.
pub(super) fn sandbox_instance(
    plan: &RunSandboxPlan,
    sandbox: &SandboxInstance,
    ready_duration_ms: u64,
) -> RunSandboxInstance {
    RunSandboxInstance {
        provider:          provider_kind(&sandbox.provider)
            .unwrap_or_else(|| plan.provider.clone()),
        image:             sandbox
            .image
            .as_ref()
            .map(ToString::to_string)
            .or_else(|| plan.image.clone()),
        snapshot:          sandbox.snapshot.as_ref().map(ToString::to_string),
        runtime:           RunSandboxRuntime {
            id:                sandbox.instance.to_string(),
            working_directory: sandbox.working_directory.to_string(),
            repo_cloned:       None,
            clone_origin_url:  None,
            clone_branch:      None,
            workspace_root:    None,
            repos_root:        None,
            primary_repo_path: None,
            primary_repo_link: None,
        },
        ready_duration_ms: Some(ready_duration_ms),
        retained:          None,
    }
}
