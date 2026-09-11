//! [`RunSandbox`] as the [`Environment`] pebble's coding agent runs in.
//!
//! Pebble's tools speak the `Environment` contract; fabro's one sandbox type
//! speaks the sandbox driver's facets. This module is the mapping between the
//! two, and nothing else: every path resolves the way fabro resolves it, every
//! command runs through [`SandboxExec`](crate::SandboxExec) with fabro's
//! environment policy, and every failure keeps its driver cause. There is no
//! adapter struct; a run sandbox *is* an environment.
//!
//! Where the two contracts differ, pebble's wins here because the model reads
//! pebble's: a glob that pebble rejects is rejected before the driver sees it,
//! a directory listing is in tree order, and a command with no retention cap
//! still drains under the driver's default buffer rather than without bound.

use std::sync::Arc;

use async_trait::async_trait;
use fabro_types::{CommandOutputStream, CommandTermination};
use fabro_util::workspace_glob::WorkspaceGlob;
use pebble_coding_agent::environment::{
    DirEntry, EnvResult, Environment, EnvironmentError, EnvironmentErrorKind, ExecOutcome,
    ExecOutputSink, ExecOutputStream, ExecRequest, ExecResult, GrepOptions,
};
use pebble_coding_agent::events::CommandTermination as PebbleTermination;
use pebble_coding_agent::tools::OutputCaptureStats as PebbleCaptureStats;
use sandbox_driver::FileKind;

use crate::driver_sandbox::RunSandbox;
use crate::sandbox::{self, CommandOutputCallback, ExecStreamingRequest};

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
        // in tree order: by name within each directory. Comparing the paths
        // segment by segment is that order.
        entries.sort_by(|left, right| left.name.split('/').cmp(right.name.split('/')));
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
        // Validated here rather than by the run sandbox's own glob so the
        // reason reaches the model in pebble's words, and so the patterns
        // pebble rejects (a trailing `/`, a `/` or wildcard inside `[...]`)
        // are rejected even though fabro's glob would accept them.
        if let Err(reason) = validate_pebble_glob(pattern) {
            return Err(EnvironmentError::new(
                EnvironmentErrorKind::InvalidInput,
                format!("Invalid glob pattern {pattern:?}: {reason}"),
            ));
        }
        if let Err(error) = WorkspaceGlob::try_new(pattern) {
            return Err(EnvironmentError::new(
                EnvironmentErrorKind::InvalidInput,
                format!("Invalid glob pattern {pattern:?}: {error}"),
            ));
        }
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
        let streaming = self
            .exec_command_streaming(ExecStreamingRequest {
                timeout_ms,
                working_dir,
                env_vars,
                cancel_token,
                stdin: None,
                output_callback: output_sink.map(adapt_output_sink),
                // `None` asks pebble for no cap at all. The driver always
                // retains under a buffer, so a command with no cap drains under
                // the driver's default rather than without bound; the capture
                // counts still say what was dropped.
                stream_output_bytes_cap: output_bytes_cap,
                ..ExecStreamingRequest::new(command)
            })
            .await
            .map_err(|error| {
                let kind = if error.is_transport() {
                    EnvironmentErrorKind::Io
                } else {
                    EnvironmentErrorKind::Spawn
                };
                EnvironmentError::with_source(kind, "Failed to run the command", error)
            })?;
        Ok(ExecOutcome {
            result:            ExecResult {
                stdout:      streaming.result.stdout,
                stderr:      streaming.result.stderr,
                exit_code:   streaming.result.exit_code,
                termination: pebble_termination(streaming.result.termination),
                duration_ms: streaming.result.duration_ms,
            },
            streams_separated: streaming.streams_separated,
            stdout_capture:    capture_stats(streaming.stdout_capture),
            stderr_capture:    capture_stats(streaming.stderr_capture),
        })
    }
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
/// A pattern names files, so one that ends with `/` is a mistake rather than a
/// directory; and `/`, `*`, or `?` inside a character class never mean what a
/// model meant by them. The messages are pebble's own, so a model corrects
/// itself the same way wherever pebble runs.
fn validate_pebble_glob(pattern: &str) -> Result<(), &'static str> {
    let trimmed = pattern.strip_prefix("./").unwrap_or(pattern);
    if trimmed.is_empty() {
        return Err("pattern cannot be empty");
    }
    if trimmed.ends_with('/') {
        return Err(
            "pattern ends with \"/\"; glob matches files, drop the trailing slash or add a \
             filename pattern",
        );
    }
    let mut in_class = false;
    for character in trimmed.chars() {
        match (in_class, character) {
            (false, '[') => in_class = true,
            (true, ']') => in_class = false,
            (true, '/') => return Err("a \"/\" cannot appear inside a character class"),
            (true, '*' | '?') => {
                return Err("wildcards are not valid inside a character class");
            }
            _ => {}
        }
    }
    if in_class {
        return Err("pattern has an unclosed character class");
    }
    Ok(())
}

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
fn adapt_output_sink(sink: ExecOutputSink) -> CommandOutputCallback {
    Arc::new(move |stream, chunk: Vec<u8>| {
        let stream = match stream {
            CommandOutputStream::Stdout => ExecOutputStream::Stdout,
            CommandOutputStream::Stderr => ExecOutputStream::Stderr,
        };
        sink(stream, &chunk);
        Box::pin(async { Ok(()) })
    })
}

fn pebble_termination(termination: CommandTermination) -> PebbleTermination {
    match termination {
        CommandTermination::Exited => PebbleTermination::Exited,
        CommandTermination::TimedOut => PebbleTermination::TimedOut,
        CommandTermination::Cancelled => PebbleTermination::Cancelled,
    }
}

fn capture_stats(stats: sandbox::OutputCaptureStats) -> PebbleCaptureStats {
    PebbleCaptureStats {
        observed_bytes: stats.observed_bytes,
        retained_bytes: stats.retained_bytes,
        omitted_bytes:  stats.omitted_bytes,
    }
}

/// A sandbox failure as pebble classifies it, keeping the driver cause.
fn environment_error(message: &str, error: crate::Error) -> EnvironmentError {
    let kind = if error.is_not_found() {
        EnvironmentErrorKind::NotFound
    } else if error.is_unsupported() {
        EnvironmentErrorKind::Unsupported
    } else {
        EnvironmentErrorKind::Io
    };
    EnvironmentError::with_source(kind, message, error)
}

#[cfg(test)]
mod tests {
    use pebble_coding_agent::test_support::EnvironmentContract;

    use super::*;
    use crate::local_sandbox;

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
    fn pebbles_glob_grammar_is_enforced_before_the_driver() {
        for pattern in ["nested/", "[a/]", "[a*]", "[ab"] {
            assert!(validate_pebble_glob(pattern).is_err(), "{pattern}");
        }
        for pattern in ["**/*.txt", "?.txt", "[ab].txt", "./src/**"] {
            assert!(validate_pebble_glob(pattern).is_ok(), "{pattern}");
        }
    }

    #[test]
    fn a_path_spelled_two_ways_is_one_path() {
        assert_eq!(normalize("/work//a/./b.txt"), "/work/a/b.txt");
        assert_eq!(parent_directory("/work/a/b.txt"), Some("/work/a"));
        assert_eq!(parent_directory("/b.txt"), Some("/"));
        assert_eq!(parent_directory("b.txt"), None);
    }
}
