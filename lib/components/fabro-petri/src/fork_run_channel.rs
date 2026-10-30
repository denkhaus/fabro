//! The per-run capability channel mount (ADR-0035, denkhaus/work#989):
//! how a run's sandbox reaches the channel socket the engine serves
//! for that run.
//!
//! The channel server itself lives server-side
//! (`fabro-server`'s `server::run_channel`: the full application
//! router on a per-run Unix socket, the run's worker principal
//! stamped on every request). This module is the SANDBOX half: a
//! [`RunChannelMount`] names the socket path as the server process
//! and the sandbox daemon see it, and the provider wrap
//! ([`crate::providers`]'s staged runtime) applies it to every
//! sandbox spec it creates:
//!
//! - **Docker**: the socket's per-run directory is bind-mounted into the
//!   container at [`CONTAINER_CHANNEL_DIR`] (a DIRECTORY bind, not a file bind
//!   — the server may unlink and re-bind the socket file after a crash rest,
//!   and the directory view follows the new inode while a file bind would pin
//!   the dead one), and `FABRO_SERVER` points at [`CONTAINER_CHANNEL_SOCKET`].
//! - **Host**: the socket is already reachable on the shared filesystem; only
//!   `FABRO_SERVER` is set, to the host-side path.
//! - **Everything else** (Daytona): no mount, no env — the channel is honestly
//!   absent for remote sandboxes.
//!
//! The env injection follows the `FABRO_STAGE` discipline
//! ([`crate::fork_stage_env`]): the engine's value OVERRIDES any
//! caller-supplied `FABRO_SERVER`, never trusts one.
//!
//! Per-run isolation is the security boundary, held by TWO facts
//! (denkhaus/work#1043 finding 1): each mount names only its own run's
//! directory AND carries that run's per-spawn HANDSHAKE token in its
//! env — on the host provider every run shares the uid, so the path
//! alone would let a sibling open ANOTHER run's socket and be served
//! under that run's principal (ADR-0035's principal-confusion case;
//! the serve refuses a client that cannot present the token). The unit
//! pins below prove the per-run paths and the token carriage; the
//! in-container proof (real sibling containers, cross-run
//! reachability) is denkhaus/work#1010.
//!
//! This file is fork-owned (new file, `fork_` prefix): an upstream
//! merge cannot silently absorb it. The upstream touch point is the
//! create-wrap branch in `providers.rs`.

use std::path::{Path, PathBuf};

use sandbox_driver::SandboxSpec;

/// The directory inside a Docker sandbox the run's channel dir mounts
/// at. A public-contract name (work#989 grill R2.2): scripts and docs
/// reference it.
pub const CONTAINER_CHANNEL_DIR: &str = "/run/fabro";

/// The channel socket inside a sandbox, container and host spelling
/// alike for Docker runs. A public-contract name (work#989 grill
/// R2.2).
pub const CONTAINER_CHANNEL_SOCKET: &str = "/run/fabro/channel.sock";

/// The socket file's name inside the per-run channel directory.
pub const CHANNEL_SOCKET_FILE: &str = "channel.sock";

/// Where a run's capability channel socket lives, as the server
/// process binds it and the sandbox daemon resolves the bind source
/// (one path by config discipline: compose pins the host directory
/// into the server container unchanged).
#[derive(Clone)]
pub struct RunChannelMount {
    host_socket: PathBuf,
    /// The run's per-spawn handshake token (work#1043 finding 1): hex,
    /// injected into the sandbox env ONLY for docker and host kinds —
    /// the serve refuses any client that cannot present it.
    token:       String,
}

// Hand-written so the handshake token never renders raw (work#1077 F2,
// mirroring the GitHubAppCredentials precedent from work#1043's
// finding 6): one `tracing::debug!("{mount:?}")` away from a log leak
// is one debug render too close.
impl std::fmt::Debug for RunChannelMount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunChannelMount")
            .field("host_socket", &self.host_socket)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl RunChannelMount {
    /// A mount for a channel socket served at `host_socket`, guarded by
    /// the handshake `token` the owning run's env must carry.
    #[must_use]
    pub fn new(host_socket: impl Into<PathBuf>, token: impl Into<String>) -> Self {
        Self {
            host_socket: host_socket.into(),
            token:       token.into(),
        }
    }

    /// The socket path on the server side (also the Host-sandbox
    /// value of `FABRO_SERVER`).
    #[must_use]
    pub fn host_socket(&self) -> &Path {
        &self.host_socket
    }

    /// The per-run directory the socket lives in — the bind source a
    /// Docker sandbox mounts. The parent of the socket path.
    #[must_use]
    pub fn host_channel_dir(&self) -> &Path {
        self.host_socket
            .parent()
            .expect("a channel socket path always has a parent directory")
    }

    /// Apply the mount to a sandbox spec for a provider of `kind`
    /// (`ProviderKind::as_str`): Docker gains the directory bind and
    /// the container-spelled env; Host gains only the host-spelled
    /// env; any other kind gains nothing (the channel is absent for
    /// remote sandboxes). The engine's `FABRO_SERVER` value overrides
    /// whatever the spec carried — the FABRO_STAGE discipline.
    pub fn apply(&self, kind: &str, spec: &mut SandboxSpec) {
        match kind {
            "docker" => {
                self.push_docker_bind(spec);
                spec.env.insert(
                    fabro_static::EnvVars::FABRO_SERVER.to_owned(),
                    CONTAINER_CHANNEL_SOCKET.to_owned(),
                );
                self.push_token(spec);
            }
            "host" => {
                spec.env.insert(
                    fabro_static::EnvVars::FABRO_SERVER.to_owned(),
                    self.host_socket.to_string_lossy().into_owned(),
                );
                self.push_token(spec);
            }
            // Remote providers (Daytona) have no filesystem path to
            // the socket: no bind, no env — the channel stays absent
            // rather than pretending to reach it (the handshake token
            // is channel material too: it goes where the channel does).
            _ => {}
        }
    }

    /// The handshake credential the serve requires (work#1043): only
    /// kinds that gain the channel gain the token — never a remote
    /// sandbox that cannot reach the socket.
    fn push_token(&self, spec: &mut SandboxSpec) {
        spec.env.insert(
            fabro_static::EnvVars::FABRO_RUN_CHANNEL_TOKEN.to_owned(),
            self.token.clone(),
        );
    }

    /// Append the per-run directory bind to the Docker provider
    /// config's `binds` array (the driver formats each entry as a
    /// `<host>:<container>:<mode>` bind string). Provider-specific
    /// JSON is the one Docker branch the kind-agnostic wrap carries
    /// (work#989 grill R2.3).
    fn push_docker_bind(&self, spec: &mut SandboxSpec) {
        let entry = serde_json::json!({
            "host":      self.host_channel_dir().to_string_lossy(),
            "container": CONTAINER_CHANNEL_DIR,
            "mode":      "rw",
        });
        if !spec.provider_config.is_object() {
            spec.provider_config = serde_json::Value::Object(serde_json::Map::new());
        }
        let object = spec
            .provider_config
            .as_object_mut()
            .expect("provider_config was just made an object");
        match object.get_mut("binds") {
            Some(binds) => {
                if binds.is_null() {
                    *binds = serde_json::Value::Array(vec![entry]);
                } else if let Some(entries) = binds.as_array_mut() {
                    entries.push(entry);
                }
            }
            None => {
                object.insert("binds".to_owned(), serde_json::Value::Array(vec![entry]));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CHANNEL_SOCKET_FILE, CONTAINER_CHANNEL_DIR, CONTAINER_CHANNEL_SOCKET, RunChannelMount,
    };

    /// Hex of 32 random bytes: the shape a handshake token has in
    /// production (work#1043) — structurally incapable of looking like
    /// a JWT or a bearer.
    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn mount() -> RunChannelMount {
        RunChannelMount::new(
            format!("/tmp/run-channels/run_1/{CHANNEL_SOCKET_FILE}"),
            TOKEN,
        )
    }

    fn spec_with_env() -> sandbox_driver::SandboxSpec {
        let mut spec =
            sandbox_driver::SandboxSpec::new(sandbox_driver::SandboxSource::HostDirectory);
        spec.env = [(
            "FABRO_SERVER".to_owned(),
            "https://declared.example".to_owned(),
        )]
        .into_iter()
        .collect();
        spec
    }

    /// Docker: the per-run directory is bound at the container channel
    /// dir and FABRO_SERVER points at the container socket, overriding
    /// whatever the spec declared.
    #[test]
    fn docker_gains_the_dir_bind_and_overrides_the_env() {
        let mut spec = spec_with_env();
        mount().apply("docker", &mut spec);

        assert_eq!(
            spec.env.get("FABRO_SERVER").map(String::as_str),
            Some(CONTAINER_CHANNEL_SOCKET),
            "the engine value overrides a declared FABRO_SERVER"
        );
        let binds = spec.provider_config["binds"]
            .as_array()
            .expect("binds became an array");
        assert_eq!(binds.len(), 1, "exactly the channel bind is added");
        assert_eq!(
            binds[0]["host"].as_str().unwrap(),
            "/tmp/run-channels/run_1"
        );
        assert_eq!(
            binds[0]["container"].as_str().unwrap(),
            CONTAINER_CHANNEL_DIR
        );
        assert_eq!(binds[0]["mode"].as_str().unwrap(), "rw");
    }

    /// Docker: an existing binds array keeps its entries — the channel
    /// bind is appended, not substituted.
    #[test]
    fn docker_appends_to_existing_binds() {
        let mut spec = spec_with_env();
        spec.provider_config = serde_json::json!({
            "binds": [{"host": "/cache", "container": "/cache", "mode": "ro"}]
        });
        mount().apply("docker", &mut spec);

        let binds = spec.provider_config["binds"].as_array().unwrap();
        assert_eq!(binds.len(), 2);
        assert_eq!(binds[0]["host"].as_str().unwrap(), "/cache");
        assert_eq!(
            binds[1]["container"].as_str().unwrap(),
            CONTAINER_CHANNEL_DIR
        );
    }

    /// Host: no bind is touched, and FABRO_SERVER carries the
    /// host-side socket path (the shared filesystem's spelling).
    #[test]
    fn host_gains_only_the_env() {
        let mut spec = spec_with_env();
        mount().apply("host", &mut spec);

        assert_eq!(
            spec.env.get("FABRO_SERVER").map(String::as_str),
            Some("/tmp/run-channels/run_1/channel.sock")
        );
        assert!(
            spec.provider_config.is_null(),
            "the host provider gets no provider_config surgery"
        );
    }

    /// Remote providers gain nothing: the channel stays honestly
    /// absent, a declared FABRO_SERVER is left as it was.
    #[test]
    fn daytona_gains_nothing() {
        let mut spec = spec_with_env();
        mount().apply("daytona", &mut spec);

        assert_eq!(
            spec.env.get("FABRO_SERVER").map(String::as_str),
            Some("https://declared.example"),
            "a remote sandbox keeps whatever it declared — no half-channel"
        );
        assert!(spec.provider_config.is_null());
    }

    /// The ADR-0035 acceptance half the mount owns, as amended by
    /// work#1043: the sandbox env gains the ADDRESS and the OWNING
    /// run's HANDSHAKE token — and nothing else. No bearer, no JWT, no
    /// Authorization anywhere: the worker bearer never leaves the
    /// server process; the handshake token is opaque hex, the
    /// connection alone is no authority on the host provider.
    #[test]
    fn the_mount_injects_address_and_handshake_only() {
        for kind in ["docker", "host"] {
            let mut spec = spec_with_env();
            mount().apply(kind, &mut spec);

            assert_eq!(
                spec.env.len(),
                2,
                "{kind}: exactly FABRO_SERVER and the handshake token, got {:?}",
                spec.env.keys().collect::<Vec<_>>()
            );
            assert!(spec.env.contains_key("FABRO_SERVER"));
            let token = spec
                .env
                .get(fabro_static::EnvVars::FABRO_RUN_CHANNEL_TOKEN)
                .unwrap_or_else(|| panic!("{kind}: the handshake token is injected"));
            assert_eq!(token, TOKEN, "{kind}: the owning run's own token");
            // JWT/bearer shape is checked on the TOKEN alone — the
            // FABRO_SERVER value is a path and may contain dots.
            assert!(
                !token.contains('.') && !token.contains("eyJ") && !token.contains("Bearer"),
                "{kind}: the handshake token is opaque, never bearer-shaped"
            );
            for value in spec.env.values() {
                assert!(
                    !value.contains("eyJ") && !value.contains("Bearer"),
                    "{kind}: no bearer or JWT material in the sandbox env"
                );
            }
        }
    }

    /// Remote sandboxes stay channel-free — including the token: a
    /// value that cannot reach the socket must not travel to where it
    /// cannot be used (work#1043: the token goes where the channel
    /// does).
    #[test]
    fn the_remote_kind_gains_no_token() {
        let mut spec = spec_with_env();
        mount().apply("daytona", &mut spec);

        assert!(
            !spec
                .env
                .contains_key(fabro_static::EnvVars::FABRO_RUN_CHANNEL_TOKEN),
            "a remote sandbox carries no channel handshake"
        );
    }

    /// The handshake token must never render raw through Debug
    /// (work#1077 F2, the finding-6 precedent): one debug render away
    /// from a log leak is one render too close.
    #[test]
    fn debug_renders_the_token_redacted() {
        let rendered = format!("{:?}", mount());
        assert!(
            rendered.contains("<redacted>"),
            "Debug must redact the token, got: {rendered}"
        );
        assert!(
            !rendered.contains(TOKEN),
            "Debug must not carry the raw token, got: {rendered}"
        );
    }
}
