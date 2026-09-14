//! [`RunSandbox`] as the [`Environment`] pebble's coding agent runs in.
//!
//! Pebble's tools speak the `Environment` contract; fabro's one sandbox type
//! speaks the sandbox driver's facets. This module is the mapping between the
//! two, and nothing else: every path resolves the way fabro resolves it, every
//! command runs through [`SandboxExec`](crate::SandboxExec) with fabro's
//! exec policy, and every failure keeps its driver cause. There is no adapter
//! struct; a run sandbox *is* an environment.
//!
//! Where the two contracts differ, pebble's wins here because the model reads
//! pebble's: a glob that pebble rejects is rejected before the driver sees it,
//! a directory listing is in tree order, and a command with no retention cap
//! still drains under the driver's default buffer rather than without bound.
//! Output a provider lost on its own transport
//! ([`ExecStreamingResult::output_loss`]) has no slot in pebble's contract,
//! so it is written where the model already reads: one line at the end of
//! stderr.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use pebble_coding_agent::environment::support::{capture_stats, tree_order, validate_glob};
use pebble_coding_agent::environment::{
    DirEntry, EnvResult, Environment, EnvironmentError, EnvironmentErrorKind, ExecOutcome,
    ExecOutputSink, ExecOutputStream, ExecRequest, ExecResult, GrepOptions,
};
use sandbox_driver::{
    ExecControls, ExecSpec, ExecStreamingResult, FileKind, OutputLoss, OutputSink, OutputStream,
};
use tracing::warn;

use crate::driver_sandbox::RunSandbox;
use crate::exec::{ExecResultExt as _, command_termination, program_exit_code};
use crate::sandbox;

#[async_trait]
impl Environment for RunSandbox {
    fn working_directory(&self) -> &str {
        Self::working_directory(self)
    }

    fn platform(&self) -> &str {
        Self::platform(self)
    }

    fn os_version(&self) -> String {
        Self::os_version(self)
    }

    async fn read_file_bytes(&self, path: &str) -> EnvResult<Vec<u8>> {
        Self::read_file_bytes(self, path)
            .await
            .map_err(|error| environment_error(&format!("Failed to read {path}"), error))
    }

    async fn write_file(&self, path: &str, content: &str) -> EnvResult<()> {
        Self::write_file(self, path, content)
            .await
            .map_err(|error| environment_error(&format!("Failed to write {path}"), error))
    }

    async fn rename_file(&self, source: &str, destination: &str) -> EnvResult<()> {
        let resolved_source = self.resolve_for_environment(source);
        let resolved_destination = self.resolve_for_environment(destination);
        if !Self::file_exists(self, source)
            .await
            .map_err(|error| environment_error(&format!("Failed to stat {source}"), error))?
        {
            return Err(EnvironmentError::new(
                EnvironmentErrorKind::NotFound,
                format!("Failed to move {source}: file does not exist"),
            ));
        }
        // The same path spelled twice is a move to itself, which must leave
        // the file where it is. Aliases the sandbox's own filesystem would
        // resolve (a symlinked parent, a hard link) are not checked: fabro has
        // no remote `realpath`, and a driver `mv a a` is a no-op anyway.
        if normalize(&resolved_source) == normalize(&resolved_destination) {
            return Ok(());
        }
        let handle = self
            .handle()
            .map_err(|error| environment_error("Sandbox is not initialized", error))?;
        // The destination's parent is created first, and a parent that is a
        // file fails here, before anything has moved, so the source stays
        // intact as the contract requires.
        if let Some(parent) = parent_directory(&resolved_destination) {
            handle.fs().create_dir(parent).await.map_err(|error| {
                environment_error(
                    &format!("Failed to create the parent directory of {destination}"),
                    crate::Error::from(error),
                )
            })?;
        }
        handle
            .fs()
            .rename(&resolved_source, &resolved_destination)
            .await
            .map_err(|error| {
                environment_error(
                    &format!("Failed to move {source} to {destination}"),
                    crate::Error::from(error),
                )
            })
    }

    async fn delete_file(&self, path: &str) -> EnvResult<()> {
        // The driver's delete is idempotent; pebble's is a `remove_file`, which
        // reports a path that is not there.
        if !Self::file_exists(self, path)
            .await
            .map_err(|error| environment_error(&format!("Failed to stat {path}"), error))?
        {
            return Err(EnvironmentError::new(
                EnvironmentErrorKind::NotFound,
                format!("Failed to delete {path}: file does not exist"),
            ));
        }
        Self::delete_file(self, path)
            .await
            .map_err(|error| environment_error(&format!("Failed to delete {path}"), error))
    }

    async fn file_exists(&self, path: &str) -> EnvResult<bool> {
        Self::file_exists(self, path)
            .await
            .map_err(|error| environment_error(&format!("Failed to stat {path}"), error))
    }

    async fn list_directory(&self, path: &str, depth: Option<usize>) -> EnvResult<Vec<DirEntry>> {
        let mut entries: Vec<DirEntry> = Self::list_directory(self, path, depth)
            .await
            .map_err(|error| environment_error(&format!("Failed to list {path}"), error))?
            .into_iter()
            .map(|entry| DirEntry {
                is_dir: entry.kind == FileKind::Directory,
                size:   (entry.kind == FileKind::File)
                    .then_some(entry.size)
                    .flatten(),
                name:   entry.path,
            })
            .collect();
        // The driver lists in flat lexicographic order of the whole relative
        // path, where `foo-bar` sorts between `foo` and `foo/x`. Pebble lists
        // in tree order, and says how.
        tree_order(&mut entries);
        Ok(entries)
    }

    async fn grep(
        &self,
        pattern: &str,
        path: &str,
        options: &GrepOptions,
    ) -> EnvResult<Vec<String>> {
        let mut driver_options = sandbox_driver::GrepOptions::default();
        driver_options.case_insensitive = options.case_insensitive;
        driver_options.max_matches = options.max_results;
        driver_options.include = options.glob_filter.clone();
        let matches = Self::grep(self, pattern, path, &driver_options)
            .await
            .map_err(|error| environment_error("Failed to search file contents", error))?;
        Ok(matches
            .into_iter()
            .map(|found| format!("{}:{}:{}", found.path, found.line_number, found.line))
            .collect())
    }

    async fn glob(&self, pattern: &str, path: Option<&str>) -> EnvResult<Vec<String>> {
        // Validated by pebble's own grammar before the driver sees the
        // pattern, so the reason reaches the model in pebble's words and the
        // patterns pebble rejects are rejected even where fabro's glob would
        // accept them.
        validate_glob(pattern)?;
        Self::glob(self, pattern, path)
            .await
            .map_err(|error| environment_error("Failed to match files", error))
    }

    async fn exec(&self, request: ExecRequest<'_>) -> EnvResult<ExecOutcome> {
        let ExecRequest {
            command,
            timeout_ms,
            working_dir,
            env_vars,
            cancel_token,
            output_bytes_cap,
            output_sink,
        } = request;
        let mut spec = ExecSpec::bash(command).no_timeout();
        if let Some(timeout_ms) = timeout_ms {
            spec = spec.timeout(Duration::from_millis(timeout_ms));
        }
        if let Some(dir) = working_dir {
            spec = spec.working_dir(dir);
        }
        for (key, value) in env_vars.into_iter().flatten() {
            spec = spec.env_var(key, value);
        }
        let controls = ExecControls {
            term: cancel_token,
            sink: output_sink.map(adapt_output_sink),
            // `None` asks pebble for no cap at all. Fabro's exec policy fills
            // its default buffer when the cap is unset, so a command with no
            // cap drains under that default rather than without bound; the
            // capture counts still say what was dropped.
            retained_output_limit: output_bytes_cap,
            ..ExecControls::default()
        };
        let streaming = self
            .exec_command_streaming(spec, controls)
            .await
            .map_err(|error| {
                let kind = match error.driver() {
                    Some(sandbox_driver::Error::Transport(_)) => EnvironmentErrorKind::Io,
                    Some(sandbox_driver::Error::Unsupported { .. }) => {
                        EnvironmentErrorKind::Unsupported
                    }
                    _ => EnvironmentErrorKind::Spawn,
                };
                EnvironmentError::with_source(kind, "Failed to run the command", error)
            })?;
        Ok(exec_outcome(
            streaming,
            output_bytes_cap,
            program_name(command),
        ))
    }
}

/// Pebble's outcome for a finished command: the driver's result read the way
/// fabro reads it, plus the provider's own output loss written where the
/// model reads stderr.
///
/// A provider whose transport tore (Daytona's text-only toolbox) completes
/// the command and reports what it discarded in
/// [`ExecStreamingResult::output_loss`] rather than failing it. The frames
/// are gone, the stream they belonged to is unknown, and the counts are of
/// encoded bytes, so they cannot be folded into either stream's capture
/// accounting without guessing; the loss is one line at the end of stderr,
/// where the model and the run log see it, and one log event for the
/// operator. The driver's `truncated` flags on the captures already say the
/// counts undercount.
fn exec_outcome(
    streaming: ExecStreamingResult,
    output_bytes_cap: Option<usize>,
    program: &str,
) -> ExecOutcome {
    let loss = streaming.output_loss;
    let result = streaming.result;
    let mut stderr = result.stderr_lossy();
    if loss.is_lossy() {
        warn!(
            program = %program,
            dropped_frames = loss.dropped_frames,
            dropped_bytes = loss.dropped_bytes,
            "Sandbox provider dropped command output"
        );
        if !stderr.is_empty() && !stderr.ends_with('\n') {
            stderr.push('\n');
        }
        stderr.push_str(&output_loss_line(loss));
    }
    ExecOutcome {
        result:            ExecResult {
            stdout: result.stdout_lossy(),
            stderr,
            exit_code: program_exit_code(result.termination, result.exit_code),
            termination: command_termination(result.termination),
            duration_ms: result.duration_ms(),
        },
        streams_separated: streaming.streams_separated,
        stdout_capture:    capture_stats(streaming.stdout_capture.observed_bytes, output_bytes_cap),
        stderr_capture:    capture_stats(streaming.stderr_capture.observed_bytes, output_bytes_cap),
    }
}

/// The line stderr ends with when the provider dropped output.
fn output_loss_line(loss: OutputLoss) -> String {
    format!(
        "[sandbox] {} output frame(s), {} bytes dropped by the provider\n",
        loss.dropped_frames, loss.dropped_bytes
    )
}

/// Bytes of a command's first word a log event carries.
const PROGRAM_NAME_BYTES: usize = 64;

/// The word a command starts with, bounded, for a log event that must not
/// carry the command itself.
fn program_name(command: &str) -> &str {
    let word = command.split_whitespace().next().unwrap_or_default();
    &word[..word.floor_char_boundary(PROGRAM_NAME_BYTES)]
}

impl RunSandbox {
    /// A caller path as the driver will see it: fabro's working directory
    /// applied where fabro applies it, and nothing more.
    fn resolve_for_environment(&self, path: &str) -> String {
        sandbox::resolve_path(path, Self::working_directory(self))
    }
}

/// Pebble's glob grammar, beyond what fabro's glob already rejects.
///
/// A path with its redundant separators and `.` segments removed, for
/// deciding whether two spellings name the same file.
fn normalize(path: &str) -> String {
    let absolute = path.starts_with('/');
    let joined = path
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect::<Vec<_>>()
        .join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

/// The directory a path is in, when the path names one.
fn parent_directory(path: &str) -> Option<&str> {
    let trimmed = path.trim_end_matches('/');
    let (parent, _) = trimmed.rsplit_once('/')?;
    if parent.is_empty() {
        return Some("/");
    }
    Some(parent)
}

/// Feeds the driver's asynchronous chunk callback into pebble's synchronous
/// sink.
fn adapt_output_sink(sink: ExecOutputSink) -> OutputSink {
    Arc::new(move |stream, chunk: Vec<u8>| {
        let stream = match stream {
            OutputStream::Stdout => ExecOutputStream::Stdout,
            OutputStream::Stderr => ExecOutputStream::Stderr,
        };
        sink(stream, &chunk);
        Box::pin(async { Ok(()) })
    })
}

/// A sandbox failure as pebble classifies it, keeping the driver cause.
fn environment_error(message: &str, error: crate::Error) -> EnvironmentError {
    let kind = match error.driver() {
        Some(sandbox_driver::Error::NotFound { .. }) => EnvironmentErrorKind::NotFound,
        Some(sandbox_driver::Error::Unsupported { .. }) => EnvironmentErrorKind::Unsupported,
        _ => EnvironmentErrorKind::Io,
    };
    EnvironmentError::with_source(kind, message, error)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use fabro_types::SandboxProviderKind;
    use pebble_coding_agent::test_support::EnvironmentContract;
    use sandbox_driver::{
        Capabilities, Exec, Filesystem, PlatformInfo, Sandbox, SandboxId, SandboxStatus, Search,
        SpawnSpec, StdioProcess, Termination,
    };
    use sandbox_driver_testing::ScriptedSandbox;

    use super::*;
    use crate::local_sandbox;
    use crate::test_support::{MockSandbox, exec_result};

    /// The run sandbox over the driver's Host provider, in a directory that
    /// goes away with the test.
    async fn host_environment() -> (tempfile::TempDir, RunSandbox) {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let sandbox = local_sandbox(directory.path().to_path_buf())
            .await
            .expect("a local sandbox");
        (directory, sandbox)
    }

    #[tokio::test]
    async fn host_files_satisfy_pebbles_environment_contract() {
        let (_directory, sandbox) = host_environment().await;
        EnvironmentContract::new(&sandbox, "contract")
            .verify_files()
            .await
            .expect("file contract");
    }

    #[tokio::test]
    async fn host_search_satisfies_pebbles_environment_contract() {
        let (_directory, sandbox) = host_environment().await;
        EnvironmentContract::new(&sandbox, "contract")
            .verify_search()
            .await
            .expect("search contract");
    }

    #[tokio::test]
    async fn host_commands_satisfy_pebbles_environment_contract() {
        let (_directory, sandbox) = host_environment().await;
        EnvironmentContract::new(&sandbox, "contract")
            .verify_commands()
            .await
            .expect("command contract");
    }

    #[tokio::test]
    async fn a_directory_listing_is_in_tree_order() {
        let (directory, sandbox) = host_environment().await;
        for name in ["foo/x.txt", "foo-bar/y.txt", "foo.txt"] {
            Environment::write_file(&sandbox, name, "content")
                .await
                .expect("fixture");
        }
        let names: Vec<String> = Environment::list_directory(&sandbox, ".", Some(2))
            .await
            .expect("listing")
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, [
            "foo",
            "foo/x.txt",
            "foo-bar",
            "foo-bar/y.txt",
            "foo.txt"
        ]);
        drop(directory);
    }

    #[test]
    fn a_path_spelled_two_ways_is_one_path() {
        assert_eq!(normalize("/work//a/./b.txt"), "/work/a/b.txt");
        assert_eq!(parent_directory("/work/a/b.txt"), Some("/work/a"));
        assert_eq!(parent_directory("/b.txt"), Some("/"));
        assert_eq!(parent_directory("b.txt"), None);
    }

    fn request(command: &str) -> ExecRequest<'_> {
        ExecRequest {
            command,
            timeout_ms: Some(10_000),
            working_dir: None,
            env_vars: None,
            cancel_token: None,
            output_bytes_cap: None,
            output_sink: None,
        }
    }

    fn output_loss(dropped_frames: u64, dropped_bytes: u64) -> OutputLoss {
        let mut loss = OutputLoss::default();
        loss.dropped_frames = dropped_frames;
        loss.dropped_bytes = dropped_bytes;
        loss
    }

    #[tokio::test]
    async fn a_lossless_command_hands_back_stderr_as_the_provider_wrote_it() {
        let mock = MockSandbox {
            exec_result: exec_result(
                "built\n",
                "warning: unused\n",
                Some(0),
                Termination::Exited,
                7,
            ),
            ..MockSandbox::linux()
        };
        let outcome = Environment::exec(&*mock.sandbox(), request("cargo build"))
            .await
            .expect("a scripted command");
        assert_eq!(outcome.result.stdout, "built\n");
        assert_eq!(outcome.result.stderr, "warning: unused\n");
        assert_eq!(outcome.result.exit_code, Some(0));
        assert_eq!(
            outcome.stderr_capture.observed_bytes,
            "warning: unused\n".len()
        );
    }

    #[test]
    fn a_provider_output_loss_ends_stderr_with_one_line() {
        let mut streaming = ExecStreamingResult::new(exec_result(
            "built\n",
            "warning: torn",
            Some(1),
            Termination::Exited,
            7,
        ));
        streaming.output_loss = output_loss(2, 4096);

        let outcome = exec_outcome(streaming, Some(1024), "cargo");

        assert_eq!(outcome.result.stdout, "built\n");
        assert_eq!(
            outcome.result.stderr,
            "warning: torn\n[sandbox] 2 output frame(s), 4096 bytes dropped by the provider\n"
        );
        assert_eq!(outcome.result.exit_code, Some(1));
        assert_eq!(outcome.result.duration_ms, 7);
        // The loss is not folded into either stream's accounting.
        assert_eq!(outcome.stdout_capture.observed_bytes, "built\n".len());
        assert_eq!(outcome.stderr_capture.observed_bytes, "warning: torn".len());
    }

    #[test]
    fn a_provider_output_loss_with_no_stderr_is_the_line_alone() {
        let mut streaming =
            ExecStreamingResult::new(exec_result("", "", Some(0), Termination::Exited, 1));
        streaming.output_loss = output_loss(1, 80);
        let outcome = exec_outcome(streaming, None, "sh");
        assert_eq!(
            outcome.result.stderr,
            "[sandbox] 1 output frame(s), 80 bytes dropped by the provider\n"
        );
    }

    #[test]
    fn a_log_event_names_the_first_word_of_a_command_bounded() {
        assert_eq!(program_name("cargo build --release"), "cargo");
        assert_eq!(program_name("  \n  ls"), "ls");
        assert_eq!(program_name(""), "");
        let long = "x".repeat(PROGRAM_NAME_BYTES + 10);
        assert_eq!(program_name(&long).len(), PROGRAM_NAME_BYTES);
        let multibyte = "é".repeat(PROGRAM_NAME_BYTES);
        assert!(program_name(&multibyte).len() <= PROGRAM_NAME_BYTES);
    }

    /// The driver's scripted sandbox with an exec facet that reports a
    /// provider output loss on every command, as Daytona does after a torn
    /// frame. The scripted double itself has no knob for the loss.
    struct LossySandbox {
        inner: Arc<ScriptedSandbox>,
        exec:  LossyExec,
    }

    struct LossyExec {
        inner: Arc<ScriptedSandbox>,
        loss:  OutputLoss,
    }

    impl LossySandbox {
        fn new(inner: Arc<ScriptedSandbox>, loss: OutputLoss) -> Self {
            Self {
                exec: LossyExec {
                    inner: Arc::clone(&inner),
                    loss,
                },
                inner,
            }
        }
    }

    #[async_trait]
    impl Exec for LossyExec {
        async fn run(&self, spec: &ExecSpec) -> sandbox_driver::Result<sandbox_driver::ExecResult> {
            self.inner.scripted_exec().run(spec).await
        }

        async fn run_streaming(
            &self,
            spec: &ExecSpec,
            controls: ExecControls,
        ) -> sandbox_driver::Result<ExecStreamingResult> {
            let mut streaming = self
                .inner
                .scripted_exec()
                .run_streaming(spec, controls)
                .await?;
            streaming.output_loss = self.loss;
            streaming.stdout_capture.truncated = true;
            streaming.stderr_capture.truncated = true;
            Ok(streaming)
        }

        async fn spawn_stdio(&self, spec: &SpawnSpec) -> sandbox_driver::Result<StdioProcess> {
            self.inner.scripted_exec().spawn_stdio(spec).await
        }
    }

    #[async_trait]
    impl Sandbox for LossySandbox {
        fn id(&self) -> &SandboxId {
            self.inner.id()
        }

        fn capabilities(&self) -> &Capabilities {
            // The scripted sandbox's builder method of the same name shadows
            // the trait's.
            Sandbox::capabilities(&*self.inner)
        }

        async fn describe(&self) -> sandbox_driver::Result<SandboxStatus> {
            self.inner.describe().await
        }

        fn working_directory(&self) -> &str {
            self.inner.working_directory()
        }

        async fn environment(&self) -> sandbox_driver::Result<BTreeMap<String, String>> {
            self.inner.environment().await
        }

        fn runtime_directory(&self) -> Option<&str> {
            Sandbox::runtime_directory(&*self.inner)
        }

        async fn platform_info(&self) -> sandbox_driver::Result<PlatformInfo> {
            self.inner.platform_info().await
        }

        async fn start(&self) -> sandbox_driver::Result<()> {
            self.inner.start().await
        }

        async fn stop(&self) -> sandbox_driver::Result<()> {
            self.inner.stop().await
        }

        async fn delete(&self) -> sandbox_driver::Result<()> {
            self.inner.delete().await
        }

        fn exec(&self) -> &dyn Exec {
            &self.exec
        }

        fn fs(&self) -> &dyn Filesystem {
            self.inner.fs()
        }

        fn provider_search(&self) -> Option<&dyn Search> {
            self.inner.provider_search()
        }
    }

    #[tokio::test]
    async fn a_lossy_command_tells_the_model_what_the_provider_dropped() {
        let scripted =
            Arc::new(
                ScriptedSandbox::with_id_and_working_dir("lossy", "/work")
                    .platform(PlatformInfo::new("linux", "x86_64", "Linux 6.1.0")),
            );
        scripted.scripted_exec().set_default(exec_result(
            "built\n",
            "warning: torn",
            Some(0),
            Termination::Exited,
            7,
        ));
        let sandbox = RunSandbox::new_with_platform(
            SandboxProviderKind::DAYTONA,
            Arc::new(LossySandbox::new(scripted, output_loss(3, 512))),
            "linux",
            "Linux 6.1.0",
        );

        let outcome = Environment::exec(&sandbox, request("cargo build"))
            .await
            .expect("a lossy command completes rather than fails");

        assert_eq!(outcome.result.stdout, "built\n");
        assert_eq!(
            outcome.result.stderr,
            "warning: torn\n[sandbox] 3 output frame(s), 512 bytes dropped by the provider\n"
        );
        assert_eq!(outcome.result.exit_code, Some(0));
        assert!(outcome.streams_separated);
    }
}
