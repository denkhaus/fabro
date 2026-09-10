use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use fabro_types::{BundledProvider, RunId, RunSandboxInstance};

use crate::driver::DaytonaCredentials;
use crate::driver_sandbox::{DriverSandbox, local_sandbox};
use crate::{SandboxEventCallback, daytona, docker};

/// Reconnect to a sandbox from a saved record.
///
/// `daytona` carries the vault credentials a `"daytona"` record needs; the
/// process environment is never consulted.
pub async fn reconnect(
    record: &RunSandboxInstance,
    daytona: Option<DaytonaCredentials>,
) -> Result<Box<dyn crate::Sandbox>> {
    reconnect_for_run(record, daytona, None).await
}

pub async fn reconnect_for_run(
    record: &RunSandboxInstance,
    daytona: Option<DaytonaCredentials>,
    run_id: Option<RunId>,
) -> Result<Box<dyn crate::Sandbox>> {
    reconnect_for_run_with_callback(record, daytona, run_id, None).await
}

pub async fn reconnect_for_run_with_callback(
    record: &RunSandboxInstance,
    daytona: Option<DaytonaCredentials>,
    run_id: Option<RunId>,
    event_callback: Option<SandboxEventCallback>,
) -> Result<Box<dyn crate::Sandbox>> {
    let sandbox = reconnect_driver_for_run(record, daytona, run_id, event_callback).await?;
    Ok(Box::new(sandbox))
}

/// Reconnects as the driver-backed sandbox type, for callers that need a
/// driver facet fabro's [`Sandbox`](crate::Sandbox) trait does not carry
/// (VNC, signed previews, leased SSH).
pub async fn reconnect_driver_for_run(
    record: &RunSandboxInstance,
    daytona: Option<DaytonaCredentials>,
    run_id: Option<RunId>,
    event_callback: Option<SandboxEventCallback>,
) -> Result<DriverSandbox> {
    let runtime = &record.runtime;
    match record.provider.bundled() {
        // A local sandbox is its working directory: rebuilding the handle
        // over that directory is the reconnect. The per-process Host
        // registry holds no state worth attaching to.
        Some(BundledProvider::Local) => {
            let mut sandbox = local_sandbox(PathBuf::from(&runtime.working_directory))
                .await
                .context("Failed to reconnect local sandbox")?;
            if let Some(callback) = event_callback {
                sandbox.set_event_callback(callback);
            }
            Ok(sandbox)
        }
        Some(BundledProvider::Docker) => {
            let repo_cloned = runtime
                .repo_cloned
                .context("Docker run sandbox missing repo_cloned metadata")?;
            let mut sandbox = docker::attach_docker(
                &runtime.id,
                repo_cloned,
                runtime.working_directory.clone(),
                runtime.clone_origin_url.clone(),
                run_id,
            )
            .await
            .context("Failed to reconnect Docker sandbox")?;
            if let Some(callback) = event_callback {
                sandbox.set_event_callback(callback);
            }
            Ok(sandbox)
        }
        Some(BundledProvider::Daytona) => {
            let repo_cloned = runtime
                .repo_cloned
                .context("Daytona run sandbox missing repo_cloned metadata")?;
            let credentials = daytona.context(
                "Daytona run sandbox cannot be reconnected without DAYTONA_API_KEY in the vault",
            )?;
            let mut sandbox = daytona::attach_daytona(
                &runtime.id,
                repo_cloned,
                runtime.working_directory.clone(),
                runtime.clone_origin_url.clone(),
                run_id,
                &credentials,
            )
            .await
            .context("Failed to reconnect Daytona sandbox")?;
            if let Some(callback) = event_callback {
                sandbox.set_event_callback(callback);
            }
            Ok(sandbox)
        }
        None => bail!(
            "sandbox provider `{}` is not bundled; plugin reconnect is not wired yet",
            record.provider
        ),
    }
}
