//! The per-run capability channel (ADR-0035): the universal bridge
//! between DETERMINISTIC workflow steps and the engine.
//!
//! Stage execution has two shapes — LLM steps call engine capabilities
//! through the run tools, deterministic steps (nu scripts in the
//! sandbox) need them from a process. The channel is that process
//! bridge: one per-run Unix socket serving the SAME router the TCP
//! server serves, with the run's worker principal (`run:worker` +
//! `agent:run_tools` — exactly the principal the ask session's tool
//! backend carries) stamped on every request internally. The existing
//! principal middleware and route extractors ARE the authorization:
//! **parity, not a new capability list** — the channel exposes exactly
//! what the run's agent could reach via tools, no more, no less, so
//! every engine capability that becomes agent-reachable arrives for
//! deterministic steps in the same breath. Work items are the first
//! consumer (the cutover, ADR-0032); they define nothing about the
//! channel's shape.
//!
//! The token is minted at spawn and never leaves the server process:
//! the sandbox side carries no secret material, the connection is the
//! authority, and closing the socket at run terminal revokes it.

// Not yet constructed in production: the lifecycle wiring (the sandbox
// bind mount + the run-terminal stop) lands in the next slice of
// denkhaus/work#989 — until then the IT under tests/it/api/run_channel.rs
// proves the surface through test-support, where this module IS used
// (clippy runs feature-less and would call it dead).
#![allow(
    dead_code,
    reason = "run-channel lifecycle wiring lands in the next denkhaus/work#989 slice; the IT proves the surface under test-support"
)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::Request;
use axum::http::HeaderValue;
use axum::http::header::AUTHORIZATION;
use axum::middleware;
use axum::middleware::Next;
use axum::routing::Router;
use fabro_types::RunId;
use tokio::fs;
use tokio::net::UnixListener;
use tokio::sync::watch;

use crate::jwt_auth::AuthMode;
use crate::server::{AppState, build_router};
use crate::worker_token::{WorkerScopeSet, issue_worker_token_with_scopes};

/// A spawned channel server: the socket path (for the sandbox mount) and
/// the shutdown handle. Dropping the handle does NOT stop the server —
/// run lifecycle code calls [`RunChannel::stop`] at terminal state.
pub(crate) struct RunChannel {
    socket_path: PathBuf,
    shutdown:    watch::Sender<bool>,
}

impl RunChannel {
    /// Binds `socket_path` and serves the application router on it with
    /// the run's worker principal stamped on every request. The socket
    /// file is created by the bind; the caller (sandbox setup) mounts it
    /// into the run's container from there.
    pub(crate) fn spawn(
        state: Arc<AppState>,
        auth_mode: AuthMode,
        run_id: RunId,
        socket_path: PathBuf,
    ) -> anyhow::Result<Self> {
        let token = issue_worker_token_with_scopes(
            &state.worker_tokens,
            &run_id,
            WorkerScopeSet::run_worker_with_agent_run_tools(),
        )
        .map_err(|error| anyhow::anyhow!("signing the run channel worker token: {error:?}"))?;
        // The same surface the agent's run tools reach — routes the
        // extractors already gate per principal.
        let router = build_router(state, auth_mode);
        let stamped = stamp_run_worker_bearer(router, &token);

        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let listener = UnixListener::bind(&socket_path)?;
        // Owner-only on the host side: the server user connects; inside
        // the sandbox the bind mount carries the socket and stage shells
        // run as root (the uid-mapped workspace precedent).
        // one-shot setup chmod, not a Tokio path — clippy's sync-I/O
        // disallow list does not cover set_permissions
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;

        let (shutdown, mut shutdown_rx) = watch::channel(false);
        tokio::spawn(async move {
            let served = axum::serve(listener, stamped)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.changed().await;
                })
                .await;
            if let Err(error) = served {
                tracing::warn!(%error, "run channel server ended with an error");
            }
        });
        Ok(Self {
            socket_path,
            shutdown,
        })
    }

    /// The socket path, for the sandbox driver's bind mount.
    pub(crate) fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Graceful stop: ends the accept loop, waits for in-flight requests,
    /// removes the socket file. The run's terminal transition calls this
    /// exactly once — a racing connect after that fails loudly instead
    /// of hanging.
    pub(crate) async fn stop(self) {
        let _ = self.shutdown.send(true);
        let _ = fs::remove_file(&self.socket_path).await;
    }
}

/// The production twin of `test_support::with_test_user`: a layer that
/// stamps the run's worker bearer on every request BEFORE the auth
/// middleware classifies it. The header is inserted server-side only —
/// a client-supplied Authorization header is REPLACED, never honored,
/// so the sandbox side cannot escalate by sending one.
fn stamp_run_worker_bearer(router: Router, token: &str) -> Router {
    let bearer = HeaderValue::from_str(&format!("Bearer {token}"))
        .expect("a signed worker token is a valid header value");
    router.layer(middleware::from_fn(move |mut req: Request, next: Next| {
        let bearer = bearer.clone();
        async move {
            req.headers_mut().insert(AUTHORIZATION, bearer);
            next.run(req).await
        }
    }))
}

#[cfg(test)]
mod tests {
    //! The channel's scope matrix (work routes pass, user-only surfaces
    //! refuse the worker principal, bearer-less connects work) is pinned
    //! by the IT under `tests/it/api/run_channel.rs`, which owns a real
    //! AppState with the scripted work tracker. Unit-level, the stamping
    //! layer's discipline (client Authorization headers are replaced,
    //! never honored) is covered there too.
}
