//! Fabro's command execution policy over the sandbox-driver [`Exec`] facet.
//!
//! A command runs as Bash source under `bash -c` with `BASH_ENV` blanked,
//! and ends in one of three ways:
//!
//! - **timeout**: the spec's timeout fires and the provider runs the stop
//!   ladder fabro asks for — `TERM`, then `KILL` after
//!   [`SandboxExec::stop_grace`]. The result reports
//!   [`CommandTermination::TimedOut`].
//! - **cancellation**: the caller's [`CancellationToken`] is the `term` stop;
//!   the provider escalates to `KILL` after the same grace. The result reports
//!   [`CommandTermination::Cancelled`].
//! - **exit**: the process ended on its own.
//!
//! Output is drained regardless of the retention cap, redacted only when a
//! tail is rendered for events or logs, and delivered live through the
//! caller's callback. Explicit environment variables pass through a
//! fail-closed secret filter under [`ExplicitEnvPolicy::FilterSensitive`],
//! matching what the Host provider already does for inherited variables.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use fabro_static::EnvVars;
use fabro_types::{CommandOutputStream, CommandTermination};
use sandbox_driver::{
    BASH_ENV_VAR, CaptureStats, Exec, ExecControls, ExecSpec, OutputStream, SpawnSpec,
    StdioProcessHandle as DriverStdioProcessHandle, Termination, TransportError,
};
use tokio_util::sync::CancellationToken;

use crate::sandbox::{
    CommandOutputCallback, ExecResult, ExecStreamingRequest, ExecStreamingResult,
    OutputCaptureStats, StderrCollector, StdioProcess, StdioProcessControl, StdioProcessHandle,
    StdioProcessTermination,
};

/// Time between `TERM` and `KILL` when fabro stops a command.
pub const DEFAULT_STOP_GRACE: Duration = Duration::from_secs(2);

/// Retention when a caller sets no cap: enough for any build log fabro
/// renders, bounded so a runaway command cannot exhaust memory.
pub const DEFAULT_RETAINED_OUTPUT_BYTES: usize = sandbox_driver::DEFAULT_BUFFER_BYTES;

/// How explicit per-command environment variables are treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplicitEnvPolicy {
    /// Drop variables whose names look like credentials unless safelisted.
    /// Used where the command runs on the worker host and the caller's env
    /// may carry worker secrets.
    FilterSensitive,
    /// Pass every variable through. Used for isolated providers, where the
    /// caller composed the environment deliberately.
    TrustCaller,
}

/// Variables that look like credentials but are needed by ordinary tools.
const ENV_SAFELIST: &[&str] = &[
    EnvVars::PATH,
    EnvVars::HOME,
    EnvVars::USER,
    EnvVars::SHELL,
    EnvVars::LANG,
    EnvVars::TERM,
    EnvVars::TMPDIR,
    EnvVars::GOPATH,
    EnvVars::CARGO_HOME,
    EnvVars::NVM_DIR,
];

/// Whether an environment variable name looks like a credential.
#[must_use]
pub fn is_sensitive_env_var(key: &str) -> bool {
    if ENV_SAFELIST.contains(&key) {
        return false;
    }
    let lower = key.to_lowercase();
    lower.ends_with("_api_key")
        || lower.ends_with("_secret")
        || lower.ends_with("_token")
        || lower.ends_with("_password")
        || lower.ends_with("_credential")
}

/// Fabro's exec policy bound to one driver [`Exec`] facet.
pub struct SandboxExec<'a> {
    exec:        &'a dyn Exec,
    env_policy:  ExplicitEnvPolicy,
    stop_grace:  Duration,
    /// Where a command runs when the caller names no directory. `None`
    /// leaves the choice to the provider's own working directory.
    working_dir: Option<String>,
}

impl<'a> SandboxExec<'a> {
    #[must_use]
    pub fn new(exec: &'a dyn Exec, env_policy: ExplicitEnvPolicy) -> Self {
        Self {
            exec,
            env_policy,
            stop_grace: DEFAULT_STOP_GRACE,
            working_dir: None,
        }
    }

    /// The directory commands run in when the caller names none. Fabro's
    /// working directory can sit below the provider's (a cloned repository
    /// inside the container workspace), so it is passed explicitly.
    #[must_use]
    pub fn with_working_dir(mut self, working_dir: impl Into<String>) -> Self {
        self.working_dir = Some(working_dir.into());
        self
    }

    /// Time between `TERM` and `KILL` when a command is stopped; the
    /// provider runs the ladder.
    #[must_use]
    pub fn with_stop_grace(mut self, stop_grace: Duration) -> Self {
        self.stop_grace = stop_grace;
        self
    }

    #[must_use]
    pub fn stop_grace(&self) -> Duration {
        self.stop_grace
    }

    /// Runs Bash source to completion and returns its captured output.
    ///
    /// Equivalent to `bash -c <command>` with a clean, non-login shell: no
    /// `errexit`, no `pipefail`, `BASH_ENV` blanked. A caller that wants
    /// different semantics writes them into the command.
    pub async fn run(
        &self,
        command: &str,
        timeout: Option<Duration>,
        working_dir: Option<&str>,
        env_vars: Option<&HashMap<String, String>>,
        cancel_token: Option<CancellationToken>,
    ) -> crate::Result<ExecResult> {
        let streaming = self
            .run_streaming(ExecStreamingRequest {
                timeout_ms: timeout
                    .map(|timeout| u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX)),
                working_dir,
                env_vars,
                cancel_token,
                ..ExecStreamingRequest::new(command)
            })
            .await?;
        Ok(streaming.result)
    }

    /// Runs Bash source, delivering output through `request.output_callback`
    /// as it arrives. Same interpreter contract as [`Self::run`].
    pub async fn run_streaming(
        &self,
        request: ExecStreamingRequest<'_>,
    ) -> crate::Result<ExecStreamingResult> {
        let ExecStreamingRequest {
            command,
            timeout_ms,
            working_dir,
            env_vars,
            cancel_token,
            stdin,
            output_callback,
            stream_output_bytes_cap,
        } = request;

        let mut spec = ExecSpec::bash(command)
            .no_timeout()
            .stop_grace(self.stop_grace);
        if let Some(timeout_ms) = timeout_ms {
            spec = spec.timeout(Duration::from_millis(timeout_ms));
        }
        if let Some(dir) = working_dir.or(self.working_dir.as_deref()) {
            spec = spec.working_dir(dir);
        }
        for (key, value) in self.explicit_env(env_vars) {
            spec = spec.env_var(key, value);
        }
        if let Some(bytes) = stdin {
            spec = spec.stdin(bytes);
        }

        // The caller's cancellation is the `term` stop; the provider runs
        // the grace and the `kill` itself.
        let controls = ExecControls {
            term:                  cancel_token,
            kill:                  None,
            stdin:                 None,
            sink:                  output_callback.map(adapt_output_callback),
            retained_output_limit: Some(
                stream_output_bytes_cap.unwrap_or(DEFAULT_RETAINED_OUTPUT_BYTES),
            ),
        };

        let streaming = self.exec.run_streaming(&spec, controls).await?;

        let termination = map_termination(streaming.result.termination);
        let duration_ms = duration_ms(streaming.result.duration);
        Ok(ExecStreamingResult {
            result:            ExecResult {
                stdout: String::from_utf8_lossy(&streaming.result.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&streaming.result.stderr).into_owned(),
                exit_code: exit_code_for(termination, streaming.result.exit_code),
                termination,
                duration_ms,
            },
            streams_separated: streaming.streams_separated,
            live_streaming:    streaming.live_streaming,
            stdout_capture:    capture_stats(streaming.stdout_capture),
            stderr_capture:    capture_stats(streaming.stderr_capture),
        })
    }

    /// Launches a long-lived process with bidirectional stdio.
    ///
    /// `command` is evaluated under the same non-login Bash contract before
    /// the shell replaces itself with the requested process. Cancelling
    /// `cancel_token` terminates the process.
    pub async fn spawn_stdio(
        &self,
        command: &str,
        working_dir: Option<&str>,
        env_vars: Option<&HashMap<String, String>>,
        cancel_token: Option<CancellationToken>,
    ) -> crate::Result<StdioProcess> {
        let mut spec = SpawnSpec::bash(format!("exec {command}"));
        if let Some(dir) = working_dir.or(self.working_dir.as_deref()) {
            spec = spec.working_dir(dir);
        }
        for (key, value) in self.explicit_env(env_vars) {
            spec = spec.env_var(key, value);
        }
        let process = self.exec.spawn_stdio(&spec).await?;
        let handle = StdioProcessHandle::new(DriverStdioControl {
            handle: Arc::from(process.handle),
        });
        if let Some(token) = cancel_token {
            let handle = handle.clone();
            tokio::spawn(async move {
                token.cancelled().await;
                if let Err(error) = handle.terminate().await {
                    tracing::warn!(error = %error, "failed to terminate stdio process on cancel");
                }
            });
        }
        Ok(StdioProcess {
            stdin: process.stdin,
            stdout: process.stdout,
            stderr: StderrCollector::from_driver_tail(process.stderr_tail),
            handle,
        })
    }

    /// The explicit environment after policy: `BASH_ENV` never passes,
    /// because the Bash helper blanks it and a caller value would override
    /// that; credential-shaped names pass only under `TrustCaller`.
    fn explicit_env(&self, env_vars: Option<&HashMap<String, String>>) -> Vec<(String, String)> {
        let mut entries: Vec<(String, String)> = env_vars
            .into_iter()
            .flatten()
            .filter(|(key, _)| key.as_str() != BASH_ENV_VAR)
            .filter(|(key, _)| {
                self.env_policy == ExplicitEnvPolicy::TrustCaller || !is_sensitive_env_var(key)
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        entries.sort();
        entries
    }
}

/// The driver says how the command ended; fabro's vocabulary has two stops.
/// A timeout is the provider's deadline (the ladder ran for it); a
/// cancelled or killed command was stopped by the caller's token, by a
/// foreign `kill`, or by a provider-side abort — it did not finish and no
/// deadline passed.
fn map_termination(termination: Termination) -> CommandTermination {
    match termination {
        Termination::TimedOut => CommandTermination::TimedOut,
        Termination::Cancelled | Termination::Killed => CommandTermination::Cancelled,
        // `Exited`, or a provider that could not tell how the command ended.
        // Nothing asserts success here: `exit_code` is whatever was observed
        // and `is_success` still requires `Some(0)`.
        _ => CommandTermination::Exited,
    }
}

/// An exit code is only the command's own when it exited on its own. A
/// stopped command may still report the shell's `128 + signal` (143 for a
/// trapped `TERM`), which callers must not mistake for a program result.
fn exit_code_for(termination: CommandTermination, exit_code: Option<i32>) -> Option<i32> {
    // `CommandTermination` is non-exhaustive: only a command that exited on
    // its own owns its exit code.
    matches!(termination, CommandTermination::Exited)
        .then_some(exit_code)
        .flatten()
}

fn capture_stats(stats: CaptureStats) -> OutputCaptureStats {
    OutputCaptureStats {
        observed_bytes: stats.observed_bytes,
        retained_bytes: stats.retained_bytes,
        omitted_bytes:  stats.omitted_bytes,
    }
}

fn adapt_output_callback(callback: CommandOutputCallback) -> sandbox_driver::OutputSink {
    Arc::new(move |stream, chunk| {
        let stream = match stream {
            OutputStream::Stdout => CommandOutputStream::Stdout,
            OutputStream::Stderr => CommandOutputStream::Stderr,
        };
        let callback = Arc::clone(&callback);
        Box::pin(async move {
            callback(stream, chunk).await.map_err(|error| {
                sandbox_driver::Error::Transport(TransportError::with_source(
                    "command output callback failed",
                    error,
                ))
            })
        })
    })
}

/// The provider's measured run time in whole milliseconds.
fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

struct DriverStdioControl {
    handle: Arc<dyn DriverStdioProcessHandle>,
}

#[async_trait]
impl StdioProcessControl for DriverStdioControl {
    async fn terminate(&self) -> crate::Result<()> {
        self.handle.terminate().await;
        Ok(())
    }

    async fn wait(&self) -> crate::Result<StdioProcessTermination> {
        let (termination, exit_code) = self.handle.wait().await;
        let termination = map_termination(termination);
        Ok(StdioProcessTermination {
            termination,
            exit_code: exit_code_for(termination, exit_code),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use sandbox_driver::{SandboxProvider as _, SandboxSource, SandboxSpec};
    use sandbox_driver_host::HostProvider;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::{fs, time};

    use super::*;

    struct HostFixture {
        workspace: tempfile::TempDir,
        provider:  HostProvider,
        sandbox:   Arc<dyn sandbox_driver::Sandbox>,
    }

    impl HostFixture {
        async fn new() -> Self {
            let workspace = tempfile::tempdir().unwrap();
            let provider = HostProvider::new();
            let sandbox = provider
                .create(
                    &SandboxSpec::new(SandboxSource::HostDirectory)
                        .working_directory(workspace.path().display().to_string()),
                    None,
                )
                .await
                .unwrap();
            Self {
                workspace,
                provider,
                sandbox,
            }
        }

        fn exec(&self, policy: ExplicitEnvPolicy) -> SandboxExec<'_> {
            let _ = &self.provider;
            SandboxExec::new(self.sandbox.exec(), policy)
        }
    }

    async fn run(fixture: &HostFixture, command: &str) -> ExecResult {
        fixture
            .exec(ExplicitEnvPolicy::FilterSensitive)
            .run(command, Some(Duration::from_secs(10)), None, None, None)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn runs_bash_source_and_reports_exit_code_and_streams() {
        let fixture = HostFixture::new().await;
        let result = run(&fixture, "echo out; echo err >&2; exit 3").await;
        assert_eq!(result.stdout, "out\n");
        assert_eq!(result.stderr, "err\n");
        assert_eq!(result.exit_code, Some(3));
        assert_eq!(result.termination, CommandTermination::Exited);
        assert!(!result.is_success());
        assert!(run(&fixture, "true").await.is_success());
    }

    #[tokio::test]
    async fn runs_bash_only_syntax_in_a_clean_non_login_shell() {
        let fixture = HostFixture::new().await;
        let result = run(
            &fixture,
            "[[ -n ${BASH_VERSION:-} ]] && shopt -q login_shell && echo login || echo nonlogin; \
             set -o | grep -E '^(errexit|pipefail)' | awk '{print $2}' | sort -u",
        )
        .await;
        assert_eq!(result.stdout, "nonlogin\noff\n", "{result:?}");
    }

    #[tokio::test]
    async fn a_caller_supplied_bash_env_never_runs() {
        let fixture = HostFixture::new().await;
        let startup = fixture.workspace.path().join("startup.sh");
        fs::write(&startup, "echo startup-source-loaded\n")
            .await
            .unwrap();
        let env = HashMap::from([(BASH_ENV_VAR.to_string(), startup.display().to_string())]);
        let result = fixture
            .exec(ExplicitEnvPolicy::TrustCaller)
            .run(
                "echo body",
                Some(Duration::from_secs(10)),
                None,
                Some(&env),
                None,
            )
            .await
            .unwrap();
        assert_eq!(result.stdout, "body\n");
    }

    #[tokio::test]
    async fn filter_sensitive_drops_credential_shaped_explicit_variables() {
        let fixture = HostFixture::new().await;
        let env = HashMap::from([
            ("FABRO_WORKER_TOKEN".to_string(), "leaked".to_string()),
            ("MY_VAR".to_string(), "ok".to_string()),
        ]);
        let filtered = fixture
            .exec(ExplicitEnvPolicy::FilterSensitive)
            .run("env", Some(Duration::from_secs(10)), None, Some(&env), None)
            .await
            .unwrap();
        assert!(!filtered.stdout.contains("FABRO_WORKER_TOKEN=leaked"));
        assert!(filtered.stdout.contains("MY_VAR=ok"));

        let trusted = fixture
            .exec(ExplicitEnvPolicy::TrustCaller)
            .run("env", Some(Duration::from_secs(10)), None, Some(&env), None)
            .await
            .unwrap();
        assert!(trusted.stdout.contains("FABRO_WORKER_TOKEN=leaked"));
    }

    #[test]
    fn sensitive_name_classification_matches_the_worker_policy() {
        for key in [
            "OPENAI_API_KEY",
            "DB_PASSWORD",
            "AWS_SECRET",
            "AUTH_TOKEN",
            "MY_CREDENTIAL",
            "FABRO_WORKER_TOKEN",
        ] {
            assert!(is_sensitive_env_var(key), "{key}");
        }
        for key in ["PATH", "HOME", "MY_VAR", "GITHUB_ACTOR"] {
            assert!(!is_sensitive_env_var(key), "{key}");
        }
    }

    #[tokio::test]
    async fn timeout_runs_the_ladder_and_reports_timed_out() {
        let fixture = HostFixture::new().await;
        let started = Instant::now();
        let result = fixture
            .exec(ExplicitEnvPolicy::FilterSensitive)
            .run(
                "sleep 10",
                Some(Duration::from_millis(200)),
                None,
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(result.termination, CommandTermination::TimedOut);
        assert_eq!(result.exit_code, None);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "sleep honours TERM, so KILL should not have been needed"
        );
    }

    #[tokio::test]
    async fn a_command_that_ignores_term_is_killed_after_the_grace_period() {
        let fixture = HostFixture::new().await;
        let started = Instant::now();
        let result = fixture
            .exec(ExplicitEnvPolicy::FilterSensitive)
            .with_stop_grace(Duration::from_millis(300))
            .run(
                "trap '' TERM; sleep 10",
                Some(Duration::from_millis(100)),
                None,
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(result.termination, CommandTermination::TimedOut);
        let elapsed = started.elapsed();
        assert!(elapsed >= Duration::from_millis(400), "{elapsed:?}");
        assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
    }

    #[tokio::test]
    async fn cancellation_reports_cancelled() {
        let fixture = HostFixture::new().await;
        let token = CancellationToken::new();
        let cancel = token.clone();
        tokio::spawn(async move {
            time::sleep(Duration::from_millis(100)).await;
            cancel.cancel();
        });
        let result = fixture
            .exec(ExplicitEnvPolicy::FilterSensitive)
            .run(
                "sleep 10",
                Some(Duration::from_secs(30)),
                None,
                None,
                Some(token),
            )
            .await
            .unwrap();
        assert_eq!(result.termination, CommandTermination::Cancelled);
        assert_eq!(result.exit_code, None);
    }

    #[tokio::test]
    async fn streaming_delivers_live_chunks_and_drains_past_the_retention_cap() {
        let fixture = HostFixture::new().await;
        let seen = Arc::new(Mutex::new(Vec::<u8>::new()));
        let sink_seen = Arc::clone(&seen);
        let callback: CommandOutputCallback = Arc::new(move |stream, chunk| {
            let seen = Arc::clone(&sink_seen);
            Box::pin(async move {
                assert_eq!(stream, CommandOutputStream::Stdout);
                seen.lock().unwrap().extend_from_slice(&chunk);
                Ok(())
            })
        });
        let streaming = fixture
            .exec(ExplicitEnvPolicy::FilterSensitive)
            .run_streaming(ExecStreamingRequest {
                timeout_ms: Some(10_000),
                output_callback: Some(callback),
                stream_output_bytes_cap: Some(64),
                ..ExecStreamingRequest::new("for i in $(seq 1 200); do echo line-$i; done")
            })
            .await
            .unwrap();
        assert!(streaming.result.is_success());
        assert!(streaming.live_streaming);
        assert!(streaming.streams_separated);
        let delivered = seen.lock().unwrap().len();
        assert_eq!(streaming.stdout_capture.observed_bytes, delivered);
        assert!(streaming.stdout_capture.omitted_bytes > 0);
        assert!(streaming.result.stdout.len() <= 64);
        assert!(streaming.result.stdout.starts_with("line-1\n"));
        assert!(streaming.result.stdout.ends_with("line-200\n"));
    }

    #[tokio::test]
    async fn stdin_bytes_are_written_exactly_then_closed() {
        let fixture = HostFixture::new().await;
        let stdin = b"first line\n$(touch must-not-run)\nlast line".to_vec();
        let streaming = fixture
            .exec(ExplicitEnvPolicy::FilterSensitive)
            .run_streaming(ExecStreamingRequest {
                timeout_ms: Some(10_000),
                stdin: Some(stdin.clone()),
                ..ExecStreamingRequest::new("cat; test -e must-not-run && echo RAN")
            })
            .await
            .unwrap();
        assert_eq!(streaming.result.stdout.as_bytes(), stdin.as_slice());
    }

    #[tokio::test]
    async fn a_failing_output_callback_stops_the_command_with_an_error() {
        let fixture = HostFixture::new().await;
        let callback: CommandOutputCallback =
            Arc::new(|_, _| Box::pin(async { Err(crate::Error::message("consumer gave up")) }));
        let error = fixture
            .exec(ExplicitEnvPolicy::FilterSensitive)
            .run_streaming(ExecStreamingRequest {
                timeout_ms: Some(10_000),
                output_callback: Some(callback),
                ..ExecStreamingRequest::new("echo hello; sleep 5")
            })
            .await
            .map(|streaming| streaming.result.termination);
        // The driver either surfaces the sink failure or reports the command
        // cancelled by it; both keep the consumer's error visible.
        match error {
            Ok(termination) => assert_eq!(termination, CommandTermination::Cancelled),
            Err(error) => assert!(error.to_string().contains("consumer gave up"), "{error}"),
        }
    }

    #[tokio::test]
    async fn stdio_process_round_trips_lines_and_reports_exit() {
        let fixture = HostFixture::new().await;
        let process = fixture
            .exec(ExplicitEnvPolicy::FilterSensitive)
            .spawn_stdio("cat", None, None, None)
            .await
            .unwrap();
        let mut stdin = process.stdin;
        let mut stdout = BufReader::new(process.stdout);
        stdin.write_all(b"ping\n").await.unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).await.unwrap();
        assert_eq!(line, "ping\n");
        drop(stdin);
        let termination = process.handle.wait().await.unwrap();
        assert_eq!(termination.termination, CommandTermination::Exited);
        assert_eq!(termination.exit_code, Some(0));
    }

    #[tokio::test]
    async fn stdio_process_terminates_on_cancel_and_keeps_a_stderr_tail() {
        let fixture = HostFixture::new().await;
        let token = CancellationToken::new();
        let process = fixture
            .exec(ExplicitEnvPolicy::FilterSensitive)
            .spawn_stdio(
                "sh -c 'echo diag >&2; sleep 30'",
                None,
                None,
                Some(token.clone()),
            )
            .await
            .unwrap();
        time::sleep(Duration::from_millis(200)).await;
        token.cancel();
        let termination = time::timeout(Duration::from_secs(5), process.handle.wait())
            .await
            .expect("cancel terminates the process")
            .unwrap();
        assert_ne!(termination.termination, CommandTermination::Exited);
        assert_eq!(process.stderr.tail_string().await, "diag\n");
    }

    #[test]
    fn termination_mapping_reads_the_drivers_verdict() {
        assert_eq!(
            map_termination(Termination::TimedOut),
            CommandTermination::TimedOut
        );
        assert_eq!(
            map_termination(Termination::Cancelled),
            CommandTermination::Cancelled
        );
        assert_eq!(
            map_termination(Termination::Killed),
            CommandTermination::Cancelled
        );
        assert_eq!(
            map_termination(Termination::Exited),
            CommandTermination::Exited
        );
    }
}
