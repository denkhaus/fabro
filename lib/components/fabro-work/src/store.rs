//! The work store: project registry, claim ledger, transition events and
//! the projection outbox (ADR-0032).
//!
//! Forge-agnostic by construction: the store knows providers only as the
//! `tracker_provider` id string on a project plus a provider-stable
//! `item_key`. Which forge serves an item is the tracker port's business
//! (`crate::tracker`); this store owns assignment: a non-terminal claim is
//! unique per `(project, item)` and per run, so two runs can never both own
//! the same item, no matter which clone they read. The claim transaction
//! commits first; the tracker projection is a durable outbox job that
//! retries.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use fabro_types::RunId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row as _, SqlitePool};

use crate::{Error, Result};

/// A project's tracker configuration. `project_id` is the run's logical
/// The claim-label prefix a project gets when its definition omits one. The
/// settings materializer, the API upsert, and the migration default all read
/// this one constant, so a project never lands with an empty prefix.
pub const DEFAULT_CLAIM_LABEL_PREFIX: &str = "fabro:run:";

/// project (not a repository name); the same tracker repository may serve
/// many projects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkProject {
    pub project_id:               String,
    pub display_name:             String,
    pub tracker_provider:         String,
    pub tracker_repo:             String,
    /// The code repository this project's runs work on (ADR-0032 phase 3:
    /// run-target project resolution). `None` keeps the project
    /// explicit-only — runs of such a project must name it.
    pub target_repo:              Option<String>,
    pub project_label:            String,
    pub lane_labels:              BTreeMap<String, String>,
    pub claim_label_prefix:       String,
    pub claim_comment_enabled:    bool,
    /// How many `awaiting_merge` claims (open run PRs) a lane tolerates
    /// before the next grant is refused (fabro-4580; 0 = fire only when
    /// drained).
    pub backpressure_open_pr_cap: i64,
}

/// The claim lifecycle. `AwaitingMerge` still holds the item: a pending
/// pull request is not available for a new run.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ClaimState {
    Granted,
    Active,
    AwaitingMerge,
    Landed,
    Released,
}

impl ClaimState {
    /// Whether the state is terminal (the item is free again).
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Landed | Self::Released)
    }

    /// Whether the state still holds the work item.
    #[must_use]
    pub fn holds_item(self) -> bool {
        !self.is_terminal()
    }

    /// The state's canonical wire/database spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        self.into()
    }

    fn from_db(value: &str) -> Result<Self> {
        value
            .parse()
            .map_err(|_| Error::InvalidClaimState(value.to_string()))
    }
}

/// One claim record. `item_key` is the provider's stable work-item identity
/// (for GitHub: `owner/repo#number`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkClaim {
    pub claim_id:       String,
    pub project_id:     String,
    pub item_key:       String,
    pub run_id:         RunId,
    pub lane:           String,
    pub state:          ClaimState,
    pub granted_at:     DateTime<Utc>,
    pub updated_at:     DateTime<Utc>,
    pub released_at:    Option<DateTime<Utc>>,
    pub release_reason: Option<String>,
    pub pr_url:         Option<String>,
}

/// One transition of a claim. `seq` starts at 1 per claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkClaimEvent {
    pub claim_id:   String,
    pub seq:        i64,
    pub at:         DateTime<Utc>,
    pub from_state: Option<ClaimState>,
    pub to_state:   ClaimState,
    pub reason:     Option<String>,
    pub run_id:     RunId,
}

/// A grant request. The caller has already selected a candidate from the
/// tracker; the ledger is the arbiter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantRequest {
    pub project_id: String,
    pub item_key:   String,
    pub run_id:     RunId,
    pub lane:       String,
}

/// The result of a grant attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantOutcome {
    /// The claim was created.
    Granted(WorkClaim),
    /// This run already holds this item (idempotent repeat of the same
    /// grant). The returned claim is the existing live claim.
    AlreadyHeld(WorkClaim),
    /// Another run holds the item, or this run already holds another item.
    /// The returned claim is the conflicting live claim.
    Conflict(WorkClaim),
}

/// A pending tracker projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkOutboxEntry {
    pub id:              String,
    pub idempotency_key: String,
    pub kind:            String,
    pub payload:         Value,
    pub attempts:        i64,
    pub next_attempt_at: DateTime<Utc>,
    pub created_at:      DateTime<Utc>,
    pub completed_at:    Option<DateTime<Utc>>,
    pub last_error:      Option<String>,
}

/// SQLite-backed project registry, claim ledger and projection outbox.
pub struct WorkStore {
    pool: SqlitePool,
}

impl std::fmt::Debug for WorkStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkStore").finish_non_exhaustive()
    }
}

impl WorkStore {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Create or update a project's tracker configuration.
    pub async fn upsert_project(&self, project: &WorkProject) -> Result<()> {
        let lane_labels = serde_json::to_string(&project.lane_labels)?;
        let now = Utc::now().timestamp_millis();
        sqlx::query(
            r"
INSERT INTO work_projects (
    project_id, display_name, tracker_provider, tracker_repo, target_repo,
    project_label, lane_labels_json, claim_label_prefix,
    claim_comment_enabled, backpressure_open_pr_cap, created_at_ms,
    updated_at_ms
) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
ON CONFLICT(project_id) DO UPDATE SET
    display_name = excluded.display_name,
    tracker_provider = excluded.tracker_provider,
    tracker_repo = excluded.tracker_repo,
    target_repo = excluded.target_repo,
    project_label = excluded.project_label,
    lane_labels_json = excluded.lane_labels_json,
    claim_label_prefix = excluded.claim_label_prefix,
    claim_comment_enabled = excluded.claim_comment_enabled,
    backpressure_open_pr_cap = excluded.backpressure_open_pr_cap,
    updated_at_ms = excluded.updated_at_ms
",
        )
        .bind(&project.project_id)
        .bind(&project.display_name)
        .bind(&project.tracker_provider)
        .bind(&project.tracker_repo)
        .bind(&project.target_repo)
        .bind(&project.project_label)
        .bind(lane_labels)
        .bind(&project.claim_label_prefix)
        .bind(i64::from(project.claim_comment_enabled))
        .bind(project.backpressure_open_pr_cap)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Read one project.
    pub async fn project(&self, project_id: &str) -> Result<Option<WorkProject>> {
        let row = sqlx::query(
            r"
SELECT project_id, display_name, tracker_provider, tracker_repo,
       target_repo, project_label, lane_labels_json, claim_label_prefix,
       claim_comment_enabled, backpressure_open_pr_cap
FROM work_projects
WHERE project_id = ?
",
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(project_from_row).transpose()
    }

    /// The project whose `target_repo` names the given code repository
    /// (run-target resolution, ADR-0032 phase 3). First match wins; the
    /// deployment contract is one project per target repo.
    pub async fn project_for_target(&self, target_repo: &str) -> Result<Option<WorkProject>> {
        let row = sqlx::query(
            r"
SELECT project_id, display_name, tracker_provider, tracker_repo,
       target_repo, project_label, lane_labels_json, claim_label_prefix,
       claim_comment_enabled, backpressure_open_pr_cap
FROM work_projects
WHERE target_repo = ?
ORDER BY project_id
LIMIT 1
",
        )
        .bind(target_repo)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(project_from_row).transpose()
    }

    /// Read every configured project, ordered by id.
    pub async fn projects(&self) -> Result<Vec<WorkProject>> {
        let rows = sqlx::query(
            r"
SELECT project_id, display_name, tracker_provider, tracker_repo,
       target_repo, project_label, lane_labels_json, claim_label_prefix,
       claim_comment_enabled, backpressure_open_pr_cap
FROM work_projects
ORDER BY project_id
",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(project_from_row).collect()
    }

    /// Atomically grant one work item to one run.
    ///
    /// The unique indexes are the mutex: a concurrent grant for the same item
    /// or for a run that already holds another item loses and returns the
    /// live conflicting claim.
    pub async fn grant(&self, request: &GrantRequest) -> Result<GrantOutcome> {
        if self.project(&request.project_id).await?.is_none() {
            return Err(Error::WorkProjectNotFound(request.project_id.clone()));
        }

        let claim_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        let now_ms = now.timestamp_millis();
        let mut tx = self.pool.begin().await?;
        let inserted = sqlx::query(
            r"
INSERT INTO work_claims (
    claim_id, project_id, item_key, run_id, lane, state,
    granted_at_ms, updated_at_ms
) VALUES (?, ?, ?, ?, ?, 'granted', ?, ?)
",
        )
        .bind(&claim_id)
        .bind(&request.project_id)
        .bind(&request.item_key)
        .bind(request.run_id.to_string())
        .bind(&request.lane)
        .bind(now_ms)
        .bind(now_ms)
        .execute(&mut *tx)
        .await;

        match inserted {
            Ok(_) => {
                self.append_event_in_tx(
                    &mut tx,
                    &claim_id,
                    &request.run_id,
                    None,
                    ClaimState::Granted,
                    None,
                    now,
                )
                .await?;
                self.enqueue_in_tx(
                    &mut tx,
                    "claim.projected",
                    serde_json::json!({
                        "claim_id": claim_id,
                        "project_id": request.project_id,
                        "item_key": request.item_key,
                        "run_id": request.run_id.to_string(),
                    }),
                    format!("claim-projected:{claim_id}:granted"),
                    now,
                )
                .await?;
                tx.commit().await?;
                let claim = self
                    .claim(&claim_id)
                    .await?
                    .ok_or_else(|| Error::WorkClaimNotFound(claim_id.clone()))?;
                Ok(GrantOutcome::Granted(claim))
            }
            Err(err) if is_unique_violation(&err) => {
                tx.rollback().await.ok();
                self.classify_grant_conflict(request).await
            }
            Err(err) => Err(err.into()),
        }
    }

    /// Move a claim to `to_state`.
    pub async fn transition(
        &self,
        claim_id: &str,
        to_state: ClaimState,
        reason: Option<&str>,
        pr_url: Option<&str>,
    ) -> Result<WorkClaim> {
        let now = Utc::now();
        let now_ms = now.timestamp_millis();
        let mut tx = self.pool.begin().await?;
        let current = sqlx::query("SELECT state, run_id FROM work_claims WHERE claim_id = ?")
            .bind(claim_id)
            .fetch_optional(&mut *tx)
            .await?;
        let Some(current) = current else {
            tx.rollback().await.ok();
            return Err(Error::WorkClaimNotFound(claim_id.to_string()));
        };
        let from_state = ClaimState::from_db(current.try_get::<String, _>("state")?.as_str())?;
        let run_id = parse_run_id(&current.try_get::<String, _>("run_id")?)?;
        if !can_transition(from_state, to_state) {
            tx.rollback().await.ok();
            return Err(Error::InvalidClaimTransition {
                from: from_state,
                to:   to_state,
            });
        }

        let released_at_ms = to_state.is_terminal().then_some(now_ms);
        sqlx::query(
            r"
UPDATE work_claims
SET state = ?,
    updated_at_ms = ?,
    released_at_ms = COALESCE(?, released_at_ms),
    release_reason = COALESCE(?, release_reason),
    pr_url = COALESCE(?, pr_url)
WHERE claim_id = ?
",
        )
        .bind(to_state.as_str())
        .bind(now_ms)
        .bind(released_at_ms)
        .bind(reason)
        .bind(pr_url)
        .bind(claim_id)
        .execute(&mut *tx)
        .await?;
        self.append_event_in_tx(
            &mut tx,
            claim_id,
            &run_id,
            Some(from_state),
            to_state,
            reason,
            now,
        )
        .await?;
        self.enqueue_in_tx(
            &mut tx,
            "claim.projected",
            serde_json::json!({
                "claim_id": claim_id,
                "run_id": run_id.to_string(),
                "state": to_state.as_str(),
            }),
            format!("claim-projected:{claim_id}:{}", to_state.as_str()),
            now,
        )
        .await?;
        tx.commit().await?;
        self.claim(claim_id)
            .await?
            .ok_or_else(|| Error::WorkClaimNotFound(claim_id.to_string()))
    }

    /// Mark the run as started (granted -> active).
    pub async fn activate(&self, claim_id: &str) -> Result<WorkClaim> {
        self.transition(claim_id, ClaimState::Active, None, None)
            .await
    }

    /// The run ended with an open pull request.
    pub async fn await_merge(&self, claim_id: &str, pr_url: &str) -> Result<WorkClaim> {
        self.transition(claim_id, ClaimState::AwaitingMerge, None, Some(pr_url))
            .await
    }

    /// The linked pull request merged.
    pub async fn land(&self, claim_id: &str, pr_url: Option<&str>) -> Result<WorkClaim> {
        self.transition(claim_id, ClaimState::Landed, None, pr_url)
            .await
    }

    /// Release the item back to the queue.
    pub async fn release(&self, claim_id: &str, reason: &str) -> Result<WorkClaim> {
        self.transition(claim_id, ClaimState::Released, Some(reason), None)
            .await
    }

    /// Read one claim.
    pub async fn claim(&self, claim_id: &str) -> Result<Option<WorkClaim>> {
        let row = sqlx::query(CLAIM_SELECT)
            .bind(claim_id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(claim_from_row).transpose()
    }

    /// The live claim for one item, if any.
    pub async fn active_claim_for_item(
        &self,
        project_id: &str,
        item_key: &str,
    ) -> Result<Option<WorkClaim>> {
        let row = sqlx::query(
            r"
SELECT claim_id, project_id, item_key, run_id, lane, state,
       granted_at_ms, updated_at_ms, released_at_ms, release_reason, pr_url
FROM work_claims
WHERE project_id = ? AND item_key = ?
  AND state IN ('granted', 'active', 'awaiting_merge')
",
        )
        .bind(project_id)
        .bind(item_key)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(claim_from_row).transpose()
    }

    /// The live claim for one run, if any.
    pub async fn active_claim_for_run(&self, run_id: &RunId) -> Result<Option<WorkClaim>> {
        let row = sqlx::query(
            r"
SELECT claim_id, project_id, item_key, run_id, lane, state,
       granted_at_ms, updated_at_ms, released_at_ms, release_reason, pr_url
FROM work_claims
WHERE run_id = ? AND state IN ('granted', 'active', 'awaiting_merge')
",
        )
        .bind(run_id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(claim_from_row).transpose()
    }

    /// The project's live claims of one lane (assignment backpressure and
    /// lane-queue views).
    pub async fn active_claims_of_lane(
        &self,
        project_id: &str,
        lane: &str,
    ) -> Result<Vec<WorkClaim>> {
        Ok(self
            .active_claims(project_id)
            .await?
            .into_iter()
            .filter(|claim| claim.lane == lane)
            .collect())
    }

    /// Every live claim of a project.
    pub async fn active_claims(&self, project_id: &str) -> Result<Vec<WorkClaim>> {
        let rows = sqlx::query(
            r"
SELECT claim_id, project_id, item_key, run_id, lane, state,
       granted_at_ms, updated_at_ms, released_at_ms, release_reason, pr_url
FROM work_claims
WHERE project_id = ? AND state IN ('granted', 'active', 'awaiting_merge')
ORDER BY granted_at_ms
",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(claim_from_row).collect()
    }

    /// The transition history of one claim, oldest first.
    pub async fn events(&self, claim_id: &str) -> Result<Vec<WorkClaimEvent>> {
        let rows = sqlx::query(
            r"
SELECT claim_id, seq, at_ms, from_state, to_state, reason, run_id
FROM work_claim_events
WHERE claim_id = ?
ORDER BY seq
",
        )
        .bind(claim_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(event_from_row).collect()
    }

    /// Release live claims whose run is not in `live_runs` and whose last
    /// update is older than `cutoff`. Used by the startup reconciler for
    /// claims whose run record is gone (the 30-minute orphan grace).
    pub async fn release_orphans(
        &self,
        project_id: &str,
        live_runs: &[RunId],
        cutoff: DateTime<Utc>,
        reason: &str,
    ) -> Result<Vec<WorkClaim>> {
        let live: std::collections::HashSet<String> =
            live_runs.iter().map(ToString::to_string).collect();
        let claims = self.active_claims(project_id).await?;
        let mut released = Vec::new();
        for claim in claims {
            if live.contains(&claim.run_id.to_string()) || claim.updated_at >= cutoff {
                continue;
            }
            released.push(self.release(&claim.claim_id, reason).await?);
        }
        Ok(released)
    }

    /// Enqueue a projection job. Re-enqueuing the same idempotency key is a
    /// no-op (the first job remains authoritative).
    pub async fn enqueue(&self, kind: &str, payload: Value, idempotency_key: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        self.enqueue_in_tx(
            &mut tx,
            kind,
            payload,
            idempotency_key.to_string(),
            Utc::now(),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Pending projection jobs due at `now`, oldest first.
    pub async fn due(&self, now: DateTime<Utc>, limit: u32) -> Result<Vec<WorkOutboxEntry>> {
        let rows = sqlx::query(
            r"
SELECT id, idempotency_key, kind, payload_json, attempts,
       next_attempt_at_ms, created_at_ms, completed_at_ms, last_error
FROM work_outbox
WHERE completed_at_ms IS NULL AND next_attempt_at_ms <= ?
ORDER BY next_attempt_at_ms, id
LIMIT ?
",
        )
        .bind(now.timestamp_millis())
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(outbox_from_row).collect()
    }

    /// Mark a projection job complete.
    pub async fn complete(&self, id: &str) -> Result<()> {
        let result = sqlx::query(
            "UPDATE work_outbox SET completed_at_ms = ? WHERE id = ? AND completed_at_ms IS NULL",
        )
        .bind(Utc::now().timestamp_millis())
        .bind(id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(Error::WorkOutboxEntryNotFound(id.to_string()));
        }
        Ok(())
    }

    /// Record a failed attempt and schedule the next one.
    pub async fn fail(&self, id: &str, error: &str, next_attempt_at: DateTime<Utc>) -> Result<()> {
        let result = sqlx::query(
            r"
UPDATE work_outbox
SET attempts = attempts + 1,
    next_attempt_at_ms = ?,
    last_error = ?
WHERE id = ? AND completed_at_ms IS NULL
",
        )
        .bind(next_attempt_at.timestamp_millis())
        .bind(error)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(Error::WorkOutboxEntryNotFound(id.to_string()));
        }
        Ok(())
    }

    /// Read one outbox job (tests and operators).
    pub async fn outbox_entry(&self, id: &str) -> Result<Option<WorkOutboxEntry>> {
        let row = sqlx::query(
            r"
SELECT id, idempotency_key, kind, payload_json, attempts,
       next_attempt_at_ms, created_at_ms, completed_at_ms, last_error
FROM work_outbox
WHERE id = ?
",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(outbox_from_row).transpose()
    }

    /// Release every live claim held by `run_id` (run terminal, cancel,
    /// duplicate close, external close).
    pub async fn release_run(&self, run_id: &RunId, reason: &str) -> Result<Vec<WorkClaim>> {
        let claims = self.active_claims_for_run(run_id).await?;
        let mut released = Vec::new();
        for claim in claims {
            released.push(self.release(&claim.claim_id, reason).await?);
        }
        Ok(released)
    }

    async fn active_claims_for_run(&self, run_id: &RunId) -> Result<Vec<WorkClaim>> {
        let rows = sqlx::query(
            r"
SELECT claim_id, project_id, item_key, run_id, lane, state,
       granted_at_ms, updated_at_ms, released_at_ms, release_reason, pr_url
FROM work_claims
WHERE run_id = ? AND state IN ('granted', 'active', 'awaiting_merge')
",
        )
        .bind(run_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(claim_from_row).collect()
    }

    async fn classify_grant_conflict(&self, request: &GrantRequest) -> Result<GrantOutcome> {
        if let Some(claim) = self
            .active_claim_for_item(&request.project_id, &request.item_key)
            .await?
        {
            if claim.run_id == request.run_id {
                return Ok(GrantOutcome::AlreadyHeld(claim));
            }
            return Ok(GrantOutcome::Conflict(claim));
        }
        if let Some(claim) = self.active_claim_for_run(&request.run_id).await? {
            return Ok(GrantOutcome::Conflict(claim));
        }
        Err(Error::WorkClaimConflict(format!(
            "grant for {} on {} lost the unique index but no live claim is visible",
            request.run_id, request.item_key
        )))
    }

    async fn append_event_in_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        claim_id: &str,
        run_id: &RunId,
        from_state: Option<ClaimState>,
        to_state: ClaimState,
        reason: Option<&str>,
        at: DateTime<Utc>,
    ) -> Result<()> {
        let next_seq: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM work_claim_events WHERE claim_id = ?",
        )
        .bind(claim_id)
        .fetch_one(&mut **tx)
        .await?;
        sqlx::query(
            r"
INSERT INTO work_claim_events (claim_id, seq, at_ms, from_state, to_state, reason, run_id)
VALUES (?, ?, ?, ?, ?, ?, ?)
",
        )
        .bind(claim_id)
        .bind(next_seq)
        .bind(at.timestamp_millis())
        .bind(from_state.map(ClaimState::as_str))
        .bind(to_state.as_str())
        .bind(reason)
        .bind(run_id.to_string())
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    async fn enqueue_in_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        kind: &str,
        payload: Value,
        idempotency_key: String,
        now: DateTime<Utc>,
    ) -> Result<()> {
        sqlx::query(
            r"
INSERT INTO work_outbox (
    id, idempotency_key, kind, payload_json, attempts,
    next_attempt_at_ms, created_at_ms
) VALUES (?, ?, ?, ?, 0, ?, ?)
ON CONFLICT(idempotency_key) DO NOTHING
",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(idempotency_key)
        .bind(kind)
        .bind(serde_json::to_string(&payload)?)
        .bind(now.timestamp_millis())
        .bind(now.timestamp_millis())
        .execute(&mut **tx)
        .await?;
        Ok(())
    }
}

const CLAIM_SELECT: &str = r"
SELECT claim_id, project_id, item_key, run_id, lane, state,
       granted_at_ms, updated_at_ms, released_at_ms, release_reason, pr_url
FROM work_claims
WHERE claim_id = ?
";

fn can_transition(from: ClaimState, to: ClaimState) -> bool {
    matches!(
        (from, to),
        (
            ClaimState::Granted,
            ClaimState::Active | ClaimState::Landed | ClaimState::Released
        ) | (
            ClaimState::Active,
            ClaimState::AwaitingMerge | ClaimState::Landed | ClaimState::Released,
        ) | (
            ClaimState::AwaitingMerge,
            ClaimState::Landed | ClaimState::Released
        )
    )
}

fn is_unique_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db) if db.is_unique_violation())
}

fn parse_run_id(value: &str) -> Result<RunId> {
    value
        .parse()
        .map_err(|err| Error::Other(format!("stored claim has an invalid run id: {err}")))
}

fn project_from_row(row: &SqliteRow) -> Result<WorkProject> {
    let lane_labels_json: String = row.try_get("lane_labels_json")?;
    Ok(WorkProject {
        project_id:               row.try_get("project_id")?,
        display_name:             row.try_get("display_name")?,
        tracker_provider:         row.try_get("tracker_provider")?,
        tracker_repo:             row.try_get("tracker_repo")?,
        target_repo:              row.try_get("target_repo")?,
        project_label:            row.try_get("project_label")?,
        lane_labels:              serde_json::from_str(&lane_labels_json)?,
        claim_label_prefix:       row.try_get("claim_label_prefix")?,
        claim_comment_enabled:    row.try_get::<i64, _>("claim_comment_enabled")? != 0,
        backpressure_open_pr_cap: row.try_get("backpressure_open_pr_cap")?,
    })
}

fn claim_from_row(row: &SqliteRow) -> Result<WorkClaim> {
    Ok(WorkClaim {
        claim_id:       row.try_get("claim_id")?,
        project_id:     row.try_get("project_id")?,
        item_key:       row.try_get("item_key")?,
        run_id:         parse_run_id(&row.try_get::<String, _>("run_id")?)?,
        lane:           row.try_get("lane")?,
        state:          ClaimState::from_db(row.try_get::<String, _>("state")?.as_str())?,
        granted_at:     timestamp_from_row(row, "work claim", "granted_at_ms")?,
        updated_at:     timestamp_from_row(row, "work claim", "updated_at_ms")?,
        released_at:    optional_timestamp(row, "released_at_ms")?,
        release_reason: row.try_get("release_reason")?,
        pr_url:         row.try_get("pr_url")?,
    })
}

fn event_from_row(row: &SqliteRow) -> Result<WorkClaimEvent> {
    Ok(WorkClaimEvent {
        claim_id:   row.try_get("claim_id")?,
        seq:        row.try_get("seq")?,
        at:         timestamp_from_row(row, "work claim event", "at_ms")?,
        from_state: row
            .try_get::<Option<String>, _>("from_state")?
            .map(|value| ClaimState::from_db(&value))
            .transpose()?,
        to_state:   ClaimState::from_db(row.try_get::<String, _>("to_state")?.as_str())?,
        reason:     row.try_get("reason")?,
        run_id:     parse_run_id(&row.try_get::<String, _>("run_id")?)?,
    })
}

fn outbox_from_row(row: &SqliteRow) -> Result<WorkOutboxEntry> {
    Ok(WorkOutboxEntry {
        id:              row.try_get("id")?,
        idempotency_key: row.try_get("idempotency_key")?,
        kind:            row.try_get("kind")?,
        payload:         serde_json::from_str(&row.try_get::<String, _>("payload_json")?)?,
        attempts:        row.try_get("attempts")?,
        next_attempt_at: timestamp_from_row(row, "work outbox", "next_attempt_at_ms")?,
        created_at:      timestamp_from_row(row, "work outbox", "created_at_ms")?,
        completed_at:    optional_timestamp(row, "completed_at_ms")?,
        last_error:      row.try_get("last_error")?,
    })
}

fn optional_timestamp(row: &SqliteRow, field: &'static str) -> Result<Option<DateTime<Utc>>> {
    let Some(value) = row.try_get::<Option<i64>, _>(field)? else {
        return Ok(None);
    };
    DateTime::from_timestamp_millis(value)
        .map(Some)
        .ok_or(Error::InvalidStoredTimestamp {
            record: "work record",
            field,
            value,
        })
}

fn timestamp_from_row(
    row: &SqliteRow,
    record: &'static str,
    field: &'static str,
) -> Result<DateTime<Utc>> {
    let value: i64 = row.try_get(field)?;
    DateTime::from_timestamp_millis(value).ok_or(Error::InvalidStoredTimestamp {
        record,
        field,
        value,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Duration;
    use tokio::task::JoinSet;

    use super::{ClaimState, GrantOutcome, GrantRequest, WorkProject, WorkStore};
    use crate::Error;

    fn project() -> WorkProject {
        WorkProject {
            project_id:               "fabro".to_string(),
            display_name:             "Fabro".to_string(),
            tracker_provider:         "github".to_string(),
            tracker_repo:             "denkhaus/work".to_string(),
            target_repo:              Some("denkhaus/fabro".to_string()),
            project_label:            "project:fabro".to_string(),
            lane_labels:              [("loop".to_string(), "lane:loop".to_string())]
                .into_iter()
                .collect(),
            claim_label_prefix:       "fabro:run:".to_string(),
            claim_comment_enabled:    false,
            backpressure_open_pr_cap: 0,
        }
    }

    async fn store() -> (tempfile::TempDir, WorkStore) {
        let directory = tempfile::tempdir().unwrap();
        let database = fabro_db::Database::connect(directory.path().join("fabro.sqlite3"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let store = WorkStore::new(database.clone_pool());
        store.upsert_project(&project()).await.unwrap();
        (directory, store)
    }

    fn request(item: &str, run_id: fabro_types::RunId) -> GrantRequest {
        GrantRequest {
            project_id: "fabro".to_string(),
            item_key: item.to_string(),
            run_id,
            lane: "loop".to_string(),
        }
    }

    #[tokio::test]
    async fn project_round_trips_and_updates() {
        let (_directory, store) = store().await;
        assert_eq!(store.project("fabro").await.unwrap(), Some(project()));
        assert_eq!(store.projects().await.unwrap(), vec![project()]);

        let mut updated = project();
        updated.display_name = "Fabro (updated)".to_string();
        updated.claim_comment_enabled = true;
        store.upsert_project(&updated).await.unwrap();
        assert_eq!(store.project("fabro").await.unwrap(), Some(updated));
    }

    #[tokio::test]
    async fn project_for_target_resolves_the_bound_project() {
        let (_directory, store) = store().await;
        assert_eq!(
            store.project_for_target("denkhaus/fabro").await.unwrap(),
            Some(project()),
            "the project bound to the target repository resolves"
        );
        assert_eq!(
            store.project_for_target("denkhaus/other").await.unwrap(),
            None,
            "an unbound repository resolves to nothing"
        );

        let mut explicit_only = project();
        explicit_only.project_id = "explicit".to_string();
        explicit_only.target_repo = None;
        store.upsert_project(&explicit_only).await.unwrap();
        assert_eq!(
            store.project_for_target("denkhaus/unbound").await.unwrap(),
            None,
            "a project without target_repo stays explicit-only"
        );
    }

    #[tokio::test]
    async fn grant_is_atomic_across_store_instances() {
        let (_directory, store) = store().await;
        let store = Arc::new(store);
        let mut tasks = JoinSet::new();
        for _ in 0..16 {
            let store = Arc::clone(&store);
            let run_id = fabro_types::RunId::new();
            tasks.spawn(async move {
                store
                    .grant(&request("denkhaus/work#1", run_id))
                    .await
                    .unwrap()
            });
        }

        let mut granted = 0;
        let mut conflicts = 0;
        while let Some(result) = tasks.join_next().await {
            match result.unwrap() {
                GrantOutcome::Granted(_) => granted += 1,
                GrantOutcome::Conflict(_) => conflicts += 1,
                GrantOutcome::AlreadyHeld(_) => unreachable!("distinct runs"),
            }
        }
        assert_eq!(granted, 1);
        assert_eq!(conflicts, 15);
    }

    #[tokio::test]
    async fn same_run_regrant_is_idempotent() {
        let (_directory, store) = store().await;
        let run_id = fabro_types::RunId::new();
        let first = store
            .grant(&request("denkhaus/work#1", run_id))
            .await
            .unwrap();
        let GrantOutcome::Granted(first) = first else {
            panic!("first grant must win");
        };

        let second = store
            .grant(&request("denkhaus/work#1", run_id))
            .await
            .unwrap();
        assert_eq!(second, GrantOutcome::AlreadyHeld(first));
    }

    #[tokio::test]
    async fn one_run_cannot_hold_two_items() {
        let (_directory, store) = store().await;
        let run_id = fabro_types::RunId::new();
        let first = store
            .grant(&request("denkhaus/work#1", run_id))
            .await
            .unwrap();
        let GrantOutcome::Granted(first) = first else {
            panic!("first grant must win");
        };

        let second = store
            .grant(&request("denkhaus/work#2", run_id))
            .await
            .unwrap();
        assert_eq!(second, GrantOutcome::Conflict(first));
    }

    #[tokio::test]
    async fn awaiting_merge_holds_the_item_until_landed_or_released() {
        let (_directory, store) = store().await;
        let first_run = fabro_types::RunId::new();
        let GrantOutcome::Granted(claim) = store
            .grant(&request("denkhaus/work#1", first_run))
            .await
            .unwrap()
        else {
            panic!("first grant must win");
        };
        store.activate(&claim.claim_id).await.unwrap();
        store
            .await_merge(&claim.claim_id, "https://github.com/denkhaus/fabro/pull/1")
            .await
            .unwrap();

        let second_run = fabro_types::RunId::new();
        assert!(matches!(
            store
                .grant(&request("denkhaus/work#1", second_run))
                .await
                .unwrap(),
            GrantOutcome::Conflict(_)
        ));

        store.land(&claim.claim_id, None).await.unwrap();
        assert!(matches!(
            store
                .grant(&request("denkhaus/work#1", second_run))
                .await
                .unwrap(),
            GrantOutcome::Granted(_)
        ));
    }

    #[tokio::test]
    async fn release_frees_the_item_and_records_the_reason() {
        let (_directory, store) = store().await;
        let first_run = fabro_types::RunId::new();
        let GrantOutcome::Granted(claim) = store
            .grant(&request("denkhaus/work#1", first_run))
            .await
            .unwrap()
        else {
            panic!("first grant must win");
        };
        let released = store
            .release(&claim.claim_id, "planner skipped")
            .await
            .unwrap();
        assert_eq!(released.state, ClaimState::Released);
        assert_eq!(released.release_reason.as_deref(), Some("planner skipped"));
        assert!(released.released_at.is_some());

        let second_run = fabro_types::RunId::new();
        assert!(matches!(
            store
                .grant(&request("denkhaus/work#1", second_run))
                .await
                .unwrap(),
            GrantOutcome::Granted(_)
        ));
    }

    #[tokio::test]
    async fn events_record_every_transition() {
        let (_directory, store) = store().await;
        let run_id = fabro_types::RunId::new();
        let GrantOutcome::Granted(claim) = store
            .grant(&request("denkhaus/work#1", run_id))
            .await
            .unwrap()
        else {
            panic!("first grant must win");
        };
        store.activate(&claim.claim_id).await.unwrap();
        store
            .await_merge(&claim.claim_id, "https://github.com/denkhaus/fabro/pull/1")
            .await
            .unwrap();
        store.land(&claim.claim_id, None).await.unwrap();

        let events = store.events(&claim.claim_id).await.unwrap();
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].from_state, None);
        assert_eq!(events[0].to_state, ClaimState::Granted);
        assert_eq!(events[1].from_state, Some(ClaimState::Granted));
        assert_eq!(events[1].to_state, ClaimState::Active);
        assert_eq!(events[2].to_state, ClaimState::AwaitingMerge);
        assert_eq!(events[3].to_state, ClaimState::Landed);
        assert!(events.iter().all(|event| event.run_id == run_id));
    }

    #[tokio::test]
    async fn invalid_transition_is_rejected() {
        let (_directory, store) = store().await;
        let run_id = fabro_types::RunId::new();
        let GrantOutcome::Granted(claim) = store
            .grant(&request("denkhaus/work#1", run_id))
            .await
            .unwrap()
        else {
            panic!("first grant must win");
        };
        let error = store
            .await_merge(&claim.claim_id, "https://github.com/denkhaus/fabro/pull/1")
            .await
            .unwrap_err();
        assert!(matches!(error, Error::InvalidClaimTransition {
            from: ClaimState::Granted,
            to:   ClaimState::AwaitingMerge,
        }));
    }

    #[tokio::test]
    async fn grant_requires_a_configured_project() {
        let (_directory, store) = store().await;
        let error = store
            .grant(&GrantRequest {
                project_id: "missing".to_string(),
                item_key:   "denkhaus/work#1".to_string(),
                run_id:     fabro_types::RunId::new(),
                lane:       "loop".to_string(),
            })
            .await
            .unwrap_err();
        assert!(matches!(error, Error::WorkProjectNotFound(project) if project == "missing"));
    }

    #[tokio::test]
    async fn release_run_frees_every_claim_of_the_run() {
        let (_directory, store) = store().await;
        let run_id = fabro_types::RunId::new();
        let GrantOutcome::Granted(claim) = store
            .grant(&request("denkhaus/work#1", run_id))
            .await
            .unwrap()
        else {
            panic!("first grant must win");
        };
        let released = store.release_run(&run_id, "run failed").await.unwrap();
        assert_eq!(released.len(), 1);
        assert_eq!(released[0].claim_id, claim.claim_id);
        assert!(store.active_claim_for_run(&run_id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn release_orphans_keeps_live_runs_and_releases_the_rest() {
        let (_directory, store) = store().await;
        let stale_run = fabro_types::RunId::new();
        let live_run = fabro_types::RunId::new();
        let stale = store
            .grant(&request("denkhaus/work#1", stale_run))
            .await
            .unwrap();
        let live = store
            .grant(&request("denkhaus/work#2", live_run))
            .await
            .unwrap();
        let (GrantOutcome::Granted(stale), GrantOutcome::Granted(live)) = (stale, live) else {
            panic!("both grants must win");
        };

        let released = store
            .release_orphans(
                "fabro",
                &[live_run],
                chrono::Utc::now() + Duration::seconds(1),
                "orphan",
            )
            .await
            .unwrap();
        assert_eq!(released.len(), 1);
        assert_eq!(released[0].claim_id, stale.claim_id);
        assert_eq!(
            store.claim(&live.claim_id).await.unwrap().unwrap().state,
            ClaimState::Granted
        );
    }

    #[tokio::test]
    async fn outbox_is_idempotent_and_retries() {
        let (_directory, store) = store().await;
        let now = chrono::Utc::now();
        store
            .enqueue("claim.projected", serde_json::json!({"a": 1}), "key")
            .await
            .unwrap();
        store
            .enqueue("claim.projected", serde_json::json!({"a": 2}), "key")
            .await
            .unwrap();
        let due = store.due(now + Duration::seconds(1), 10).await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].payload, serde_json::json!({"a": 1}));

        store
            .fail(&due[0].id, "github down", now + Duration::seconds(5))
            .await
            .unwrap();
        let entry = store.outbox_entry(&due[0].id).await.unwrap().unwrap();
        assert_eq!(entry.attempts, 1);
        assert_eq!(entry.last_error.as_deref(), Some("github down"));
        assert!(
            store
                .due(now + Duration::seconds(2), 10)
                .await
                .unwrap()
                .is_empty()
        );

        store.complete(&due[0].id).await.unwrap();
        assert!(
            store
                .due(now + Duration::seconds(10), 10)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            store.complete(&due[0].id).await.unwrap_err(),
            Error::WorkOutboxEntryNotFound(_)
        ));
    }
}
