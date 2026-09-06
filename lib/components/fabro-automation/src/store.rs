use std::str::FromStr as _;

use fabro_db::DbPool;
use fabro_types::{GitRunTarget, RunTarget};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row as _, Sqlite, Transaction};

use crate::{
    ApiTrigger, Automation, AutomationDraft, AutomationGitWorkflowSource, AutomationId,
    AutomationReplace, AutomationRevision, AutomationStoreError, AutomationTrigger,
    AutomationTriggerId, ScheduleTrigger,
};

/// Shared projection for loading automations with their schedule triggers.
/// A macro rather than a `const` because sqlx requires `&'static str` SQL.
macro_rules! select_automations_sql {
    ($suffix:expr) => {
        concat!(
            "SELECT
                a.id,
                a.revision,
                a.name,
                a.description,
                a.environment_id,
                a.last_error,
                a.api_enabled,
                a.on_overlap,
                a.target_repository,
                a.target_branch,
                a.target_tag,
                a.target_sha,
                a.target_workflow,
                a.workflow_source_repository,
                a.workflow_source_branch,
                a.workflow_source_tag,
                a.workflow_source_sha,
                t.id AS trigger_id,
                t.enabled AS trigger_enabled,
                t.expression AS trigger_expression,
                t.breaker_threshold AS trigger_breaker_threshold,
                t.breaker_signature AS trigger_breaker_signature,
                t.breaker_consecutive_count AS trigger_breaker_consecutive_count,
                t.breaker_last_run_id AS trigger_breaker_last_run_id,
                t.breaker_paused_at_ms AS trigger_breaker_paused_at_ms
            FROM automations AS a
            LEFT JOIN automation_triggers AS t ON t.automation_id = a.id
            ",
            $suffix
        )
    };
}

#[derive(Clone)]
pub struct AutomationStore {
    pool: DbPool,
}

impl std::fmt::Debug for AutomationStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutomationStore").finish_non_exhaustive()
    }
}

impl AutomationStore {
    #[must_use]
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn list(&self) -> Result<Vec<Automation>, AutomationStoreError> {
        let rows = sqlx::query(select_automations_sql!("ORDER BY a.id, t.id"))
            .fetch_all(&self.pool)
            .await?;
        automations_from_rows(&rows)
    }

    pub async fn get(&self, id: &AutomationId) -> Result<Option<Automation>, AutomationStoreError> {
        let rows = sqlx::query(select_automations_sql!("WHERE a.id = ? ORDER BY t.id"))
            .bind(id.as_str())
            .fetch_all(&self.pool)
            .await?;
        Ok(automations_from_rows(&rows)?.into_iter().next())
    }

    pub async fn exists(&self, id: &AutomationId) -> Result<bool, AutomationStoreError> {
        let row = sqlx::query("SELECT 1 FROM automations WHERE id = ?")
            .bind(id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }

    pub async fn references_environment(
        &self,
        environment_id: &str,
    ) -> Result<bool, AutomationStoreError> {
        let row = sqlx::query("SELECT 1 FROM automations WHERE environment_id = ? LIMIT 1")
            .bind(environment_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }

    pub async fn set_last_error(
        &self,
        id: &AutomationId,
        message: Option<&str>,
    ) -> Result<(), AutomationStoreError> {
        let result = sqlx::query("UPDATE automations SET last_error = ? WHERE id = ?")
            .bind(message)
            .bind(id.as_str())
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(AutomationStoreError::NotFound { id: id.clone() });
        }
        Ok(())
    }

    /// Persist breaker counter facts for one schedule trigger and optionally
    /// pause it (fabro-3d97). A pause is a compare-and-set on `enabled = 1`,
    /// so exactly the first caller that trips the breaker disables the
    /// trigger; the return value reports whether THIS call paused it (use it
    /// to emit the single aggregated notification).
    pub async fn apply_schedule_breaker(
        &self,
        id: &AutomationId,
        trigger_id: &AutomationTriggerId,
        signature: Option<&str>,
        consecutive_count: u32,
        last_run_id: &str,
        pause: bool,
        paused_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool, AutomationStoreError> {
        let sql = if pause {
            r"
            UPDATE automation_triggers SET
                breaker_signature = ?,
                breaker_consecutive_count = ?,
                breaker_last_run_id = ?,
                breaker_paused_at_ms = ?,
                enabled = 0
            WHERE automation_id = ? AND id = ? AND enabled = 1
            "
        } else {
            r"
            UPDATE automation_triggers SET
                breaker_signature = ?,
                breaker_consecutive_count = ?,
                breaker_last_run_id = ?,
                breaker_paused_at_ms = NULL
            WHERE automation_id = ? AND id = ?
            "
        };
        let mut query = sqlx::query(sql)
            .bind(signature)
            .bind(i64::from(consecutive_count))
            .bind(last_run_id);
        query = if pause {
            query
                .bind(paused_at.timestamp_millis())
                .bind(id.as_str())
                .bind(trigger_id.as_str())
        } else {
            query.bind(id.as_str()).bind(trigger_id.as_str())
        };
        let result = query.execute(&self.pool).await?;
        if result.rows_affected() == 0 {
            // Either the trigger row is gone (replaced/deleted) or the pause
            // raced with another pause; nothing to notify for either.
            return Ok(false);
        }
        Ok(pause)
    }

    pub async fn create(&self, draft: AutomationDraft) -> Result<Automation, AutomationStoreError> {
        let (id, replace) = draft.into();
        let (automation, _) = Automation::from_replace(id.clone(), replace)?;
        let mut transaction = self.pool.begin().await?;
        if !insert_automation_ignoring_conflict(&mut transaction, &automation).await? {
            return Err(AutomationStoreError::AlreadyExists { id });
        }
        transaction.commit().await?;
        Ok(automation)
    }

    pub async fn replace(
        &self,
        id: &AutomationId,
        expected: &AutomationRevision,
        draft: AutomationReplace,
    ) -> Result<Automation, AutomationStoreError> {
        let (automation, _) = Automation::from_replace(id.clone(), draft)?;
        let target = stored_git_target(&automation);
        let workflow_source = automation.workflow_source.as_ref();
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query(
            r"
            UPDATE automations SET
                revision = ?,
                name = ?,
                description = ?,
                environment_id = ?,
                last_error = NULL,
                api_enabled = ?,
                target_repository = ?,
                target_branch = ?,
                target_tag = ?,
                target_sha = ?,
                target_workflow = ?,
                workflow_source_repository = ?,
                workflow_source_branch = ?,
                workflow_source_tag = ?,
                workflow_source_sha = ?,
                on_overlap = ?
            WHERE id = ? AND revision = ?
            ",
        )
        .bind(automation.revision.as_str())
        .bind(&automation.name)
        .bind(automation.description.as_deref())
        .bind(automation.environment_id.as_deref())
        .bind(automation.api_enabled())
        .bind(&target.repo)
        .bind(&target.branch)
        .bind(target.tag.as_deref())
        .bind(target.sha.as_deref())
        .bind(&automation.workflow)
        .bind(workflow_source.map(|source| source.repo.as_str()))
        .bind(workflow_source.map(|source| source.branch.as_str()))
        .bind(workflow_source.and_then(|source| source.tag.as_deref()))
        .bind(workflow_source.and_then(|source| source.sha.as_deref()))
        .bind(stored_overlap(automation.on_overlap))
        .bind(id.as_str())
        .bind(expected.as_str())
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() == 0 {
            return Err(revision_mismatch_error(&mut transaction, id, expected).await?);
        }

        sqlx::query("DELETE FROM automation_triggers WHERE automation_id = ?")
            .bind(id.as_str())
            .execute(&mut *transaction)
            .await?;
        insert_schedule_triggers(&mut transaction, &automation).await?;
        transaction.commit().await?;
        Ok(automation)
    }

    pub async fn delete(
        &self,
        id: &AutomationId,
        expected: &AutomationRevision,
    ) -> Result<(), AutomationStoreError> {
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query("DELETE FROM automations WHERE id = ? AND revision = ?")
            .bind(id.as_str())
            .bind(expected.as_str())
            .execute(&mut *transaction)
            .await?;
        if result.rows_affected() == 0 {
            return Err(revision_mismatch_error(&mut transaction, id, expected).await?);
        }
        transaction.commit().await?;
        Ok(())
    }
}

struct StoredAutomation {
    id:                AutomationId,
    revision:          AutomationRevision,
    name:              String,
    description:       Option<String>,
    environment_id:    Option<String>,
    last_error:        Option<String>,
    api_enabled:       bool,
    target:            RunTarget,
    workflow:          String,
    workflow_source:   Option<AutomationGitWorkflowSource>,
    on_overlap:        Option<crate::AutomationOverlapPolicy>,
    schedule_triggers: Vec<ScheduleTrigger>,
}

impl StoredAutomation {
    fn from_row(row: &SqliteRow) -> Result<Self, AutomationStoreError> {
        let id_value = row.try_get::<String, _>("id")?;
        let id = AutomationId::new(id_value.clone()).map_err(|source| {
            AutomationStoreError::StoredId {
                value: id_value,
                source,
            }
        })?;
        let revision = AutomationRevision::from_str(&row.try_get::<String, _>("revision")?)
            .map_err(|source| AutomationStoreError::InvalidRevision {
                id: id.clone(),
                source,
            })?;
        let workflow_source = stored_workflow_source(row, &id)?;
        Ok(Self {
            id,
            revision,
            name: row.try_get("name")?,
            description: row.try_get("description")?,
            environment_id: row.try_get("environment_id")?,
            last_error: row.try_get("last_error")?,
            api_enabled: row.try_get("api_enabled")?,
            target: RunTarget::Git(GitRunTarget {
                repo:   row.try_get("target_repository")?,
                branch: row.try_get("target_branch")?,
                tag:    row.try_get("target_tag")?,
                sha:    row.try_get("target_sha")?,
            }),
            workflow: row.try_get("target_workflow")?,
            on_overlap: stored_overlap_from(
                row.try_get::<Option<String>, _>("on_overlap")?.as_deref(),
            ),
            workflow_source,
            schedule_triggers: Vec::new(),
        })
    }

    fn push_trigger_row(&mut self, row: &SqliteRow) -> Result<(), AutomationStoreError> {
        let Some(id_value) = row.try_get::<Option<String>, _>("trigger_id")? else {
            return Ok(());
        };
        let id = AutomationTriggerId::new(id_value).map_err(|source| {
            AutomationStoreError::StoredValidation {
                id: self.id.clone(),
                source,
            }
        })?;
        let expression = row
            .try_get::<Option<String>, _>("trigger_expression")?
            .ok_or_else(|| AutomationStoreError::StoredTriggerShape {
                id: self.id.clone(),
            })?;
        let breaker_threshold = row
            .try_get::<Option<i64>, _>("trigger_breaker_threshold")?
            .map(u32::try_from)
            .transpose()
            .map_err(|_| AutomationStoreError::StoredTriggerShape {
                id: self.id.clone(),
            })?;
        self.schedule_triggers.push(ScheduleTrigger {
            id,
            enabled: row
                .try_get::<Option<bool>, _>("trigger_enabled")?
                .ok_or_else(|| AutomationStoreError::StoredTriggerShape {
                    id: self.id.clone(),
                })?,
            expression,
            breaker_threshold,
            breaker: stored_breaker_state(row, &self.id)?,
        });
        Ok(())
    }

    fn finish(mut self) -> Result<Automation, AutomationStoreError> {
        // Input normalization strips scheduler-owned breaker facts, so keep
        // them aside and re-attach after canonicalization (fabro-3d97).
        let breaker_by_id = self
            .schedule_triggers
            .iter()
            .map(|trigger| (trigger.id.clone(), trigger.breaker.clone()))
            .collect::<std::collections::HashMap<_, _>>();
        // `from_stored` canonicalizes trigger order and the manual API trigger.
        let mut triggers = self
            .schedule_triggers
            .drain(..)
            .map(AutomationTrigger::Schedule)
            .collect::<Vec<_>>();
        if self.api_enabled {
            triggers.push(AutomationTrigger::Api(ApiTrigger::manual()));
        }
        let id = self.id;
        let mut automation =
            Automation::from_stored(id.clone(), self.revision, AutomationReplace {
                name: self.name,
                description: self.description,
                environment_id: self.environment_id,
                target: self.target,
                workflow: self.workflow,
                workflow_source: self.workflow_source,
                on_overlap: self.on_overlap,
                triggers,
            })
            .map_err(|source| AutomationStoreError::StoredValidation { id, source })?;
        automation.last_error = self.last_error;
        for trigger in &mut automation.triggers {
            if let AutomationTrigger::Schedule(trigger) = trigger {
                trigger.breaker = breaker_by_id.get(&trigger.id).cloned().flatten();
            }
        }
        Ok(automation)
    }
}

fn automations_from_rows(rows: &[SqliteRow]) -> Result<Vec<Automation>, AutomationStoreError> {
    let mut automations = Vec::new();
    let mut current: Option<StoredAutomation> = None;

    for row in rows {
        let row_id = row.try_get::<String, _>("id")?;
        if current
            .as_ref()
            .is_some_and(|automation| automation.id.as_str() != row_id)
        {
            automations.push(
                current
                    .take()
                    .expect("current automation exists")
                    .finish()?,
            );
        }
        if current.is_none() {
            current = Some(StoredAutomation::from_row(row)?);
        }
        current
            .as_mut()
            .expect("current automation exists")
            .push_trigger_row(row)?;
    }

    if let Some(automation) = current {
        automations.push(automation.finish()?);
    }
    Ok(automations)
}

pub(crate) async fn insert_automation_ignoring_conflict(
    transaction: &mut Transaction<'_, Sqlite>,
    automation: &Automation,
) -> Result<bool, AutomationStoreError> {
    let target = stored_git_target(automation);
    let workflow_source = automation.workflow_source.as_ref();
    let result = sqlx::query(
        r"
        INSERT INTO automations (
            id,
            revision,
            name,
            description,
            environment_id,
            api_enabled,
            target_repository,
            target_branch,
            target_tag,
            target_sha,
            target_workflow,
            workflow_source_repository,
            workflow_source_branch,
            workflow_source_tag,
            workflow_source_sha,
            on_overlap
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(id) DO NOTHING
        ",
    )
    .bind(automation.id.as_str())
    .bind(automation.revision.as_str())
    .bind(&automation.name)
    .bind(automation.description.as_deref())
    .bind(automation.environment_id.as_deref())
    .bind(automation.api_enabled())
    .bind(&target.repo)
    .bind(&target.branch)
    .bind(target.tag.as_deref())
    .bind(target.sha.as_deref())
    .bind(&automation.workflow)
    .bind(workflow_source.map(|source| source.repo.as_str()))
    .bind(workflow_source.map(|source| source.branch.as_str()))
    .bind(workflow_source.and_then(|source| source.tag.as_deref()))
    .bind(workflow_source.and_then(|source| source.sha.as_deref()))
    .bind(stored_overlap(automation.on_overlap))
    .execute(&mut **transaction)
    .await?;
    if result.rows_affected() == 0 {
        return Ok(false);
    }
    insert_schedule_triggers(transaction, automation).await?;
    Ok(true)
}

fn stored_workflow_source(
    row: &SqliteRow,
    id: &AutomationId,
) -> Result<Option<AutomationGitWorkflowSource>, AutomationStoreError> {
    let repository = row.try_get::<Option<String>, _>("workflow_source_repository")?;
    let branch = row.try_get::<Option<String>, _>("workflow_source_branch")?;
    let tag = row.try_get::<Option<String>, _>("workflow_source_tag")?;
    let sha = row.try_get::<Option<String>, _>("workflow_source_sha")?;
    match (repository, branch) {
        (None, None) if tag.is_none() && sha.is_none() => Ok(None),
        (Some(repo), Some(branch)) => Ok(Some(AutomationGitWorkflowSource {
            repo,
            branch,
            tag,
            sha,
        })),
        _ => Err(AutomationStoreError::StoredWorkflowSourceShape { id: id.clone() }),
    }
}

/// SQL round-trip for the overlap policy: the canonical lowercase string,
/// NULL for the `Fire` default.
fn stored_overlap(policy: Option<crate::AutomationOverlapPolicy>) -> Option<&'static str> {
    policy.map(std::convert::Into::into)
}

fn stored_overlap_from(value: Option<&str>) -> Option<crate::AutomationOverlapPolicy> {
    value.and_then(|value| value.parse().ok())
}

/// Breaker facts loaded from a trigger row. Facts exist only with both a
/// signature and a processed-run high-water mark; a NULL signature means the
/// counter is clean even if the count column still holds a stale zero.
fn stored_breaker_state(
    row: &SqliteRow,
    id: &AutomationId,
) -> Result<Option<crate::ScheduleBreakerState>, AutomationStoreError> {
    let signature = row.try_get::<Option<String>, _>("trigger_breaker_signature")?;
    let last_run_id = row.try_get::<Option<String>, _>("trigger_breaker_last_run_id")?;
    // Facts exist once a high-water mark exists; a NULL signature is a clean
    // counter (the scheduler persists an empty signature for count 0).
    let state = match last_run_id {
        None => None,
        Some(last_run_id) => {
            let consecutive_count = row
                .try_get::<Option<i64>, _>("trigger_breaker_consecutive_count")?
                .unwrap_or(0);
            let consecutive_count = u32::try_from(consecutive_count)
                .map_err(|_| AutomationStoreError::StoredTriggerShape { id: id.clone() })?;
            let paused_at = row
                .try_get::<Option<i64>, _>("trigger_breaker_paused_at_ms")?
                .map(|paused_at_ms| {
                    chrono::DateTime::from_timestamp_millis(paused_at_ms)
                        .ok_or_else(|| AutomationStoreError::StoredTriggerShape { id: id.clone() })
                })
                .transpose()?;
            Some(crate::ScheduleBreakerState {
                signature: signature.unwrap_or_default(),
                consecutive_count,
                last_run_id,
                paused_at,
            })
        }
    };
    Ok(state)
}

fn stored_git_target(automation: &Automation) -> &GitRunTarget {
    automation
        .git_target()
        .expect("stored automations have already passed Git-only validation")
}

async fn insert_schedule_triggers(
    transaction: &mut Transaction<'_, Sqlite>,
    automation: &Automation,
) -> Result<(), AutomationStoreError> {
    for trigger in automation.schedule_triggers() {
        sqlx::query(
            r"
            INSERT INTO automation_triggers (
                automation_id, id, enabled, expression, breaker_threshold
            )
            VALUES (?, ?, ?, ?, ?)
            ",
        )
        .bind(automation.id.as_str())
        .bind(trigger.id.as_str())
        .bind(trigger.enabled)
        .bind(&trigger.expression)
        .bind(trigger.breaker_threshold.map(i64::from))
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

async fn current_revision(
    transaction: &mut Transaction<'_, Sqlite>,
    id: &AutomationId,
) -> Result<Option<AutomationRevision>, AutomationStoreError> {
    let current = sqlx::query_scalar::<_, String>("SELECT revision FROM automations WHERE id = ?")
        .bind(id.as_str())
        .fetch_optional(&mut **transaction)
        .await?;
    current
        .map(|revision| {
            AutomationRevision::from_str(&revision).map_err(|source| {
                AutomationStoreError::InvalidRevision {
                    id: id.clone(),
                    source,
                }
            })
        })
        .transpose()
}

async fn revision_mismatch_error(
    transaction: &mut Transaction<'_, Sqlite>,
    id: &AutomationId,
    expected: &AutomationRevision,
) -> Result<AutomationStoreError, AutomationStoreError> {
    let Some(actual) = current_revision(transaction, id).await? else {
        return Err(AutomationStoreError::NotFound { id: id.clone() });
    };
    Ok(AutomationStoreError::StaleRevision {
        id: id.clone(),
        expected: expected.clone(),
        actual,
    })
}
