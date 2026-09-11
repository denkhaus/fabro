use std::path::Path;

use anyhow::{Context, Result};
use fabro_types::{BundledProvider, RunId, RunSandboxInstance};
use sandbox_driver::{EventContext, PtySession, PtySize};
use sandbox_driver_host::HostProvider;

use crate::driver::ProviderAccess;
use crate::driver_sandbox::RunSandbox;
use crate::provider_sandbox;

/// Reconnect to a run's sandbox from its saved record.
///
/// `access` carries the provider settings and vault credentials the record's
/// provider needs; the process environment is never consulted. `run_id`
/// narrows the ownership scope to the run when known, and the driver reports
/// the sandbox's lifecycle from here on through `events`.
pub async fn reconnect_for_run(
    record: &RunSandboxInstance,
    access: &ProviderAccess,
    run_id: Option<RunId>,
    events: Option<EventContext>,
) -> Result<RunSandbox> {
    let runtime = &record.runtime;
    let sandbox_id = sandbox_id(record).await;
    provider_sandbox::attach_provider_sandbox(
        record.provider.clone(),
        access,
        &sandbox_id,
        // A record without the flag was written for a sandbox fabro never
        // cloned into.
        runtime.repo_cloned.unwrap_or(false),
        runtime.working_directory.clone(),
        runtime.clone_origin_url.clone(),
        run_id,
        events,
    )
    .await
    .with_context(|| format!("Failed to reconnect {} sandbox", record.provider))
}

/// The id the record's sandbox attaches by. A local sandbox is its working
/// directory, and the Host provider derives the directory's id from its
/// path, so the record's id is recomputed from the directory: a record
/// written before directories had ids attaches the same way.
async fn sandbox_id(record: &RunSandboxInstance) -> String {
    let runtime = &record.runtime;
    if record.provider.bundled() == Some(BundledProvider::Local) {
        if let Some(id) = HostProvider::directory_id(Path::new(&runtime.working_directory)).await {
            return id.to_string();
        }
    }
    runtime.id.clone()
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
    let sandbox = reconnect_for_run(record, access, run_id, None)
        .await
        .map_err(|err| crate::Error::context_anyhow("Failed to reconnect sandbox", err))?;
    sandbox.activate().await?;
    sandbox.open_terminal(size).await
}
