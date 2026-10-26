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

// Not yet constructed in production: the SPAWN caller (the sandbox bind
// mount at run start) lands in the next slice of denkhaus/work#989 —
// until then the IT under tests/it/api/run_channel.rs proves the
// surface through test-support, where this module IS used (clippy runs
// feature-less and would call it dead). The TERMINAL DRAIN exists
// (denkhaus/work#1003: the registry and its three server-side call
// sites are production code).
#![allow(
    dead_code,
    reason = "the production spawn caller lands in the next denkhaus/work#989 slice; the terminal drain (work#1003) and the ITs use the rest"
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

/// A spawned channel server's registration: everything needed to shut
/// it down, owned by the [`RunChannelRegistry`] until one taker claims
/// it (denkhaus/work#1003).
pub(crate) struct RegisteredChannel {
    shutdown:    watch::Sender<bool>,
    socket_path: PathBuf,
}

impl RegisteredChannel {
    /// Graceful stop: ends the accept loop (the serve task sees the
    /// watch flip), waits are the server's; removes the socket file.
    async fn stop(self) {
        let _ = self.shutdown.send(true);
        let _ = fs::remove_file(&self.socket_path).await;
    }
}

/// The run-keyed registry that gives the channel an owner
/// (denkhaus/work#1003): spawn registers BEFORE the serve task starts
/// (no leak window — every earlier error path left nothing behind),
/// and `take` removes the entry so exactly ONE caller — terminal fold,
/// Handle::stop, or a test — can stop it. A second take finds nothing:
/// the double stop is a no-op by construction. Dropping the
/// [`RunChannel`] handle stays inert (ADR-0035: the registry owns
/// shutdown), process-local like the socket itself.
#[derive(Default)]
pub(crate) struct RunChannelRegistry {
    entries: std::sync::Mutex<std::collections::HashMap<RunId, RegisteredChannel>>,
}

impl RunChannelRegistry {
    fn insert(&self, run_id: RunId, registered: RegisteredChannel) {
        self.entries
            .lock()
            .expect("run channel registry lock poisoned")
            .insert(run_id, registered);
    }

    /// Removes and returns the run's registration — `None` for a
    /// double drain. The one cleanup path (no AtomicBool beside it).
    pub(crate) fn take(&self, run_id: RunId) -> Option<RegisteredChannel> {
        self.entries
            .lock()
            .expect("run channel registry lock poisoned")
            .remove(&run_id)
    }
}

/// What the terminal transition calls — the channel dies where the
/// worker control bus dies (the three `cleanup_worker_control_bus_for_run`
/// sites in server.rs). Returns whether THIS call won the take.
pub(crate) fn cleanup_run_channel_for_run(state: &AppState, run_id: RunId) -> bool {
    let Some(registered) = state.run_channels.take(run_id) else {
        return false;
    };
    tokio::spawn(async move {
        registered.stop().await;
    });
    true
}

/// A spawned channel server: the socket path (for the sandbox mount) and
/// a stop PROXY — dropping the handle does NOT stop the server; stop()
/// takes the registry entry and shuts down only if it won the take
/// (denkhaus/work#1003), and the run's terminal transition drains the
/// registry whether or not anyone still holds this handle.
pub(crate) struct RunChannel {
    socket_path: PathBuf,
    state:       Arc<AppState>,
    run_id:      RunId,
}

impl RunChannel {
    /// Binds `socket_path`, registers the shutdown (BEFORE the serve
    /// task starts — no leak window), and serves the application router
    /// on it with the run's worker principal stamped on every request.
    /// The socket file is created by the bind; the caller (sandbox
    /// setup) mounts it into the run's container from there.
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
        let router = build_router(Arc::clone(&state), auth_mode);
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
        state.run_channels.insert(run_id, RegisteredChannel {
            shutdown,
            socket_path: socket_path.clone(),
        });
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
            state,
            run_id,
        })
    }

    /// The socket path, for the sandbox driver's bind mount.
    pub(crate) fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Graceful stop through the registry: takes the entry, and if this
    /// call won it, ends the accept loop and removes the socket file.
    /// A racing terminal drain (or a second stop) finds nothing and is
    /// a no-op — "exactly once" by construction, not by convention
    /// (denkhaus/work#1003). A racing connect after removal fails
    /// loudly instead of hanging.
    pub(crate) async fn stop(self) {
        if let Some(registered) = self.state.run_channels.take(self.run_id) {
            registered.stop().await;
        }
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

    use fabro_types::RunId;

    use super::RunChannelRegistry;

    /// The registry IS the idempotency: one take claims the
    /// registration, the second finds nothing — a double stop cannot
    /// exist because there is nothing left to stop (work#1003).
    #[test]
    fn the_second_take_finds_nothing() {
        let registry = RunChannelRegistry::default();
        let run_id = RunId::new();
        let (shutdown, _rx) = tokio::sync::watch::channel(false);
        registry.insert(run_id, super::RegisteredChannel {
            shutdown,
            socket_path: std::path::PathBuf::from("/tmp/never-removed-by-this-test.sock"),
        });

        assert!(
            registry.take(run_id).is_some(),
            "the first take claims the registration"
        );
        assert!(
            registry.take(run_id).is_none(),
            "the second take finds nothing — the double stop is a no-op by construction"
        );
    }
}
