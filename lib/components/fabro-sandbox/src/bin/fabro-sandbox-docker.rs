//! The bundled Docker provider served as a sandbox-driver plugin over stdio.
//!
//! Fabro links Docker in-process for normal runs. This executable exists so
//! CI can run the same provider through the plugin protocol and so a
//! deployment can move it out of process. The daemon named by `DOCKER_HOST`
//! (or the local default) is not required to answer at launch: an
//! unreachable daemon is reported through `provider/health`. Stdout belongs
//! to the protocol; logs go to stderr.

use std::io::stderr;
use std::sync::Arc;

use anyhow::Context as _;
use sandbox_driver_docker::DockerProvider;
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
        .context("configuring docker plugin diagnostics")?;

    let provider = DockerProvider::connect_unverified().context("configuring the docker client")?;
    serve_stdio(Arc::new(provider))
        .await
        .context("serving the docker provider plugin")
}
