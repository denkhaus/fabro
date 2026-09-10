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
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use fabro_github::GitHubCredentials;
use fabro_github::token_source::InstallationTokenSource;
use fabro_types::SandboxProviderKind;
use sandbox_driver::{
    Action, Event, EventBody, EventContext, EventObserver, FileKind, LifecycleTimers, ProgressCode,
    PtyOptions, PtySize, Sandbox as DriverHandle, SandboxProvider as DriverProvider, SandboxSource,
    SandboxSpec as DriverSpec, SandboxState, Search as _, WaitOptions,
};
use sandbox_driver_host::HostProvider;
use tokio::fs;
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;

use crate::clone::{self, GitHubClone};
use crate::clone_source::{self, CloneDecision, EmptyWorkspaceReason};
use crate::push_credentials::{self, PushCredentialState};
use crate::terminal::{DriverTerminalSession, TerminalSize};
use crate::{GitRunInfo, GitSetupIntent, RefreshOutcome, RetryPlan};

/// A sandbox on the worker host at `working_directory`, the fabro `local`
/// kind, served by the driver's in-process Host provider.
///
/// The directory is designated: the sandbox uses it in place and never
/// removes it. It is created when missing so a run can point at a fresh
/// scratch path. The registry lives in a per-process temporary root, so a
/// later process rebuilds the handle by calling this again with the
/// persisted working directory rather than by id.
pub async fn local_sandbox(working_directory: impl Into<PathBuf>) -> crate::Result<DriverSandbox> {
    let working_directory: PathBuf = working_directory.into();
    fs::create_dir_all(&working_directory)
        .await
        .map_err(|error| crate::Error::context("Failed to create working directory", error))?;
    let provider = HostProvider::new();
    let spec = DriverSpec::new(SandboxSource::HostDirectory)
        .working_directory(working_directory.display().to_string());
    let handle = provider
        .create(&spec, None)
        .await
        .map_err(|error| crate::Error::context("Failed to create local sandbox", error))?;
    let sandbox = DriverSandbox::new(SandboxProviderKind::LOCAL, handle);
    sandbox.learn_platform().await?;
    Ok(sandbox)
}
use crate::exec::{ExplicitEnvPolicy, SandboxExec};
use crate::sandbox::{
    self, DirEntry, ExecResult, ExecStreamingRequest, ExecStreamingResult, GrepOptions, PushError,
    PushReport, Sandbox, SandboxEvent, SandboxEventCallback, SandboxFile, SandboxWorkspaceLayout,
    StdioProcess, WalkOptions,
};

/// Where a clone-based provider puts its files: the run works under
/// `workspace_root`, and repositories check out under `repos_root`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkspaceLayout {
    pub(crate) workspace_root: String,
    pub(crate) repos_root:     String,
}

impl WorkspaceLayout {
    /// The layout for a provider whose working directory fabro does not
    /// choose: repositories check out beside the workspace contents under
    /// `.repos`, and the run works in the link the workspace root carries.
    pub(crate) fn within(working_directory: &str) -> Self {
        Self {
            workspace_root: working_directory.to_string(),
            repos_root:     sandbox::join_sandbox_path(working_directory, ".repos"),
        }
    }
}

/// How a workspace learns its layout.
pub(crate) enum LayoutSource {
    /// Fabro fixes the roots before the sandbox exists.
    Fixed(WorkspaceLayout),
    /// The roots follow the provider's working directory, known once the
    /// sandbox exists.
    ProviderWorkingDirectory,
}

/// What `initialize` does to the workspace once the sandbox runs.
enum WorkspacePlan {
    /// Clone this GitHub repository into the layout.
    Clone(GitHubClone),
    /// Create the empty workspace root and nothing else.
    Empty(EmptyWorkspaceReason),
    /// The workspace was prepared by an earlier process; leave it alone.
    Attached,
}

/// Fabro's clone-based workspace on an isolated sandbox: the layout, the
/// clone it performs, and the GitHub credentials its checkout carries.
pub(crate) struct RepoWorkspace {
    layout:              OnceLock<WorkspaceLayout>,
    plan:                WorkspacePlan,
    credentials:         PushCredentialState,
    repo_cloned:         OnceLock<bool>,
    origin_url:          OnceLock<String>,
    /// The directory the run works in once known: the repository link for a
    /// clone, the workspace root otherwise.
    execution_directory: OnceLock<String>,
    /// The real checkout behind the workspace link, for traversals that
    /// must not start at a symlink.
    checkout_path:       OnceLock<String>,
}

impl RepoWorkspace {
    /// Decide the clone for a new sandbox. Fails before any provider call
    /// when the selectors are inconsistent (a pin without a branch, a
    /// non-GitHub origin without `skip_clone`).
    #[expect(
        clippy::too_many_arguments,
        reason = "the clone selectors are validated together by decide_clone"
    )]
    pub(crate) fn plan(
        layout: LayoutSource,
        skip_clone: bool,
        clone_origin_url: Option<&str>,
        clone_branch: Option<&str>,
        clone_tag: Option<&str>,
        clone_commit_sha: Option<&str>,
        clone_depth: Option<u32>,
        github_app: Option<&GitHubCredentials>,
    ) -> crate::Result<Self> {
        let decision = clone_source::decide_clone(
            skip_clone,
            clone_origin_url,
            clone_branch,
            clone_tag,
            clone_commit_sha,
        )?;
        let credentials = PushCredentialState::new(push_credentials::build_token_source(
            github_app,
            clone_origin_url,
        )?);
        let plan = match decision {
            CloneDecision::EmptyWorkspace { reason } => WorkspacePlan::Empty(reason),
            CloneDecision::GitHub {
                origin_url,
                branch,
                tag,
                commit_sha,
            } => WorkspacePlan::Clone(GitHubClone {
                origin_url,
                branch,
                tag,
                commit_sha,
                depth: clone_depth,
            }),
        };
        Ok(Self {
            layout: layout.into_cell(),
            plan,
            credentials,
            repo_cloned: OnceLock::new(),
            origin_url: OnceLock::new(),
            execution_directory: OnceLock::new(),
            checkout_path: OnceLock::new(),
        })
    }

    /// A workspace prepared by an earlier process, described by the run
    /// record. Pushes from a reattached sandbox use whatever credentials the
    /// checkout's `origin` already carries.
    pub(crate) fn attached(
        layout: LayoutSource,
        repo_cloned: bool,
        working_directory: String,
        clone_origin_url: Option<String>,
    ) -> Self {
        let workspace = Self {
            layout:              layout.into_cell(),
            plan:                WorkspacePlan::Attached,
            credentials:         PushCredentialState::new(None),
            repo_cloned:         OnceLock::new(),
            origin_url:          OnceLock::new(),
            execution_directory: OnceLock::new(),
            checkout_path:       OnceLock::new(),
        };
        let _ = workspace.repo_cloned.set(repo_cloned);
        let _ = workspace.execution_directory.set(working_directory);
        if repo_cloned {
            if let Some(origin) = clone_origin_url {
                let _ = workspace.origin_url.set(origin);
            }
        }
        workspace.derive_checkout_path();
        workspace
    }

    /// Settle a provider-dependent layout from the sandbox's working
    /// directory. A fixed layout is left alone.
    fn resolve_layout(&self, provider_working_directory: &str) -> &WorkspaceLayout {
        let layout = self
            .layout
            .get_or_init(|| WorkspaceLayout::within(provider_working_directory));
        self.derive_checkout_path();
        layout
    }

    /// The checkout behind an attached clone, once the layout is known.
    fn derive_checkout_path(&self) {
        if self.checkout_path.get().is_some() || !self.repo_cloned() {
            return;
        }
        let (Some(layout), Some(origin)) = (self.layout.get(), self.origin_url.get()) else {
            return;
        };
        if let Ok(repo_layout) =
            clone_source::github_repo_layout(origin, &layout.workspace_root, &layout.repos_root)
        {
            let _ = self.checkout_path.set(repo_layout.primary_repo_path);
        }
    }

    fn repo_cloned(&self) -> bool {
        self.repo_cloned.get().copied().unwrap_or(false)
    }

    fn working_directory(&self) -> Option<&str> {
        self.execution_directory
            .get()
            .map(String::as_str)
            .or_else(|| {
                self.layout
                    .get()
                    .map(|layout| layout.workspace_root.as_str())
            })
    }

    fn record(&self) -> Option<SandboxWorkspaceLayout> {
        let layout = self.layout.get()?;
        let repo = if self.repo_cloned() {
            self.origin_url.get().and_then(|origin| {
                clone_source::github_repo_layout(origin, &layout.workspace_root, &layout.repos_root)
                    .ok()
            })
        } else {
            None
        };
        Some(SandboxWorkspaceLayout {
            workspace_root:    layout.workspace_root.clone(),
            repos_root:        layout.repos_root.clone(),
            primary_repo_path: repo.as_ref().map(|repo| repo.primary_repo_path.clone()),
            primary_repo_link: repo.as_ref().map(|repo| repo.primary_repo_link.clone()),
        })
    }
}

impl LayoutSource {
    fn into_cell(self) -> OnceLock<WorkspaceLayout> {
        let cell = OnceLock::new();
        if let Self::Fixed(layout) = self {
            let _ = cell.set(layout);
        }
        cell
    }
}

/// What a create needs once its inputs are settled.
#[derive(Clone)]
pub(crate) struct PreparedCreate {
    pub(crate) spec:     DriverSpec,
    /// The image or snapshot named by the spec, for pull progress events.
    pub(crate) source:   Option<String>,
    /// The provider snapshot the sandbox is created from, when the provider
    /// has that concept; recorded on the run.
    pub(crate) snapshot: Option<String>,
}

/// Settles a create's inputs right before the provider call. A plan may
/// build provider resources first (a Daytona snapshot) and report progress
/// through fabro's events.
#[async_trait]
pub(crate) trait CreatePlan: Send + Sync {
    async fn prepare(
        &self,
        emit: &(dyn Fn(SandboxEvent) + Send + Sync),
    ) -> crate::Result<PreparedCreate>;
}

/// A create whose spec is known up front.
struct SpecPlan(PreparedCreate);

#[async_trait]
impl CreatePlan for SpecPlan {
    async fn prepare(
        &self,
        _emit: &(dyn Fn(SandboxEvent) + Send + Sync),
    ) -> crate::Result<PreparedCreate> {
        Ok(self.0.clone())
    }
}

/// A sandbox that does not exist yet: `initialize` creates it on the
/// provider from the plan's spec.
struct PendingCreate {
    provider: Arc<dyn DriverProvider>,
    plan:     Box<dyn CreatePlan>,
}

/// A fabro sandbox backed by a sandbox-driver handle.
pub struct DriverSandbox {
    kind:           SandboxProviderKind,
    /// Set at construction for an existing sandbox, at `initialize` for a
    /// pending one.
    handle:         OnceCell<Arc<dyn DriverHandle>>,
    pending:        Option<PendingCreate>,
    workspace:      Option<RepoWorkspace>,
    env_policy:     ExplicitEnvPolicy,
    event_callback: Option<SandboxEventCallback>,
    /// `(platform, os_version)` learned from the sandbox at initialize or
    /// start; unknown until then.
    platform:       OnceLock<(String, String)>,
    /// The provider snapshot the sandbox was created from, when known.
    snapshot:       OnceLock<String>,
}

impl DriverSandbox {
    /// Wraps a driver handle. `local` runs on the worker host, so explicit
    /// environment variables pass the credential filter; every other kind
    /// is isolated and takes the caller's environment as composed.
    #[must_use]
    pub fn new(kind: SandboxProviderKind, handle: Arc<dyn DriverHandle>) -> Self {
        let sandbox = Self::empty(kind);
        let _ = sandbox.handle.set(handle);
        sandbox
    }

    /// A sandbox `initialize` will create from `spec` on `provider`, then
    /// prepare per `workspace`.
    pub(crate) fn pending(
        kind: SandboxProviderKind,
        provider: Arc<dyn DriverProvider>,
        spec: DriverSpec,
        source: Option<String>,
        workspace: RepoWorkspace,
    ) -> Self {
        Self::pending_with_plan(
            kind,
            provider,
            Box::new(SpecPlan(PreparedCreate {
                spec,
                source,
                snapshot: None,
            })),
            workspace,
        )
    }

    /// A sandbox `initialize` will create on `provider` once `plan` has
    /// settled its spec, then prepare per `workspace`.
    pub(crate) fn pending_with_plan(
        kind: SandboxProviderKind,
        provider: Arc<dyn DriverProvider>,
        plan: Box<dyn CreatePlan>,
        workspace: RepoWorkspace,
    ) -> Self {
        let mut sandbox = Self::empty(kind);
        sandbox.pending = Some(PendingCreate { provider, plan });
        sandbox.workspace = Some(workspace);
        sandbox
    }

    /// Records the provider snapshot an attached sandbox was created from.
    pub(crate) fn set_snapshot(&self, snapshot: String) {
        let _ = self.snapshot.set(snapshot);
    }

    /// An existing sandbox reattached by handle, with the workspace an
    /// earlier process prepared.
    pub(crate) fn attached(
        kind: SandboxProviderKind,
        handle: Arc<dyn DriverHandle>,
        workspace: RepoWorkspace,
    ) -> Self {
        workspace.resolve_layout(handle.working_directory());
        let mut sandbox = Self::new(kind, handle);
        sandbox.workspace = Some(workspace);
        sandbox
    }

    fn empty(kind: SandboxProviderKind) -> Self {
        let env_policy = if kind.is_local() {
            ExplicitEnvPolicy::FilterSensitive
        } else {
            ExplicitEnvPolicy::TrustCaller
        };
        Self {
            kind,
            handle: OnceCell::new(),
            pending: None,
            workspace: None,
            env_policy,
            event_callback: None,
            platform: OnceLock::new(),
            snapshot: OnceLock::new(),
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
    /// not carry (git, services, access). Absent until a pending sandbox is
    /// initialized.
    pub fn handle(&self) -> crate::Result<&Arc<dyn DriverHandle>> {
        self.handle.get().ok_or_else(|| {
            crate::Error::message(format!(
                "{} sandbox is not initialized; call initialize() first",
                self.kind
            ))
        })
    }

    fn exec(&self) -> crate::Result<SandboxExec<'_>> {
        let mut exec = SandboxExec::new(self.handle()?.exec(), self.env_policy);
        if let Some(workspace) = &self.workspace {
            if let Some(dir) = workspace.execution_directory.get() {
                exec = exec.with_working_dir(dir.clone());
            }
        }
        Ok(exec)
    }

    /// Resolve a caller path against fabro's working directory. The driver
    /// resolves relative paths against the sandbox's own working directory,
    /// which sits above a cloned repository's link.
    fn resolve(&self, path: &str) -> String {
        match self
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.execution_directory.get())
        {
            Some(working_directory) => sandbox::resolve_path(path, working_directory),
            None => path.to_string(),
        }
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
        self.handle()?.search().ok_or_else(|| {
            crate::Error::message(format!(
                "sandbox provider `{}` does not support search",
                self.kind
            ))
        })
    }

    /// Create the sandbox on the provider when it does not exist yet.
    async fn ensure_created(&self) -> crate::Result<()> {
        if self.handle.get().is_some() {
            return Ok(());
        }
        let Some(pending) = &self.pending else {
            return self.handle().map(|_| ());
        };
        let prepared = pending.plan.prepare(&|event| self.emit(event)).await?;
        if let Some(snapshot) = prepared.snapshot {
            let _ = self.snapshot.set(snapshot);
        }
        let observer = Arc::new(CreateProgress::new(
            prepared.source,
            self.event_callback.clone(),
        ));
        let handle = pending
            .provider
            .create(&prepared.spec, Some(EventContext::new(observer)))
            .await
            .map_err(|error| {
                crate::Error::context(format!("Failed to create {} sandbox", self.kind), error)
            })?;
        let _ = self.handle.set(handle);
        Ok(())
    }

    /// Bring the sandbox to `Running` with a verified Bash, and learn its
    /// platform. Shared by initialize and start.
    async fn make_ready(&self) -> crate::Result<()> {
        sandbox_driver::activate(self.handle()?.as_ref(), &WaitOptions::default()).await?;
        self.learn_platform().await
    }

    /// Prepare the workspace after the sandbox runs for the first time:
    /// an empty root, or fabro's clone.
    async fn prepare_workspace(&self) -> crate::Result<()> {
        let Some(workspace) = &self.workspace else {
            return Ok(());
        };
        let layout = workspace
            .resolve_layout(self.handle()?.working_directory())
            .clone();
        match &workspace.plan {
            WorkspacePlan::Attached => Ok(()),
            WorkspacePlan::Empty(reason) => {
                if matches!(reason, EmptyWorkspaceReason::MissingOrigin) {
                    tracing::warn!(
                        provider = %self.kind,
                        reason = reason.message(),
                        "Clone source missing for clone-based sandbox"
                    );
                }
                self.handle()?
                    .fs()
                    .create_dir(&layout.workspace_root)
                    .await
                    .map_err(|error| {
                        crate::Error::context(
                            format!("Failed to create {}", layout.workspace_root),
                            error,
                        )
                    })?;
                let _ = workspace.repo_cloned.set(false);
                let _ = workspace
                    .execution_directory
                    .set(layout.workspace_root.clone());
                Ok(())
            }
            WorkspacePlan::Clone(plan) => {
                self.emit(SandboxEvent::GitCloneStarted {
                    url:    plan.origin_url.clone(),
                    branch: plan.branch.clone(),
                });
                let started = Instant::now();
                let handle = self.handle()?;
                // The clone names every directory it touches, so it runs
                // without fabro's working-directory override.
                let exec = SandboxExec::new(handle.exec(), self.env_policy);
                let outcome = clone::clone_github_repo(
                    &self.kind,
                    handle.as_ref(),
                    &exec,
                    plan,
                    &layout.workspace_root,
                    &layout.repos_root,
                    &workspace.credentials,
                )
                .await;
                match outcome {
                    Ok(outcome) => {
                        let _ = workspace.repo_cloned.set(true);
                        let _ = workspace.origin_url.set(plan.origin_url.clone());
                        let _ = workspace
                            .checkout_path
                            .set(outcome.layout.primary_repo_path.clone());
                        let _ = workspace
                            .execution_directory
                            .set(outcome.layout.execution_directory.clone());
                        self.emit(SandboxEvent::GitCloneCompleted {
                            url:         plan.origin_url.clone(),
                            duration_ms: elapsed_ms(started),
                        });
                        Ok(())
                    }
                    Err(error) => {
                        self.emit(SandboxEvent::GitCloneFailed {
                            url:    plan.origin_url.clone(),
                            error:  error.to_string(),
                            causes: error.causes(),
                        });
                        Err(error)
                    }
                }
            }
        }
    }

    /// Open an interactive shell in the sandbox's working directory over the
    /// driver's Pty facet.
    pub async fn open_terminal(&self, size: TerminalSize) -> crate::Result<DriverTerminalSession> {
        let handle = self.handle()?;
        let pty = handle.pty().ok_or_else(|| {
            crate::Error::message(format!(
                "sandbox provider `{}` does not support terminals",
                self.kind
            ))
        })?;
        let mut options = PtyOptions::default();
        options.size = PtySize {
            rows: size.rows,
            cols: size.cols,
        };
        options.working_dir = Some(self.working_directory().to_string());
        let session = pty
            .open(&options)
            .await
            .map_err(|error| crate::Error::context("Failed to open sandbox terminal", error))?;
        Ok(DriverTerminalSession::new(session))
    }

    /// Ask the sandbox for its platform once; `platform` and `os_version`
    /// report `unknown` until this has run.
    async fn learn_platform(&self) -> crate::Result<()> {
        if self.platform.get().is_none() {
            let info = self.handle()?.platform_info().await?;
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
            // A cloned repository is reached through a workspace link. The
            // driver refuses a symlinked traversal root, so walk the real
            // checkout; results are reported under the link.
            if let Some(checkout) = self
                .workspace
                .as_ref()
                .and_then(|workspace| workspace.checkout_path.get())
            {
                return sandbox::join_sandbox_path(checkout, relative_start);
            }
            if relative_start.is_empty() {
                ".".to_string()
            } else {
                relative_start.to_string()
            }
        } else {
            sandbox::join_sandbox_path(&self.resolve(base), relative_start)
        }
    }
}

/// Turns the driver's create-time progress into fabro's snapshot events:
/// an image pull starts `SnapshotPulling` and the create's completion ends
/// it. A create without a pull emits nothing.
struct CreateProgress {
    source:       Option<String>,
    callback:     Option<SandboxEventCallback>,
    pull_started: Mutex<Option<Instant>>,
}

impl CreateProgress {
    fn new(source: Option<String>, callback: Option<SandboxEventCallback>) -> Self {
        Self {
            source,
            callback,
            pull_started: Mutex::new(None),
        }
    }

    fn emit(&self, event: SandboxEvent) {
        event.trace();
        if let Some(cb) = &self.callback {
            cb(event);
        }
    }

    fn name(&self) -> String {
        self.source.clone().unwrap_or_default()
    }
}

#[async_trait]
impl EventObserver for CreateProgress {
    async fn observe(&self, event: Event) {
        match &event.body {
            EventBody::OperationProgress { progress, .. }
                if progress.code.as_str() == ProgressCode::IMAGE_PULL =>
            {
                let mut started = self
                    .pull_started
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if started.is_none() {
                    *started = Some(Instant::now());
                    drop(started);
                    self.emit(SandboxEvent::SnapshotPulling { name: self.name() });
                }
            }
            EventBody::OperationCompleted {
                action: Action::Create,
                ..
            } => {
                let started = self
                    .pull_started
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take();
                if let Some(started) = started {
                    self.emit(SandboxEvent::SnapshotReady {
                        name:        self.name(),
                        duration_ms: elapsed_ms(started),
                    });
                }
            }
            EventBody::OperationFailed {
                action: Action::Create,
                error,
                ..
            } => {
                let started = self
                    .pull_started
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take();
                if started.is_some() {
                    self.emit(SandboxEvent::SnapshotFailed {
                        name:   self.name(),
                        error:  error.message.clone(),
                        causes: error.causes.clone(),
                    });
                }
            }
            _ => {}
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
        self.handle()?
            .fs()
            .read(&self.resolve(path))
            .await
            .map_err(|error| crate::Error::context(file_context("read", path), error))
    }

    async fn write_file(&self, path: &str, content: &str) -> crate::Result<()> {
        self.handle()?
            .fs()
            .write(&self.resolve(path), content.as_bytes())
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
        self.handle()?
            .fs()
            .delete(&self.resolve(path), false)
            .await
            .map_err(|error| crate::Error::context(file_context("delete", path), error))
    }

    async fn file_exists(&self, path: &str) -> crate::Result<bool> {
        self.handle()?
            .fs()
            .exists(&self.resolve(path))
            .await
            .map_err(|error| crate::Error::context(file_context("stat", path), error))
    }

    async fn list_directory(
        &self,
        path: &str,
        depth: Option<usize>,
    ) -> crate::Result<Vec<DirEntry>> {
        let entries = self
            .handle()?
            .fs()
            .list_dir(&self.resolve(path), depth.unwrap_or(1))
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
        self.exec()?
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
        self.exec()?.run_streaming(request).await
    }

    async fn spawn_stdio_process(
        &self,
        command: &str,
        working_dir: Option<&str>,
        env_vars: Option<&HashMap<String, String>>,
        cancel_token: Option<CancellationToken>,
    ) -> crate::Result<StdioProcess> {
        self.exec()?
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
            .grep(pattern, &self.resolve(path), &driver_options)
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
                    self.handle()?
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
        self.handle()?
            .fs()
            .download(&self.resolve(remote_path), local_path)
            .await
            .map_err(|error| crate::Error::context(file_context("download", remote_path), error))
    }

    async fn upload_file_from_local(
        &self,
        local_path: &Path,
        remote_path: &str,
    ) -> crate::Result<()> {
        self.handle()?
            .fs()
            .upload(local_path, &self.resolve(remote_path))
            .await
            .map_err(|error| crate::Error::context(file_context("upload", remote_path), error))
    }

    /// Create the sandbox when it is pending, bring it to `Running`, and
    /// prepare fabro's workspace (empty root or clone) on first use.
    async fn initialize(&self) -> crate::Result<()> {
        self.emit(SandboxEvent::Initializing {
            provider: self.provider_name(),
        });
        let started = Instant::now();
        let result = async {
            self.ensure_created().await?;
            self.make_ready().await?;
            self.prepare_workspace().await
        }
        .await;
        let duration_ms = elapsed_ms(started);
        match &result {
            Ok(()) => {
                // The provider's console page, when it has one. Best effort:
                // a failed describe never fails a successful initialize.
                let url = match self.handle() {
                    Ok(handle) if !self.kind.is_local() => handle
                        .describe()
                        .await
                        .ok()
                        .and_then(|status| status.web_url),
                    _ => None,
                };
                self.emit(SandboxEvent::Ready {
                    provider: self.provider_name(),
                    duration_ms,
                    name: Some(self.sandbox_info()).filter(|name| !name.is_empty()),
                    cpu: None,
                    memory: None,
                    url,
                });
            }
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
        let status = self.handle()?.describe().await?;
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
        let result = match self.handle() {
            Ok(handle) => handle.stop().await.map_err(crate::Error::from),
            Err(error) => Err(error),
        };
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
        let result = self.release().await;
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
        let result = self.release().await;
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

    /// The directory the run works in: the cloned repository's link for a
    /// clone-based workspace, the provider's working directory otherwise.
    fn working_directory(&self) -> &str {
        if let Some(directory) = self
            .workspace
            .as_ref()
            .and_then(RepoWorkspace::working_directory)
        {
            return directory;
        }
        self.handle
            .get()
            .map_or("", |handle| handle.working_directory())
    }

    fn runtime_directory(&self) -> Option<&str> {
        self.handle
            .get()
            .and_then(|handle| handle.runtime_directory())
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

    /// The provider's id for this sandbox, or empty for `local`: a local
    /// sandbox is its working directory, which the run record already
    /// carries, and its Host registry id does not outlive the process.
    /// Empty for a pending sandbox that has not been created.
    fn sandbox_info(&self) -> String {
        if self.kind.is_local() {
            return String::new();
        }
        self.handle
            .get()
            .map(|handle| handle.id().to_string())
            .unwrap_or_default()
    }

    fn snapshot_info(&self) -> Option<String> {
        self.snapshot.get().cloned()
    }

    fn workspace_layout(&self) -> Option<SandboxWorkspaceLayout> {
        self.workspace.as_ref().and_then(RepoWorkspace::record)
    }

    async fn set_autostop_interval(&self, minutes: i32) -> crate::Result<()> {
        let mut timers = LifecycleTimers::default();
        timers.auto_stop_after_idle = u64::try_from(minutes)
            .ok()
            .filter(|minutes| *minutes > 0)
            .map(Duration::from_mins);
        match self.handle()?.set_timers(&timers).await {
            // A provider without timers has nothing to stop automatically.
            Ok(()) | Err(sandbox_driver::Error::Unsupported { .. }) => Ok(()),
            Err(error) => Err(crate::Error::context(
                "Failed to set sandbox auto-stop",
                error,
            )),
        }
    }

    async fn setup_git(&self, intent: &GitSetupIntent) -> crate::Result<Option<GitRunInfo>> {
        if !self.repo_cloned() {
            return Ok(None);
        }
        sandbox::setup_git_via_exec(self, intent).await.map(Some)
    }

    fn resume_setup_commands(&self, run_branch: &str) -> Vec<String> {
        if !self.repo_cloned() {
            return Vec::new();
        }
        vec![format!(
            "git fetch origin {} && git checkout {}",
            sandbox::shell_quote(run_branch),
            sandbox::shell_quote(run_branch)
        )]
    }

    async fn git_push_ref(&self, refspec: &str, plan: &RetryPlan) -> Result<PushReport, PushError> {
        let Some(workspace) = &self.workspace else {
            // A designated directory: push only when the checkout has an
            // origin, with whatever credentials its URL already carries.
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
            return sandbox::git_push_via_exec(self, None, refspec, plan).await;
        };
        if !workspace.repo_cloned() {
            return Ok(PushReport::default());
        }
        let credentials = workspace
            .origin_url
            .get()
            .map(|origin_url| (&workspace.credentials, origin_url.as_str()));
        sandbox::git_push_via_exec(self, credentials, refspec, plan).await
    }

    fn origin_url(&self) -> Option<&str> {
        let workspace = self.workspace.as_ref()?;
        if !workspace.repo_cloned() {
            return None;
        }
        workspace.origin_url.get().map(String::as_str)
    }

    #[tracing::instrument(name = "git_op", skip_all, fields(op = "refresh-credentials"))]
    async fn refresh_push_credentials(&self) -> crate::Result<RefreshOutcome> {
        let Some(workspace) = &self.workspace else {
            return Ok(RefreshOutcome::none());
        };
        if !workspace.repo_cloned() {
            return Ok(RefreshOutcome::none());
        }
        let Some(origin_url) = workspace.origin_url.get() else {
            return Ok(RefreshOutcome::none());
        };
        workspace
            .credentials
            .refresh(origin_url, |auth_url| {
                push_credentials::set_auth_url_via_exec(self, auth_url)
            })
            .await
    }

    fn push_token_source(&self) -> Option<Arc<InstallationTokenSource>> {
        self.workspace
            .as_ref()
            .and_then(|workspace| workspace.credentials.source().cloned())
    }

    /// The local command that opens a shell in the sandbox, from the
    /// provider's access facet. `None` when the provider has no such
    /// command (the local sandbox is the host).
    async fn ssh_access_command(&self) -> crate::Result<Option<String>> {
        let Some(shell) = self.handle()?.shell_command() else {
            return Ok(None);
        };
        shell
            .shell_command()
            .await
            .map(Some)
            .map_err(|error| crate::Error::context("Failed to build sandbox shell command", error))
    }

    async fn get_preview_url(
        &self,
        port: u16,
    ) -> crate::Result<Option<(String, HashMap<String, String>)>> {
        let Some(previews) = self.handle()?.preview_urls() else {
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

impl DriverSandbox {
    fn repo_cloned(&self) -> bool {
        self.workspace
            .as_ref()
            .is_some_and(RepoWorkspace::repo_cloned)
    }

    /// Delete the sandbox on the provider. A pending sandbox that was never
    /// created has nothing to release.
    async fn release(&self) -> crate::Result<()> {
        match self.handle.get() {
            Some(handle) => handle.delete().await.map_err(crate::Error::from),
            None if self.pending.is_some() => Ok(()),
            None => self.handle().map(|_| ()),
        }
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
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
            "",
            "local sandboxes are identified by directory"
        );
        let handle = Arc::clone(f.sandbox.handle().unwrap());
        let isolated = DriverSandbox::new(SandboxProviderKind::DOCKER, Arc::clone(&handle));
        assert_eq!(isolated.sandbox_info(), handle.id().to_string());

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
    async fn local_sandbox_designates_the_directory_and_knows_its_platform() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("fresh");
        let sandbox = local_sandbox(&workspace).await.unwrap();
        assert!(workspace.is_dir(), "a missing working directory is created");
        assert_eq!(sandbox.kind(), &SandboxProviderKind::LOCAL);
        assert_ne!(sandbox.platform(), "unknown");
        assert_eq!(
            Path::new(sandbox.working_directory()),
            workspace.canonicalize().unwrap()
        );
        sandbox.cleanup().await.unwrap();
        assert!(workspace.is_dir());
    }

    #[tokio::test]
    async fn preview_urls_come_from_the_access_facet() {
        let f = fixture().await;
        let (url, headers) = f.sandbox.get_preview_url(8080).await.unwrap().unwrap();
        assert_eq!(url, "http://127.0.0.1:8080");
        assert!(headers.is_empty());
    }
}
