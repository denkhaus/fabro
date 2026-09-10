use std::path::PathBuf;

use anyhow::{Context, Result};
use fabro_types::{BundledProvider, RunId, RunSandboxInstance};

use crate::driver::ProviderAccess;
use crate::driver_sandbox::{DriverSandbox, local_sandbox};
use crate::{SandboxEventCallback, daytona, docker, plugin};

/// Reconnect to a sandbox from a saved record.
///
/// `access` carries the provider settings and vault credentials the record's
/// provider needs; the process environment is never consulted.
pub async fn reconnect(
    record: &RunSandboxInstance,
    access: &ProviderAccess,
) -> Result<Box<dyn crate::Sandbox>> {
    reconnect_for_run(record, access, None).await
}

pub async fn reconnect_for_run(
    record: &RunSandboxInstance,
    access: &ProviderAccess,
    run_id: Option<RunId>,
) -> Result<Box<dyn crate::Sandbox>> {
    reconnect_for_run_with_callback(record, access, run_id, None).await
}

pub async fn reconnect_for_run_with_callback(
    record: &RunSandboxInstance,
    access: &ProviderAccess,
    run_id: Option<RunId>,
    event_callback: Option<SandboxEventCallback>,
) -> Result<Box<dyn crate::Sandbox>> {
    let sandbox = reconnect_driver_for_run(record, access, run_id, event_callback).await?;
    Ok(Box::new(sandbox))
}

/// Reconnects as the driver-backed sandbox type, for callers that need a
/// driver facet fabro's [`Sandbox`](crate::Sandbox) trait does not carry
/// (VNC, signed previews, leased SSH).
pub async fn reconnect_driver_for_run(
    record: &RunSandboxInstance,
    access: &ProviderAccess,
    run_id: Option<RunId>,
    event_callback: Option<SandboxEventCallback>,
) -> Result<DriverSandbox> {
    let runtime = &record.runtime;
    let settings = access.settings_for(&record.provider);
    // A bundled kind whose entry names a plugin executable is served out of
    // process, exactly as a third-party kind is.
    let plugin_served = settings
        .as_ref()
        .is_some_and(|settings| settings.plugin.is_some());
    let bundled = record.provider.bundled().filter(|_| !plugin_served);
    match bundled {
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
            let credentials = access.daytona.clone().context(
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
        None => {
            let settings = settings.with_context(|| {
                format!(
                    "sandbox provider `{}` is not configured; add [server.sandbox.providers.{}] to settings.toml",
                    record.provider, record.provider
                )
            })?;
            let repo_cloned = runtime
                .repo_cloned
                .context("run sandbox missing repo_cloned metadata")?;
            let mut sandbox = plugin::attach_plugin(
                record.provider.clone(),
                &settings,
                &runtime.id,
                repo_cloned,
                runtime.working_directory.clone(),
                runtime.clone_origin_url.clone(),
                run_id,
            )
            .await
            .with_context(|| format!("Failed to reconnect {} sandbox", record.provider))?;
            if let Some(callback) = event_callback {
                sandbox.set_event_callback(callback);
            }
            Ok(sandbox)
        }
    }
}
