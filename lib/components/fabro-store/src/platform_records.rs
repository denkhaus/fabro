//! Fabro's own facts about a Petri run: the platform records.
//!
//! Petri's records are a Petri run's source of truth for everything the
//! engine did. What Fabro itself does for a run (its lifecycle before and
//! after the engine, a checkpoint commit, a pull request, a notification, a
//! pairing) is not a Petri record. Those facts live here, in the
//! `platform_records` table, one row per fact, keyed by `(run_id, seq)`
//! with `seq` per run assigned by the store, and tied to a Petri stage
//! through `(execution, firing)` when they belong to one.
//!
//! [`PlatformRecord`] is the one enum of record kinds, each with its typed
//! payload, tagged by `kind` on the wire; [`PlatformRecordKind`] names the
//! kinds. The writer of a record is whoever performs the effect: the
//! `run.created` and `run.lifecycle` kinds by the run's create and lifecycle
//! paths (the server at create, the worker around the engine), through
//! [`PlatformRecordStore::append`] or the worker's client. The
//! `run.branch`, `git.identity`, `checkpoint`, `artifact.collected`,
//! `run.diff`, `pull_request.created`, `notification.sent` and `run.paired`
//! kinds are defined here and written by the adapters that perform those
//! effects.
//!
//! Every record may carry an [`OperationKey`]: the identity of the external
//! effect it records (the execution, the Petri decision and the effect
//! kind), so the record and the effect share one identity and a retry after
//! a crash finds the effect already done.

use std::collections::BTreeMap;
use std::str::FromStr as _;
use std::sync::Arc;

use fabro_types::{
    AskFabroCapability, BlobHash, DiffSummary, GitIdentity, OrchestrationField,
    OrchestrationVerdict, PairId, PairTarget, Principal, PullRequestCreationId, PullRequestLink,
    RunControlAction, RunId, RunNoticeLevel, RunSpec, RunStatus, SandboxProviderKind, SessionId,
};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnection, SqliteRow};
use sqlx::{Row as _, Sqlite, SqlitePool};
use strum::{Display, EnumString, IntoStaticStr, VariantArray};

use crate::{Error, Result};

/// What the run summary store calls after it commits a platform record for
/// a run: the server's wake-up for the run's projector.
pub type PlatformRecordHook = Arc<dyn Fn(RunId) + Send + Sync>;

/// The Petri stage a platform record belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StagePosition {
    pub execution: u64,
    pub firing:    u64,
}

/// A Petri decision, as the engine's `DecisionId` names it on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionRef {
    ExecutionStart,
    AttemptStart { firing: u64, attempt: u32 },
    Route { firing: u64, attempt: u32 },
}

/// The identity of one external effect Fabro performed for a run: the
/// execution, the Petri decision it was performed under, and the effect
/// kind (`commit`, `push`, `pull_request`, `child_run`, ...). The run key is
/// the record's run. An effect is performed at most once per key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationKey {
    pub execution: u64,
    pub decision:  DecisionRef,
    pub effect:    String,
}

/// The kinds of platform record, as their `kind` tags spell them.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    IntoStaticStr,
    VariantArray,
)]
pub enum PlatformRecordKind {
    #[serde(rename = "run.created")]
    #[strum(serialize = "run.created")]
    RunCreated,
    #[serde(rename = "run.lifecycle")]
    #[strum(serialize = "run.lifecycle")]
    RunLifecycle,
    #[serde(rename = "run.title")]
    #[strum(serialize = "run.title")]
    RunTitle,
    #[serde(rename = "run.parent")]
    #[strum(serialize = "run.parent")]
    RunParent,
    #[serde(rename = "run.archived")]
    #[strum(serialize = "run.archived")]
    RunArchived,
    #[serde(rename = "run.unarchived")]
    #[strum(serialize = "run.unarchived")]
    RunUnarchived,
    #[serde(rename = "run.superseded")]
    #[strum(serialize = "run.superseded")]
    RunSuperseded,
    #[serde(rename = "run.notice")]
    #[strum(serialize = "run.notice")]
    RunNotice,
    #[serde(rename = "interview.answered")]
    #[strum(serialize = "interview.answered")]
    InterviewAnswered,
    #[serde(rename = "run.branch")]
    #[strum(serialize = "run.branch")]
    RunBranch,
    #[serde(rename = "git.identity")]
    #[strum(serialize = "git.identity")]
    GitIdentity,
    #[serde(rename = "checkpoint")]
    #[strum(serialize = "checkpoint")]
    Checkpoint,
    #[serde(rename = "artifact.collected")]
    #[strum(serialize = "artifact.collected")]
    ArtifactCollected,
    #[serde(rename = "run.diff")]
    #[strum(serialize = "run.diff")]
    RunDiff,
    #[serde(rename = "pull_request.requested")]
    #[strum(serialize = "pull_request.requested")]
    PullRequestRequested,
    #[serde(rename = "pull_request.created")]
    #[strum(serialize = "pull_request.created")]
    PullRequestCreated,
    #[serde(rename = "pull_request.failed")]
    #[strum(serialize = "pull_request.failed")]
    PullRequestFailed,
    #[serde(rename = "pull_request.linked")]
    #[strum(serialize = "pull_request.linked")]
    PullRequestLinked,
    #[serde(rename = "pull_request.unlinked")]
    #[strum(serialize = "pull_request.unlinked")]
    PullRequestUnlinked,
    #[serde(rename = "pull_request.merged")]
    #[strum(serialize = "pull_request.merged")]
    PullRequestMerged,
    #[serde(rename = "pull_request.closed")]
    #[strum(serialize = "pull_request.closed")]
    PullRequestClosed,
    #[serde(rename = "notification.sent")]
    #[strum(serialize = "notification.sent")]
    NotificationSent,
    #[serde(rename = "run.paired")]
    #[strum(serialize = "run.paired")]
    RunPaired,
    #[serde(rename = "run.branch_published")]
    #[strum(serialize = "run.branch_published")]
    RunBranchPublished,
    #[serde(rename = "run.branch_pushed")]
    #[strum(serialize = "run.branch_pushed")]
    RunBranchPushed,
    #[serde(rename = "work.claimed")]
    #[strum(serialize = "work.claimed")]
    WorkClaimed,
    #[serde(rename = "work.released")]
    #[strum(serialize = "work.released")]
    WorkReleased,
    #[serde(rename = "work.landed")]
    #[strum(serialize = "work.landed")]
    WorkLanded,
    #[serde(rename = "observation.recorded")]
    #[strum(serialize = "observation.recorded")]
    ObservationRecorded,
    #[serde(rename = "run.orchestration")]
    #[strum(serialize = "run.orchestration")]
    RunOrchestration,
    #[serde(rename = "sandbox.reaped")]
    #[strum(serialize = "sandbox.reaped")]
    SandboxReaped,
    #[serde(rename = "session.capability")]
    #[strum(serialize = "session.capability")]
    SessionCapability,
}

/// One platform record, tagged by `kind` on the wire.
#[allow(
    clippy::large_enum_variant,
    reason = "the created record carries the run spec, as the run's first event does"
)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum PlatformRecord {
    /// The run exists: the spec Fabro built for it.
    #[serde(rename = "run.created")]
    RunCreated(RunCreatedRecord),
    /// A lifecycle transition Fabro decided before, beside or after the
    /// engine: the queue, approval, a control request, the terminal status
    /// Fabro reports.
    #[serde(rename = "run.lifecycle")]
    RunLifecycle(RunLifecycleRecord),
    #[serde(rename = "run.title")]
    RunTitle(RunTitleRecord),
    #[serde(rename = "run.parent")]
    RunParent(RunParentRecord),
    #[serde(rename = "run.archived")]
    RunArchived,
    #[serde(rename = "run.unarchived")]
    RunUnarchived,
    #[serde(rename = "run.superseded")]
    RunSuperseded(RunSupersededRecord),
    #[serde(rename = "run.notice")]
    RunNotice(RunNoticeRecord),
    /// Who answered a question, beside the answer Petri recorded.
    #[serde(rename = "interview.answered")]
    InterviewAnswered(InterviewAnsweredRecord),
    /// The run branch and base commit Fabro created for the run.
    #[serde(rename = "run.branch")]
    RunBranch(RunBranchRecord),
    #[serde(rename = "git.identity")]
    GitIdentity(GitIdentityRecord),
    /// A stage's files committed on the run branch: the position-to-snapshot
    /// record, written after the commit succeeds.
    #[serde(rename = "checkpoint")]
    Checkpoint(CheckpointRecord),
    /// A file a stage's attempt left in its workspace, collected under
    /// `[run.artifacts] include` into the blob table.
    #[serde(rename = "artifact.collected")]
    ArtifactCollected(ArtifactCollectedRecord),
    /// The run's whole diff, its run branch against its base commit, written
    /// when the run finishes.
    #[serde(rename = "run.diff")]
    RunDiff(RunDiffRecord),
    /// The platform published the run's branch: pushed the run's final
    /// commit from its snapshot repository to the origin, or recorded why
    /// it did not (fabro-ac40).
    #[serde(rename = "run.branch_published")]
    RunBranchPublished(RunBranchPublishedRecord),
    /// The merger resolve door pushed a resolution commit onto the run's
    /// own branch (ADR-0047): the run-stream audit of the push — one
    /// record per push, naming the gear run that drove the door.
    #[serde(rename = "run.branch_pushed")]
    RunBranchPushed(RunBranchPushedRecord),
    /// The run's claim on one work item was granted (ADR-0032).
    #[serde(rename = "work.claimed")]
    WorkClaimed(WorkClaimedRecord),
    /// The run's claim on one work item ended (released or landed).
    #[serde(rename = "work.released")]
    WorkReleased(WorkReleasedRecord),
    /// The run's claim on one work item LANDED — merge-complete, the
    /// adjudicator's close of a closable-enabled item (work#1128: land
    /// and release are distinct events; consumers match the kind, never
    /// a reason string).
    #[serde(rename = "work.landed")]
    WorkLanded(WorkLandedRecord),
    /// A pull request was asked for: the supervisor creates it.
    #[serde(rename = "pull_request.requested")]
    PullRequestRequested(PullRequestRequestedRecord),
    #[serde(rename = "pull_request.created")]
    PullRequestCreated(PullRequestCreatedRecord),
    /// The requested pull request could not be created.
    #[serde(rename = "pull_request.failed")]
    PullRequestFailed(PullRequestFailedRecord),
    /// An existing pull request was linked to the run by hand.
    #[serde(rename = "pull_request.linked")]
    PullRequestLinked(PullRequestLinkedRecord),
    #[serde(rename = "pull_request.unlinked")]
    PullRequestUnlinked(PullRequestLinkedRecord),
    /// The run's pull request merged — the merge door's squash merge, or
    /// the merged-wait's observation of an external merge. Folds the
    /// recorded `state` onto the run's pull request link
    /// (denkhaus/work#1173).
    #[serde(rename = "pull_request.merged")]
    PullRequestMerged,
    /// The run's pull request closed without merging — the close door, or
    /// a closed-unmerged wait observation. Folds the recorded `state`
    /// onto the run's pull request link (denkhaus/work#1173).
    #[serde(rename = "pull_request.closed")]
    PullRequestClosed,
    #[serde(rename = "notification.sent")]
    NotificationSent(NotificationSentRecord),
    #[serde(rename = "run.paired")]
    RunPaired(RunPairedRecord),
    /// One durable observation a stage's agent recorded through
    /// `fabro_observation_record` (ADR-0038): the platform-owned half of
    /// the transitive closure chain (work item <- observation <- run).
    /// The genus is the observation; a painpoint is one category of it.
    /// Identity is the record's `(run_id, seq)`; the stage tie is the
    /// `(execution, firing)` pair the recording session carried.
    #[serde(rename = "observation.recorded")]
    ObservationRecorded(ObservationRecordedRecord),
    /// One gear's verdict on one orchestration field of the run
    /// (ADR-0039): the adjudicator writes `merge`, the merger writes
    /// `closable` — one writer per field. The projection folds the
    /// LATEST record per field, so a later verdict transitions the run
    /// (parked → enabled); absent records mean `pending`.
    #[serde(rename = "run.orchestration")]
    RunOrchestration(RunOrchestrationRecord),
    /// The retention arm reaped the run's sandbox (work#1299 disk clock,
    /// #168's decided design): the sandbox resources are gone through
    /// Petri's lease ledger while every run record stays. The projection
    /// folds the instance `retained: false` — the cursor the reap
    /// candidates read.
    #[serde(rename = "sandbox.reaped")]
    SandboxReaped(SandboxReapedRecord),
    /// An Ask Fabro session's tool capability changed (ADR-0046): an
    /// authenticated principal flipped the session between `read_only`
    /// and `full_access`. This record is the run-stream audit of the
    /// flip; the per-session posture itself is the session's own
    /// `run.session.capability.changed` event.
    #[serde(rename = "session.capability")]
    SessionCapability(SessionCapabilityRecord),
}

impl PlatformRecord {
    #[must_use]
    pub fn kind(&self) -> PlatformRecordKind {
        match self {
            Self::RunCreated(_) => PlatformRecordKind::RunCreated,
            Self::RunLifecycle(_) => PlatformRecordKind::RunLifecycle,
            Self::RunTitle(_) => PlatformRecordKind::RunTitle,
            Self::RunParent(_) => PlatformRecordKind::RunParent,
            Self::RunArchived => PlatformRecordKind::RunArchived,
            Self::RunUnarchived => PlatformRecordKind::RunUnarchived,
            Self::RunSuperseded(_) => PlatformRecordKind::RunSuperseded,
            Self::RunNotice(_) => PlatformRecordKind::RunNotice,
            Self::InterviewAnswered(_) => PlatformRecordKind::InterviewAnswered,
            Self::RunBranch(_) => PlatformRecordKind::RunBranch,
            Self::GitIdentity(_) => PlatformRecordKind::GitIdentity,
            Self::Checkpoint(_) => PlatformRecordKind::Checkpoint,
            Self::ArtifactCollected(_) => PlatformRecordKind::ArtifactCollected,
            Self::RunDiff(_) => PlatformRecordKind::RunDiff,
            Self::PullRequestRequested(_) => PlatformRecordKind::PullRequestRequested,
            Self::PullRequestCreated(_) => PlatformRecordKind::PullRequestCreated,
            Self::PullRequestFailed(_) => PlatformRecordKind::PullRequestFailed,
            Self::PullRequestLinked(_) => PlatformRecordKind::PullRequestLinked,
            Self::PullRequestUnlinked(_) => PlatformRecordKind::PullRequestUnlinked,
            Self::PullRequestMerged => PlatformRecordKind::PullRequestMerged,
            Self::PullRequestClosed => PlatformRecordKind::PullRequestClosed,
            Self::NotificationSent(_) => PlatformRecordKind::NotificationSent,
            Self::RunPaired(_) => PlatformRecordKind::RunPaired,
            Self::RunBranchPublished(_) => PlatformRecordKind::RunBranchPublished,
            Self::RunBranchPushed(_) => PlatformRecordKind::RunBranchPushed,
            Self::WorkClaimed(_) => PlatformRecordKind::WorkClaimed,
            Self::WorkReleased(_) => PlatformRecordKind::WorkReleased,
            Self::WorkLanded(_) => PlatformRecordKind::WorkLanded,
            Self::ObservationRecorded(_) => PlatformRecordKind::ObservationRecorded,
            Self::SessionCapability(_) => PlatformRecordKind::SessionCapability,
            Self::RunOrchestration(_) => PlatformRecordKind::RunOrchestration,
            Self::SandboxReaped(_) => PlatformRecordKind::SandboxReaped,
        }
    }

    /// The operation identity the record carries, when it records an
    /// external effect.
    #[must_use]
    pub fn operation(&self) -> Option<&OperationKey> {
        match self {
            Self::Checkpoint(record) => record.operation.as_ref(),
            Self::ArtifactCollected(record) => record.operation.as_ref(),
            Self::PullRequestCreated(record) => record.operation.as_ref(),
            Self::NotificationSent(record) => record.operation.as_ref(),
            Self::RunCreated(_)
            | Self::RunLifecycle(_)
            | Self::RunTitle(_)
            | Self::RunParent(_)
            | Self::RunArchived
            | Self::RunUnarchived
            | Self::RunSuperseded(_)
            | Self::RunNotice(_)
            | Self::InterviewAnswered(_)
            | Self::RunBranch(_)
            | Self::GitIdentity(_)
            | Self::RunDiff(_)
            | Self::PullRequestRequested(_)
            | Self::PullRequestFailed(_)
            | Self::PullRequestLinked(_)
            | Self::PullRequestUnlinked(_)
            | Self::PullRequestMerged
            | Self::PullRequestClosed
            | Self::RunPaired(_)
            | Self::RunBranchPublished(_)
            | Self::RunBranchPushed(_)
            | Self::WorkClaimed(_)
            | Self::WorkReleased(_)
            | Self::WorkLanded(_)
            | Self::ObservationRecorded(_)
            | Self::RunOrchestration(_)
            | Self::SandboxReaped(_)
            | Self::SessionCapability(_) => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunCreatedRecord {
    pub spec:         RunSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title:        Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id:    Option<RunId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retried_from: Option<RunId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_url:      Option<String>,
}

/// Which lifecycle transition a `run.lifecycle` record is.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    IntoStaticStr,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum RunLifecycleKind {
    Submitted,
    StartRequested,
    Pending,
    Approved,
    Denied,
    Runnable,
    Starting,
    Running,
    Blocked,
    Unblocked,
    Paused,
    Unpaused,
    Removing,
    Succeeded,
    Failed,
    Dead,
    CancelRequested,
    PauseRequested,
    UnpauseRequested,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunLifecycleRecord {
    /// Which transition this is. Named apart from the record's `kind` tag.
    pub transition: RunLifecycleKind,
    /// The status the transition leads to, for a transition that is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status:     Option<RunStatus>,
    /// Why: a denial's reason, a failure's message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason:     Option<String>,
    /// What made the run runnable, or whether a start request is a resume.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source:     Option<String>,
    /// The control a `*_requested` transition asks for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action:     Option<RunControlAction>,
}

impl RunLifecycleRecord {
    #[must_use]
    pub fn new(transition: RunLifecycleKind) -> Self {
        Self {
            transition,
            status: None,
            reason: None,
            source: None,
            action: None,
        }
    }

    #[must_use]
    pub fn with_status(mut self, status: RunStatus) -> Self {
        self.status = Some(status);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunTitleRecord {
    pub title: String,
}

/// A granted work-item claim (ADR-0032): which item this run now owns.
/// The claim ledger in `fabro-work` is the authority; this record is the
/// run-stream audit entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkClaimedRecord {
    pub claim_id:   String,
    pub project_id: String,
    pub item_key:   String,
    pub lane:       String,
}

/// An ended work-item claim (released or landed); `reason` names why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkReleasedRecord {
    pub claim_id:   String,
    pub project_id: String,
    pub item_key:   String,
    pub reason:     String,
}

/// A LANDED work-item claim (work#1128): merge-complete, the close door
/// land path. The WorkReleased shape MINUS the reason string — the kind
/// itself carries the meaning; no free text rides the typed channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkLandedRecord {
    pub claim_id:   String,
    pub project_id: String,
    pub item_key:   String,
}

/// One observation entry a stage's agent recorded through the
/// `fabro_observation_record` tool (ADR-0038). The entry is append-only
/// and immutable; its identity is the store's `(run_id, seq)` pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationRecordedRecord {
    /// Whether this entry is a painpoint (a friction worth acting on) or
    /// a note (worth keeping, no action demand).
    pub category:  fabro_types::ObservationCategory,
    /// The entry as the agent wrote it, trimmed and non-blank.
    pub text:      String,
    /// The Petri execution the recording stage ran under.
    pub execution: u64,
    /// The Petri firing the recording stage was.
    pub firing:    u64,
}

/// One verdict on one orchestration field (ADR-0039): which field the
/// verdict names, the verdict itself, and the run that wrote it (the
/// adjudicator or merger workflow run; absent for a server-side or
/// operator write). Identity is the record's `(run_id, seq)`;
/// `seq` order decides which verdict is current — the projection folds
/// the latest per field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunOrchestrationRecord {
    /// Which field this verdict applies to: `merge` (the adjudicator's)
    /// or `closable` (the merger's).
    pub field:   OrchestrationField,
    /// The verdict. No `pending` variant: pending is the absence of a
    /// record for the field.
    pub verdict: OrchestrationVerdict,
    /// The gear run that wrote the verdict, when a workflow run did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by_run:  Option<RunId>,
}

/// Why the retention arm reaped a sandbox: the terminal-age floor or the
/// disk brake (work#1299; `strum` per the repo's enum rule).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, strum::Display, strum::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum SandboxReapReason {
    /// Past `terminal_sandbox_age_hours` after the run went terminal.
    TerminalAge,
    /// The disk brake widened the reap window below the age floor.
    DiskBrake,
}

/// The retention arm reaped this run's sandbox (work#1299 disk clock):
/// containers, volumes, and the sandbox's host workspace are gone through
/// Petri's lease ledger; every run record stays. `provider_id` keeps the
/// deleted resource addressable for post-mortems.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxReapedRecord {
    /// The provider the sandbox lived on.
    pub provider:    SandboxProviderKind,
    /// The provider's own id of the reaped resource.
    pub provider_id: String,
    /// Why the arm took it: the age floor or the disk brake.
    pub reason:      SandboxReapReason,
}

/// An Ask Fabro session's tool capability changed (ADR-0046). Identity is
/// the record's `(run_id, seq)`; the session the flip belongs to is named
/// by `session_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionCapabilityRecord {
    /// The Ask Fabro session whose capability changed.
    pub session_id: SessionId,
    /// The posture the session moved to.
    pub capability: AskFabroCapability,
    /// Who flipped the session, as the principal's login.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor:      Option<String>,
}

/// How long one observation entry may be — enforced at the append
/// boundary (the raw record endpoint) so the tool-side cap (fabro-tool)
/// cannot be bypassed by a worker posting a record directly. Entries
/// are one durable thought, not transcripts; the refusal names the
/// limit and the actual length so one trim is enough (#1075
/// ergonomics, applied to the platform surface).
pub const MAX_OBSERVATION_TEXT_CHARS: usize = 2000;

/// The derived triage state of one observation record, MATERIALIZED
/// beside the record (work#1385): the write doors (record append, link,
/// unlink, decision) re-derive and store it, and the global feed's
/// triage filter matches this column in SQL. The vocabulary matches the
/// wire's `triage_state` enum; the derivation rules live with the write
/// doors (the server layer holds both stores the derivation reads).
/// The tracker-side-change corner is by design: an item closing in the
/// tracker outside the write doors keeps its materialized state until a
/// write touches the record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ObservationTriageState {
    /// No linked item and no decision.
    Untriaged,
    /// Linked, no live item state contradicts it.
    Filed,
    /// Linked, an item carries a live claim.
    InProgress,
    /// Linked, every item closed.
    Closed,
    /// A `no_filing` decision and no link — the triager judged no item
    /// was warranted. A later filing overrides it.
    Reviewed,
    /// A `filing_blocked` decision and no link (work#1585) — the
    /// triager's filing write FAILED (rate limited, 4xx) and the record
    /// parked for the decision's bounded TTL. Past the TTL the
    /// derivation (the server layer owns the clock) re-derives
    /// `untriaged`, so the record re-serves; the materialized column
    /// keeps the parked state until a write touches the record.
    FilingBlocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunParentRecord {
    /// The parent after the change; absent when the link was removed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id:          Option<RunId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_parent_id: Option<RunId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSupersededRecord {
    pub new_run_id:                RunId,
    pub target_checkpoint_ordinal: usize,
    pub target_node_id:            String,
    pub target_visit:              usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunNoticeRecord {
    pub level:   RunNoticeLevel,
    pub code:    String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InterviewAnsweredRecord {
    /// The question's id, as Petri's `parsed.question` names it.
    pub question:  String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal: Option<Principal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel:   Option<String>,
    /// The question's text, for a reader that shows the answer beside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text:      Option<String>,
    /// The answer as the person gave it, rendered as text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer:    Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunBranchRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_sha:   Option<String>,
    /// The Petri workspace the branch was created in: where the run's diff
    /// is measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace:  Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitIdentityRecord {
    #[serde(flatten)]
    pub identity: GitIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointRecord {
    pub execution:      u64,
    pub firing:         u64,
    /// The attempt whose files the commit holds; absent on a record written
    /// before the field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt:        Option<u32>,
    /// The Petri workspace id the commit was made in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace:      Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_commit_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_summary:   Option<DiffSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch_blob:     Option<BlobHash>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation:      Option<OperationKey>,
}

/// One file collected from a stage's workspace after its attempt finished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactCollectedRecord {
    pub execution: u64,
    pub firing:    u64,
    /// The attempt whose workspace the file was read from, 1-based.
    pub attempt:   u32,
    /// The file's path relative to the workspace root.
    pub path:      String,
    /// The blob that holds the file's bytes.
    pub blob:      BlobHash,
    pub bytes:     u64,
    /// The SHA-256 of the bytes as lowercase hex: with `path`, the identity
    /// a later capture of the same unchanged file is matched by.
    pub digest:    String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<OperationKey>,
}

/// The run's diff: its run branch's head against its base commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunDiffRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_sha:     Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_sha:     Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_summary: Option<DiffSummary>,
    /// The patch as a text blob; absent when the diff is empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch_blob:   Option<BlobHash>,
}

/// How the platform's publish of a run's branch ended (fabro-ac40): the
/// final commit pushed to the origin, or the reason it was not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RunBranchPublishOutcome {
    /// The final commit is on the origin under the run branch.
    Published,
    /// Publishing does not apply to this run: push disabled, nothing to
    /// push, or no repository to push to. Terminal.
    Skipped {
        /// Machine-readable skip code (`run_branch_push_disabled`, ...).
        reason: String,
    },
    /// Publishing was attempted and failed terminally after its bounded
    /// retries; the error is the last attempt's.
    Failed { error: String },
}

/// The platform's publish of one run's branch (fabro-ac40): written once
/// per run when it reaches terminal success, after the push from the
/// run's snapshot repository to the origin succeeded, was skipped, or
/// failed terminally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunBranchPublishedRecord {
    /// The run branch the publish targets (`fabro/run/<id>`).
    pub run_branch: String,
    /// The final commit the publish pushes.
    pub head_sha:   String,
    pub outcome:    RunBranchPublishOutcome,
}

/// One push through the merger resolve door (ADR-0047): the server
/// landed a resolution commit on the run's OWN branch, with the run's
/// server-side credentials — the run-stream audit of the push, the
/// capability-class record for the door (ADR-0019's second instance).
/// Identity is the record's `(run_id, seq)` on the TARGET run's stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunBranchPushedRecord {
    /// The branch the door pushed — always the run's own
    /// `fabro/run/<id>`; the door refuses every other ref.
    pub run_branch: String,
    /// The resolution commit the door landed as the branch head.
    pub head_sha:   String,
    /// The gear run that drove the door (the merger run; the worker
    /// principal's own run).
    pub by_run:     RunId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestCreatedRecord {
    pub number:    u64,
    pub owner:     String,
    pub repo:      String,
    pub html_url:  String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_sha:  Option<String>,
    #[serde(default)]
    pub draft:     bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<OperationKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestRequestedRecord {
    pub creation_id: PullRequestCreationId,
    pub model:       String,
    #[serde(default)]
    pub force:       bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestFailedRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creation_id: Option<PullRequestCreationId>,
    pub error:       String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestLinkedRecord {
    pub owner:  String,
    pub repo:   String,
    pub number: u64,
}

impl PullRequestLinkedRecord {
    #[must_use]
    pub fn link(&self) -> PullRequestLink {
        PullRequestLink {
            owner:  self.owner.clone(),
            repo:   self.repo.clone(),
            number: self.number,
            state:  None,
        }
    }

    #[must_use]
    pub fn from_link(link: &PullRequestLink) -> Self {
        Self {
            owner:  link.owner.clone(),
            repo:   link.repo.clone(),
            number: link.number,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationSentRecord {
    pub route:      String,
    pub event:      String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel:    Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread:     Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question:   Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation:  Option<OperationKey>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunPairedRecord {
    pub pair_id: PairId,
    pub target:  PairTarget,
}

/// A platform record as the store holds it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredPlatformRecord {
    pub seq:         u64,
    /// Milliseconds since the Unix epoch when the record was stored.
    pub recorded_at: u64,
    pub record:      PlatformRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position:    Option<StagePosition>,
}

/// The `platform_records` table.
#[derive(Clone)]
pub struct PlatformRecordStore {
    pool: SqlitePool,
}

impl std::fmt::Debug for PlatformRecordStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformRecordStore")
            .finish_non_exhaustive()
    }
}

const SELECT_AFTER_SQL: &str = "SELECT seq, recorded_at, record_json, execution, firing FROM \
                                platform_records WHERE run_id = ? AND seq > ? ORDER BY seq";
const SELECT_KIND_SQL: &str = "SELECT seq, recorded_at, record_json, execution, firing FROM \
                               platform_records WHERE run_id = ? AND kind = ? ORDER BY seq";

impl PlatformRecordStore {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Store a record at the run's next seq, in a transaction of its own.
    pub async fn append(
        &self,
        run_id: &RunId,
        record: &PlatformRecord,
        position: Option<StagePosition>,
    ) -> Result<StoredPlatformRecord> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let stored =
            Self::append_on_connection(&mut transaction, run_id, now_ms(), record, position)
                .await?;
        transaction.commit().await?;
        Ok(stored)
    }

    /// Store a record at the run's next seq on a connection the caller
    /// holds a transaction on.
    pub async fn append_on_connection(
        connection: &mut SqliteConnection,
        run_id: &RunId,
        recorded_at: u64,
        record: &PlatformRecord,
        position: Option<StagePosition>,
    ) -> Result<StoredPlatformRecord> {
        let head: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(seq), 0) FROM platform_records WHERE run_id = ?",
        )
        .bind(run_id.to_string())
        .fetch_one(&mut *connection)
        .await?;
        let seq = u64::try_from(head).unwrap_or(0).saturating_add(1);
        let record_json = serde_json::to_string(record)?;
        sqlx::query(
            "INSERT INTO platform_records (run_id, seq, recorded_at, kind, record_json, \
             execution, firing) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(run_id.to_string())
        .bind(column(seq))
        .bind(column(recorded_at))
        .bind(record.kind().to_string())
        .bind(record_json)
        .bind(position.map(|position| column(position.execution)))
        .bind(position.map(|position| column(position.firing)))
        .execute(&mut *connection)
        .await?;
        // The record append is the first write door of the observation
        // triage read model (work#1385): the initial derivation rides the
        // same transaction so a triage-filtered feed can never miss a
        // stored record. A fresh `(run_id, seq)` identity cannot carry a
        // link or decision yet — both key on the identity and their doors
        // refuse absent records — so the initial derivation is always
        // `untriaged`. `DO NOTHING` keeps a row a door materialized for a
        // pre-linked sequence (a user link written before the record
        // existed) — it is more truthful than the default.
        if record.kind() == PlatformRecordKind::ObservationRecorded {
            sqlx::query(
                "INSERT INTO observation_triage_state (run_id, seq, triage_state, \
                 derived_at_ms) VALUES (?, ?, 'untriaged', ?) \
                 ON CONFLICT (run_id, seq) DO NOTHING",
            )
            .bind(run_id.to_string())
            .bind(column(seq))
            .bind(column(recorded_at))
            .execute(&mut *connection)
            .await?;
        }
        Ok(StoredPlatformRecord {
            seq,
            recorded_at,
            record: record.clone(),
            position,
        })
    }

    /// Every record of the run, in seq order.
    pub async fn read(&self, run_id: &RunId) -> Result<Vec<StoredPlatformRecord>> {
        self.read_after(run_id, 0).await
    }

    /// The run's records past `seq`, in seq order.
    pub async fn read_after(&self, run_id: &RunId, seq: u64) -> Result<Vec<StoredPlatformRecord>> {
        let rows = sqlx::query(SELECT_AFTER_SQL)
            .bind(run_id.to_string())
            .bind(column(seq))
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(decode_row).collect()
    }

    /// One page of one record kind ACROSS runs (the global feed's read:
    /// newest or oldest first), narrowed by an optional exact run id, a
    /// case-insensitive text containment over the stored record body,
    /// and — for records whose payload names a category — the exact
    /// serde fragment of one category (`"category":"painpoint"`). The
    /// category filter rides `record_json` the same way the text filter
    /// does; a serde rename of the field would silently widen the
    /// match, so the fragment is pinned by test. `offset` pages after
    /// the sort; the caller derives per-row state and filters beyond
    /// SQL's reach with its own over-fetch strategy. Returns the page
    /// (run id beside each record) and the total count under the SAME
    /// SQL filters (kind, run, text, category) — the pager's exact
    /// total (denkhaus/work#1296).
    ///
    /// `triage` matches the MATERIALIZED triage state
    /// ([`ObservationTriageState`], work#1385): the write doors keep the
    /// side table current, so the filter — page and total alike — rides
    /// an indexed `EXISTS` probe instead of a read-time derivation. The
    /// caller re-resolves the returned page's live states and drops rows
    /// whose live derivation drifted from the filter.
    pub async fn read_kind_global(
        &self,
        kind: PlatformRecordKind,
        newest_first: bool,
        limit: usize,
        offset: usize,
        run_id: Option<&RunId>,
        text_contains: Option<&str>,
        category: Option<&str>,
        triage: Option<ObservationTriageState>,
    ) -> Result<(Vec<(RunId, StoredPlatformRecord)>, u64)> {
        use sqlx::QueryBuilder;

        // The WHERE body the page fetch and the COUNT share: one
        // definition, so the total can never drift from the page's
        // filters (the drift class denkhaus/work#1296 closes).
        fn push_global_feed_filters(
            query: &mut QueryBuilder<Sqlite>,
            kind_tag: &str,
            run_id: Option<&RunId>,
            text_contains: Option<&str>,
            category: Option<&str>,
            triage: Option<ObservationTriageState>,
        ) {
            query.push(" WHERE kind = ").push_bind(kind_tag);
            if let Some(id) = run_id {
                query.push(" AND run_id = ").push_bind(id.to_string());
            }
            if let Some(needle) = text_contains {
                query
                    .push(" AND instr(lower(record_json), lower(")
                    .push_bind(needle.to_owned())
                    .push(")) > 0");
            }
            if let Some(category) = category {
                query
                    .push(" AND instr(record_json, ")
                    .push_bind(format!("\"category\":\"{category}\""))
                    .push(") > 0");
            }
            if let Some(triage) = triage {
                // The untriaged filter's membership is WIDENED to the
                // parked rows (work#1585): a `filing_blocked` decision
                // materializes `filing_blocked` for the park's whole
                // TTL — nothing rewrites the column until a write door
                // touches the record — so an exact match would never
                // offer an expired park to the page again. The widened
                // membership rides the SQL; the TTL judgment stays in
                // the derivation: the page's live overlay drops young
                // parks (they derive filing_blocked) and keeps
                // past-TTL rows (they derive untriaged again). The
                // queried `filing_blocked` state itself stays an exact
                // match — the park page shows young parks only.
                let matches: &[ObservationTriageState] = match &triage {
                    ObservationTriageState::Untriaged => &[
                        ObservationTriageState::Untriaged,
                        ObservationTriageState::FilingBlocked,
                    ],
                    other => std::slice::from_ref(other),
                };
                query.push(
                    " AND EXISTS (SELECT 1 FROM observation_triage_state t \
                       WHERE t.run_id = platform_records.run_id \
                       AND t.seq = platform_records.seq AND t.triage_state IN (",
                );
                for (index, state) in matches.iter().enumerate() {
                    if index > 0 {
                        query.push(", ");
                    }
                    query.push_bind(state.to_string());
                }
                query.push("))");
            }
        }

        let kind_tag: &str = kind.into();
        let order = if newest_first { "DESC" } else { "ASC" };

        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT run_id, seq, recorded_at, record_json, execution, firing FROM platform_records",
        );
        push_global_feed_filters(
            &mut query,
            kind_tag,
            run_id,
            text_contains,
            category,
            triage,
        );
        query
            .push(" ORDER BY recorded_at ")
            .push(order)
            .push(", run_id ")
            .push(order)
            .push(", seq ")
            .push(order)
            .push(" LIMIT ")
            .push_bind(i64::try_from(limit).unwrap_or(i64::MAX))
            .push(" OFFSET ")
            .push_bind(i64::try_from(offset).unwrap_or(i64::MAX));
        let rows = query.build().fetch_all(&self.pool).await?;

        let mut count = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM platform_records");
        push_global_feed_filters(
            &mut count,
            kind_tag,
            run_id,
            text_contains,
            category,
            triage,
        );
        let total: i64 = count.build_query_scalar().fetch_one(&self.pool).await?;

        let mut records = Vec::with_capacity(rows.len());
        for row in &rows {
            let raw_run_id: String = row.try_get("run_id")?;
            let stored = decode_row(row)?;
            let run =
                RunId::from_str(&raw_run_id).map_err(|_| crate::Error::InvalidStoredRunId {
                    value: raw_run_id.clone(),
                })?;
            records.push((run, stored));
        }
        Ok((records, total.unsigned_abs()))
    }

    /// The run's records of one kind, in seq order.
    pub async fn read_kind(
        &self,
        run_id: &RunId,
        kind: PlatformRecordKind,
    ) -> Result<Vec<StoredPlatformRecord>> {
        let rows = sqlx::query(SELECT_KIND_SQL)
            .bind(run_id.to_string())
            .bind(kind.to_string())
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(decode_row).collect()
    }

    /// Materialize one observation record's derived triage state
    /// (work#1385): the write doors' keyed upsert after they re-derived
    /// the state. Upsert, latest write wins — a re-derivation replaces
    /// the earlier state, exactly like the decision row's upsert beside
    /// which it lives.
    pub async fn materialize_observation_triage(
        &self,
        run_id: &RunId,
        seq: u64,
        triage: ObservationTriageState,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO observation_triage_state (run_id, seq, triage_state, derived_at_ms) \
             VALUES (?, ?, ?, ?) ON CONFLICT (run_id, seq) DO UPDATE SET \
             triage_state = excluded.triage_state, derived_at_ms = excluded.derived_at_ms",
        )
        .bind(run_id.to_string())
        .bind(column(seq))
        .bind(triage.to_string())
        .bind(column(now_ms()))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// One record's MATERIALIZED triage state, `None` before the record's
    /// first write door ran (a record that predates the read model and a
    /// backfill that has not covered it). The pins read this door to
    /// assert the write doors keep the column current.
    pub async fn observation_triage_state(
        &self,
        run_id: &RunId,
        seq: u64,
    ) -> Result<Option<ObservationTriageState>> {
        let state: Option<String> = sqlx::query_scalar(
            "SELECT triage_state FROM observation_triage_state \
                                WHERE run_id = ? AND seq = ?",
        )
        .bind(run_id.to_string())
        .bind(column(seq))
        .fetch_optional(&self.pool)
        .await?;
        state
            .map(|raw| {
                ObservationTriageState::from_str(&raw).map_err(|_| Error::InvalidStoredRecord {
                    record: "observation triage state",
                    field:  "triage_state",
                    value:  raw,
                })
            })
            .transpose()
    }

    /// The last seq stored for the run, or `None` when it has none.
    pub async fn head(&self, run_id: &RunId) -> Result<Option<u64>> {
        let head: Option<i64> =
            sqlx::query_scalar("SELECT MAX(seq) FROM platform_records WHERE run_id = ?")
                .bind(run_id.to_string())
                .fetch_one(&self.pool)
                .await?;
        Ok(head.and_then(|head| u64::try_from(head).ok()))
    }

    /// The LATEST verdict each run carries on one orchestration field
    /// (ADR-0039), ascending by run id — the substrate query the gears
    /// derive their queues from: a run carrying `Enabled` on `merge` is
    /// the merger's queue, a run ABSENT from the `merge` map is
    /// `pending` (the adjudicator's first-pass queue), and `Parked`
    /// marks an open operator escalation. Reads all
    /// `run.orchestration` records and groups per run in Rust — the
    /// field and verdict live in the record payload, and duplicating
    /// their JSON shape into SQL would drift from the serde form.
    pub async fn latest_verdicts_by_run(
        &self,
        field: OrchestrationField,
    ) -> Result<Vec<(RunId, OrchestrationVerdict)>> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT run_id, record_json FROM platform_records \
             WHERE kind = ? ORDER BY run_id, seq",
        )
        .bind(PlatformRecordKind::RunOrchestration.to_string())
        .fetch_all(&self.pool)
        .await?;
        let mut latest: BTreeMap<RunId, OrchestrationVerdict> = BTreeMap::new();
        for (run_id, record_json) in rows {
            let record: PlatformRecord = serde_json::from_str(&record_json)?;
            let PlatformRecord::RunOrchestration(record) = record else {
                continue;
            };
            if record.field == field {
                latest.insert(
                    RunId::from_str(&run_id)
                        .map_err(|_| Error::InvalidStoredRunId { value: run_id })?,
                    record.verdict,
                );
            }
        }
        Ok(latest.into_iter().collect())
    }

    /// The LATEST verdict ONE run carries on one orchestration field
    /// (work#1126): the single-run form of [`Self::latest_verdicts_by_run`]
    /// — the doors and the reconciler read one run's verdict without
    /// folding every run's records into a map first. `None` means the
    /// field is pending on that run (no verdict record exists).
    pub async fn latest_verdict_of_run(
        &self,
        field: OrchestrationField,
        run_id: &RunId,
    ) -> Result<Option<OrchestrationVerdict>> {
        Ok(self
            .latest_verdicts_by_run(field)
            .await?
            .into_iter()
            .find(|(candidate, _verdict)| candidate == run_id)
            .map(|(_run_id, verdict)| verdict))
    }
}

fn decode_row(row: &SqliteRow) -> Result<StoredPlatformRecord> {
    let seq: i64 = row.try_get("seq")?;
    let recorded_at: i64 = row.try_get("recorded_at")?;
    let record_json: String = row.try_get("record_json")?;
    let execution: Option<i64> = row.try_get("execution")?;
    let firing: Option<i64> = row.try_get("firing")?;
    let record: PlatformRecord = serde_json::from_str(&record_json)?;
    let position = match (execution, firing) {
        (Some(execution), Some(firing)) => Some(StagePosition {
            execution: u64::try_from(execution).unwrap_or(0),
            firing:    u64::try_from(firing).unwrap_or(0),
        }),
        _ => None,
    };
    Ok(StoredPlatformRecord {
        seq: u64::try_from(seq).map_err(|_| Error::InvalidStoredTimestamp {
            record: "platform record",
            field:  "seq",
            value:  seq,
        })?,
        recorded_at: u64::try_from(recorded_at).map_err(|_| Error::InvalidStoredTimestamp {
            record: "platform record",
            field:  "recorded_at",
            value:  recorded_at,
        })?,
        record,
        position,
    })
}

fn column(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Milliseconds since the Unix epoch.
#[must_use]
pub fn now_ms() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use fabro_types::{RunStatus, fixtures, test_support as types_support};
    use serde_json::json;

    use super::*;
    use crate::test_support;

    fn json(records: &[StoredPlatformRecord]) -> serde_json::Value {
        serde_json::to_value(records).expect("stored records serialize")
    }

    fn store() -> PlatformRecordStore {
        PlatformRecordStore::new(test_support::in_memory_pool_with(&[
            fabro_db::PETRI_PROJECTION_MIGRATION_SQL,
            // The triage read model's write doors run against the record
            // append (work#1385); the side table and the tables its
            // migration's backfill joins install beside the records.
            fabro_db::WORK_QUEUE_MIGRATION_SQL,
            fabro_db::WORK_PROJECT_TARGET_MIGRATION_SQL,
            fabro_db::WORK_OBSERVATION_LINKS_MIGRATION_SQL,
            fabro_db::OBSERVATION_TRIAGE_DECISIONS_MIGRATION_SQL,
            fabro_db::OBSERVATION_TRIAGE_STATE_MIGRATION_SQL,
            fabro_db::OBSERVATION_TRIAGE_FILING_BLOCKED_MIGRATION_SQL,
        ]))
    }

    fn sample(kind: PlatformRecordKind) -> PlatformRecord {
        match kind {
            PlatformRecordKind::RunCreated => PlatformRecord::RunCreated(RunCreatedRecord {
                spec:         types_support::test_run_spec(),
                title:        Some("A run".to_string()),
                parent_id:    None,
                retried_from: None,
                web_url:      None,
            }),
            PlatformRecordKind::RunLifecycle => PlatformRecord::RunLifecycle(
                RunLifecycleRecord::new(RunLifecycleKind::Running).with_status(RunStatus::Running),
            ),
            PlatformRecordKind::RunTitle => PlatformRecord::RunTitle(RunTitleRecord {
                title: "Renamed".to_string(),
            }),
            PlatformRecordKind::WorkClaimed => PlatformRecord::WorkClaimed(WorkClaimedRecord {
                claim_id:   "claim-1".to_string(),
                project_id: "fabro".to_string(),
                item_key:   "denkhaus/work#1".to_string(),
                lane:       "loop".to_string(),
            }),
            PlatformRecordKind::WorkReleased => PlatformRecord::WorkReleased(WorkReleasedRecord {
                claim_id:   "claim-1".to_string(),
                project_id: "fabro".to_string(),
                item_key:   "denkhaus/work#1".to_string(),
                reason:     "run failed".to_string(),
            }),
            PlatformRecordKind::WorkLanded => PlatformRecord::WorkLanded(WorkLandedRecord {
                claim_id:   "claim-1".to_string(),
                project_id: "fabro".to_string(),
                item_key:   "denkhaus/work#1".to_string(),
            }),
            PlatformRecordKind::RunParent => PlatformRecord::RunParent(RunParentRecord {
                parent_id:          Some(fixtures::RUN_2),
                previous_parent_id: None,
            }),
            PlatformRecordKind::RunArchived => PlatformRecord::RunArchived,
            PlatformRecordKind::RunUnarchived => PlatformRecord::RunUnarchived,
            PlatformRecordKind::RunOrchestration => {
                PlatformRecord::RunOrchestration(RunOrchestrationRecord {
                    field:   OrchestrationField::Merge,
                    verdict: OrchestrationVerdict::Enabled,
                    by_run:  Some(fixtures::RUN_2),
                })
            }
            PlatformRecordKind::SandboxReaped => {
                PlatformRecord::SandboxReaped(SandboxReapedRecord {
                    provider:    SandboxProviderKind::DOCKER,
                    provider_id: "petri-env-fixture".to_string(),
                    reason:      SandboxReapReason::TerminalAge,
                })
            }
            PlatformRecordKind::SessionCapability => {
                PlatformRecord::SessionCapability(SessionCapabilityRecord {
                    session_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV"
                        .parse()
                        .expect("a parseable session id"),
                    capability: fabro_types::AskFabroCapability::FullAccess,
                    actor:      Some("octocat".to_string()),
                })
            }
            PlatformRecordKind::ObservationRecorded => {
                PlatformRecord::ObservationRecorded(ObservationRecordedRecord {
                    category:  fabro_types::ObservationCategory::Painpoint,
                    text:      "the gate refused every push".to_string(),
                    execution: 0,
                    firing:    3,
                })
            }
            PlatformRecordKind::RunSuperseded => {
                PlatformRecord::RunSuperseded(RunSupersededRecord {
                    new_run_id:                fixtures::RUN_2,
                    target_checkpoint_ordinal: 1,
                    target_node_id:            "plan".to_string(),
                    target_visit:              1,
                })
            }
            PlatformRecordKind::RunNotice => PlatformRecord::RunNotice(RunNoticeRecord {
                level:   RunNoticeLevel::Warn,
                code:    "sandbox.slow".to_string(),
                message: "the sandbox took a while".to_string(),
            }),
            PlatformRecordKind::InterviewAnswered => {
                PlatformRecord::InterviewAnswered(InterviewAnsweredRecord {
                    question:  "q-1".to_string(),
                    principal: None,
                    channel:   Some("web".to_string()),
                    text:      None,
                    answer:    None,
                })
            }
            PlatformRecordKind::RunBranch => PlatformRecord::RunBranch(RunBranchRecord {
                run_branch: Some("fabro/run-1".to_string()),
                base_sha:   Some("abc".to_string()),
                workspace:  Some("invocation-0-scope-0".to_string()),
            }),
            PlatformRecordKind::GitIdentity => PlatformRecord::GitIdentity(GitIdentityRecord {
                identity: GitIdentity {
                    name:   "Fabro".to_string(),
                    email:  "fabro@example.com".to_string(),
                    source: fabro_types::GitIdentitySource::Default,
                },
            }),
            PlatformRecordKind::Checkpoint => PlatformRecord::Checkpoint(CheckpointRecord {
                execution:      0,
                firing:         3,
                attempt:        Some(1),
                workspace:      Some("invocation-0-scope-0".to_string()),
                git_commit_sha: Some("def".to_string()),
                diff_summary:   Some(DiffSummary {
                    files_changed: 1,
                    additions:     2,
                    deletions:     0,
                }),
                patch_blob:     None,
                operation:      Some(OperationKey {
                    execution: 0,
                    decision:  DecisionRef::Route {
                        firing:  3,
                        attempt: 1,
                    },
                    effect:    "commit".to_string(),
                }),
            }),
            PlatformRecordKind::ArtifactCollected => {
                PlatformRecord::ArtifactCollected(ArtifactCollectedRecord {
                    execution: 0,
                    firing:    3,
                    attempt:   1,
                    path:      "assets/report.txt".to_string(),
                    blob:      BlobHash::new(b"report"),
                    bytes:     6,
                    digest:    BlobHash::new(b"report").to_string(),
                    operation: Some(OperationKey {
                        execution: 0,
                        decision:  DecisionRef::AttemptStart {
                            firing:  3,
                            attempt: 1,
                        },
                        effect:    "artifact".to_string(),
                    }),
                })
            }
            PlatformRecordKind::RunDiff => PlatformRecord::RunDiff(RunDiffRecord {
                base_sha:     Some("abc".to_string()),
                head_sha:     Some("def".to_string()),
                diff_summary: Some(DiffSummary {
                    files_changed: 1,
                    additions:     2,
                    deletions:     0,
                }),
                patch_blob:   Some(BlobHash::new(b"patch")),
            }),
            PlatformRecordKind::PullRequestCreated => {
                PlatformRecord::PullRequestCreated(PullRequestCreatedRecord {
                    number:    7,
                    owner:     "acme".to_string(),
                    repo:      "widgets".to_string(),
                    html_url:  "https://github.com/acme/widgets/pull/7".to_string(),
                    head_sha:  None,
                    draft:     false,
                    operation: None,
                })
            }
            PlatformRecordKind::PullRequestRequested => {
                PlatformRecord::PullRequestRequested(PullRequestRequestedRecord {
                    creation_id: PullRequestCreationId::new(),
                    model:       "gpt-5.4".to_string(),
                    force:       false,
                })
            }
            PlatformRecordKind::PullRequestFailed => {
                PlatformRecord::PullRequestFailed(PullRequestFailedRecord {
                    creation_id: None,
                    error:       "no remote".to_string(),
                })
            }
            PlatformRecordKind::PullRequestLinked => {
                PlatformRecord::PullRequestLinked(PullRequestLinkedRecord {
                    owner:  "fabro-sh".to_string(),
                    repo:   "fabro".to_string(),
                    number: 7,
                })
            }
            PlatformRecordKind::PullRequestUnlinked => {
                PlatformRecord::PullRequestUnlinked(PullRequestLinkedRecord {
                    owner:  "fabro-sh".to_string(),
                    repo:   "fabro".to_string(),
                    number: 7,
                })
            }
            PlatformRecordKind::PullRequestMerged => PlatformRecord::PullRequestMerged,
            PlatformRecordKind::PullRequestClosed => PlatformRecord::PullRequestClosed,
            PlatformRecordKind::NotificationSent => {
                PlatformRecord::NotificationSent(NotificationSentRecord {
                    route:      "slack".to_string(),
                    event:      "run.completed".to_string(),
                    channel:    Some("#runs".to_string()),
                    thread:     None,
                    message_id: None,
                    question:   None,
                    operation:  None,
                })
            }
            PlatformRecordKind::RunPaired => PlatformRecord::RunPaired(RunPairedRecord {
                pair_id: PairId::new(),
                target:  PairTarget {
                    stage_id:   fabro_types::StageId::new("plan", 1),
                    node_label: "Plan".to_string(),
                },
            }),
            PlatformRecordKind::RunBranchPublished => {
                PlatformRecord::RunBranchPublished(RunBranchPublishedRecord {
                    run_branch: "fabro/run/01TEST".to_string(),
                    head_sha:   "abc123".to_string(),
                    outcome:    RunBranchPublishOutcome::Published,
                })
            }
            PlatformRecordKind::RunBranchPushed => {
                PlatformRecord::RunBranchPushed(RunBranchPushedRecord {
                    run_branch: "fabro/run/01TEST".to_string(),
                    head_sha:   "abc123".to_string(),
                    by_run:     fixtures::RUN_2,
                })
            }
        }
    }

    #[test]
    fn every_kind_tags_its_record_the_same_way_it_spells_itself() {
        for kind in PlatformRecordKind::VARIANTS {
            let record = sample(*kind);
            assert_eq!(record.kind(), *kind);
            let value = serde_json::to_value(&record).expect("the record serializes");
            assert_eq!(value["kind"], kind.to_string(), "{kind}");
            assert_eq!(
                serde_json::to_value(kind).expect("the kind serializes"),
                json!(kind.to_string())
            );
            let decoded: PlatformRecord =
                serde_json::from_value(value.clone()).expect("the record round-trips");
            assert_eq!(
                serde_json::to_value(&decoded).expect("the decoded record serializes"),
                value
            );
            assert_eq!(
                kind.to_string().parse::<PlatformRecordKind>().ok(),
                Some(*kind)
            );
        }
    }

    #[tokio::test]
    async fn records_get_seqs_per_run_and_read_back_in_order() {
        let store = store();
        let run = fixtures::RUN_1;
        let other = fixtures::RUN_2;
        let first = store
            .append(&run, &sample(PlatformRecordKind::RunCreated), None)
            .await
            .expect("the first record stores");
        let second = store
            .append(
                &run,
                &sample(PlatformRecordKind::Checkpoint),
                Some(StagePosition {
                    execution: 0,
                    firing:    3,
                }),
            )
            .await
            .expect("the second record stores");
        let elsewhere = store
            .append(&other, &sample(PlatformRecordKind::RunArchived), None)
            .await
            .expect("another run's record stores");
        assert_eq!((first.seq, second.seq, elsewhere.seq), (1, 2, 1));

        let stored = store.read(&run).await.expect("the run reads");
        assert_eq!(json(&stored), json(&[first, second.clone()]));
        assert_eq!(
            json(&store.read_after(&run, 1).await.expect("the tail reads")),
            json(std::slice::from_ref(&second))
        );
        assert_eq!(
            json(
                &store
                    .read_kind(&run, PlatformRecordKind::Checkpoint)
                    .await
                    .expect("the kind reads")
            ),
            json(&[second])
        );
        assert_eq!(store.head(&run).await.expect("the head reads"), Some(2));
        assert_eq!(
            store
                .head(&fixtures::RUN_3)
                .await
                .expect("an empty head reads"),
            None
        );
    }

    /// The global feed's exact total (denkhaus/work#1296): the COUNT
    /// beside the page fetch honors every SQL filter — kind, run, text
    /// AND the category serde fragment — so the pager's total never
    /// disagrees with the page it sits under. The fragment assertion
    /// doubles as the serde-drift guard: a rename of the record's
    /// category field fails here before the SQL filter silently widens.
    #[tokio::test]
    async fn read_kind_global_counts_and_filters_the_exact_total() {
        let store = store();
        let run = fixtures::RUN_1;
        let other = fixtures::RUN_2;
        let painpoint = PlatformRecord::ObservationRecorded(ObservationRecordedRecord {
            category:  fabro_types::ObservationCategory::Painpoint,
            text:      "the pager grew while paging".to_string(),
            execution: 0,
            firing:    1,
        });
        let note = PlatformRecord::ObservationRecorded(ObservationRecordedRecord {
            category:  fabro_types::ObservationCategory::Note,
            text:      "nothing durable to record".to_string(),
            execution: 0,
            firing:    1,
        });
        let encoded = serde_json::to_string(&note).expect("the record serializes");
        assert!(
            encoded.contains("\"category\":\"note\""),
            "the serde fragment the category filter greps: {encoded}"
        );
        store
            .append(&run, &painpoint, None)
            .await
            .expect("the painpoint stores");
        store
            .append(&run, &note, None)
            .await
            .expect("the note stores");
        store
            .append(&other, &painpoint.clone(), None)
            .await
            .expect("the other run's painpoint stores");
        store
            .append(&other, &sample(PlatformRecordKind::RunArchived), None)
            .await
            .expect("a non-observation record stores");

        let (entries, total) = store
            .read_kind_global(
                PlatformRecordKind::ObservationRecorded,
                false,
                10,
                0,
                None,
                None,
                None,
                None,
            )
            .await
            .expect("the unfiltered feed reads");
        assert_eq!((entries.len(), total), (3, 3));

        let (entries, total) = store
            .read_kind_global(
                PlatformRecordKind::ObservationRecorded,
                false,
                10,
                0,
                None,
                None,
                Some("note"),
                None,
            )
            .await
            .expect("the category-filtered feed reads");
        assert_eq!((entries.len(), total), (1, 1));

        let (entries, total) = store
            .read_kind_global(
                PlatformRecordKind::ObservationRecorded,
                false,
                10,
                0,
                None,
                Some("pager grew"),
                None,
                None,
            )
            .await
            .expect("the text-filtered feed reads");
        // Both painpoints share the text (the clone on the other run), so
        // the text filter matches two — page and total agree.
        assert_eq!((entries.len(), total), (2, 2));

        let (entries, total) = store
            .read_kind_global(
                PlatformRecordKind::ObservationRecorded,
                false,
                10,
                0,
                Some(&run),
                None,
                None,
                None,
            )
            .await
            .expect("the run-narrowed feed reads");
        assert_eq!((entries.len(), total), (2, 2));
    }

    /// The materialized triage read model (work#1385): the record append
    /// materializes `untriaged` in its own transaction; a write door's
    /// upsert replaces it; the triage filter matches the column and its
    /// COUNT — page and total share one WHERE, so the exact-total
    /// semantics (denkhaus/work#1296) survive as SQL semantics.
    #[tokio::test]
    async fn the_triage_filter_matches_the_materialized_state() {
        use ObservationTriageState as State;

        let store = store();
        let run = fixtures::RUN_1;
        let observation = PlatformRecord::ObservationRecorded(ObservationRecordedRecord {
            category:  fabro_types::ObservationCategory::Painpoint,
            text:      "materialized painpoint".to_string(),
            execution: 0,
            firing:    1,
        });
        let stored = store
            .append(&run, &observation, None)
            .await
            .expect("the observation stores");

        // The append's initial derivation: untriaged, in the append's
        // own transaction — no write door can interleave.
        assert_eq!(
            store
                .observation_triage_state(&run, stored.seq)
                .await
                .expect("the materialized state reads"),
            Some(State::Untriaged),
            "the append materializes the initial derivation"
        );

        // A triage-filtered read with NO materialized state (a foreign
        // run's record predating the read model) matches nothing.
        let (entries, total) = store
            .read_kind_global(
                PlatformRecordKind::ObservationRecorded,
                false,
                10,
                0,
                None,
                None,
                None,
                Some(State::Filed),
            )
            .await
            .expect("the triage-filtered feed reads");
        assert_eq!((entries.len(), total), (0, 0));

        // The write door's upsert: the derivation lands, page and count
        // move together.
        store
            .materialize_observation_triage(&run, stored.seq, State::Filed)
            .await
            .expect("the state materializes");
        let (entries, total) = store
            .read_kind_global(
                PlatformRecordKind::ObservationRecorded,
                false,
                10,
                0,
                None,
                None,
                None,
                Some(State::Filed),
            )
            .await
            .expect("the triage-filtered feed reads");
        assert_eq!((entries.len(), total), (1, 1));
        assert_eq!(entries[0].1.seq, stored.seq);

        // A page of one under three matches: the total stays exact. The
        // extra records ride their own appends (each appends its initial
        // `untriaged` row) and then their write-door upserts.
        let second = store
            .append(&run, &observation, None)
            .await
            .expect("the second observation stores");
        let third = store
            .append(&run, &observation, None)
            .await
            .expect("the third observation stores");
        store
            .materialize_observation_triage(&run, second.seq, State::Filed)
            .await
            .expect("the second record's state materializes");
        store
            .materialize_observation_triage(&run, third.seq, State::Filed)
            .await
            .expect("the third record's state materializes");
        let (entries, total) = store
            .read_kind_global(
                PlatformRecordKind::ObservationRecorded,
                false,
                1,
                0,
                None,
                None,
                None,
                Some(State::Filed),
            )
            .await
            .expect("the paged triage feed reads");
        assert_eq!(entries.len(), 1);
        assert_eq!(total, 3, "the total is the exact materialized membership");

        // The upsert is latest-wins: re-deriving replaces the state.
        store
            .materialize_observation_triage(&run, stored.seq, State::Untriaged)
            .await
            .expect("the re-derivation lands");
        assert_eq!(
            store
                .observation_triage_state(&run, stored.seq)
                .await
                .expect("the re-materialized state reads"),
            Some(State::Untriaged)
        );
    }

    /// The untriaged filter's widened membership (work#1585): the SQL
    /// matches a materialized `filing_blocked` column beside `untriaged`
    /// — the materialized park outlives the TTL (nothing rewrites it
    /// until a write door touches the record), so an exact-match filter
    /// would never offer an expired park to the page again and the
    /// painpoint would strand forever. The widening rides the SQL; the
    /// TTL judgment itself stays in the derivation — the store carries
    /// no clock. `filing_blocked` as the QUERIED state stays an exact
    /// match: the park page shows young parks only.
    #[tokio::test]
    async fn the_untriaged_filter_matches_parked_rows_too() {
        use ObservationTriageState as State;

        let store = store();
        let run = fixtures::RUN_1;
        let observation = PlatformRecord::ObservationRecorded(ObservationRecordedRecord {
            category:  fabro_types::ObservationCategory::Painpoint,
            text:      "parked and plain painpoints".to_string(),
            execution: 0,
            firing:    1,
        });
        let plain = store
            .append(&run, &observation, None)
            .await
            .expect("the plain record stores");
        let parked = store
            .append(&run, &observation, None)
            .await
            .expect("the parked record stores");
        store
            .materialize_observation_triage(&run, parked.seq, State::FilingBlocked)
            .await
            .expect("the parked state materializes — the migration's CHECK accepts it");

        // The untriaged query matches BOTH materialized states: the
        // plain record and the parked one (the widened membership).
        let (entries, total) = store
            .read_kind_global(
                PlatformRecordKind::ObservationRecorded,
                false,
                10,
                0,
                None,
                None,
                None,
                Some(State::Untriaged),
            )
            .await
            .expect("the untriaged feed reads");
        assert_eq!(
            total, 2,
            "the untriaged filter matches untriaged AND parked rows — the widened membership \
             is what lets an expired park re-enter the page"
        );
        let matched: Vec<u64> = entries.iter().map(|(_run, stored)| stored.seq).collect();
        assert!(
            matched.contains(&plain.seq) && matched.contains(&parked.seq),
            "the page carries the plain record beside the parked one: {matched:?}"
        );

        // The parked state queried directly stays an exact match: only
        // the parked row.
        let (entries, total) = store
            .read_kind_global(
                PlatformRecordKind::ObservationRecorded,
                false,
                10,
                0,
                None,
                None,
                None,
                Some(State::FilingBlocked),
            )
            .await
            .expect("the filing_blocked feed reads");
        assert_eq!((entries.len(), total), (1, 1));
        assert_eq!(entries[0].1.seq, parked.seq);

        // Other states are untouched: nothing derives filed here.
        let (_, total) = store
            .read_kind_global(
                PlatformRecordKind::ObservationRecorded,
                false,
                10,
                0,
                None,
                None,
                None,
                Some(State::Filed),
            )
            .await
            .expect("the filed feed reads");
        assert_eq!(total, 0, "the widening never leaks into other states");
    }

    /// The migration's backfill (work#1385): records that predate the
    /// read model derive ONCE from the best-effort inputs the migration
    /// can see — link presence (`filed`, the conservative default: the
    /// live item state is unknowable inside a SQL migration and the
    /// feed's live overlay owns the correction) and a `no_filing`
    /// decision (`reviewed`). Everything else lands `untriaged`.
    #[tokio::test]
    async fn the_triage_state_migration_backfills_existing_records() {
        // The upgrade order: everything EXCEPT the triage-state
        // migration installs first, the old-shape data seeds, then the
        // migration file's SQL runs.
        let pool = test_support::in_memory_pool_with(&[
            fabro_db::PETRI_PROJECTION_MIGRATION_SQL,
            fabro_db::WORK_QUEUE_MIGRATION_SQL,
            fabro_db::WORK_PROJECT_TARGET_MIGRATION_SQL,
            fabro_db::WORK_OBSERVATION_LINKS_MIGRATION_SQL,
            fabro_db::OBSERVATION_TRIAGE_DECISIONS_MIGRATION_SQL,
        ]);
        let run = fixtures::RUN_1;
        let observation = PlatformRecord::ObservationRecorded(ObservationRecordedRecord {
            category:  fabro_types::ObservationCategory::Painpoint,
            text:      "backfilled painpoint".to_string(),
            execution: 0,
            firing:    1,
        });
        let record_json = serde_json::to_string(&observation).expect("the record serializes");
        // The OLD binary's append shape — plain rows, no side table (the
        // read model's append door does not exist yet at this point in
        // the upgrade).
        for seq in 1..=3 {
            sqlx::query(
                "INSERT INTO platform_records (run_id, seq, recorded_at, kind, record_json) \
                 VALUES (?, ?, ?, 'observation.recorded', ?)",
            )
            .bind(run.to_string())
            .bind(seq)
            .bind(1000 + seq)
            .bind(&record_json)
            .execute(&pool)
            .await
            .expect("the pre-migration observation stores");
        }
        // The link and decision rows the derivation reads. The link keys
        // a project (the FK), the decision keys the record alone.
        sqlx::query(
            "INSERT INTO work_projects (project_id, display_name, tracker_provider, \
             tracker_repo, project_label, created_at_ms, updated_at_ms) \
             VALUES ('fabro', 'Fabro', 'github', 'denkhaus/work', 'project:fabro', 0, 0)",
        )
        .execute(&pool)
        .await
        .expect("the project seeds");
        sqlx::query(
            "INSERT INTO work_observation_links (project_id, item_key, run_id, seq, \
             linked_at_ms) VALUES ('fabro', 'denkhaus/work#1', ?, 1, 0)",
        )
        .bind(run.to_string())
        .execute(&pool)
        .await
        .expect("the link seeds");
        sqlx::query(
            "INSERT INTO observation_triage_decisions (run_id, seq, decision, reason, \
             decided_at_ms) VALUES (?, 2, 'no_filing', 'one-off', 0)",
        )
        .bind(run.to_string())
        .execute(&pool)
        .await
        .expect("the decision seeds");

        sqlx::raw_sql(fabro_db::OBSERVATION_TRIAGE_STATE_MIGRATION_SQL)
            .execute(&pool)
            .await
            .expect("the migration runs");

        let state_of = |seq: u64| {
            let pool = pool.clone();
            async move {
                let state: Option<String> = sqlx::query_scalar(
                    "SELECT triage_state FROM observation_triage_state WHERE run_id = ? AND seq = ?",
                )
                .bind(run.to_string())
                .bind(i64::try_from(seq).expect("seq fits"))
                .fetch_one(&pool)
                .await
                .expect("the backfilled row reads");
                state
            }
        };
        assert_eq!(
            state_of(1).await.as_deref(),
            Some("filed"),
            "a linked record materializes the conservative filed default"
        );
        assert_eq!(
            state_of(2).await.as_deref(),
            Some("reviewed"),
            "a linkless no_filing decision materializes reviewed"
        );
        assert_eq!(
            state_of(3).await.as_deref(),
            Some("untriaged"),
            "a record with neither link nor decision materializes untriaged"
        );
    }

    /// The gear substrate query (ADR-0039): `latest_verdicts_by_run`
    /// keeps the LATEST verdict per run per FIELD — a parked → enabled
    /// transition overwrites, the two fields stay independent, and a
    /// run with no verdict on a field is absent (pending needs no
    /// row). Ascending by run id so a gear's queue is deterministic.
    #[tokio::test]
    async fn latest_verdicts_by_run_keeps_the_latest_per_field() {
        use OrchestrationVerdict as Verdict;

        let store = store();
        let run_a = RunId::new();
        let run_b = RunId::new();
        let mark = |field, verdict, by_run| {
            PlatformRecord::RunOrchestration(RunOrchestrationRecord {
                field,
                verdict,
                by_run,
            })
        };
        let merge = OrchestrationField::Merge;
        let closable = OrchestrationField::Closable;

        // run_a: the escalation resolves — parked then enabled on merge.
        store
            .append(&run_a, &mark(merge, Verdict::Parked, None), None)
            .await
            .expect("the parked verdict stores");
        store
            .append(&run_a, &mark(merge, Verdict::Enabled, Some(run_b)), None)
            .await
            .expect("the enabled verdict stores");
        // run_b: merge refused, closable enabled — independent fields.
        store
            .append(&run_b, &mark(merge, Verdict::Refused, None), None)
            .await
            .expect("the refused verdict stores");
        store
            .append(&run_b, &mark(closable, Verdict::Enabled, None), None)
            .await
            .expect("the closable verdict stores");

        let merge_verdicts = store
            .latest_verdicts_by_run(merge)
            .await
            .expect("the merge verdicts read");
        let want = if run_a < run_b {
            vec![(run_a, Verdict::Enabled), (run_b, Verdict::Refused)]
        } else {
            vec![(run_b, Verdict::Refused), (run_a, Verdict::Enabled)]
        };
        assert_eq!(
            merge_verdicts, want,
            "the LATEST merge verdict wins per run"
        );

        let closable_verdicts = store
            .latest_verdicts_by_run(closable)
            .await
            .expect("the closable verdicts read");
        assert_eq!(closable_verdicts, vec![(run_b, Verdict::Enabled)]);
    }
}
