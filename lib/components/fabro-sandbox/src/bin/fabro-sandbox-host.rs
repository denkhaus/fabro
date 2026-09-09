//! The bundled Host provider served as a sandbox-driver plugin over stdio.
//!
//! Fabro links Host in-process for normal runs. This executable exists so
//! CI can run the same provider through the plugin protocol and so a
//! deployment can move it out of process by configuring
//! `[server.sandbox.providers.host]` instead of `local`. Stdout belongs to
//! the protocol; logs go to stderr. `SANDBOX_DRIVER_HOST_REGISTRY` names a
//! persistent registry directory; without it sandboxes live in a fresh
//! temporary registry.

use std::io::stderr;
use std::sync::Arc;

use anyhow::Context as _;
use sandbox_driver_host::HostProvider;
use sandbox_driver_protocol::serve_stdio;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, fmt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();
    // Stdout carries the protocol, so diagnostics must go to the process
    // stderr handle; tracing's writer contract is synchronous.
    #[expect(
        clippy::disallowed_methods,
        reason = "tracing-subscriber requires a synchronous writer and stdout is reserved for \
                  the plugin protocol"
    )]
    let diagnostics = fmt::layer().with_writer(stderr);
    tracing_subscriber::registry()
        .with(filter)
        .with(diagnostics)
        .try_init()
        .context("configuring host plugin diagnostics")?;

    #[expect(
        clippy::disallowed_methods,
        reason = "a plugin executable starts from the scrubbed environment its host declared; \
                  reading it here is the configured channel"
    )]
    let registry = std::env::var_os("SANDBOX_DRIVER_HOST_REGISTRY");
    let provider = match registry {
        Some(root) => HostProvider::with_registry(root)
            .await
            .context("opening the host registry")?,
        None => HostProvider::new(),
    };
    serve_stdio(Arc::new(provider))
        .await
        .context("serving the host provider plugin")
}
