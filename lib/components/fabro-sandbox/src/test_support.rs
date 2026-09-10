//! Test doubles for fabro's sandbox layer.
//!
//! [`MockSandbox`] is a configuration and a recorder over the sandbox
//! driver's scripted double: a test writes down the files, the command
//! answer, and the failures it wants, takes a [`RunSandbox`] from it, and
//! reads back what the code under test ran or wrote. Nothing here fakes
//! fabro's own logic; every call goes through the real `RunSandbox` and
//! fabro's exec policy, down to the scripted driver.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use fabro_types::{CommandTermination, SandboxProviderKind};
use sandbox_driver::{GrepMatch, PlatformInfo, SandboxState, Termination, WalkedFile};
pub use sandbox_driver_testing::{ScriptedExec, ScriptedSandbox, ScriptedStdioProcess};
use tokio::io::DuplexStream;

use crate::driver_sandbox::RunSandbox;
use crate::sandbox::{ExecResult, SandboxEventCallback, SandboxFile, StderrCollector};

// --- MockSandbox ---

/// What a test wants its sandbox to be, and what the code under test did
/// with it.
///
/// Build it with a struct literal over [`MockSandbox::default`] (or
/// [`MockSandbox::linux`]), then take the run sandbox with
/// [`MockSandbox::sandbox`]. Every command answers with `exec_result`
/// unless `exec_error` is set, in which case every command fails as a
/// transport error. Files seed an in-memory filesystem under
/// `working_dir`; absolute paths are kept as given.
pub struct MockSandbox {
    pub files:               HashMap<String, String>,
    pub exec_result:         ExecResult,
    /// Fails every command before any process runs, so callers see a
    /// transport error rather than an `ExecResult`.
    pub exec_error:          Option<String>,
    pub working_dir:         &'static str,
    /// The run-scoped scratch directory the sandbox reports, outside any
    /// checkout; `None` models a provider without one.
    pub runtime_dir:         Option<&'static str>,
    pub platform_str:        &'static str,
    pub os_version_str:      String,
    /// Fails `activate` after the sandbox is built, as a sandbox whose
    /// Bash contract broke would.
    pub activate_error:      Option<String>,
    pub event_callback:      Option<SandboxEventCallback>,
    pub stdio_process:       Option<MockStdioProcess>,
    pub stdio_process_error: Option<String>,
    /// Lines every grep returns, as `path:line:content`.
    pub grep_results:        Vec<String>,
    /// Files returned by `walk_files` instead of the seeded files, before
    /// traversal-root and exclusion filtering.
    pub walk_files:          Vec<SandboxFile>,
    pub walk_files_error:    Option<String>,
    /// Reported by streaming execution. Set to `false` to model a provider
    /// that cannot separate stdout from stderr.
    pub streams_separated:   bool,
    /// The sandbox once built. Public only so `..Default::default()` works
    /// from other crates; leave it at its default.
    pub built:               OnceLock<Built>,
}

/// The lazily built sandbox and its scripted driver.
pub struct Built {
    run:    Arc<RunSandbox>,
    driver: Arc<ScriptedSandbox>,
}

impl Default for MockSandbox {
    fn default() -> Self {
        Self {
            files:               HashMap::new(),
            exec_result:         ExecResult {
                stdout:      "mock output".into(),
                stderr:      String::new(),
                exit_code:   Some(0),
                termination: CommandTermination::Exited,
                duration_ms: 10,
            },
            exec_error:          None,
            working_dir:         "/work",
            runtime_dir:         None,
            platform_str:        "darwin",
            os_version_str:      "Darwin 24.0.0".into(),
            activate_error:      None,
            event_callback:      None,
            stdio_process:       None,
            stdio_process_error: None,
            grep_results:        Vec::new(),
            walk_files:          Vec::new(),
            walk_files_error:    None,
            streams_separated:   true,
            built:               OnceLock::new(),
        }
    }
}

impl MockSandbox {
    pub fn linux() -> Self {
        Self {
            working_dir: "/home/test",
            platform_str: "linux",
            os_version_str: "Linux 6.1.0".into(),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn with_walk_files(mut self, files: Vec<SandboxFile>) -> Self {
        self.walk_files = files;
        self
    }

    #[must_use]
    pub fn with_walk_files_error(mut self, error: impl Into<String>) -> Self {
        self.walk_files_error = Some(error.into());
        self
    }

    #[must_use]
    pub fn with_activate_error(mut self, error: impl Into<String>) -> Self {
        self.activate_error = Some(error.into());
        self
    }

    /// The run sandbox this configuration describes, built once: repeated
    /// calls return the same sandbox over the same recorder.
    pub fn sandbox(&self) -> Arc<RunSandbox> {
        Arc::clone(&self.built().run)
    }

    /// The scripted driver double behind [`MockSandbox::sandbox`], for
    /// scripting beyond what the fields express.
    pub fn driver(&self) -> Arc<ScriptedSandbox> {
        Arc::clone(&self.built().driver)
    }

    /// Answers commands by their Bash source, ahead of the queue and
    /// `exec_result`: a responder that returns `Some` decides the result,
    /// `None` falls through. For tests that interleave different commands
    /// and want each answered by what it is rather than by its position.
    pub fn respond_with(
        &self,
        responder: impl Fn(&str) -> Option<ExecResult> + Send + Sync + 'static,
    ) -> &Self {
        self.driver().scripted_exec().respond_with(move |spec| {
            let command = spec.args.last().map(String::as_str).unwrap_or_default();
            responder(command).map(|result| driver_result(&result))
        });
        self
    }

    /// Queues the result for the next command, ahead of `exec_result`.
    /// Results answer in the order they were pushed.
    pub fn push_exec_result(&self, result: &ExecResult) -> &Self {
        self.driver()
            .scripted_exec()
            .push_result(driver_result(result));
        self
    }

    fn built(&self) -> &Built {
        self.built.get_or_init(|| {
            let driver = Arc::new(self.build_driver());
            // An isolated provider: explicit environment passes as the
            // caller composed it, as it does for Docker and Daytona runs.
            let mut run = RunSandbox::new_with_platform(
                SandboxProviderKind::DOCKER,
                Arc::clone(&driver) as Arc<dyn sandbox_driver::Sandbox>,
                self.platform_str,
                self.os_version_str.clone(),
            );
            if let Some(callback) = &self.event_callback {
                run.set_event_callback(Arc::clone(callback));
            }
            Built {
                run: Arc::new(run),
                driver,
            }
        })
    }

    fn build_driver(&self) -> ScriptedSandbox {
        let mut driver =
            ScriptedSandbox::with_id_and_working_dir("mock-sandbox", self.working_dir).platform(
                PlatformInfo::new(self.platform_str, "x86_64", self.os_version_str.clone()),
            );
        if let Some(directory) = self.runtime_dir {
            driver = driver.runtime_directory(directory);
        }
        if let Some(message) = &self.activate_error {
            // A stopped sandbox whose provider cannot start it.
            driver = driver
                .state(SandboxState::Stopped)
                .start_error(message.clone());
        }
        for (path, content) in &self.files {
            driver = driver.file(path, content);
        }
        let exec = driver.scripted_exec();
        match &self.exec_error {
            Some(message) => exec.fail_by_default(message.clone()),
            None => exec.set_default(driver_result(&self.exec_result)),
        };
        exec.set_streams_separated(self.streams_separated);
        if let Some(message) = &self.stdio_process_error {
            exec.set_stdio_error(message.clone());
        }
        if let Some(process) = self.stdio_process.as_ref() {
            if let Some(scripted) = process.take() {
                exec.set_stdio_process(scripted);
            }
        }
        let search = driver.scripted_search();
        search.set_grep(
            self.grep_results
                .iter()
                .map(|line| {
                    let mut parts = line.splitn(3, ':');
                    let path = parts.next().unwrap_or_default();
                    let line_number = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
                    GrepMatch::new(path, line_number, parts.next().unwrap_or_default())
                })
                .collect(),
        );
        if let Some(message) = &self.walk_files_error {
            search.set_walk_error(message.clone());
        } else if !self.walk_files.is_empty() {
            search.set_walk(
                self.walk_files
                    .iter()
                    .map(|file| WalkedFile::new(file.relative_path.clone(), Some(file.size)))
                    .collect(),
            );
        }
        driver
    }

    fn recorded(&self) -> Vec<sandbox_driver::ExecSpec> {
        self.built
            .get()
            .map(|built| built.driver.scripted_exec().recorded())
            .unwrap_or_default()
    }

    /// The Bash source of every command run so far, in order.
    pub fn captured_commands(&self) -> Vec<String> {
        self.recorded()
            .iter()
            .map(|spec| spec.args.last().cloned().unwrap_or_default())
            .collect()
    }

    /// The last command's Bash source.
    pub fn captured_command(&self) -> Option<String> {
        self.captured_commands().pop()
    }

    /// The last command's timeout in milliseconds.
    pub fn captured_timeout(&self) -> Option<u64> {
        self.recorded()
            .last()
            .and_then(|spec| spec.timeout)
            .map(|timeout| u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX))
    }

    /// The timeout of every command in milliseconds, in order.
    pub fn captured_timeouts(&self) -> Vec<u64> {
        self.recorded()
            .iter()
            .filter_map(|spec| spec.timeout)
            .map(|timeout| u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX))
            .collect()
    }

    /// Whether each command was given the run's cancellation to stop on,
    /// in order.
    pub fn captured_term_stops(&self) -> Vec<bool> {
        self.built
            .get()
            .map(|built| built.driver.scripted_exec().term_stops())
            .unwrap_or_default()
    }

    /// The working directory of every command, in order.
    pub fn captured_working_dirs(&self) -> Vec<Option<String>> {
        self.recorded()
            .iter()
            .map(|spec| spec.working_dir.clone())
            .collect()
    }

    /// The explicit variables of the last command as the caller passed them.
    /// The exec policy's own `BASH_ENV` blank is not the caller's.
    pub fn captured_env_vars(&self) -> Option<HashMap<String, String>> {
        self.recorded().last().map(|spec| {
            spec.env
                .iter()
                .filter(|(key, _)| key.as_str() != sandbox_driver::BASH_ENV_VAR)
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
    }

    /// The bytes the last streaming command was fed on standard input.
    pub fn captured_stdin(&self) -> Option<Vec<u8>> {
        self.built
            .get()
            .and_then(|built| built.driver.scripted_exec().captured_stdin().pop())
    }

    /// Every file written so far as `(path, content)`, in order.
    pub fn written_files(&self) -> Vec<(String, String)> {
        self.built
            .get()
            .map(|built| {
                built
                    .driver
                    .memory_fs()
                    .writes()
                    .into_iter()
                    .map(|(path, bytes)| (path, String::from_utf8_lossy(&bytes).into_owned()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Every file deleted so far by absolute path, in order.
    pub fn deleted_files(&self) -> Vec<String> {
        self.built
            .get()
            .map(|built| built.driver.memory_fs().deletes())
            .unwrap_or_default()
    }

    /// How many times the code under test asked whether a path exists.
    pub fn exists_calls(&self) -> usize {
        self.built
            .get()
            .map_or(0, |built| built.driver.memory_fs().exists_calls())
    }

    pub fn start_count(&self) -> u32 {
        self.built
            .get()
            .map_or(0, |built| built.driver.start_count())
    }

    pub fn stop_count(&self) -> u32 {
        self.built
            .get()
            .map_or(0, |built| built.driver.stop_count())
    }

    pub fn delete_count(&self) -> u32 {
        self.built
            .get()
            .map_or(0, |built| built.driver.delete_count())
    }

    /// How many walks the code under test ran.
    pub fn walk_files_was_called(&self) -> bool {
        self.built
            .get()
            .is_some_and(|built| built.driver.scripted_search().walk_calls() > 0)
    }
}

/// The driver result fabro's exec policy reads back as `result`.
fn driver_result(result: &ExecResult) -> sandbox_driver::ExecResult {
    let termination = match result.termination {
        CommandTermination::Exited => Termination::Exited,
        CommandTermination::TimedOut => Termination::TimedOut,
        CommandTermination::Cancelled => Termination::Cancelled,
    };
    let mut driver = sandbox_driver::ExecResult::new(
        termination,
        result.exit_code,
        Duration::from_millis(result.duration_ms),
    );
    driver.stdout = result.stdout.clone().into_bytes();
    driver.stderr = result.stderr.clone().into_bytes();
    driver
}

// --- MockStdioProcess ---

/// A stdio process a test drives, over the driver's scripted process.
///
/// The driver closure receives the process's end of standard input, its
/// end of standard output, and fabro's stderr collector for the process.
pub struct MockStdioProcess {
    inner: std::sync::Mutex<Option<ScriptedStdioProcess>>,
}

impl MockStdioProcess {
    pub fn new(
        driver: impl FnOnce(DuplexStream, DuplexStream, StderrCollector) + Send + 'static,
    ) -> Self {
        Self {
            inner: std::sync::Mutex::new(Some(ScriptedStdioProcess::new(
                move |stdin, stdout, tail| {
                    driver(stdin, stdout, StderrCollector::from_driver_tail(tail));
                },
            ))),
        }
    }

    #[must_use]
    pub fn with_exit_code(self, exit_code: Option<i32>) -> Self {
        let inner = self.inner.lock().expect("stdio process").take();
        Self {
            inner: std::sync::Mutex::new(inner.map(|process| process.exit_code(exit_code))),
        }
    }

    #[must_use]
    pub fn with_wait_delay(self, wait_delay: Duration) -> Self {
        let inner = self.inner.lock().expect("stdio process").take();
        Self {
            inner: std::sync::Mutex::new(inner.map(|process| process.wait_delay(wait_delay))),
        }
    }

    fn take(&self) -> Option<ScriptedStdioProcess> {
        self.inner.lock().expect("stdio process").take()
    }
}

// --- FakeSandboxProvider ---

pub use fake_provider::{FakeGet, FakeList, FakeSandboxProvider, fake_registry, fake_sandbox_info};

mod fake_provider {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use async_trait::async_trait;
    use fabro_types::{
        SandboxInfo, SandboxNetwork, SandboxProviderKind, SandboxResources, SandboxState,
        SandboxTimestamps,
    };

    use crate::provider::{SandboxProvider, SandboxProviderRegistry};

    #[derive(Clone)]
    pub enum FakeList {
        Ok(Vec<SandboxInfo>),
        Err(&'static str),
    }

    #[derive(Clone)]
    pub enum FakeGet {
        Found(Box<SandboxInfo>),
        Missing,
        Err(&'static str),
    }

    pub struct FakeSandboxProvider {
        kind: SandboxProviderKind,
        list: FakeList,
        get:  FakeGet,
    }

    impl FakeSandboxProvider {
        pub fn new(kind: SandboxProviderKind, list: FakeList, get: FakeGet) -> Self {
            Self { kind, list, get }
        }
    }

    #[async_trait]
    impl SandboxProvider for FakeSandboxProvider {
        fn kind(&self) -> SandboxProviderKind {
            self.kind.clone()
        }

        async fn list(&self) -> crate::Result<Vec<SandboxInfo>> {
            match &self.list {
                FakeList::Ok(sandboxes) => Ok(sandboxes.clone()),
                FakeList::Err(message) => Err(crate::Error::message(*message)),
            }
        }

        async fn get(&self, _id: &str) -> crate::Result<Option<SandboxInfo>> {
            match &self.get {
                FakeGet::Found(sandbox) => Ok(Some((**sandbox).clone())),
                FakeGet::Missing => Ok(None),
                FakeGet::Err(message) => Err(crate::Error::message(*message)),
            }
        }

        async fn delete(&self, _id: &str) -> crate::Result<()> {
            Ok(())
        }
    }

    pub fn fake_registry(providers: Vec<FakeSandboxProvider>) -> SandboxProviderRegistry {
        SandboxProviderRegistry::new(
            providers
                .into_iter()
                .map(|provider| Arc::new(provider) as Arc<dyn SandboxProvider>)
                .collect(),
        )
    }

    pub fn fake_sandbox_info(provider: SandboxProviderKind, id: &str) -> SandboxInfo {
        SandboxInfo {
            provider,
            id: id.to_string(),
            display_name: None,
            state: SandboxState::Running,
            native_state: None,
            image: None,
            snapshot: None,
            region: None,
            web_url: None,
            working_directory: None,
            resources: SandboxResources::default(),
            network: SandboxNetwork::unknown(),
            labels: BTreeMap::new(),
            timestamps: SandboxTimestamps::default(),
        }
    }
}
