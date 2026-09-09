//! Fabro's [`Sandbox`] over a sandbox-driver handle.
//!
//! Every operation goes to a public driver facet: files through
//! [`Filesystem`], content and tree search through [`Search`], commands
//! through fabro's [`SandboxExec`] policy over the [`Exec`] facet, lifecycle
//! through the handle itself. Nothing here knows which provider is behind
//! the handle or whether it runs in-process or over the plugin wire.
//!
//! What stays fabro's: the exec ladder, the credential filter on explicit
//! environment variables, the lifecycle events fabro records on a run, and
//! the run-facing conventions (`platform` names, grep line format, walk
//! results relative to a caller-declared base).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use fabro_types::SandboxProviderKind;
use sandbox_driver::{
    FileKind, LifecycleTimers, Sandbox as DriverHandle, SandboxState, Search as _, WaitOptions,
};
use tokio_util::sync::CancellationToken;

use crate::RetryPlan;
use crate::exec::{ExplicitEnvPolicy, SandboxExec};
use crate::sandbox::{
    self, DirEntry, ExecResult, ExecStreamingRequest, ExecStreamingResult, GrepOptions, PushError,
    PushReport, Sandbox, SandboxEvent, SandboxEventCallback, SandboxFile, StdioProcess,
    WalkOptions,
};

/// A fabro sandbox backed by a sandbox-driver handle.
pub struct DriverSandbox {
    kind:           SandboxProviderKind,
    handle:         Arc<dyn DriverHandle>,
    env_policy:     ExplicitEnvPolicy,
    event_callback: Option<SandboxEventCallback>,
    /// `(platform, os_version)` learned from the sandbox at initialize or
    /// start; unknown until then.
    platform:       OnceLock<(String, String)>,
}

impl DriverSandbox {
    /// Wraps a driver handle. `local` runs on the worker host, so explicit
    /// environment variables pass the credential filter; every other kind
    /// is isolated and takes the caller's environment as composed.
    #[must_use]
    pub fn new(kind: SandboxProviderKind, handle: Arc<dyn DriverHandle>) -> Self {
        let env_policy = if kind.is_local() {
            ExplicitEnvPolicy::FilterSensitive
        } else {
            ExplicitEnvPolicy::TrustCaller
        };
        Self {
            kind,
            handle,
            env_policy,
            event_callback: None,
            platform: OnceLock::new(),
        }
    }

    pub fn set_event_callback(&mut self, cb: SandboxEventCallback) {
        self.event_callback = Some(cb);
    }

    /// The provider kind fabro persists for this sandbox.
    #[must_use]
    pub fn kind(&self) -> &SandboxProviderKind {
        &self.kind
    }

    /// The driver handle, for callers that need a facet fabro's trait does
    /// not carry (git, services, access).
    #[must_use]
    pub fn handle(&self) -> &Arc<dyn DriverHandle> {
        &self.handle
    }

    fn exec(&self) -> SandboxExec<'_> {
        SandboxExec::new(self.handle.exec(), self.env_policy)
    }

    fn provider_name(&self) -> String {
        self.kind.to_string()
    }

    fn emit(&self, event: SandboxEvent) {
        event.trace();
        if let Some(cb) = &self.event_callback {
            cb(event);
        }
    }

    fn search(&self) -> crate::Result<sandbox_driver::SearchFacet<'_>> {
        self.handle.search().ok_or_else(|| {
            crate::Error::message(format!(
                "sandbox provider `{}` does not support search",
                self.kind
            ))
        })
    }

    /// Bring the sandbox to `Running` with a verified Bash, and learn its
    /// platform. Shared by initialize and start.
    async fn make_ready(&self) -> crate::Result<()> {
        sandbox_driver::activate(self.handle.as_ref(), &WaitOptions::default()).await?;
        if self.platform.get().is_none() {
            let info = self.handle.platform_info().await?;
            let platform = fabro_platform_name(&info.os).to_string();
            let os_version = if info.version.is_empty() {
                platform.clone()
            } else {
                format!("{platform} {}", info.version)
            };
            let _ = self.platform.set((platform, os_version));
        }
        Ok(())
    }

    /// The traversal base the driver walks. A base at the sandbox working
    /// directory walks relative to it so every path component of
    /// `relative_start` is checked against symlinks; any other base is
    /// walked as given.
    fn walk_base(&self, base: &str, relative_start: &str) -> String {
        if base == self.working_directory() || base.is_empty() || base == "." {
            if relative_start.is_empty() {
                ".".to_string()
            } else {
                relative_start.to_string()
            }
        } else {
            sandbox::join_sandbox_path(base, relative_start)
        }
    }
}

/// Fabro names the macOS platform `darwin`, as `uname -s` does.
fn fabro_platform_name(os: &str) -> &str {
    match os {
        "macos" => "darwin",
        other => other,
    }
}

fn file_context(action: &str, path: &str) -> String {
    format!("Failed to {action} {path}")
}

#[async_trait]
impl Sandbox for DriverSandbox {
    async fn read_file_bytes(&self, path: &str) -> crate::Result<Vec<u8>> {
        self.handle
            .fs()
            .read(path)
            .await
            .map_err(|error| crate::Error::context(file_context("read", path), error))
    }

    async fn write_file(&self, path: &str, content: &str) -> crate::Result<()> {
        self.handle
            .fs()
            .write(path, content.as_bytes())
            .await
            .map_err(|error| crate::Error::context(file_context("write", path), error))
    }

    async fn delete_file(&self, path: &str) -> crate::Result<()> {
        // Fabro's contract fails on a missing file; the driver's delete is
        // idempotent, so check first.
        if !self.file_exists(path).await? {
            return Err(crate::Error::message(format!(
                "{}: file does not exist",
                file_context("delete", path)
            )));
        }
        self.handle
            .fs()
            .delete(path, false)
            .await
            .map_err(|error| crate::Error::context(file_context("delete", path), error))
    }

    async fn file_exists(&self, path: &str) -> crate::Result<bool> {
        self.handle
            .fs()
            .exists(path)
            .await
            .map_err(|error| crate::Error::context(file_context("stat", path), error))
    }

    async fn list_directory(
        &self,
        path: &str,
        depth: Option<usize>,
    ) -> crate::Result<Vec<DirEntry>> {
        let entries = self
            .handle
            .fs()
            .list_dir(path, depth.unwrap_or(1))
            .await
            .map_err(|error| crate::Error::context(file_context("list", path), error))?;
        let mut entries: Vec<DirEntry> = entries
            .into_iter()
            .map(|entry| DirEntry {
                name:   entry.path,
                is_dir: entry.kind == FileKind::Directory,
                size:   (entry.kind == FileKind::File)
                    .then_some(entry.size)
                    .flatten(),
            })
            .collect();
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(entries)
    }

    async fn exec_command(
        &self,
        command: &str,
        timeout_ms: u64,
        working_dir: Option<&str>,
        env_vars: Option<&HashMap<String, String>>,
        cancel_token: Option<CancellationToken>,
    ) -> crate::Result<ExecResult> {
        self.exec()
            .run(
                command,
                Some(Duration::from_millis(timeout_ms)),
                working_dir,
                env_vars,
                cancel_token,
            )
            .await
    }

    async fn exec_command_streaming(
        &self,
        request: ExecStreamingRequest<'_>,
    ) -> crate::Result<ExecStreamingResult> {
        self.exec().run_streaming(request).await
    }

    async fn spawn_stdio_process(
        &self,
        command: &str,
        working_dir: Option<&str>,
        env_vars: Option<&HashMap<String, String>>,
        cancel_token: Option<CancellationToken>,
    ) -> crate::Result<StdioProcess> {
        self.exec()
            .spawn_stdio(command, working_dir, env_vars, cancel_token)
            .await
    }

    async fn grep(
        &self,
        pattern: &str,
        path: &str,
        options: &GrepOptions,
    ) -> crate::Result<Vec<String>> {
        let mut driver_options = sandbox_driver::GrepOptions::default();
        driver_options.case_insensitive = options.case_insensitive;
        driver_options.max_matches = options.max_results;
        driver_options.include.clone_from(&options.glob_filter);
        let matches = self
            .search()?
            .grep(pattern, path, &driver_options)
            .await
            .map_err(|error| crate::Error::context("Failed to search file contents", error))?;
        Ok(matches
            .into_iter()
            .map(|m| format!("{}:{}:{}", m.path, m.line_number, m.line))
            .collect())
    }

    async fn walk_files(
        &self,
        base: &str,
        relative_start: &str,
        options: &WalkOptions,
    ) -> crate::Result<Vec<SandboxFile>> {
        if options.excludes_relative_path(relative_start) {
            return Ok(Vec::new());
        }
        let mut driver_options = sandbox_driver::WalkOptions::default();
        driver_options
            .exclude_dirs
            .clone_from(&options.excluded_directory_names);
        let walk_base = self.walk_base(base, relative_start);
        let walked = self
            .search()?
            .walk(&walk_base, &driver_options)
            .await
            .map_err(|error| crate::Error::context("Failed to enumerate files", error))?;
        let mut files = Vec::with_capacity(walked.len());
        for file in walked {
            let relative_path = sandbox::join_sandbox_path(relative_start, &file.path);
            let path = sandbox::join_sandbox_path(base, &relative_path);
            // A transport without sizes (BSD `find`) reports `None`; fabro's
            // callers budget by size, so ask the filesystem rather than guess.
            let size = match file.size {
                Some(size) => size,
                None => {
                    self.handle
                        .fs()
                        .metadata(&path)
                        .await
                        .map_err(|error| crate::Error::context(file_context("stat", &path), error))?
                        .size
                }
            };
            files.push(SandboxFile {
                path,
                relative_path,
                size,
            });
        }
        Ok(files)
    }

    async fn download_file_to_local(
        &self,
        remote_path: &str,
        local_path: &Path,
    ) -> crate::Result<()> {
        self.handle
            .fs()
            .download(remote_path, local_path)
            .await
            .map_err(|error| crate::Error::context(file_context("download", remote_path), error))
    }

    async fn upload_file_from_local(
        &self,
        local_path: &Path,
        remote_path: &str,
    ) -> crate::Result<()> {
        self.handle
            .fs()
            .upload(local_path, remote_path)
            .await
            .map_err(|error| crate::Error::context(file_context("upload", remote_path), error))
    }

    async fn initialize(&self) -> crate::Result<()> {
        self.emit(SandboxEvent::Initializing {
            provider: self.provider_name(),
        });
        let started = Instant::now();
        let result = self.make_ready().await;
        let duration_ms = elapsed_ms(started);
        match &result {
            Ok(()) => self.emit(SandboxEvent::Ready {
                provider: self.provider_name(),
                duration_ms,
                name: Some(self.handle.id().to_string()),
                cpu: None,
                memory: None,
                url: None,
            }),
            Err(error) => self.emit(SandboxEvent::InitializeFailed {
                provider: self.provider_name(),
                error: error.to_string(),
                causes: error.causes(),
                duration_ms,
            }),
        }
        result
    }

    /// Idempotent access-time check: a running sandbox is left alone; a
    /// stopped or paused one is brought back and its Bash verified.
    async fn activate(&self) -> crate::Result<()> {
        let status = self.handle.describe().await?;
        if status.state == SandboxState::Running {
            return Ok(());
        }
        self.make_ready().await
    }

    async fn start(&self) -> crate::Result<()> {
        self.emit(SandboxEvent::StartStarted {
            provider: self.provider_name(),
        });
        let started = Instant::now();
        let result = self.make_ready().await;
        match &result {
            Ok(()) => self.emit(SandboxEvent::StartCompleted {
                provider:    self.provider_name(),
                duration_ms: elapsed_ms(started),
            }),
            Err(error) => self.emit(SandboxEvent::StartFailed {
                provider: self.provider_name(),
                error:    error.to_string(),
                causes:   error.causes(),
            }),
        }
        result
    }

    async fn stop(&self) -> crate::Result<()> {
        self.emit(SandboxEvent::StopStarted {
            provider: self.provider_name(),
        });
        let started = Instant::now();
        let result = self.handle.stop().await.map_err(crate::Error::from);
        match &result {
            Ok(()) => self.emit(SandboxEvent::StopCompleted {
                provider:    self.provider_name(),
                duration_ms: elapsed_ms(started),
            }),
            Err(error) => self.emit(SandboxEvent::StopFailed {
                provider: self.provider_name(),
                error:    error.to_string(),
                causes:   error.causes(),
            }),
        }
        result
    }

    async fn delete(&self) -> crate::Result<()> {
        self.emit(SandboxEvent::DeleteStarted {
            provider: self.provider_name(),
        });
        let started = Instant::now();
        let result = self.handle.delete().await.map_err(crate::Error::from);
        match &result {
            Ok(()) => self.emit(SandboxEvent::DeleteCompleted {
                provider:    self.provider_name(),
                duration_ms: elapsed_ms(started),
            }),
            Err(error) => self.emit(SandboxEvent::DeleteFailed {
                provider: self.provider_name(),
                error:    error.to_string(),
                causes:   error.causes(),
            }),
        }
        result
    }

    /// Releases the sandbox. For a designated host directory this frees the
    /// handle and leaves the directory in place; for an isolated provider it
    /// removes the sandbox.
    async fn cleanup(&self) -> crate::Result<()> {
        self.emit(SandboxEvent::CleanupStarted {
            provider: self.provider_name(),
        });
        let started = Instant::now();
        let result = self.handle.delete().await.map_err(crate::Error::from);
        match &result {
            Ok(()) => self.emit(SandboxEvent::CleanupCompleted {
                provider:    self.provider_name(),
                duration_ms: elapsed_ms(started),
            }),
            Err(error) => self.emit(SandboxEvent::CleanupFailed {
                provider: self.provider_name(),
                error:    error.to_string(),
                causes:   error.causes(),
            }),
        }
        result
    }

    fn working_directory(&self) -> &str {
        self.handle.working_directory()
    }

    fn runtime_directory(&self) -> Option<&str> {
        self.handle.runtime_directory()
    }

    fn platform(&self) -> &str {
        self.platform
            .get()
            .map_or("unknown", |(platform, _)| platform.as_str())
    }

    fn os_version(&self) -> String {
        self.platform.get().map_or_else(
            || self.platform().to_string(),
            |(_, version)| version.clone(),
        )
    }

    fn sandbox_info(&self) -> String {
        self.handle.id().to_string()
    }

    async fn set_autostop_interval(&self, minutes: i32) -> crate::Result<()> {
        let mut timers = LifecycleTimers::default();
        timers.auto_stop_after_idle = u64::try_from(minutes)
            .ok()
            .filter(|minutes| *minutes > 0)
            .map(Duration::from_mins);
        match self.handle.set_timers(&timers).await {
            // A provider without timers has nothing to stop automatically.
            Ok(()) | Err(sandbox_driver::Error::Unsupported { .. }) => Ok(()),
            Err(error) => Err(crate::Error::context(
                "Failed to set sandbox auto-stop",
                error,
            )),
        }
    }

    async fn git_push_ref(&self, refspec: &str, plan: &RetryPlan) -> Result<PushReport, PushError> {
        let has_origin = match self
            .exec_command("git remote get-url origin", 10_000, None, None, None)
            .await
        {
            Ok(result) if result.is_success() => true,
            Ok(_) => false,
            Err(err) => {
                return Err(PushError {
                    report: PushReport::default(),
                    error:  crate::Error::context("git remote get-url origin", err),
                });
            }
        };
        if !has_origin {
            return Ok(PushReport::default());
        }
        // No managed credentials yet: the checkout pushes with whatever the
        // remote URL already carries.
        sandbox::git_push_via_exec(self, None, refspec, plan).await
    }

    async fn get_preview_url(
        &self,
        port: u16,
    ) -> crate::Result<Option<(String, HashMap<String, String>)>> {
        let Some(previews) = self.handle.preview_urls() else {
            return Ok(None);
        };
        let preview = previews
            .preview_url(port)
            .await
            .map_err(|error| crate::Error::context("Failed to obtain a preview URL", error))?;
        Ok(Some((
            preview.url,
            preview.headers.into_iter().collect::<HashMap<_, _>>(),
        )))
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use fabro_types::CommandTermination;
    use sandbox_driver::{SandboxProvider as _, SandboxSource, SandboxSpec};
    use sandbox_driver_host::HostProvider;
    use tokio::fs;

    use super::*;

    struct Fixture {
        dir:       tempfile::TempDir,
        _provider: HostProvider,
        sandbox:   DriverSandbox,
    }

    async fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let provider = HostProvider::new();
        let handle = provider
            .create(
                &SandboxSpec::new(SandboxSource::HostDirectory)
                    .working_directory(dir.path().display().to_string()),
                None,
            )
            .await
            .unwrap();
        Fixture {
            dir,
            _provider: provider,
            sandbox: DriverSandbox::new(SandboxProviderKind::LOCAL, handle),
        }
    }

    #[tokio::test]
    async fn files_round_trip_through_the_filesystem_facet() {
        let f = fixture().await;
        f.sandbox
            .write_file("sub/dir/test.txt", "content")
            .await
            .unwrap();
        assert!(f.dir.path().join("sub/dir/test.txt").is_file());
        assert_eq!(
            f.sandbox.read_file_text("sub/dir/test.txt").await.unwrap(),
            "content"
        );
        assert!(f.sandbox.file_exists("sub/dir/test.txt").await.unwrap());
        f.sandbox.delete_file("sub/dir/test.txt").await.unwrap();
        assert!(!f.sandbox.file_exists("sub/dir/test.txt").await.unwrap());
        let missing = f.sandbox.delete_file("sub/dir/test.txt").await.unwrap_err();
        assert!(missing.to_string().contains("does not exist"), "{missing}");
        let read = f
            .sandbox
            .read_file("nonexistent.txt", None, None)
            .await
            .unwrap_err();
        assert!(read.is_not_found(), "{read}");
    }

    #[tokio::test]
    async fn list_directory_is_sorted_with_sizes_for_files_only() {
        let f = fixture().await;
        fs::write(f.dir.path().join("b.txt"), "b").await.unwrap();
        fs::write(f.dir.path().join("a.txt"), "aa").await.unwrap();
        fs::create_dir(f.dir.path().join("c_dir")).await.unwrap();
        fs::write(f.dir.path().join("c_dir/inner.txt"), "x")
            .await
            .unwrap();

        let entries = f.sandbox.list_directory(".", None).await.unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["a.txt", "b.txt", "c_dir"]);
        assert_eq!(entries[0].size, Some(2));
        assert!(!entries[0].is_dir);
        assert!(entries[2].is_dir);
        assert_eq!(entries[2].size, None);

        let deep = f.sandbox.list_directory(".", Some(2)).await.unwrap();
        assert!(deep.iter().any(|e| e.name == "c_dir/inner.txt"));
    }

    #[tokio::test]
    async fn exec_runs_bash_with_fabro_termination_semantics() {
        let f = fixture().await;
        let ok = f
            .sandbox
            .exec_command(
                "echo hello; [[ 1 == 1 ]] && echo bash",
                5000,
                None,
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(ok.stdout, "hello\nbash\n");
        assert!(ok.is_success());
        let timed_out = f
            .sandbox
            .exec_command("sleep 10", 200, None, None, None)
            .await
            .unwrap();
        assert_eq!(timed_out.termination, CommandTermination::TimedOut);
        assert_eq!(timed_out.exit_code, None);
    }

    #[tokio::test]
    async fn grep_returns_path_line_content_triples() {
        let f = fixture().await;
        fs::write(
            f.dir.path().join("test.rs"),
            "fn main() {\n    println!(\"hello\");\n}\n",
        )
        .await
        .unwrap();
        let results = f
            .sandbox
            .grep("println", "test.rs", &GrepOptions::default())
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].starts_with("test.rs:2:"), "{results:?}");
        assert!(results[0].contains("println"));

        let insensitive = f
            .sandbox
            .grep("PRINTLN", ".", &GrepOptions {
                case_insensitive: true,
                ..GrepOptions::default()
            })
            .await
            .unwrap();
        assert_eq!(insensitive.len(), 1);
    }

    #[tokio::test]
    async fn walk_and_glob_report_paths_relative_to_the_declared_base() {
        let f = fixture().await;
        fs::create_dir_all(f.dir.path().join(".ai/reports"))
            .await
            .unwrap();
        fs::create_dir_all(f.dir.path().join(".ai/target"))
            .await
            .unwrap();
        fs::write(f.dir.path().join(".ai/reports/result.md"), "report")
            .await
            .unwrap();
        fs::write(f.dir.path().join(".ai/reports/empty.md"), "")
            .await
            .unwrap();
        fs::write(f.dir.path().join(".ai/target/ignored.md"), "ignored")
            .await
            .unwrap();

        let files = f
            .sandbox
            .walk_files(f.sandbox.working_directory(), ".ai", &WalkOptions {
                excluded_directory_names: vec!["target".to_string()],
            })
            .await
            .unwrap();
        let mut metadata: Vec<_> = files
            .iter()
            .map(|file| (file.relative_path.as_str(), file.size))
            .collect();
        metadata.sort_unstable();
        assert_eq!(metadata, vec![
            (".ai/reports/empty.md", 0),
            (".ai/reports/result.md", 6),
        ]);
        let root = f.sandbox.working_directory().to_string();
        assert!(files.iter().all(|file| file.path.starts_with(&root)));

        let globbed = f.sandbox.glob("**/*.md", None).await.unwrap();
        assert_eq!(globbed, vec![
            format!("{root}/.ai/reports/empty.md"),
            format!("{root}/.ai/reports/result.md"),
            format!("{root}/.ai/target/ignored.md"),
        ]);
        let scoped = f.sandbox.glob("*.md", Some(".ai/reports")).await.unwrap();
        assert_eq!(scoped.len(), 2);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn glob_does_not_follow_symlinked_directories_below_the_root() {
        let f = fixture().await;
        let target = f.dir.path().join("elsewhere");
        fs::create_dir_all(&target).await.unwrap();
        fs::write(target.join("lib.rs"), "").await.unwrap();
        std::os::unix::fs::symlink(&target, f.dir.path().join("linked")).unwrap();

        let results = f.sandbox.glob("linked/**/*.rs", None).await.unwrap();
        assert!(results.is_empty(), "{results:?}");
    }

    #[tokio::test]
    async fn download_and_upload_copy_binary_files() {
        let f = fixture().await;
        let bytes = vec![0u8, 159, 146, 150, 255];
        fs::write(f.dir.path().join("source.bin"), &bytes)
            .await
            .unwrap();
        let dest = f.dir.path().join("out/nested/copy.bin");
        f.sandbox
            .download_file_to_local("source.bin", &dest)
            .await
            .unwrap();
        assert_eq!(fs::read(&dest).await.unwrap(), bytes);
        f.sandbox
            .upload_file_from_local(&dest, "in/again.bin")
            .await
            .unwrap();
        assert_eq!(
            f.sandbox.read_file_bytes("in/again.bin").await.unwrap(),
            bytes
        );
    }

    #[tokio::test]
    async fn initialize_emits_lifecycle_events_and_learns_the_platform() {
        let mut f = fixture().await;
        let events: Arc<Mutex<Vec<SandboxEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        f.sandbox.set_event_callback(Arc::new(move |event| {
            captured.lock().unwrap().push(event);
        }));
        assert_eq!(f.sandbox.platform(), "unknown");

        f.sandbox.initialize().await.unwrap();
        let expected = if cfg!(target_os = "macos") {
            "darwin"
        } else {
            std::env::consts::OS
        };
        assert_eq!(f.sandbox.platform(), expected);
        assert!(f.sandbox.os_version().starts_with(expected));
        assert_eq!(
            f.sandbox.sandbox_info(),
            f.sandbox.handle().id().to_string()
        );

        f.sandbox.stop().await.unwrap();
        f.sandbox.activate().await.unwrap();
        f.sandbox.cleanup().await.unwrap();
        assert!(
            f.dir.path().is_dir(),
            "designated directories survive cleanup"
        );

        let captured = events.lock().unwrap();
        let names: Vec<&str> = captured
            .iter()
            .map(|event| match event {
                SandboxEvent::Initializing { .. } => "initializing",
                SandboxEvent::Ready { .. } => "ready",
                SandboxEvent::StopStarted { .. } => "stop_started",
                SandboxEvent::StopCompleted { .. } => "stop_completed",
                SandboxEvent::CleanupStarted { .. } => "cleanup_started",
                SandboxEvent::CleanupCompleted { .. } => "cleanup_completed",
                _ => "other",
            })
            .collect();
        assert_eq!(names, vec![
            "initializing",
            "ready",
            "stop_started",
            "stop_completed",
            "cleanup_started",
            "cleanup_completed",
        ]);
        assert!(captured.iter().all(|event| match event {
            SandboxEvent::Initializing { provider }
            | SandboxEvent::Ready { provider, .. }
            | SandboxEvent::StopStarted { provider }
            | SandboxEvent::StopCompleted { provider, .. }
            | SandboxEvent::CleanupStarted { provider }
            | SandboxEvent::CleanupCompleted { provider, .. } => provider == "local",
            _ => true,
        }));
    }

    #[tokio::test]
    async fn preview_urls_come_from_the_access_facet() {
        let f = fixture().await;
        let (url, headers) = f.sandbox.get_preview_url(8080).await.unwrap().unwrap();
        assert_eq!(url, "http://127.0.0.1:8080");
        assert!(headers.is_empty());
    }
}
