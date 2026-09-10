use std::path::PathBuf;

use anyhow::{Context, Result};
use fabro_types::{BundledProvider, RunId, RunSandboxInstance};
use sandbox_driver::{EventContext, PtySession, PtySize};

use crate::driver::ProviderAccess;
use crate::driver_sandbox::{RunSandbox, local_sandbox_with_events};
use crate::provider_sandbox;

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
    reconnect_for_run_with_events(record, access, run_id, None).await
}

pub async fn reconnect_for_run_with_events(
    record: &RunSandboxInstance,
    access: &ProviderAccess,
    run_id: Option<RunId>,
    events: Option<EventContext>,
) -> Result<RunSandbox> {
    reconnect_driver_for_run(record, access, run_id, events).await
}

/// Reconnects as the driver-backed sandbox type, for callers that need a
/// driver facet fabro's [`Sandbox`](crate::Sandbox) trait does not carry
/// (VNC, signed previews, leased SSH).
pub async fn reconnect_driver_for_run(
    record: &RunSandboxInstance,
    access: &ProviderAccess,
    run_id: Option<RunId>,
    events: Option<EventContext>,
) -> Result<RunSandbox> {
    let runtime = &record.runtime;
    // A local sandbox is its working directory: rebuilding the handle over
    // that directory is the reconnect. The per-process Host registry holds
    // no state worth attaching to.
    let sandbox = if record.provider.bundled() == Some(BundledProvider::Local) {
        local_sandbox_with_events(PathBuf::from(&runtime.working_directory), events)
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
            events,
        )
        .await
        .with_context(|| format!("Failed to reconnect {} sandbox", record.provider))?
    };
    Ok(sandbox)
}

/// Opens an interactive shell in a run's sandbox over the driver's Pty
/// facet, reconnecting from the run record first. The session is the
/// driver's own; it is closed by the caller.
pub async fn open_terminal_for_run(
    record: &RunSandboxInstance,
    access: &ProviderAccess,
    run_id: Option<RunId>,
    size: PtySize,
) -> crate::Result<Box<dyn PtySession>> {
    if record.provider.bundled() == Some(BundledProvider::Local) {
        return Err(crate::Error::message(
            "Local sandboxes do not support embedded terminals",
        ));
    }
    let sandbox = reconnect_driver_for_run(record, access, run_id, None)
        .await
        .map_err(|err| crate::Error::context_anyhow("Failed to reconnect sandbox", err))?;
    sandbox.activate().await?;
    sandbox.open_terminal(size).await
}
