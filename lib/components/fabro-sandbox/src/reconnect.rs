use std::path::PathBuf;

use anyhow::{Context, Result};
use fabro_types::{BundledProvider, RunId, RunSandboxInstance};

use crate::driver::ProviderAccess;
use crate::driver_sandbox::{RunSandbox, local_sandbox};
use crate::{SandboxEventCallback, provider_sandbox};

/// Reconnect to a sandbox from a saved record.
///
/// `access` carries the provider settings and vault credentials the record's
/// provider needs; the process environment is never consulted.
pub async fn reconnect(record: &RunSandboxInstance, access: &ProviderAccess) -> Result<RunSandbox> {
    reconnect_for_run(record, access, None).await
}

pub async fn reconnect_for_run(
    record: &RunSandboxInstance,
    access: &ProviderAccess,
    run_id: Option<RunId>,
) -> Result<RunSandbox> {
    reconnect_for_run_with_callback(record, access, run_id, None).await
}

pub async fn reconnect_for_run_with_callback(
    record: &RunSandboxInstance,
    access: &ProviderAccess,
    run_id: Option<RunId>,
    event_callback: Option<SandboxEventCallback>,
) -> Result<RunSandbox> {
    reconnect_driver_for_run(record, access, run_id, event_callback).await
}

/// Reconnects as the driver-backed sandbox type, for callers that need a
/// driver facet fabro's [`Sandbox`](crate::Sandbox) trait does not carry
/// (VNC, signed previews, leased SSH).
pub async fn reconnect_driver_for_run(
    record: &RunSandboxInstance,
    access: &ProviderAccess,
    run_id: Option<RunId>,
    event_callback: Option<SandboxEventCallback>,
) -> Result<RunSandbox> {
    let runtime = &record.runtime;
    // A local sandbox is its working directory: rebuilding the handle over
    // that directory is the reconnect. The per-process Host registry holds
    // no state worth attaching to.
    let mut sandbox = if record.provider.bundled() == Some(BundledProvider::Local) {
        local_sandbox(PathBuf::from(&runtime.working_directory))
            .await
            .context("Failed to reconnect local sandbox")?
    } else {
        let repo_cloned = runtime.repo_cloned.with_context(|| {
            format!(
                "{} run sandbox missing repo_cloned metadata",
                record.provider
            )
        })?;
        provider_sandbox::attach_provider_sandbox(
            record.provider.clone(),
            access,
            &runtime.id,
            repo_cloned,
            runtime.working_directory.clone(),
            runtime.clone_origin_url.clone(),
            run_id,
        )
        .await
        .with_context(|| format!("Failed to reconnect {} sandbox", record.provider))?
    };
    if let Some(callback) = event_callback {
        sandbox.set_event_callback(callback);
    }
    Ok(sandbox)
}
