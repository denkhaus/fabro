//! The engine-injected stage environment (fabro-6e7f, MOTOR-VARIANTE):
//! every process a run's sandbox spawns for a stage sees
//! `FABRO_STAGE=<node-id>`, and no caller-supplied value of that variable
//! survives — the engine's value overrides it, and outside a stage
//! dispatch the variable is removed rather than trusted.
//!
//! The seam is the same shape as the tool-policy and preamble-policy
//! wrappers the runtime already installs: Petri composes each step's
//! process environment deep inside its step kinds (the agent session,
//! the command script), where a fabro-side edit cannot reach, but every
//! one of those processes reaches the sandbox through the sandbox
//! driver's `exec` facet. Fabro owns the built-in provider factories
//! ([`crate::providers`]), so the injection wraps the handle those
//! factories connect: a [`StageFactory`] delegates every factory method
//! and wraps the provider, a [`StageProvider`] wraps every sandbox it
//! hands out, and the sandbox's `exec` facet rewrites each spec's
//! environment immediately before the driver runs it.
//!
//! The node id itself is known only at dispatch, so the run's hook
//! service ([`crate::tool_policy::ToolPolicyHooks`], which sees every
//! stage's `FiringView` at `BeforeVisit`/`BeforeAttempt`/`Retrying`,
//! before the step spawns anything) records it on the shared
//! [`StageDispatch`] cell the exec facet reads. One cell serves a run:
//! the workflow graph lowers to a single scope, so a run's stages share
//! one sandbox and run one at a time. Concurrent firings inside one
//! sandbox (a parallel family the engine interleaves there) would race
//! on the cell — recorded as the seam's known boundary, not silently
//! absorbed.
//!
//! The facet applies two injections, each its own cell and its own file:
//! the dispatched stage (this file, fabro-6e7f) and the run's Git identity
//! ([`crate::fork_git_identity`], fabro-19f9), so a stage that commits
//! inside the sandbox commits as the run's identity.
//!
//! This file is fork-owned (new file, `fork_` prefix): an upstream merge
//! cannot silently absorb it. The upstream touch points are the factory
//! wrap in `providers.rs` and the cell hand-off in `runtime.rs`, pinned
//! by the tests here and in `tests/hooks.rs`.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use petri_runtime::{ProviderContext, ProviderFactory, ProviderNetwork};
use sandbox_driver::{
    Capabilities, EventContext, Exec, ExecControls, ExecResult, ExecSpec, ExecStreamingResult,
    Filesystem, ForkOptions, Git, GitFacet, LifecycleTimers, Logs, NetworkPolicy, OneShot,
    PlatformInfo, PreviewUrls, ProviderHealth, ProviderKind, Pty, Resources, Sandbox,
    SandboxFilter, SandboxId, SandboxProvider, SandboxSnapshotOptions, SandboxSpec, SandboxStatus,
    Search, SearchFacet, Services, ServicesFacet, ShellCommand, SnapshotId, SpawnSpec, SshAccess,
    StdioProcess, Vnc, WebTerminal,
};

use crate::fork_git_identity::RunGitIdentity;

/// The environment variable the engine injects: the id of the node being
/// dispatched, as the deterministic verify dispatcher (`just verify`)
/// reads it.
pub const STAGE_ENV_VAR: &str = "FABRO_STAGE";

/// The node currently dispatched in a run, shared between the hook
/// service that observes dispatch and the exec facet that injects.
pub struct StageDispatch {
    current: RwLock<Option<String>>,
}

impl StageDispatch {
    /// A shared cell for one run's runtime.
    #[must_use]
    pub fn shared() -> Arc<Self> {
        Arc::new(Self {
            current: RwLock::new(None),
        })
    }

    /// Record the node being dispatched.
    pub fn set(&self, node: &str) {
        if let Ok(mut current) = self.current.write() {
            *current = Some(node.to_owned());
        }
    }

    /// The node being dispatched, when one is.
    #[must_use]
    pub fn stage(&self) -> Option<String> {
        self.current.read().ok().and_then(|current| current.clone())
    }

    /// Apply the dispatch to a process environment: the engine's stage
    /// overrides any caller-supplied value, and no stage in dispatch
    /// removes the variable rather than trusting what arrived.
    fn apply(&self, env: &mut BTreeMap<String, String>) {
        match self.stage() {
            Some(stage) => {
                env.insert(STAGE_ENV_VAR.to_owned(), stage);
            }
            None => {
                env.remove(STAGE_ENV_VAR);
            }
        }
    }
}

/// A factory that wraps its provider's sandboxes with the run's injected
/// process environment — the dispatched stage and the run's Git identity:
/// the one touch point on the provider path every built-in kind connects
/// through.
pub struct StageFactory {
    inner:    Arc<dyn ProviderFactory>,
    stages:   Arc<StageDispatch>,
    identity: Arc<RunGitIdentity>,
}

impl StageFactory {
    /// Wrap `inner` so every sandbox it connects carries the injections:
    /// the dispatched stage and the run's Git identity.
    #[must_use]
    pub fn new(
        inner: Arc<dyn ProviderFactory>,
        stages: Arc<StageDispatch>,
        identity: Arc<RunGitIdentity>,
    ) -> Self {
        Self {
            inner,
            stages,
            identity,
        }
    }
}

#[async_trait]
impl ProviderFactory for StageFactory {
    fn kind(&self) -> &str {
        self.inner.kind()
    }

    fn fingerprint_seed(&self, context: &ProviderContext) -> String {
        self.inner.fingerprint_seed(context)
    }

    fn region(&self) -> Option<&str> {
        self.inner.region()
    }

    fn network(&self) -> ProviderNetwork {
        self.inner.network()
    }

    async fn connect(
        &self,
        context: &ProviderContext,
    ) -> sandbox_driver::Result<Arc<dyn SandboxProvider>> {
        let provider = self.inner.connect(context).await?;
        Ok(Arc::new(StageProvider {
            inner:    provider,
            stages:   Arc::clone(&self.stages),
            identity: Arc::clone(&self.identity),
        }))
    }
}

/// A provider whose sandboxes inject the dispatched stage and the run's
/// Git identity into every process they run. Delegates everything but
/// handle creation, the same shape as the driver's own `OwnedProvider`.
pub struct StageProvider {
    inner:    Arc<dyn SandboxProvider>,
    stages:   Arc<StageDispatch>,
    identity: Arc<RunGitIdentity>,
}

#[async_trait]
impl SandboxProvider for StageProvider {
    fn kind(&self) -> &ProviderKind {
        self.inner.kind()
    }

    fn capabilities(&self) -> &Capabilities {
        self.inner.capabilities()
    }

    async fn create(
        &self,
        spec: &SandboxSpec,
        events: Option<EventContext>,
    ) -> sandbox_driver::Result<Arc<dyn Sandbox>> {
        let sandbox = self.inner.create(spec, events).await?;
        Ok(stage_sandbox(sandbox, &self.stages, &self.identity))
    }

    async fn attach(
        &self,
        id: &SandboxId,
        events: Option<EventContext>,
    ) -> sandbox_driver::Result<Arc<dyn Sandbox>> {
        let sandbox = self.inner.attach(id, events).await?;
        Ok(stage_sandbox(sandbox, &self.stages, &self.identity))
    }

    async fn undelete(
        &self,
        id: &SandboxId,
        events: Option<EventContext>,
    ) -> sandbox_driver::Result<Arc<dyn Sandbox>> {
        let sandbox = self.inner.undelete(id, events).await?;
        Ok(stage_sandbox(sandbox, &self.stages, &self.identity))
    }

    async fn delete(
        &self,
        id: &SandboxId,
        events: Option<EventContext>,
    ) -> sandbox_driver::Result<()> {
        self.inner.delete(id, events).await
    }

    async fn list(&self, filter: &SandboxFilter) -> sandbox_driver::Result<Vec<SandboxStatus>> {
        self.inner.list(filter).await
    }

    async fn health(&self) -> sandbox_driver::Result<ProviderHealth> {
        self.inner.health().await
    }
}

/// Wrap `sandbox` for the injected process environment: the handle
/// delegates everything but its `exec` facet.
fn stage_sandbox(
    sandbox: Arc<dyn Sandbox>,
    stages: &Arc<StageDispatch>,
    identity: &Arc<RunGitIdentity>,
) -> Arc<dyn Sandbox> {
    Arc::new(StageSandbox {
        exec:     StageExec {
            inner:    Arc::clone(&sandbox),
            stages:   Arc::clone(stages),
            identity: Arc::clone(identity),
        },
        inner:    sandbox,
        stages:   Arc::clone(stages),
        identity: Arc::clone(identity),
    })
}

/// A sandbox handle whose `exec` facet injects the dispatched stage and the
/// run's Git identity.
pub struct StageSandbox {
    exec:     StageExec,
    inner:    Arc<dyn Sandbox>,
    stages:   Arc<StageDispatch>,
    identity: Arc<RunGitIdentity>,
}

#[async_trait]
impl Sandbox for StageSandbox {
    fn id(&self) -> &SandboxId {
        self.inner.id()
    }

    fn capabilities(&self) -> &Capabilities {
        self.inner.capabilities()
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
        self.inner.runtime_directory()
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

    async fn pause(&self) -> sandbox_driver::Result<()> {
        self.inner.pause().await
    }

    async fn resume(&self) -> sandbox_driver::Result<()> {
        self.inner.resume().await
    }

    async fn archive(&self) -> sandbox_driver::Result<()> {
        self.inner.archive().await
    }

    async fn fork(&self, options: &ForkOptions) -> sandbox_driver::Result<Arc<dyn Sandbox>> {
        let sandbox = self.inner.fork(options).await?;
        Ok(stage_sandbox(sandbox, &self.stages, &self.identity))
    }

    async fn resize(&self, resources: &Resources) -> sandbox_driver::Result<()> {
        self.inner.resize(resources).await
    }

    async fn snapshot(
        &self,
        options: &SandboxSnapshotOptions,
    ) -> sandbox_driver::Result<SnapshotId> {
        self.inner.snapshot(options).await
    }

    async fn recover(&self) -> sandbox_driver::Result<()> {
        self.inner.recover().await
    }

    async fn refresh_activity(&self) -> sandbox_driver::Result<()> {
        self.inner.refresh_activity().await
    }

    async fn set_timers(&self, timers: &LifecycleTimers) -> sandbox_driver::Result<()> {
        self.inner.set_timers(timers).await
    }

    async fn set_labels(&self, labels: &BTreeMap<String, String>) -> sandbox_driver::Result<()> {
        self.inner.set_labels(labels).await
    }

    async fn update_network(&self, policy: &NetworkPolicy) -> sandbox_driver::Result<()> {
        self.inner.update_network(policy).await
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

    fn search(&self) -> Option<SearchFacet<'_>> {
        self.inner.search()
    }

    fn provider_git(&self) -> Option<&dyn Git> {
        self.inner.provider_git()
    }

    fn git(&self) -> Option<GitFacet<'_>> {
        self.inner.git()
    }

    fn provider_services(&self) -> Option<&dyn Services> {
        self.inner.provider_services()
    }

    fn services(&self) -> Option<ServicesFacet<'_>> {
        self.inner.services()
    }

    fn pty(&self) -> Option<&dyn Pty> {
        self.inner.pty()
    }

    fn logs(&self) -> Option<&dyn Logs> {
        self.inner.logs()
    }

    fn one_shot(&self) -> Option<&dyn OneShot> {
        self.inner.one_shot()
    }

    fn preview_urls(&self) -> Option<&dyn PreviewUrls> {
        self.inner.preview_urls()
    }

    fn ssh(&self) -> Option<&dyn SshAccess> {
        self.inner.ssh()
    }

    fn shell_command(&self) -> Option<&dyn ShellCommand> {
        self.inner.shell_command()
    }

    fn web_terminal(&self) -> Option<&dyn WebTerminal> {
        self.inner.web_terminal()
    }

    fn vnc(&self) -> Option<&dyn Vnc> {
        self.inner.vnc()
    }
}

/// The exec facet that applies the injections to every spec immediately
/// before the driver runs it: buffered, streaming, and stdio process
/// spawns alike.
pub struct StageExec {
    inner:    Arc<dyn Sandbox>,
    stages:   Arc<StageDispatch>,
    identity: Arc<RunGitIdentity>,
}

impl StageExec {
    /// Apply both injections to a spec's environment: the dispatched stage
    /// (fabro-6e7f) and the run's Git identity (fabro-19f9).
    fn apply(&self, env: &mut BTreeMap<String, String>) {
        self.stages.apply(env);
        self.identity.apply(env);
    }
}

#[async_trait]
impl Exec for StageExec {
    async fn run(&self, spec: &ExecSpec) -> sandbox_driver::Result<ExecResult> {
        let mut spec = spec.clone();
        self.apply(&mut spec.env);
        self.inner.exec().run(&spec).await
    }

    async fn run_streaming(
        &self,
        spec: &ExecSpec,
        controls: ExecControls,
    ) -> sandbox_driver::Result<ExecStreamingResult> {
        let mut spec = spec.clone();
        self.apply(&mut spec.env);
        self.inner.exec().run_streaming(&spec, controls).await
    }

    async fn spawn_stdio(&self, spec: &SpawnSpec) -> sandbox_driver::Result<StdioProcess> {
        let mut staged =
            SpawnSpec::new(spec.program.clone()).args(spec.args.iter().map(String::as_str));
        if let Some(dir) = &spec.working_dir {
            staged = staged.working_dir(dir.clone());
        }
        staged.env = spec.env.clone();
        self.apply(&mut staged.env);
        self.inner.exec().spawn_stdio(&staged).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork_git_identity::{AUTHOR_EMAIL, AUTHOR_NAME, COMMITTER_EMAIL, COMMITTER_NAME};

    /// An exec facet that records the environment of every spec it is
    /// asked, then refuses: the fixture asserts what reached the facet,
    /// never a result.
    struct RecordingExec {
        runs:    RwLock<Vec<BTreeMap<String, String>>>,
        spawns:  RwLock<Vec<BTreeMap<String, String>>>,
        streams: RwLock<Vec<BTreeMap<String, String>>>,
    }

    impl RecordingExec {
        fn new() -> Self {
            Self {
                runs:    RwLock::new(Vec::new()),
                spawns:  RwLock::new(Vec::new()),
                streams: RwLock::new(Vec::new()),
            }
        }

        fn refuse() -> sandbox_driver::Error {
            sandbox_driver::Error::unsupported(sandbox_driver::Capability::ExecStdioProcess)
        }
    }

    #[async_trait]
    impl Exec for RecordingExec {
        async fn run(&self, spec: &ExecSpec) -> sandbox_driver::Result<ExecResult> {
            self.runs.write().unwrap().push(spec.env.clone());
            Err(Self::refuse())
        }

        async fn run_streaming(
            &self,
            spec: &ExecSpec,
            _controls: ExecControls,
        ) -> sandbox_driver::Result<ExecStreamingResult> {
            self.streams.write().unwrap().push(spec.env.clone());
            Err(Self::refuse())
        }

        async fn spawn_stdio(&self, spec: &SpawnSpec) -> sandbox_driver::Result<StdioProcess> {
            self.spawns.write().unwrap().push(spec.env.clone());
            Err(Self::refuse())
        }
    }

    fn wrapped(stages: &Arc<StageDispatch>) -> (Arc<StageExec>, Arc<RecordingExec>) {
        wrapped_with_identity(stages, &RunGitIdentity::shared())
    }

    fn wrapped_with_identity(
        stages: &Arc<StageDispatch>,
        identity: &Arc<RunGitIdentity>,
    ) -> (Arc<StageExec>, Arc<RecordingExec>) {
        let recorder = Arc::new(RecordingExec::new());
        let sandbox: Arc<dyn Sandbox> = Arc::new(FakeSandbox {
            exec: Arc::clone(&recorder),
        });
        let exec = Arc::new(StageExec {
            inner:    sandbox,
            stages:   Arc::clone(stages),
            identity: Arc::clone(identity),
        });
        (exec, recorder)
    }

    /// The sandbox half of the fixture: only the exec facet is asked for.
    struct FakeSandbox {
        exec: Arc<RecordingExec>,
    }

    #[async_trait]
    impl Sandbox for FakeSandbox {
        fn id(&self) -> &SandboxId {
            unreachable!("the fixture never reaches identity")
        }

        fn capabilities(&self) -> &Capabilities {
            unreachable!("the fixture never reaches capabilities")
        }

        async fn describe(&self) -> sandbox_driver::Result<SandboxStatus> {
            unreachable!("the fixture never reaches describe")
        }

        fn working_directory(&self) -> &str {
            unreachable!("the fixture never reaches the working directory")
        }

        async fn platform_info(&self) -> sandbox_driver::Result<PlatformInfo> {
            unreachable!("the fixture never reaches platform info")
        }

        async fn start(&self) -> sandbox_driver::Result<()> {
            unreachable!("the fixture never starts")
        }

        async fn stop(&self) -> sandbox_driver::Result<()> {
            unreachable!("the fixture never stops")
        }

        async fn delete(&self) -> sandbox_driver::Result<()> {
            unreachable!("the fixture never deletes")
        }

        fn exec(&self) -> &dyn Exec {
            &*self.exec
        }

        fn fs(&self) -> &dyn Filesystem {
            unreachable!("the fixture never reaches the fs facet")
        }
    }

    fn spec_with(env: &[(&str, &str)]) -> ExecSpec {
        let mut spec = ExecSpec::new("true");
        for (key, value) in env {
            spec = spec.env_var(*key, *value);
        }
        spec
    }

    /// Presence pin (fork feature, fabro-6e7f): a dispatched stage's node
    /// id reaches the exec facet's environment.
    #[tokio::test]
    async fn the_dispatched_stage_reaches_every_spawn() {
        let stages = StageDispatch::shared();
        stages.set("implementer");
        let (exec, recorder) = wrapped(&stages);

        let _ = exec.run(&spec_with(&[("PATH", "/bin")])).await;
        let _ = exec
            .run_streaming(&spec_with(&[]), ExecControls::default())
            .await;
        let _ = exec.spawn_stdio(&SpawnSpec::new("agent")).await;

        for lane in [&recorder.runs, &recorder.streams, &recorder.spawns] {
            let recorded = lane.read().unwrap();
            assert_eq!(recorded.len(), 1, "one spec per lane");
            assert_eq!(
                recorded[0].get(STAGE_ENV_VAR).map(String::as_str),
                Some("implementer")
            );
        }
    }

    /// The negative spoof case: a caller-supplied `FABRO_STAGE` never
    /// survives — the engine's value overrides it.
    #[tokio::test]
    async fn a_spoofed_stage_is_overridden() {
        let stages = StageDispatch::shared();
        stages.set("tester");
        let (exec, recorder) = wrapped(&stages);

        let _ = exec
            .run(&spec_with(&[(STAGE_ENV_VAR, "implementer")]))
            .await;

        let recorded = recorder.runs.read().unwrap();
        assert_eq!(
            recorded[0].get(STAGE_ENV_VAR).map(String::as_str),
            Some("tester"),
            "the engine's stage wins over the caller's"
        );
    }

    /// Outside a dispatch the variable is removed, never trusted: a
    /// spoofed value must not leak through a run-level spawn.
    #[tokio::test]
    async fn an_undispatched_spawn_carries_no_stage() {
        let stages = StageDispatch::shared();
        let (exec, recorder) = wrapped(&stages);

        let _ = exec
            .run(&spec_with(&[(STAGE_ENV_VAR, "implementer")]))
            .await;

        let recorded = recorder.runs.read().unwrap();
        assert!(
            !recorded[0].contains_key(STAGE_ENV_VAR),
            "no stage in dispatch: the variable is absent, not inherited"
        );
    }

    /// Presence pin (fork feature, fabro-19f9): the run's Git identity
    /// reaches every process the exec facet runs, so a stage that commits
    /// inside its sandbox needs no repository-local identity.
    #[tokio::test]
    async fn the_runs_git_identity_reaches_every_spawn() {
        let identity = RunGitIdentity::shared();
        identity.set(fabro_types::GitIdentity {
            name:   "Fabro Pin".to_string(),
            email:  "pin@fabro.test".to_string(),
            source: fabro_types::GitIdentitySource::Explicit,
        });
        let (exec, recorder) = wrapped_with_identity(&StageDispatch::shared(), &identity);

        let _ = exec.run(&spec_with(&[("PATH", "/bin")])).await;
        let _ = exec
            .run_streaming(&spec_with(&[]), ExecControls::default())
            .await;
        let _ = exec.spawn_stdio(&SpawnSpec::new("agent")).await;

        for lane in [&recorder.runs, &recorder.streams, &recorder.spawns] {
            let recorded = lane.read().unwrap();
            assert_eq!(recorded.len(), 1, "one spec per lane");
            assert_eq!(
                recorded[0].get(AUTHOR_NAME).map(String::as_str),
                Some("Fabro Pin")
            );
            assert_eq!(
                recorded[0].get(AUTHOR_EMAIL).map(String::as_str),
                Some("pin@fabro.test")
            );
            assert_eq!(
                recorded[0].get(COMMITTER_NAME).map(String::as_str),
                Some("Fabro Pin")
            );
            assert_eq!(
                recorded[0].get(COMMITTER_EMAIL).map(String::as_str),
                Some("pin@fabro.test")
            );
        }
    }

    /// A run that recorded no identity injects none: the facet never
    /// invents an author.
    #[tokio::test]
    async fn a_run_without_an_identity_injects_none() {
        let (exec, recorder) =
            wrapped_with_identity(&StageDispatch::shared(), &RunGitIdentity::shared());

        let _ = exec.run(&spec_with(&[])).await;

        let recorded = recorder.runs.read().unwrap();
        assert!(!recorded[0].contains_key(AUTHOR_NAME));
    }
}
