//! Run sandboxes on any provider fabro can name: a bundled kind in process
//! or a sandbox-driver plugin executable.
//!
//! One path builds them all. The environment's [`SandboxOptions`] become
//! the driver spec once, the provider is connected through the single
//! construction function, and a bundled provider adds only what its
//! backend needs on top: Docker its fixed working directory and default
//! image, Daytona the snapshot it creates sandboxes from and its lifecycle
//! timers. A plugin gets the spec as is, laid out inside the working
//! directory the provider chooses.

use std::sync::Arc;

use fabro_github::GitHubCredentials;
use fabro_types::{BundledProvider, RunId, SandboxProviderKind};
use sandbox_driver::{SandboxId, SandboxProvider};

use crate::driver::{ProviderAccess, connect_provider};
use crate::driver_sandbox::{DriverSandbox, LayoutSource, RepoWorkspace};
use crate::options::{self, SandboxOptions};
use crate::{daytona, docker, managed_labels};

/// A sandbox for a run on `kind`. The sandbox is created by `initialize`;
/// construction validates the clone request and connects the provider, so
/// a bad spec, a missing credential, or a missing plugin executable fails
/// before any backend call.
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors SandboxSpec::Provider; clone inputs are validated together"
)]
pub async fn provider_sandbox(
    kind: SandboxProviderKind,
    access: &ProviderAccess,
    options: SandboxOptions,
    github_app: Option<&GitHubCredentials>,
    run_id: Option<RunId>,
    clone_origin_url: Option<String>,
    clone_branch: Option<String>,
    clone_tag: Option<String>,
    clone_commit_sha: Option<String>,
) -> crate::Result<DriverSandbox> {
    let workspace = RepoWorkspace::plan(
        layout_source(&kind),
        options.skip_clone,
        clone_origin_url.as_deref(),
        clone_branch.as_deref(),
        clone_tag.as_deref(),
        clone_commit_sha.as_deref(),
        options.clone_depth,
        github_app,
    )?;
    let provider = connect(&kind, access).await?;
    let base = options::base_spec(&options, run_id.as_ref());
    Ok(match kind.bundled() {
        Some(BundledProvider::Docker) => {
            let (spec, image) = docker::overlay(base, &options);
            DriverSandbox::pending(kind, provider, spec, Some(image), workspace)
        }
        Some(BundledProvider::Daytona) => {
            let credentials = access
                .daytona
                .as_ref()
                .ok_or_else(|| crate::Error::message(MISSING_DAYTONA_CREDENTIALS))?;
            let plan = daytona::create_plan(
                Arc::clone(&provider),
                credentials.api_key.clone(),
                base,
                options,
                run_id,
            );
            DriverSandbox::pending_with_plan(kind, provider, Box::new(plan), workspace)
        }
        Some(BundledProvider::Local) => {
            return Err(crate::Error::message(
                "local sandboxes are built from a working directory, not a provider spec",
            ));
        }
        None => {
            let mut spec = base;
            spec.network = options::supported_network(spec.network, provider.capabilities());
            DriverSandbox::pending(kind, provider, spec, options.image.clone(), workspace)
        }
    })
}

/// Reattach to a run's sandbox on `kind` by its persisted id.
///
/// The sandbox must carry fabro's managed label and, when a run id is
/// known, the matching run label: the provider shares its backend with
/// every other application, and fabro never operates on a sandbox it did
/// not create.
pub async fn attach_provider_sandbox(
    kind: SandboxProviderKind,
    access: &ProviderAccess,
    sandbox_id: &str,
    repo_cloned: bool,
    working_directory: String,
    clone_origin_url: Option<String>,
    run_id: Option<RunId>,
) -> crate::Result<DriverSandbox> {
    let provider = connect(&kind, access).await?;
    let id = SandboxId::try_new(sandbox_id)
        .map_err(|error| crate::Error::context(format!("Invalid {kind} sandbox id"), error))?;
    let handle = provider.attach(&id, None).await.map_err(|error| {
        crate::Error::context(
            format!("Failed to reconnect {kind} sandbox '{sandbox_id}'"),
            error,
        )
    })?;
    let status = handle.describe().await?;
    managed_labels::verify_managed(&kind, sandbox_id, &status.labels, run_id.as_ref())?;
    let workspace = RepoWorkspace::attached(
        layout_source(&kind),
        repo_cloned,
        working_directory,
        clone_origin_url,
    );
    let sandbox = DriverSandbox::attached(kind.clone(), handle, workspace);
    if kind.bundled() == Some(BundledProvider::Daytona) {
        if let Some(snapshot) = status.source {
            sandbox.set_snapshot(snapshot);
        }
    }
    Ok(sandbox)
}

/// The image the run record names for a sandbox on `kind`: the
/// environment's, or Docker's default when the environment names none.
pub(crate) fn recorded_image(
    kind: &SandboxProviderKind,
    options: &SandboxOptions,
) -> Option<String> {
    match kind.bundled() {
        Some(BundledProvider::Docker) => Some(docker::effective_image(options)),
        _ => options.image.clone(),
    }
}

/// Where a run's repository checks out on `kind`: fabro fixes the roots
/// inside the containers and VMs it shapes itself, and follows the working
/// directory a plugin provider chooses.
pub(crate) fn layout_source(kind: &SandboxProviderKind) -> LayoutSource {
    match kind.bundled() {
        Some(BundledProvider::Docker) => LayoutSource::Fixed(docker::layout()),
        Some(BundledProvider::Daytona) => LayoutSource::Fixed(daytona::layout()),
        Some(BundledProvider::Local) | None => LayoutSource::ProviderWorkingDirectory,
    }
}

/// The in-process Docker provider with default settings, for `fabro doctor`.
pub(crate) async fn connect_bundled_docker(
    access: &ProviderAccess,
) -> crate::Result<Arc<dyn SandboxProvider>> {
    connect(&SandboxProviderKind::DOCKER, access).await
}

const MISSING_DAYTONA_CREDENTIALS: &str = "Daytona sandboxes require DAYTONA_API_KEY in the vault; run `fabro secret set DAYTONA_API_KEY`";

async fn connect(
    kind: &SandboxProviderKind,
    access: &ProviderAccess,
) -> crate::Result<Arc<dyn SandboxProvider>> {
    if kind.bundled() == Some(BundledProvider::Daytona) && access.daytona.is_none() {
        return Err(crate::Error::message(MISSING_DAYTONA_CREDENTIALS));
    }
    let settings = access.settings_for(kind).ok_or_else(|| {
        crate::Error::message(format!(
            "sandbox provider `{kind}` is not configured; add [server.sandbox.providers.{kind}] to settings.toml"
        ))
    })?;
    connect_provider(kind, &settings, &access.connect_options())
        .await
        .map(|connected| connected.provider)
        .map_err(|error| {
            crate::Error::context(format!("Failed to connect to the {kind} provider"), error)
        })
}
