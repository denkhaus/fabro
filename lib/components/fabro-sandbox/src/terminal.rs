use async_trait::async_trait;
use fabro_types::{BundledProvider, RunId, RunSandboxInstance};

use crate::driver::DaytonaCredentials;
use crate::{Sandbox, daytona, docker};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
}

impl Default for TerminalSize {
    fn default() -> Self {
        Self {
            cols: 120,
            rows: 32,
        }
    }
}

#[async_trait]
pub trait TerminalSession: Send + Sync {
    async fn write_input(&self, bytes: &[u8]) -> crate::Result<()>;
    async fn read_output(&self) -> crate::Result<Option<Vec<u8>>>;
    async fn resize(&self, size: TerminalSize) -> crate::Result<()>;
    async fn close(&self) -> crate::Result<()>;
}

/// A terminal over the driver's Pty facet.
pub struct DriverTerminalSession {
    session: Box<dyn sandbox_driver::PtySession>,
}

impl DriverTerminalSession {
    #[must_use]
    pub fn new(session: Box<dyn sandbox_driver::PtySession>) -> Self {
        Self { session }
    }
}

#[async_trait]
impl TerminalSession for DriverTerminalSession {
    async fn write_input(&self, bytes: &[u8]) -> crate::Result<()> {
        self.session
            .write_input(bytes)
            .await
            .map_err(|err| crate::Error::context("Failed to write terminal input", err))
    }

    async fn read_output(&self) -> crate::Result<Option<Vec<u8>>> {
        self.session
            .read_output()
            .await
            .map_err(|err| crate::Error::context("Failed to read terminal output", err))
    }

    async fn resize(&self, size: TerminalSize) -> crate::Result<()> {
        self.session
            .resize(sandbox_driver::PtySize {
                rows: size.rows,
                cols: size.cols,
            })
            .await
            .map_err(|err| crate::Error::context("Failed to resize terminal", err))
    }

    async fn close(&self) -> crate::Result<()> {
        self.session
            .close()
            .await
            .map_err(|err| crate::Error::context("Failed to close terminal", err))
    }
}

pub async fn open_terminal_for_run(
    record: &RunSandboxInstance,
    daytona: Option<DaytonaCredentials>,
    run_id: Option<RunId>,
    size: TerminalSize,
) -> crate::Result<Box<dyn TerminalSession>> {
    let runtime = &record.runtime;

    match record.provider.bundled() {
        Some(BundledProvider::Daytona) => {
            let repo_cloned = runtime.repo_cloned.ok_or_else(|| {
                crate::Error::message("Daytona run sandbox is missing clone metadata")
            })?;
            let credentials = daytona.ok_or_else(|| {
                crate::Error::message("Daytona terminals require DAYTONA_API_KEY in the vault")
            })?;
            let sandbox = daytona::attach_daytona(
                &runtime.id,
                repo_cloned,
                runtime.working_directory.clone(),
                runtime.clone_origin_url.clone(),
                run_id,
                &credentials,
            )
            .await?;
            sandbox.activate().await?;
            Ok(Box::new(sandbox.open_terminal(size).await?))
        }
        Some(BundledProvider::Docker) => {
            let repo_cloned = runtime.repo_cloned.ok_or_else(|| {
                crate::Error::message("Docker run sandbox is missing clone metadata")
            })?;
            let sandbox = docker::attach_docker(
                &runtime.id,
                repo_cloned,
                runtime.working_directory.clone(),
                runtime.clone_origin_url.clone(),
                run_id,
            )
            .await?;
            sandbox.activate().await?;
            Ok(Box::new(sandbox.open_terminal(size).await?))
        }
        Some(BundledProvider::Local) => Err(crate::Error::message(
            "Local sandboxes do not support embedded terminals",
        )),
        None => Err(crate::Error::message(format!(
            "Sandbox provider '{}' does not support embedded terminals yet",
            record.provider
        ))),
    }
}
