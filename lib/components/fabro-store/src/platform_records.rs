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
//! kinds. The writer of a record is whoever performs the effect. The
//! lifecycle kinds are written by the run's create and lifecycle paths, which
//! today still append Fabro's legacy run events: for a Petri run the run
//! summary store derives the platform record from the legacy event through
//! [`platform_record_for`] and stores both in the event's transaction. The
//! `checkpoint`, `pull_request.created`, `notification.sent` and
//! `run.paired` kinds are defined here and written by the adapters that
//! perform those effects.
//!
//! Every record may carry an [`OperationKey`]: the identity of the external
//! effect it records (the execution, the Petri decision and the effect
//! kind), so the record and the effect share one identity and a retry after
//! a crash finds the effect already done.

use std::collections::HashMap;
use std::sync::Arc;

use fabro_types::run_event::{
    InterviewCompletedProps, PullRequestCreatedProps, RunCreatedProps, RunFailedProps,
    RunNoticeLevel, RunPairStartedProps, RunRunnableSource, RunStartedProps, RunSupersededByProps,
};
use fabro_types::{
    BlobHash, DiffSummary, EventBody, GitIdentity, PairId, PairTarget, Principal, RunControlAction,
    RunEvent, RunId, RunSpec, RunStatus,
};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnection, SqliteRow};
use sqlx::{Row as _, SqlitePool};
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
    #[serde(rename = "pull_request.created")]
    #[strum(serialize = "pull_request.created")]
    PullRequestCreated,
    #[serde(rename = "notification.sent")]
    #[strum(serialize = "notification.sent")]
    NotificationSent,
    #[serde(rename = "run.paired")]
    #[strum(serialize = "run.paired")]
    RunPaired,
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
    #[serde(rename = "pull_request.created")]
    PullRequestCreated(PullRequestCreatedRecord),
    #[serde(rename = "notification.sent")]
    NotificationSent(NotificationSentRecord),
    #[serde(rename = "run.paired")]
    RunPaired(RunPairedRecord),
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
            Self::PullRequestCreated(_) => PlatformRecordKind::PullRequestCreated,
            Self::NotificationSent(_) => PlatformRecordKind::NotificationSent,
            Self::RunPaired(_) => PlatformRecordKind::RunPaired,
        }
    }

    /// The operation identity the record carries, when it records an
    /// external effect.
    #[must_use]
    pub fn operation(&self) -> Option<&OperationKey> {
        match self {
            Self::Checkpoint(record) => record.operation.as_ref(),
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
            | Self::RunPaired(_) => None,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunBranchRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_sha:   Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_commit_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_summary:   Option<DiffSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch_blob:     Option<BlobHash>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation:      Option<OperationKey>,
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

    /// The last seq stored for the run, or `None` when it has none.
    pub async fn head(&self, run_id: &RunId) -> Result<Option<u64>> {
        let head: Option<i64> =
            sqlx::query_scalar("SELECT MAX(seq) FROM platform_records WHERE run_id = ?")
                .bind(run_id.to_string())
                .fetch_one(&self.pool)
                .await?;
        Ok(head.and_then(|head| u64::try_from(head).ok()))
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

/// The platform record a legacy run event of a Petri run stands for, when
/// it stands for one. The lifecycle paths append legacy events until the
/// old executor is deleted; for a Petri run the store derives the platform
/// record from the event and keeps both, so the projection over Petri's
/// records reads the lifecycle from platform records alone.
#[must_use]
pub fn platform_record_for(event: &RunEvent) -> Option<PlatformRecord> {
    use RunLifecycleKind as Kind;
    let lifecycle = |kind: Kind| Some(PlatformRecord::RunLifecycle(RunLifecycleRecord::new(kind)));
    let status = |kind: Kind, status: RunStatus| {
        Some(PlatformRecord::RunLifecycle(
            RunLifecycleRecord::new(kind).with_status(status),
        ))
    };
    let control = |kind: Kind, action: RunControlAction| {
        let mut record = RunLifecycleRecord::new(kind);
        record.action = Some(action);
        Some(PlatformRecord::RunLifecycle(record))
    };
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "stage, agent and sandbox events are Petri's records for a Petri run"
    )]
    match &event.body {
        EventBody::RunCreated(props) => Some(PlatformRecord::RunCreated(run_created_record(
            event.run_id,
            props,
        ))),
        EventBody::RunSubmitted(_) => status(Kind::Submitted, RunStatus::Submitted),
        EventBody::RunStartRequested(props) => {
            let mut record = RunLifecycleRecord::new(Kind::StartRequested);
            record.source = Some(if props.resume { "resume" } else { "start" }.to_string());
            Some(PlatformRecord::RunLifecycle(record))
        }
        EventBody::RunPending(props) => status(Kind::Pending, RunStatus::Pending {
            reason: props.reason,
        }),
        EventBody::RunApproved(_) => lifecycle(Kind::Approved),
        EventBody::RunDenied(props) => {
            let mut record = RunLifecycleRecord::new(Kind::Denied);
            record.reason.clone_from(&props.reason);
            Some(PlatformRecord::RunLifecycle(record))
        }
        EventBody::RunRunnable(props) => {
            let mut record =
                RunLifecycleRecord::new(Kind::Runnable).with_status(RunStatus::Runnable);
            record.source = Some(runnable_source(props.source).to_string());
            Some(PlatformRecord::RunLifecycle(record))
        }
        EventBody::RunStarting(_) => status(Kind::Starting, RunStatus::Starting),
        EventBody::RunRunning(_) => status(Kind::Running, RunStatus::Running),
        EventBody::RunBlocked(props) => status(Kind::Blocked, RunStatus::Blocked {
            blocked_reason: props.blocked_reason,
        }),
        EventBody::RunUnblocked(_) => status(Kind::Unblocked, RunStatus::Running),
        EventBody::RunRemoving(_) => status(Kind::Removing, RunStatus::Removing),
        EventBody::RunCancelRequested(props) => control(Kind::CancelRequested, props.action),
        EventBody::RunPauseRequested(props) => control(Kind::PauseRequested, props.action),
        EventBody::RunUnpauseRequested(props) => control(Kind::UnpauseRequested, props.action),
        EventBody::RunPaused(_) => lifecycle(Kind::Paused),
        EventBody::RunUnpaused(_) => lifecycle(Kind::Unpaused),
        EventBody::RunCompleted(props) => status(Kind::Succeeded, RunStatus::Succeeded {
            reason: props.reason,
        }),
        EventBody::RunFailed(props) => Some(PlatformRecord::RunLifecycle(failed_record(props))),
        EventBody::RunSupersededBy(props) => {
            Some(PlatformRecord::RunSuperseded(superseded_record(props)))
        }
        EventBody::RunArchived(_) => Some(PlatformRecord::RunArchived),
        EventBody::RunUnarchived(_) => Some(PlatformRecord::RunUnarchived),
        EventBody::RunTitleUpdated(props) => Some(PlatformRecord::RunTitle(RunTitleRecord {
            title: props.title.clone(),
        })),
        EventBody::RunParentLinked(props) => Some(PlatformRecord::RunParent(RunParentRecord {
            parent_id:          Some(props.parent_id),
            previous_parent_id: props.previous_parent_id,
        })),
        EventBody::RunParentUnlinked(props) => Some(PlatformRecord::RunParent(RunParentRecord {
            parent_id:          None,
            previous_parent_id: Some(props.previous_parent_id),
        })),
        EventBody::RunNotice(props) => Some(PlatformRecord::RunNotice(RunNoticeRecord {
            level:   props.level,
            code:    props.code.clone(),
            message: props.message.clone(),
        })),
        EventBody::RunStarted(props) => Some(PlatformRecord::RunBranch(run_branch_record(props))),
        EventBody::GitIdentityResolved(props) => {
            Some(PlatformRecord::GitIdentity(GitIdentityRecord {
                identity: props.identity.clone(),
            }))
        }
        EventBody::PullRequestCreated(props) => Some(PlatformRecord::PullRequestCreated(
            pull_request_created_record(props),
        )),
        EventBody::RunPairStarted(props) => {
            Some(PlatformRecord::RunPaired(run_paired_record(props)))
        }
        EventBody::InterviewCompleted(props) => Some(PlatformRecord::InterviewAnswered(
            interview_answered_record(props, event.actor.clone()),
        )),
        _ => None,
    }
}

fn runnable_source(source: RunRunnableSource) -> &'static str {
    source.into()
}

fn run_created_record(run_id: RunId, props: &RunCreatedProps) -> RunCreatedRecord {
    let labels = props.labels.clone().into_iter().collect::<HashMap<_, _>>();
    RunCreatedRecord {
        spec:         RunSpec {
            run_id,
            settings: props.settings.clone(),
            graph: props.graph.clone(),
            graph_source: props.workflow_source.clone(),
            workflow_slug: props.workflow_slug.clone(),
            workflow_version_id: props.workflow_version_id,
            target: props.target.clone(),
            automation: props.automation.clone(),
            source_directory: props.source_directory.clone(),
            labels,
            provenance: props.provenance.clone(),
            definition_blob: None,
            spec_blob: props.spec_blob,
            git: props.git.clone(),
            fork_source_ref: props.fork_source_ref.clone(),
            engine: props.engine.clone(),
        },
        title:        props.title.clone(),
        parent_id:    props.parent_id,
        retried_from: props.retried_from,
        web_url:      props.web_url.clone(),
    }
}

fn failed_record(props: &RunFailedProps) -> RunLifecycleRecord {
    let mut record =
        RunLifecycleRecord::new(RunLifecycleKind::Failed).with_status(RunStatus::Failed {
            reason: props.failure.reason,
        });
    record.reason = Some(props.failure.detail.message.clone());
    record
}

fn superseded_record(props: &RunSupersededByProps) -> RunSupersededRecord {
    RunSupersededRecord {
        new_run_id:                props.new_run_id,
        target_checkpoint_ordinal: props.target_checkpoint_ordinal,
        target_node_id:            props.target_node_id.clone(),
        target_visit:              props.target_visit,
    }
}

fn run_branch_record(props: &RunStartedProps) -> RunBranchRecord {
    RunBranchRecord {
        run_branch: props.run_branch.clone(),
        base_sha:   props.base_sha.clone(),
    }
}

fn pull_request_created_record(props: &PullRequestCreatedProps) -> PullRequestCreatedRecord {
    PullRequestCreatedRecord {
        number:    props.pr_number,
        owner:     props.owner.clone(),
        repo:      props.repo.clone(),
        html_url:  props.pr_url.clone(),
        head_sha:  props.head_sha.clone(),
        draft:     props.draft,
        operation: None,
    }
}

fn run_paired_record(props: &RunPairStartedProps) -> RunPairedRecord {
    RunPairedRecord {
        pair_id: props.pair_id,
        target:  props.target.clone(),
    }
}

fn interview_answered_record(
    props: &InterviewCompletedProps,
    principal: Option<Principal>,
) -> InterviewAnsweredRecord {
    InterviewAnsweredRecord {
        question: props.question_id.clone(),
        principal,
        channel: None,
    }
}

#[cfg(test)]
mod tests {
    use fabro_types::{FailureReason, RunStatus, fixtures, test_support as types_support};
    use serde_json::json;

    use super::*;
    use crate::test_support;

    fn json(records: &[StoredPlatformRecord]) -> serde_json::Value {
        serde_json::to_value(records).expect("stored records serialize")
    }

    fn store() -> PlatformRecordStore {
        PlatformRecordStore::new(test_support::in_memory_pool_with(&[
            fabro_db::PETRI_PROJECTION_MIGRATION_SQL,
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
            PlatformRecordKind::RunParent => PlatformRecord::RunParent(RunParentRecord {
                parent_id:          Some(fixtures::RUN_2),
                previous_parent_id: None,
            }),
            PlatformRecordKind::RunArchived => PlatformRecord::RunArchived,
            PlatformRecordKind::RunUnarchived => PlatformRecord::RunUnarchived,
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
                })
            }
            PlatformRecordKind::RunBranch => PlatformRecord::RunBranch(RunBranchRecord {
                run_branch: Some("fabro/run-1".to_string()),
                base_sha:   Some("abc".to_string()),
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

    #[test]
    fn a_failed_legacy_event_becomes_a_failed_lifecycle_record_with_its_message() {
        let event = fabro_types::RunEvent {
            id:                 "evt".to_string(),
            ts:                 chrono::Utc::now(),
            run_id:             fixtures::RUN_1,
            node_id:            None,
            node_label:         None,
            stage_id:           None,
            parallel_group_id:  None,
            parallel_branch_id: None,
            session_id:         None,
            parent_session_id:  None,
            tool_call_id:       None,
            actor:              None,
            body:               EventBody::RunFailed(RunFailedProps {
                failure:              fabro_types::RunFailure {
                    reason: FailureReason::Cancelled,
                    detail: fabro_types::FailureDetail::new(
                        "stopped",
                        fabro_types::FailureCategory::Canceled,
                    ),
                },
                timing:               fabro_types::RunTiming::default(),
                final_git_commit_sha: None,
                final_patch:          None,
                diff_summary:         None,
                usage:                None,
            }),
        };
        let Some(PlatformRecord::RunLifecycle(record)) = platform_record_for(&event) else {
            panic!("a failed run maps to a lifecycle record");
        };
        assert_eq!(record.transition, RunLifecycleKind::Failed);
        assert_eq!(
            record.status,
            Some(RunStatus::Failed {
                reason: FailureReason::Cancelled,
            })
        );
        assert_eq!(record.reason.as_deref(), Some("stopped"));
    }
}
