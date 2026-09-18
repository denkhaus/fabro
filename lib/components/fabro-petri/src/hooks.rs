//! Fabro's awaited extension points on a Petri run: the checkpoint commit,
//! its platform record, and the run-level ends, wrapped around Petri's own
//! hook service so `[[run.hooks]]` keep running.
//!
//! [`FabroHooks`] implements Petri's `ExecutionHooks` and is installed with
//! `Runtime::hooks` by [`engine::run`](crate::engine::run). It holds the
//! hooks the runtime installed before it (Petri's local hook service behind
//! its adapter, which serves `[[run.hooks]]`) and forwards every point to
//! them, `run_finished` and `scope_released` included, the way Petri's
//! embedding host does. Its own work, at each point:
//!
//! - `prepare_result`: the checkpoint commit, before the `StepFinished` record
//!   is appended, so a durable finish implies a durable snapshot. A stage that
//!   failed on its own terms is committed like a successful one; only a
//!   cancelled attempt is not. A failed commit is fatal to the run: the outcome
//!   becomes a failure of class `checkpoint_failed`, the run is cancelled
//!   through the coordinator handle, and `transition` refuses the firing's
//!   routes, so no route is taken.
//! - `transition`: the platform checkpoint record, keyed on the Petri position
//!   and the checkpoint's operation identity. A failed write is a recorded
//!   problem on the transition, never a blocked route.
//! - `run_finished` and `scope_released`: forwarded, so the local service runs
//!   `run_complete`, `run_failed` and `sandbox_cleanup` with the sandbox in
//!   place. Fabro's own end-of-run work (the terminal lifecycle event,
//!   notifications on it) is the run lifecycle path's, on the worker's and
//!   server's side of the engine, and the workspace's retention is Petri's
//!   (`Retention::Always`).
//!
//! # Operation identities
//!
//! Every external effect here is keyed on `(run key, execution, DecisionId,
//! effect kind)` from the hook context and deduplicated on retry: the
//! checkpoint's key is the attempt's decision in its execution, effect
//! `checkpoint`. A re-dispatched attempt whose commit already landed
//! reuses it when the workspace still sits on it unchanged (see
//! [`RunWorkspaces::commit`]); a reissued routing decision finds the
//! record, or the commit by its trailers, and writes nothing twice.
//!
//! # Where the workspace is
//!
//! On the local provider the commit runs on the host, in the workspace
//! Petri's host backend keeps under the run directory (`crate::checkpoint`).
//! On Docker or Daytona the workspace lives inside the scope's sandbox: the
//! hooks keep the environment Petri hands them at `scope_acquired`, run
//! `git` inside the scope through it, and move the commit out as a bundle
//! into the same snapshot repository the host path pushes to. The same
//! point is where a resumed run brings a sandbox workspace to the snapshot
//! its durable state names, before the first attempt runs in it: verified,
//! reset, or, in a fresh sandbox (Petri replaces a lost one on Fabro's
//! request), restored from a bundle of the checkpoint. The plan is
//! [`recovery::plan`](crate::recovery::plan), the one the server applied
//! to host workspaces before it relaunched the worker.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use fabro_checkpoint::author::GitAuthor;
use fabro_store::platform_records::CheckpointRecord;
use fabro_store::{PlatformRecord, PlatformRecordKind, StagePosition};
use fabro_types::settings::run::{RunCheckpointSettings, RunNamespace};
use fabro_types::{RunId, SandboxProviderKind};
use fabro_util::error::collect_chain;
use petri_execution::{CancelReason, CoordinatorHandle, InvocationId, RunKey, RunStore};
use petri_runtime::driver::lifecycle::{
    AdmitAttempt, AttemptDecision, ExecutionHooks, HookContext, Note, PrepareError, PrepareResult,
    Prepared, Recorded, ResultOrigin, RunFinished, ScopeAcquired, ScopeAcquiredError,
    ScopeReleased, Transition, TransitionError, TransitionReport,
};
use petri_runtime::executor::ExecEnv;
use petri_runtime::ir::{ExecutionId, FailureInfo, ScopeId, Status};
use serde_json::json;
use tokio::sync::{Mutex as AsyncMutex, OnceCell};
use tokio::{fs, time};
use tracing::{debug, info, warn};

use crate::checkpoint::{CHECKPOINT_FAILED_CLASS, CheckpointKey, RunWorkspaces};
use crate::platform_records::PlatformRecords;
use crate::recovery::{self, Plan, RestoreTarget};
use crate::workspace::{self, WorkspaceLookup};

/// The note kind the hooks record on a firing about its checkpoint.
pub const CHECKPOINT_NOTE: &str = "fabro.checkpoint";

/// How often a held checkpoint polls its test gate.
const GATE_POLL: Duration = Duration::from_millis(50);

/// What Fabro's hooks need beside the run: where the platform records go,
/// who authors the commits, and the checkpoint settings.
pub struct HooksSpec {
    pub records:         Arc<dyn PlatformRecords>,
    pub author:          GitAuthor,
    pub checkpoint:      RunCheckpointSettings,
    /// Whether the run's workspaces are on this host (the local sandbox
    /// provider). A run elsewhere snapshots inside its sandboxes.
    pub host_workspaces: bool,
    /// A test's gate directory: a checkpoint point named by a `.hold` file
    /// there waits for its `.release` file. `None` outside tests.
    pub test_gates:      Option<PathBuf>,
}

impl HooksSpec {
    /// The spec a run's settings give: its Git author, its checkpoint
    /// settings, and whether its sandbox provider keeps workspaces on this
    /// host.
    #[must_use]
    pub fn for_run(records: Arc<dyn PlatformRecords>, settings: &RunNamespace) -> Self {
        Self {
            records,
            author: settings
                .git
                .author
                .as_ref()
                .map(GitAuthor::from)
                .unwrap_or_default(),
            checkpoint: settings.checkpoint.clone(),
            host_workspaces: settings.environment.provider == SandboxProviderKind::LOCAL,
            test_gates: None,
        }
    }

    #[must_use]
    pub fn with_test_gates(mut self, gates: Option<PathBuf>) -> Self {
        self.test_gates = gates;
        self
    }
}

/// A scope's sandbox environment as the hooks keep it: the workspace id
/// the executor named, and the environment `git` runs in.
type AcquiredEnv = (String, Arc<dyn ExecEnv>);

/// Fabro's `ExecutionHooks`, around the hooks the runtime installed.
pub struct FabroHooks {
    inner:           Arc<dyn ExecutionHooks>,
    run_id:          RunId,
    records:         Arc<dyn PlatformRecords>,
    workspaces:      RunWorkspaces,
    lookup:          WorkspaceLookup,
    host_workspaces: bool,
    test_gates:      Option<PathBuf>,
    handle:          OnceLock<CoordinatorHandle>,
    /// The workspace and commit of every checkpoint this process made.
    committed:       Mutex<HashMap<CheckpointKey, (String, String)>>,
    /// Which checkpoints have their platform record, loaded from the store
    /// once and kept up to date with every append.
    recorded:        Mutex<HashSet<CheckpointKey>>,
    recorded_loaded: OnceCell<()>,
    /// Inherited workspaces resolved through the run's records.
    inherited:       Mutex<HashMap<InvocationId, Option<String>>>,
    /// One lock per workspace: the branches of a parallel node and a nested
    /// invocation share their caller's workspace, and Git allows one index
    /// operation at a time in it.
    workspace_locks: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    /// The checkpoint failure that ended the run, when one did.
    failure:         Mutex<Option<String>>,
    /// The sandbox environment of every acquired scope, by execution and
    /// scope, with the workspace id the executor named: where `git` runs
    /// when the workspaces are not on this host. Dropped at release.
    envs:            Mutex<HashMap<(ExecutionId, ScopeId), AcquiredEnv>>,
    /// Whether the run continues from its records: a sandbox workspace is
    /// then brought to its snapshot when its scope is first acquired.
    resumed:         bool,
    /// The snapshot every live sandbox workspace must sit on before work
    /// resumes in it, read once from the records; an entry leaves when it
    /// is applied.
    restore:         OnceCell<Mutex<BTreeMap<String, RestoreTarget>>>,
    store:           Arc<dyn RunStore>,
}

impl FabroHooks {
    /// Wrap `inner` (the hooks `Runtime::installed_hooks` returned) for the
    /// run whose records are in `store` under `run_key`, with its
    /// workspaces under `run_dir`. `resumed` says the run continues from
    /// its records, so a sandbox workspace is brought to its snapshot at
    /// its scope's first acquisition.
    #[must_use]
    pub fn new(
        spec: HooksSpec,
        inner: Arc<dyn ExecutionHooks>,
        run_id: RunId,
        run_key: RunKey,
        run_dir: PathBuf,
        store: Arc<dyn RunStore>,
        resumed: bool,
    ) -> Self {
        let workspaces =
            RunWorkspaces::new(run_dir, run_id.to_string(), spec.author, &spec.checkpoint);
        Self {
            inner,
            run_id,
            records: spec.records,
            workspaces,
            lookup: WorkspaceLookup::new(Arc::clone(&store), run_key),
            host_workspaces: spec.host_workspaces,
            test_gates: spec.test_gates,
            handle: OnceLock::new(),
            committed: Mutex::default(),
            recorded: Mutex::default(),
            recorded_loaded: OnceCell::new(),
            inherited: Mutex::default(),
            workspace_locks: Mutex::default(),
            failure: Mutex::default(),
            envs: Mutex::default(),
            resumed,
            restore: OnceCell::new(),
            store,
        }
    }

    /// Hand the hooks the running coordinator, so a fatal checkpoint can
    /// cancel the run. Called once, from the host's handle callback.
    pub fn attach(&self, handle: CoordinatorHandle) {
        if self.handle.set(handle).is_err() {
            debug!("the coordinator handle was already attached to the hooks");
        }
    }

    /// The checkpoint failure that ended the run, when one did: what the
    /// engine reports the run failed with.
    #[must_use]
    pub fn checkpoint_failure(&self) -> Option<String> {
        lock(&self.failure).clone()
    }

    /// The run's workspaces on this host, as the hooks reach them.
    #[must_use]
    pub fn workspaces(&self) -> &RunWorkspaces {
        &self.workspaces
    }

    /// The lock that serializes Git work in one workspace.
    fn workspace_lock(&self, workspace: &str) -> Arc<AsyncMutex<()>> {
        Arc::clone(
            lock(&self.workspace_locks)
                .entry(workspace.to_string())
                .or_default(),
        )
    }

    fn fail_run(&self, message: &str) {
        let mut failure = lock(&self.failure);
        if failure.is_none() {
            *failure = Some(message.to_string());
        }
        drop(failure);
        if let Some(handle) = self.handle.get() {
            info!(run_id = %self.run_id, "cancelling the Petri run after a failed checkpoint");
            handle.cancel_root_for(CancelReason::Control);
        } else {
            warn!(
                run_id = %self.run_id,
                "no coordinator handle is attached; the failed checkpoint cannot cancel the run"
            );
        }
    }

    /// The workspace id of `scope` in the context's invocation: the
    /// isolated name when its workspace exists, else the inherited one the
    /// records name, else the isolated name for the caller to report.
    async fn workspace_of(&self, context: &HookContext, scope: ScopeId) -> Result<String, String> {
        let isolated = workspace::isolated_workspace(context.invocation, scope);
        if self.workspaces.workspace_exists(&isolated).await {
            return Ok(isolated);
        }
        let cached = lock(&self.inherited).get(&context.invocation).cloned();
        let inherited = if let Some(inherited) = cached {
            inherited
        } else {
            let inherited = self
                .lookup
                .inherited(context.invocation)
                .await
                .map_err(|error| {
                    format!(
                        "the workspace of scope {scope} in invocation {} could not be found: {}",
                        context.invocation,
                        collect_chain(&error).join(": ")
                    )
                })?;
            lock(&self.inherited).insert(context.invocation, inherited.clone());
            inherited
        };
        Ok(inherited.unwrap_or(isolated))
    }

    /// The checkpoint commit for one attempt's result. `Ok(Some)` is the
    /// note to record, `Ok(None)` nothing to record, `Err` the fatal
    /// failure message.
    async fn snapshot(
        &self,
        context: &HookContext,
        scope: ScopeId,
        key: CheckpointKey,
        node: &str,
        status: &Status,
        origin: ResultOrigin,
    ) -> Result<Option<Note>, String> {
        if !self.host_workspaces {
            return self
                .snapshot_in_sandbox(context, scope, key, node, status, origin)
                .await;
        }
        let workspace = self.workspace_of(context, scope).await?;
        if !self.workspaces.workspace_exists(&workspace).await {
            // A skipped node or a driver-made outcome may precede the scope's
            // environment; nothing of the stage's is on disk to snapshot.
            if origin == ResultOrigin::Driver || matches!(status, Status::Skipped) {
                return Ok(Some(Note::new(
                    CHECKPOINT_NOTE,
                    json!({
                        "execution": key.execution,
                        "firing": key.firing,
                        "attempt": key.attempt,
                        "workspace": workspace,
                        "skipped": "the workspace does not exist yet",
                    }),
                )));
            }
            return Err(format!(
                "the workspace `{workspace}` of scope {scope} does not exist at {}",
                self.workspaces.workspace_path(&workspace).display()
            ));
        }
        self.gate("commit", node).await;
        let serialized = self.workspace_lock(&workspace);
        let _held = serialized.lock().await;
        match self
            .workspaces
            .commit(&workspace, key, node, status.tag())
            .await
        {
            Ok(snapshot) => {
                debug!(
                    run_id = %self.run_id,
                    node,
                    execution = key.execution,
                    firing = key.firing,
                    attempt = key.attempt,
                    reused = snapshot.reused,
                    "checkpoint committed"
                );
                lock(&self.committed).insert(key, (workspace.clone(), snapshot.sha.clone()));
                Ok(Some(Note::new(
                    CHECKPOINT_NOTE,
                    json!({
                        "execution": key.execution,
                        "firing": key.firing,
                        "attempt": key.attempt,
                        "workspace": workspace,
                        "git_commit_sha": snapshot.sha,
                        "reused": snapshot.reused,
                    }),
                )))
            }
            Err(error) => Err(format!(
                "the checkpoint commit of `{node}` failed: {}",
                collect_chain(&error).join(": ")
            )),
        }
    }

    /// [`snapshot`](Self::snapshot) for a workspace inside the scope's
    /// sandbox, through the environment kept at `scope_acquired`.
    async fn snapshot_in_sandbox(
        &self,
        context: &HookContext,
        scope: ScopeId,
        key: CheckpointKey,
        node: &str,
        status: &Status,
        origin: ResultOrigin,
    ) -> Result<Option<Note>, String> {
        let held = lock(&self.envs).get(&(context.execution, scope)).cloned();
        let Some((workspace, env)) = held else {
            // A skipped node or a driver-made outcome may precede the scope's
            // environment; nothing of the stage's exists to snapshot.
            if origin == ResultOrigin::Driver || matches!(status, Status::Skipped) {
                return Ok(Some(Note::new(
                    CHECKPOINT_NOTE,
                    json!({
                        "execution": key.execution,
                        "firing": key.firing,
                        "attempt": key.attempt,
                        "skipped": "the scope has no environment yet",
                    }),
                )));
            }
            return Err(format!(
                "scope {scope} of execution {} has no sandbox environment to snapshot in",
                context.execution
            ));
        };
        self.gate("commit", node).await;
        let serialized = self.workspace_lock(&workspace);
        let _held = serialized.lock().await;
        match self
            .workspaces
            .commit_in(&env, &workspace, key, node, status.tag())
            .await
        {
            Ok(snapshot) => {
                debug!(
                    run_id = %self.run_id,
                    node,
                    execution = key.execution,
                    firing = key.firing,
                    attempt = key.attempt,
                    reused = snapshot.reused,
                    "checkpoint committed in the sandbox"
                );
                lock(&self.committed).insert(key, (workspace.clone(), snapshot.sha.clone()));
                Ok(Some(Note::new(
                    CHECKPOINT_NOTE,
                    json!({
                        "execution": key.execution,
                        "firing": key.firing,
                        "attempt": key.attempt,
                        "workspace": workspace,
                        "git_commit_sha": snapshot.sha,
                        "reused": snapshot.reused,
                    }),
                )))
            }
            Err(error) => Err(format!(
                "the checkpoint commit of `{node}` in the sandbox failed: {}",
                collect_chain(&error).join(": ")
            )),
        }
    }

    /// The restore plan of a resumed run, read once: what every live
    /// sandbox workspace must be brought to at its first acquisition.
    async fn restore_targets(
        &self,
    ) -> Result<&Mutex<BTreeMap<String, RestoreTarget>>, ScopeAcquiredError> {
        self.restore
            .get_or_try_init(|| async {
                let plan = recovery::plan(
                    Arc::clone(&self.store),
                    self.records.as_ref(),
                    &self.run_id,
                    &self.workspaces,
                )
                .await
                .map_err(|error| {
                    ScopeAcquiredError::new(format!(
                        "the run's restore plan could not be read: {}",
                        collect_chain(&error).join(": ")
                    ))
                })?;
                match plan {
                    Plan::Resume { targets } => Ok(Mutex::new(targets)),
                    Plan::Start => Ok(Mutex::default()),
                    Plan::Failed { reason } => Err(ScopeAcquiredError::new(reason)),
                }
            })
            .await
    }

    /// Bring a sandbox workspace to the snapshot the resumed run's durable
    /// state names, once, at its first acquisition.
    async fn restore_sandbox(
        &self,
        workspace: &str,
        env: &Arc<dyn ExecEnv>,
    ) -> Result<(), ScopeAcquiredError> {
        let targets = self.restore_targets().await?;
        let target = lock(targets).remove(workspace);
        let Some(target) = target else {
            return Ok(());
        };
        let serialized = self.workspace_lock(workspace);
        let _held = serialized.lock().await;
        let action = recovery::bring_sandbox_to(&self.workspaces, env, workspace, &target)
            .await
            .map_err(|error| {
                ScopeAcquiredError::new(format!(
                    "the sandbox workspace `{workspace}` could not be brought to its snapshot: {}",
                    collect_chain(&error).join(": ")
                ))
            })?;
        info!(
            run_id = %self.run_id,
            workspace,
            sha = target.sha,
            action = ?action,
            "sandbox workspace brought to its durable snapshot"
        );
        Ok(())
    }

    /// The checkpoint's platform record, once per operation identity.
    async fn record(
        &self,
        context: &HookContext,
        scope: ScopeId,
        key: CheckpointKey,
    ) -> Result<(), String> {
        self.recorded_loaded
            .get_or_try_init(|| self.load_recorded())
            .await?;
        if lock(&self.recorded).contains(&key) {
            return Ok(());
        }
        let committed = lock(&self.committed).get(&key).cloned();
        let (workspace, sha) = if let Some(committed) = committed {
            committed
        } else {
            let acquired = lock(&self.envs)
                .get(&(context.execution, scope))
                .map(|(workspace, _)| workspace.clone());
            let workspace = match acquired {
                Some(workspace) => workspace,
                None => self.workspace_of(context, scope).await?,
            };
            let serialized = self.workspace_lock(&workspace);
            let held = serialized.lock().await;
            let found = self.workspaces.find(&workspace, key).await;
            drop(held);
            let sha = found
                .map_err(|error| {
                    format!(
                        "the checkpoint commit could not be looked up: {}",
                        collect_chain(&error).join(": ")
                    )
                })?
                .ok_or_else(|| {
                    format!(
                        "no checkpoint commit exists for execution {} firing {} attempt {}",
                        key.execution, key.firing, key.attempt
                    )
                })?;
            (workspace, sha)
        };
        let record = PlatformRecord::Checkpoint(CheckpointRecord {
            execution:      key.execution,
            firing:         key.firing,
            attempt:        Some(key.attempt),
            workspace:      Some(workspace),
            git_commit_sha: Some(sha),
            diff_summary:   None,
            patch_blob:     None,
            operation:      Some(key.operation()),
        });
        self.records
            .append(
                &self.run_id,
                &record,
                Some(StagePosition {
                    execution: key.execution,
                    firing:    key.firing,
                }),
            )
            .await
            .map_err(|error| {
                format!(
                    "the checkpoint record could not be written: {}",
                    collect_chain(&error).join(": ")
                )
            })?;
        lock(&self.recorded).insert(key);
        Ok(())
    }

    /// The checkpoints already recorded for the run, read once: what a
    /// resume's reissued routing decisions must not record again.
    async fn load_recorded(&self) -> Result<(), String> {
        let stored = self
            .records
            .read_kind(&self.run_id, PlatformRecordKind::Checkpoint)
            .await
            .map_err(|error| {
                format!(
                    "the run's checkpoint records could not be read: {}",
                    collect_chain(&error).join(": ")
                )
            })?;
        let mut recorded = lock(&self.recorded);
        for record in stored {
            let PlatformRecord::Checkpoint(checkpoint) = &record.record else {
                continue;
            };
            if let Some(key) = checkpoint
                .operation
                .as_ref()
                .and_then(CheckpointKey::from_operation)
            {
                recorded.insert(key);
            }
        }
        Ok(())
    }

    /// Hold at a test gate when one is set for this point and node.
    async fn gate(&self, point: &str, node: &str) {
        let Some(dir) = &self.test_gates else {
            return;
        };
        let hold = dir.join(format!("{point}.{node}.hold"));
        if !fs::try_exists(&hold).await.unwrap_or(false) {
            return;
        }
        let release = dir.join(format!("{point}.{node}.release"));
        info!(point, node, "checkpoint held at a test gate");
        while !fs::try_exists(&release).await.unwrap_or(false) {
            time::sleep(GATE_POLL).await;
        }
        info!(point, node, "checkpoint released by its test gate");
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn is_checkpoint_failure(status: &Status) -> bool {
    matches!(status, Status::Failure(info) if info.class.as_str() == CHECKPOINT_FAILED_CLASS)
}

#[async_trait::async_trait]
impl ExecutionHooks for FabroHooks {
    async fn before_attempt(
        &self,
        context: &HookContext,
        request: AdmitAttempt,
    ) -> AttemptDecision {
        self.inner.before_attempt(context, request).await
    }

    async fn prepare_result(
        &self,
        context: &HookContext,
        request: PrepareResult,
    ) -> Result<Prepared, PrepareError> {
        let node = request.view.node_name().to_owned();
        let scope = request.view.scope;
        let key = CheckpointKey {
            execution: context.execution.raw(),
            firing:    request.view.firing.raw(),
            attempt:   request.view.attempt.raw(),
        };
        let original = request.outcome.status.clone();
        let origin = request.origin;
        let mut prepared = self.inner.prepare_result(context, request).await?;
        let effective = prepared.adjustment.status.clone().unwrap_or(original);
        if matches!(effective, Status::Cancelled) {
            return Ok(prepared);
        }
        match self
            .snapshot(context, scope, key, &node, &effective, origin)
            .await
        {
            Ok(Some(note)) => prepared.notes.push(note),
            Ok(None) => {}
            Err(message) => {
                warn!(
                    run_id = %self.run_id,
                    node,
                    execution = key.execution,
                    firing = key.firing,
                    attempt = key.attempt,
                    error = %message,
                    "checkpoint failed; the run ends"
                );
                self.fail_run(&message);
                prepared.adjustment.status = Some(Status::Failure(
                    FailureInfo::new(message.clone()).with_class(CHECKPOINT_FAILED_CLASS),
                ));
                prepared.adjustment.reason = Some(message);
            }
        }
        Ok(prepared)
    }

    async fn after_record(&self, context: &HookContext, recorded: Recorded) -> Vec<Note> {
        self.inner.after_record(context, recorded).await
    }

    async fn transition(
        &self,
        context: &HookContext,
        transition: Transition,
    ) -> Result<TransitionReport, TransitionError> {
        if is_checkpoint_failure(&transition.outcome.status) {
            return Err(TransitionError::new(
                "the stage's checkpoint commit failed; no route is taken",
            ));
        }
        let node = transition.view.node_name().to_owned();
        let scope = transition.view.scope;
        let key = CheckpointKey {
            execution: context.execution.raw(),
            firing:    transition.view.firing.raw(),
            attempt:   transition.view.attempt.raw(),
        };
        let mut problems = Vec::new();
        self.gate("record", &node).await;
        if let Err(problem) = self.record(context, scope, key).await {
            warn!(
                run_id = %self.run_id,
                node,
                execution = key.execution,
                firing = key.firing,
                error = %problem,
                "the checkpoint record was not written"
            );
            problems.push(problem);
        }
        let mut report = self.inner.transition(context, transition).await?;
        report.problems.extend(problems);
        Ok(report)
    }

    async fn run_finished(&self, context: &HookContext, finished: RunFinished) -> Vec<Note> {
        info!(
            run_id = %self.run_id,
            status = ?finished.status,
            failure = finished.failure.as_deref().unwrap_or(""),
            "Petri run finished; running the run-end hooks"
        );
        self.inner.run_finished(context, finished).await
    }

    async fn scope_released(&self, context: &HookContext, released: ScopeReleased) -> Vec<Note> {
        debug!(
            run_id = %self.run_id,
            scope = %released.scope,
            outcome = ?released.outcome,
            "scope released; running the sandbox cleanup hooks"
        );
        let scope = released.scope;
        let notes = self.inner.scope_released(context, released).await;
        lock(&self.envs).remove(&(context.execution, scope));
        notes
    }

    async fn scope_acquired(
        &self,
        context: &HookContext,
        acquired: ScopeAcquired,
    ) -> Result<(), ScopeAcquiredError> {
        self.inner.scope_acquired(context, acquired.clone()).await?;
        if self.host_workspaces {
            return Ok(());
        }
        let workspace = acquired.workspace.as_str().to_owned();
        lock(&self.envs).insert(
            (context.execution, acquired.scope),
            (workspace.clone(), Arc::clone(&acquired.env)),
        );
        if !self.resumed {
            return Ok(());
        }
        self.restore_sandbox(&workspace, &acquired.env).await
    }
}
