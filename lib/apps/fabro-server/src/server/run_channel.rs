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

// The production SPAWN caller (denkhaus/work#989) is
// `spawn_for_run` below; the IT under tests/it/api/run_channel.rs
// exercises the same surface through test-support. The TERMINAL
// DRAIN is denkhaus/work#1003 (the registry and its three
// server-side call sites).
#![allow(
    dead_code,
    reason = "the handle's accessor and stop serve the test-support ITs (TestRunChannel); the production spawn hands the path out directly and lets the registry own shutdown — a feature-less clippy run sees neither test-support consumer"
)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context as _;
use axum::extract::Request;
use axum::http::HeaderValue;
use axum::http::header::AUTHORIZATION;
use axum::middleware;
use axum::middleware::Next;
use axum::routing::Router;
use fabro_petri::fork_run_channel::CHANNEL_SOCKET_FILE;
use fabro_static::EnvVars;
use fabro_types::RunId;
use tokio::fs;
use tokio::net::UnixListener;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::jwt_auth::AuthMode;
use crate::server::{AppState, build_router};
use crate::worker_token::{WorkerScopeSet, issue_worker_token_with_scopes};

/// A spawned channel server's registration: everything needed to shut
/// it down, owned by the [`RunChannelRegistry`] until one taker claims
/// it (denkhaus/work#1003).
pub(crate) struct RegisteredChannel {
    shutdown:    watch::Sender<bool>,
    socket_path: PathBuf,
    /// The run this channel serves — the join-failure log below is
    /// per-run and unidentifiable without it (logging strategy:
    /// attach structured ID fields).
    run_id:      RunId,
    /// The serve task itself: `stop` awaits it before removing the
    /// socket, so a respawn for the same run (launch, resume, retry)
    /// binds AFTER the old listener is gone — never racing a removal
    /// against the fresh bind (denkhaus/work#989).
    served:      JoinHandle<()>,
}

impl RegisteredChannel {
    /// Graceful stop: ends the accept loop, WAITS for the serve task
    /// to leave the socket, then removes the socket file — the file is
    /// gone before this returns, so the next spawn binds clean.
    async fn stop(self) {
        let _ = self.shutdown.send(true);
        if let Err(join_error) = self.served.await {
            tracing::warn!(
                run_id = %self.run_id,
                ?join_error,
                "run channel serve task ended with an error"
            );
        }
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

/// The spawn caller of ADR-0035 (denkhaus/work#989): start the run's
/// channel BEFORE the launch builds anything the sandbox will see, so
/// the socket exists by the time the spec gains its bind. Returns the
/// socket path for the run's worker (`RuntimeSpec::run_channel`), or
/// `None` when the operator's kill switch disables the channel
/// (`[server.run_channel] enabled = false`, work#989 grill R2.4).
///
/// Layout (work#989 grill F2/R2.1): the channel root — env override
/// `FABRO_RUN_CHANNEL_DIR`, then `[server.run_channel] dir`, then
/// `<storage>/run-channels` — holds one SUBDIR per run, and the
/// sandbox mounts only that subdir: one daemon-visible path serves
/// server and daemon alike (compose pins the root into the server
/// container unchanged), and a crash-rest re-bind (a new inode) stays
/// visible through the directory the container holds.
pub(crate) async fn spawn_for_run(state: &Arc<AppState>, run_id: RunId) -> Option<PathBuf> {
    if !state.server_settings().server.run_channel.enabled {
        return None;
    }
    let root = (state.env_lookup)(EnvVars::FABRO_RUN_CHANNEL_DIR)
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            state
                .server_settings()
                .server
                .run_channel
                .dir
                .as_ref()
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| state.server_storage_dir().join("run-channels"));
    let socket_path = root.join(run_id.to_string()).join(CHANNEL_SOCKET_FILE);
    // An earlier launch of the same run (resume, retry) registered its
    // channel: AWAIT its stop — old listener gone, file removed — so
    // the respawn can neither race the old removal nor hit EADDRINUSE
    // against the old listener.
    if let Some(stale) = state.run_channels.take(run_id) {
        stale.stop().await;
    }
    match RunChannel::spawn(
        Arc::clone(state),
        state.auth_mode.clone(),
        run_id,
        socket_path.clone(),
    ) {
        Ok(_channel) => Some(socket_path),
        Err(error) => {
            tracing::error!(
                run_id = %run_id,
                ?error,
                "run channel spawn failed; the run launches without a channel"
            );
            None
        }
    }
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
        .context("signing the run channel worker token")?;
        // The same surface the agent's run tools reach — routes the
        // extractors already gate per principal.
        let router = build_router(Arc::clone(&state), auth_mode);
        let stamped = stamp_run_worker_bearer(router, &token);

        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Crash rest (re-grilled 2026-10-26 after the review disproved
        // the original premise — bind does NOT overwrite): a leftover
        // file from a dead process makes bind fail EADDRINUSE. Probe
        // ownership first — a connect that SUCCEEDS means a live
        // listener serves this path and keeps the refusal (never unlink
        // a serving socket); a refused connect means the file is stale:
        // remove it ONCE and retry the bind.
        let listener = match UnixListener::bind(&socket_path) {
            Ok(listener) => listener,
            Err(bind_error) if bind_error.kind() == std::io::ErrorKind::AddrInUse => {
                if std::os::unix::net::UnixStream::connect(&socket_path).is_ok() {
                    return Err(bind_error.into());
                }
                std::fs::remove_file(&socket_path)?;
                UnixListener::bind(&socket_path)?
            }
            Err(error) => return Err(error.into()),
        };
        // Owner-only on the host side: the server user connects; inside
        // the sandbox the bind mount carries the socket and stage shells
        // run as root (the uid-mapped workspace precedent).
        // one-shot setup chmod, not a Tokio path — clippy's sync-I/O
        // disallow list does not cover set_permissions
        if let Err(perm_error) =
            std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))
        {
            // registration has not happened yet — every error path
            // before it leaves nothing behind
            let _ = std::fs::remove_file(&socket_path);
            return Err(perm_error.into());
        }

        let (shutdown, mut shutdown_rx) = watch::channel(false);
        // Every error path above left nothing behind; the task starts
        // serving and the registration claims it in the very next
        // statement — nothing fails between the two.
        let served = tokio::spawn(async move {
            let served = axum::serve(listener, stamped)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.changed().await;
                })
                .await;
            if let Err(error) = served {
                tracing::warn!(
                    run_id = %run_id,
                    %error,
                    "run channel server ended with an error"
                );
            }
        });
        state.run_channels.insert(run_id, RegisteredChannel {
            shutdown,
            socket_path: socket_path.clone(),
            run_id,
            served,
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

    /// The stamping layer as a UNIT (denkhaus/work#991 deliverable —
    /// the review's "empty test module" finding): a client-supplied
    /// Authorization is REPLACED, never honored, and a bearer-less
    /// request gets the run's token stamped in.
    #[tokio::test]
    async fn stamping_replaces_any_client_authorization() {
        use axum::Router;
        use axum::body::Body;
        use axum::http::Request;
        use axum::routing::get;
        use tower::ServiceExt as _;

        async fn echo_auth(req: axum::extract::Request) -> String {
            req.headers()
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("<missing>")
                .to_string()
        }

        let router = Router::new().route("/", get(echo_auth));
        let stamped = super::stamp_run_worker_bearer(router, "run-token");

        let forged = stamped
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("authorization", "Bearer forged-by-the-sandbox")
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("stamped router answers");
        let body = axum::body::to_bytes(forged.into_body(), usize::MAX)
            .await
            .expect("response body");
        assert_eq!(
            String::from_utf8(body.to_vec()).expect("utf-8"),
            "Bearer run-token",
            "the client bearer is replaced, never honored"
        );

        let bearer_less = stamped
            .oneshot(
                Request::builder()
                    .uri("/")
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("stamped router answers");
        let body = axum::body::to_bytes(bearer_less.into_body(), usize::MAX)
            .await
            .expect("response body");
        assert_eq!(
            String::from_utf8(body.to_vec()).expect("utf-8"),
            "Bearer run-token",
            "a bearer-less request gets the run's token stamped in"
        );
    }

    /// The registry IS the idempotency: one take claims the
    /// registration, the second finds nothing — a double stop cannot
    /// exist because there is nothing left to stop (work#1003).
    #[tokio::test]
    async fn the_second_take_finds_nothing() {
        let registry = RunChannelRegistry::default();
        let run_id = RunId::new();
        let (shutdown, _rx) = tokio::sync::watch::channel(false);
        registry.insert(run_id, super::RegisteredChannel {
            shutdown,
            socket_path: std::path::PathBuf::from("/tmp/never-removed-by-this-test.sock"),
            run_id,
            served: tokio::spawn(std::future::ready(())),
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
