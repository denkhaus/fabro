use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;

use anyhow::{Context, Result};
use async_trait::async_trait;
use fabro_static::EnvVars;
use fabro_types::RunId;
use fabro_types::settings::server::LogDestination;
use futures_util::future::BoxFuture;
use tokio::io::AsyncRead;
use tokio::process::Command;

use crate::spawn_env::apply_worker_env;

#[async_trait]
pub(crate) trait WorkerRuntime: Send + Sync {
    async fn start(&self, spec: WorkerLaunchSpec) -> Result<StartedWorker>;
    async fn request_stop(&self, worker_ref: &WorkerRef);
    async fn force_stop(&self, worker_ref: &WorkerRef);
    async fn is_alive(&self, worker_ref: &WorkerRef) -> bool;
}

#[derive(Clone, Debug, PartialEq, Eq, strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub(crate) enum WorkerRef {
    /// A worker running as a local subprocess. `pre_exec_setpgid` ensures the
    /// child is the leader of its own process group with `pgid == pid`, so a
    /// single PID identifies both the process and its group.
    Local { pid: u32 },
}

impl WorkerRef {
    pub(crate) fn kind(&self) -> &'static str {
        self.into()
    }
}

pub(crate) struct WorkerLaunchSpec {
    pub(crate) executable:         PathBuf,
    pub(crate) server_target:      String,
    pub(crate) storage_dir:        PathBuf,
    pub(crate) run_dir:            PathBuf,
    pub(crate) run_id:             RunId,
    pub(crate) mode:               &'static str,
    pub(crate) worker_token:       String,
    pub(crate) log_destination:    LogDestination,
    pub(crate) fabro_log:          Option<String>,
    pub(crate) active_config_path: PathBuf,
    /// The Fabro home the server resolved, so a Petri run's skills step
    /// reads the same home whatever the worker's environment says.
    pub(crate) fabro_home:         PathBuf,
    /// Host-side directory holding the run's hook-declared files, exported
    /// to the worker as `FABRO_HOOK_ASSETS` so host-side hooks execute the
    /// staged bytes instead of embedding them in the hook command. `None`
    /// leaves the variable unset and hook scripts fall back to
    /// workspace-relative paths.
    pub(crate) hook_assets:        Option<PathBuf>,
    /// The path of this run's capability channel socket (ADR-0035),
    /// exported to the worker as `FABRO_RUN_CHANNEL_SOCK` (the
    /// `FABRO_HOOK_ASSETS` transport pattern) so the worker's
    /// `RuntimeSpec` mounts it into the run's sandboxes. `None` when
    /// the operator's kill switch is off or the spawn failed.
    pub(crate) run_channel_socket: Option<PathBuf>,
    /// The channel's per-spawn HANDSHAKE token (ADR-0035 as amended by
    /// work#1043 finding 1), exported as `FABRO_RUN_CHANNEL_TOKEN` —
    /// the serve refuses any client whose env does not carry it.
    /// Present exactly when `run_channel_socket` is.
    pub(crate) run_channel_token:  Option<String>,
}

pub(crate) struct StartedWorker {
    pub(crate) worker_ref: WorkerRef,
    pub(crate) stderr:     Pin<Box<dyn AsyncRead + Send + 'static>>,
    pub(crate) wait:       BoxFuture<'static, Result<WorkerExit>>,
}

#[derive(Debug)]
pub(crate) struct WorkerExit {
    pub(crate) success: bool,
    pub(crate) detail:  String,
}

#[derive(Default)]
pub(crate) struct LocalWorkerRuntime;

impl LocalWorkerRuntime {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn command_for_spec(spec: &WorkerLaunchSpec) -> Command {
        let worker_stdout = match spec.log_destination {
            LogDestination::Stdout => Stdio::inherit(),
            LogDestination::File => Stdio::null(),
        };
        let log_destination_env: &'static str = spec.log_destination.into();

        let mut cmd = Command::new(&spec.executable);
        cmd.arg("__run-worker")
            .arg("--server")
            .arg(&spec.server_target)
            .arg("--storage-dir")
            .arg(&spec.storage_dir)
            .arg("--run-dir")
            .arg(&spec.run_dir)
            .arg("--run-id")
            .arg(spec.run_id.to_string())
            .arg("--mode")
            .arg(spec.mode)
            .arg("--fabro-home")
            .arg(&spec.fabro_home)
            .stdin(Stdio::null())
            .stdout(worker_stdout)
            .stderr(Stdio::piped());

        apply_worker_env(&mut cmd);
        if let Some(level) = spec.fabro_log.as_deref() {
            cmd.env(EnvVars::FABRO_LOG, level);
        }
        cmd.env(EnvVars::FABRO_LOG_DESTINATION, log_destination_env);
        cmd.env(EnvVars::FABRO_CONFIG, &spec.active_config_path);
        cmd.env_remove(EnvVars::FABRO_WORKER_TOKEN);
        cmd.env(EnvVars::FABRO_WORKER_TOKEN, &spec.worker_token);
        // The GitHub App PEM is deliberately NOT injected (work#1043
        // finding 2: no worker consumer — the allowlist already
        // excludes it and spawn_env's leak test pins that; nothing
        // authority-bearing crosses on the worker's command).
        if let Some(hook_assets) = spec.hook_assets.as_deref() {
            cmd.env(EnvVars::FABRO_HOOK_ASSETS, hook_assets);
        }
        if let Some(socket) = spec.run_channel_socket.as_deref() {
            cmd.env(EnvVars::FABRO_RUN_CHANNEL_SOCK, socket);
        }
        // The handshake half of the same channel (work#1043 finding 1):
        // the serve refuses a client whose env does not carry the
        // owning run's token.
        if let Some(token) = spec.run_channel_token.as_deref() {
            cmd.env(EnvVars::FABRO_RUN_CHANNEL_TOKEN, token);
        }
        // The worker's storage anchor for host-side artifacts (the
        // judgment stream): the worker knows its storage dir as an
        // argument, and a hook script must not guess it (fabro-225f — the
        // missing anchor is why the shadow never wrote a line).
        cmd.env(EnvVars::FABRO_STORAGE_DIR, &spec.storage_dir);

        #[cfg(unix)]
        fabro_proc::pre_exec_setpgid(cmd.as_std_mut());

        cmd
    }
}

#[async_trait]
impl WorkerRuntime for LocalWorkerRuntime {
    async fn start(&self, spec: WorkerLaunchSpec) -> Result<StartedWorker> {
        let mut child = Self::command_for_spec(&spec)
            .spawn()
            .context("spawning run worker process")?;

        let Some(pid) = child.id() else {
            let _ = child.start_kill();
            anyhow::bail!("worker process did not report a PID");
        };
        let Some(stderr) = child.stderr.take() else {
            let _ = child.start_kill();
            anyhow::bail!("worker child stderr should be piped");
        };
        let stderr: Pin<Box<dyn AsyncRead + Send + 'static>> = Box::pin(stderr);
        let wait: BoxFuture<'static, Result<WorkerExit>> = Box::pin(async move {
            let status = child.wait().await.context("worker wait failed")?;
            Ok(WorkerExit {
                success: status.success(),
                detail:  status.to_string(),
            })
        });

        Ok(StartedWorker {
            worker_ref: WorkerRef::Local { pid },
            stderr,
            wait,
        })
    }

    async fn request_stop(&self, worker_ref: &WorkerRef) {
        let WorkerRef::Local { pid } = worker_ref;
        #[cfg(unix)]
        fabro_proc::sigterm_process_group(*pid);
        #[cfg(not(unix))]
        let _ = pid;
    }

    async fn force_stop(&self, worker_ref: &WorkerRef) {
        let WorkerRef::Local { pid } = worker_ref;
        #[cfg(unix)]
        fabro_proc::sigkill_process_group(*pid);
        #[cfg(not(unix))]
        let _ = pid;
    }

    async fn is_alive(&self, worker_ref: &WorkerRef) -> bool {
        let WorkerRef::Local { pid } = worker_ref;
        #[cfg(unix)]
        {
            fabro_proc::process_group_alive(*pid)
        }
        #[cfg(not(unix))]
        {
            fabro_proc::process_running(*pid)
        }
    }
}
