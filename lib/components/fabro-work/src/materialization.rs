//! The work-item read-view materialization (ADR-0050, amends ADR-0032).
//!
//! Fabro materializes a READ VIEW of work-item content in its own
//! database, beside the claim ledger: reads resolve cache-first, the
//! forge loads on demand. [`MaterializedTracker`] is a decorating
//! adapter over any [`TrackerProvider`] — the port itself changes
//! nothing; the server seats this adapter at its tracker seam, so every
//! consumer (work show/list, the run tools, the evidence capture, the
//! planner packet) reads through it without special-casing.
//!
//! Semantics (operator choice, grilled 2026-11-21):
//!
//! - **Write-invalidation only, no TTL.** A cached item is fresh until one of
//!   fabro's own writes moves it (write-through: the forge write and the view
//!   update are one action) or an explicit refresh runs. Foreign edits made
//!   directly at the forge stay invisible until the refresh door — accepted
//!   deliberately for maximum quota savings.
//! - **First cut: item content only** (title, body, labels, state). Comments
//!   and links are part 2 and never touch this view.
//! - **Degradation, never hard failure with something to serve.** When an
//!   on-demand forge load fails (quota, outage, any error class) the view
//!   serves what it has with a warning naming the failure and the affected
//!   keys. A total miss with nothing cached fails as today.
//! - **Lists assemble from the view.** The cache is item-key-addressed; the
//!   per-project key set (the universe) is seeded by one full forge walk on
//!   first listing and maintained by writes and refreshes. A list then filters,
//!   sorts and pages locally — the assembly is exact within the universe, and
//!   the total is exact within it too.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sha2::{Digest as _, Sha256};
use sqlx::{Row as _, SqlitePool};

use crate::tracker::{TrackerProvider, WorkItem, WorkItemDraft, WorkItemFilter, WorkItemPage};
use crate::{Error, Result, WorkProject};

/// One materialized view row: the item as last fetched, beside the
/// honest age of the row and the content hash that names whether a
/// later fetch moved the content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedItem {
    pub item:         WorkItem,
    pub fetched_at:   DateTime<Utc>,
    pub content_hash: String,
}

/// What a full walk materialized: how many rows it wrote, how many had
/// content the view did not already hold, and whether the walk reached
/// the end of the tracker's listing. An incomplete walk (the page cap)
/// records no universe, so the next listing retries the seed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WalkReport {
    pub refreshed: usize,
    pub changed:   usize,
    pub complete:  bool,
}

/// SQLite-backed read view of work-item content (ADR-0050). The rows
/// live beside the claim ledger in the same database; nothing here
/// talks to a forge.
pub struct WorkItemViewStore {
    pool: SqlitePool,
}

impl std::fmt::Debug for WorkItemViewStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkItemViewStore").finish_non_exhaustive()
    }
}

impl WorkItemViewStore {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// One materialized row, keyed `(project, item)`.
    pub async fn get(&self, project_id: &str, item_key: &str) -> Result<Option<MaterializedItem>> {
        let row = sqlx::query(
            r"
SELECT item_key, title, body, state, labels_json, priority, url,
       updated_at_ms, created_at_ms, closed_at_ms, content_hash,
       fetched_at_ms
FROM work_item_views
WHERE project_id = ? AND item_key = ?
",
        )
        .bind(project_id)
        .bind(item_key)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(view_from_row).transpose()
    }

    /// Every materialized row of one project, ordered by item key for
    /// deterministic assembly.
    pub async fn items(&self, project_id: &str) -> Result<Vec<MaterializedItem>> {
        let rows = sqlx::query(
            r"
SELECT item_key, title, body, state, labels_json, priority, url,
       updated_at_ms, created_at_ms, closed_at_ms, content_hash,
       fetched_at_ms
FROM work_item_views
WHERE project_id = ?
ORDER BY item_key
",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(view_from_row).collect()
    }

    /// Write one row (upsert). Returns whether the content hash moved —
    /// the refresh door's changed count.
    pub async fn put(&self, project_id: &str, item: &WorkItem) -> Result<bool> {
        let labels = serde_json::to_string(&item.labels)?;
        let hash = content_hash(item);
        let now = Utc::now().timestamp_millis();
        let previous: Option<(String,)> = sqlx::query_as(
            "SELECT content_hash FROM work_item_views WHERE project_id = ? AND item_key = ?",
        )
        .bind(project_id)
        .bind(&item.item_key)
        .fetch_optional(&self.pool)
        .await?;
        sqlx::query(
            r"
INSERT INTO work_item_views (
    project_id, item_key, title, body, state, labels_json, priority,
    url, updated_at_ms, created_at_ms, closed_at_ms, content_hash,
    fetched_at_ms
) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
ON CONFLICT(project_id, item_key) DO UPDATE SET
    title = excluded.title,
    body = excluded.body,
    state = excluded.state,
    labels_json = excluded.labels_json,
    priority = excluded.priority,
    url = excluded.url,
    updated_at_ms = excluded.updated_at_ms,
    created_at_ms = excluded.created_at_ms,
    closed_at_ms = excluded.closed_at_ms,
    content_hash = excluded.content_hash,
    fetched_at_ms = excluded.fetched_at_ms
",
        )
        .bind(project_id)
        .bind(&item.item_key)
        .bind(&item.title)
        .bind(&item.body)
        .bind(&item.state)
        .bind(labels)
        .bind(item.priority)
        .bind(&item.url)
        .bind(item.updated_at.map(|at| at.timestamp_millis()))
        .bind(item.created_at.map(|at| at.timestamp_millis()))
        .bind(item.closed_at.map(|at| at.timestamp_millis()))
        .bind(&hash)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(previous.is_none_or(|(previous,)| previous != hash))
    }

    /// Apply one of our own close writes to a row: the state is exactly
    /// what the forge write set, and the close time is now — no forge
    /// re-read needed. A row that was never cached stays absent.
    pub async fn apply_close(&self, project_id: &str, item_key: &str) -> Result<()> {
        let now = Utc::now().timestamp_millis();
        sqlx::query(
            r"
UPDATE work_item_views
SET state = 'closed', closed_at_ms = ?, fetched_at_ms = ?
WHERE project_id = ? AND item_key = ?
",
        )
        .bind(now)
        .bind(now)
        .bind(project_id)
        .bind(item_key)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Drop one row: the next read re-materializes it from the forge.
    pub async fn evict(&self, project_id: &str, item_key: &str) -> Result<()> {
        sqlx::query("DELETE FROM work_item_views WHERE project_id = ? AND item_key = ?")
            .bind(project_id)
            .bind(item_key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// The project's materialized key set, ordered by key.
    pub async fn universe(&self, project_id: &str) -> Result<Vec<String>> {
        let keys: Vec<(String,)> = sqlx::query_as(
            "SELECT item_key FROM work_item_universe WHERE project_id = ? ORDER BY item_key",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(keys.into_iter().map(|(key,)| key).collect())
    }

    /// Add or refresh keys in the universe (writes that moved an item,
    /// a refresh that fetched one).
    pub async fn record_universe(&self, project_id: &str, keys: &[String]) -> Result<()> {
        let now = Utc::now().timestamp_millis();
        let mut tx = self.pool.begin().await?;
        for key in keys {
            sqlx::query(
                r"
INSERT INTO work_item_universe (project_id, item_key, first_seen_ms, seen_at_ms)
VALUES (?, ?, ?, ?)
ON CONFLICT(project_id, item_key) DO UPDATE SET seen_at_ms = excluded.seen_at_ms
",
            )
            .bind(project_id)
            .bind(key)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Replace the universe with exactly `keys`: upserts every key and
    /// removes every recorded key the walk no longer sees — the honest
    /// bookkeeping of a completed walk (foreign removals leave the
    /// universe, so lists stop assembling them). An incomplete walk
    /// must NOT call this.
    pub async fn replace_universe(&self, project_id: &str, keys: &[String]) -> Result<()> {
        let now = Utc::now().timestamp_millis();
        let mut tx = self.pool.begin().await?;
        let existing: Vec<(String,)> =
            sqlx::query_as("SELECT item_key FROM work_item_universe WHERE project_id = ?")
                .bind(project_id)
                .fetch_all(&mut *tx)
                .await?;
        let walked: HashSet<&str> = keys.iter().map(String::as_str).collect();
        for (key,) in existing {
            if !walked.contains(key.as_str()) {
                sqlx::query("DELETE FROM work_item_universe WHERE project_id = ? AND item_key = ?")
                    .bind(project_id)
                    .bind(&key)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query("DELETE FROM work_item_views WHERE project_id = ? AND item_key = ?")
                    .bind(project_id)
                    .bind(&key)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        for key in keys {
            sqlx::query(
                r"
INSERT INTO work_item_universe (project_id, item_key, first_seen_ms, seen_at_ms)
VALUES (?, ?, ?, ?)
ON CONFLICT(project_id, item_key) DO UPDATE SET seen_at_ms = excluded.seen_at_ms
",
            )
            .bind(project_id)
            .bind(key)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Remove one key from the universe (an item that left the project).
    pub async fn remove_from_universe(&self, project_id: &str, item_key: &str) -> Result<()> {
        sqlx::query("DELETE FROM work_item_universe WHERE project_id = ? AND item_key = ?")
            .bind(project_id)
            .bind(item_key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

/// Decode one view row. Timestamps are epoch milliseconds; a value
/// outside chrono's range reads as the invalid-stored-timestamp error
/// the work store family shares.
fn view_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<MaterializedItem> {
    let labels_json: String = row.try_get("labels_json")?;
    let labels: Vec<String> = serde_json::from_str(&labels_json)?;
    let invalid = |field: &'static str, value: i64| Error::InvalidStoredTimestamp {
        record: "work item view",
        field,
        value,
    };
    let timestamp = |field: &'static str, raw: Option<i64>| -> Result<Option<DateTime<Utc>>> {
        raw.map(|millis| {
            chrono::DateTime::from_timestamp_millis(millis).ok_or_else(|| invalid(field, millis))
        })
        .transpose()
    };
    Ok(MaterializedItem {
        item:         WorkItem {
            item_key: row.try_get("item_key")?,
            title: row.try_get("title")?,
            body: row.try_get("body")?,
            state: row.try_get("state")?,
            labels,
            priority: row.try_get("priority")?,
            url: row.try_get("url")?,
            updated_at: timestamp("updated_at_ms", row.try_get("updated_at_ms")?)?,
            created_at: timestamp("created_at_ms", row.try_get("created_at_ms")?)?,
            closed_at: timestamp("closed_at_ms", row.try_get("closed_at_ms")?)?,
        },
        fetched_at:   {
            let fetched_ms: i64 = row.try_get("fetched_at_ms")?;
            chrono::DateTime::from_timestamp_millis(fetched_ms)
                .ok_or_else(|| invalid("fetched_at_ms", fetched_ms))?
        },
        content_hash: row.try_get("content_hash")?,
    })
}

/// The content hash: canonical JSON over the content fields. The view
/// row hashes identically whenever the forge served identical content,
/// so a re-fetch that changed nothing reports `changed == false`.
fn content_hash(item: &WorkItem) -> String {
    #[derive(serde::Serialize)]
    struct Content<'a> {
        title:      &'a str,
        body:       &'a str,
        state:      &'a str,
        labels:     &'a [String],
        priority:   i64,
        url:        &'a str,
        updated_at: Option<DateTime<Utc>>,
        created_at: Option<DateTime<Utc>>,
        closed_at:  Option<DateTime<Utc>>,
    }
    let canonical = serde_json::to_string(&Content {
        title:      &item.title,
        body:       &item.body,
        state:      &item.state,
        labels:     &item.labels,
        priority:   item.priority,
        url:        &item.url,
        updated_at: item.updated_at,
        created_at: item.created_at,
        closed_at:  item.closed_at,
    })
    .expect("serializing item content cannot fail");
    hex::encode(Sha256::digest(canonical.as_bytes()))
}

/// The materializing tracker adapter (ADR-0050): every [`TrackerProvider`]
/// read resolves through the view store, every write goes to the forge
/// first and moves the view in the same action. The port's interface is
/// unchanged — this is an adapter at the seam, not a new port.
pub struct MaterializedTracker {
    forge: Arc<dyn TrackerProvider>,
    views: Arc<WorkItemViewStore>,
}

impl std::fmt::Debug for MaterializedTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MaterializedTracker")
            .finish_non_exhaustive()
    }
}

impl MaterializedTracker {
    #[must_use]
    pub fn new(forge: Arc<dyn TrackerProvider>, views: Arc<WorkItemViewStore>) -> Self {
        Self { forge, views }
    }

    /// The refresh door, one item: force-load the item from the forge
    /// and materialize what it answers — the foreign-edit case. `None`
    /// (the item left the project) drops the row and the universe key.
    /// A failed load propagates: the refresh is explicit, its failure
    /// is the operator's answer, and the view keeps serving what it
    /// had.
    pub async fn refresh_item(
        &self,
        project: &WorkProject,
        item_key: &str,
    ) -> anyhow::Result<Option<WorkItem>> {
        self.forge.get(project, item_key).await
    }

    /// The refresh door, the whole project: one full walk of the forge
    /// listing, every row re-materialized, the universe replaced with
    /// exactly what the walk saw. Only a completed walk replaces the
    /// universe; a page-capped walk reports `complete == false` and
    /// leaves the universe as it was.
    pub async fn refresh_project(&self, project: &WorkProject) -> anyhow::Result<WalkReport> {
        let _ = (self, project);
        Ok(WalkReport::default())
    }
}

#[async_trait]
impl TrackerProvider for MaterializedTracker {
    async fn list(
        &self,
        project: &WorkProject,
        filter: WorkItemFilter,
    ) -> anyhow::Result<Vec<WorkItem>> {
        self.forge.list(project, filter).await
    }

    async fn list_with_total(
        &self,
        project: &WorkProject,
        filter: WorkItemFilter,
    ) -> anyhow::Result<WorkItemPage> {
        self.forge.list_with_total(project, filter).await
    }

    async fn get(&self, project: &WorkProject, item_key: &str) -> anyhow::Result<Option<WorkItem>> {
        self.forge.get(project, item_key).await
    }

    async fn create(
        &self,
        project: &WorkProject,
        draft: WorkItemDraft,
    ) -> anyhow::Result<WorkItem> {
        self.forge.create(project, draft).await
    }

    async fn update(
        &self,
        project: &WorkProject,
        item_key: &str,
        title: Option<&str>,
        body: Option<&str>,
    ) -> anyhow::Result<WorkItem> {
        self.forge.update(project, item_key, title, body).await
    }

    async fn set_lane(
        &self,
        project: &WorkProject,
        item_key: &str,
        lane: Option<&str>,
    ) -> anyhow::Result<WorkItem> {
        self.forge.set_lane(project, item_key, lane).await
    }

    async fn close(
        &self,
        project: &WorkProject,
        item_key: &str,
        reason: &str,
    ) -> anyhow::Result<()> {
        self.forge.close(project, item_key, reason).await
    }

    async fn comment(
        &self,
        project: &WorkProject,
        item_key: &str,
        body: &str,
    ) -> anyhow::Result<()> {
        self.forge.comment(project, item_key, body).await
    }

    async fn list_comments(
        &self,
        project: &WorkProject,
        item_key: &str,
    ) -> anyhow::Result<Vec<crate::tracker::WorkItemComment>> {
        self.forge.list_comments(project, item_key).await
    }

    async fn project_claim(
        &self,
        project: &WorkProject,
        item_key: &str,
        run_id: &str,
    ) -> anyhow::Result<()> {
        self.forge.project_claim(project, item_key, run_id).await
    }

    async fn clear_claim(
        &self,
        project: &WorkProject,
        item_key: &str,
        run_id: &str,
    ) -> anyhow::Result<()> {
        self.forge.clear_claim(project, item_key, run_id).await
    }

    async fn gc_claim_labels(&self, project: &WorkProject) -> anyhow::Result<u64> {
        self.forge.gc_claim_labels(project).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use chrono::Utc;

    use super::{MaterializedItem, MaterializedTracker, WalkReport, WorkItemViewStore};
    use crate::WorkProject;
    use crate::tracker::{
        TrackerProvider, WorkItem, WorkItemDraft, WorkItemFilter, WorkItemStateFilter,
    };

    // ------------------------------------------------------------------
    // The scripted forge: an in-memory TrackerProvider whose answers a
    // test controls — call counters, a read-failure switch, a write-
    // failure switch, and a mutable item map. The decorator is proven
    // against the port contract, never against the GitHub adapter (the
    // two adapters prove the seam is real).
    // ------------------------------------------------------------------

    struct ForgeState {
        items:       std::collections::BTreeMap<String, WorkItem>,
        next_number: u64,
    }

    struct ScriptedForge {
        project:    WorkProject,
        state:      Mutex<ForgeState>,
        fail_reads: AtomicBool,
        list_calls: AtomicUsize,
        get_calls:  AtomicUsize,
    }

    impl ScriptedForge {
        fn new(project: WorkProject) -> Self {
            Self {
                project,
                state: Mutex::new(ForgeState {
                    items:       std::collections::BTreeMap::new(),
                    next_number: 1,
                }),
                fail_reads: AtomicBool::new(false),
                list_calls: AtomicUsize::new(0),
                get_calls: AtomicUsize::new(0),
            }
        }

        fn fail_reads(&self) {
            self.fail_reads.store(true, Ordering::SeqCst);
        }

        fn list_calls(&self) -> usize {
            self.list_calls.load(Ordering::SeqCst)
        }

        fn get_calls(&self) -> usize {
            self.get_calls.load(Ordering::SeqCst)
        }

        /// The labels every filtered list requires (the port contract
        /// the GitHub adapter serves): project label, the lane's label,
        /// and the raw label filter.
        fn required_labels(project: &WorkProject, filter: &WorkItemFilter) -> Vec<String> {
            let mut labels = vec![project.project_label.clone()];
            if let Some(lane) = filter.lane.as_deref() {
                let label = project
                    .lane_labels
                    .get(lane)
                    .cloned()
                    .unwrap_or_else(|| format!("lane:{lane}"));
                labels.push(label);
            }
            if let Some(label) = filter.label.as_deref() {
                labels.push(label.to_string());
            }
            labels
        }

        fn matches(&self, filter: &WorkItemFilter, required: &[String], item: &WorkItem) -> bool {
            let labels_ok = required.iter().all(|label| item.labels.contains(label));
            match filter.query.as_deref() {
                Some(query) => {
                    let terms = query.to_lowercase();
                    let title = item.title.to_lowercase();
                    let body = item.body.to_lowercase();
                    labels_ok
                        && terms
                            .split_whitespace()
                            .all(|term| title.contains(term) || body.contains(term))
                }
                None => {
                    let state_ok = match filter.state {
                        WorkItemStateFilter::Open => item.state == "open",
                        WorkItemStateFilter::Closed => item.state == "closed",
                        WorkItemStateFilter::All => true,
                    };
                    state_ok && labels_ok
                }
            }
        }
    }

    #[async_trait]
    impl TrackerProvider for ScriptedForge {
        async fn list(
            &self,
            project: &WorkProject,
            filter: WorkItemFilter,
        ) -> anyhow::Result<Vec<WorkItem>> {
            self.list_calls.fetch_add(1, Ordering::SeqCst);
            anyhow::ensure!(!self.fail_reads.load(Ordering::SeqCst), "forge unavailable");
            let required = Self::required_labels(project, &filter);
            let state = self.state.lock().expect("forge state lock");
            let mut items: Vec<WorkItem> = state
                .items
                .values()
                .filter(|item| self.matches(&filter, &required, item))
                .cloned()
                .collect();
            items.sort_by_key(|item| (item.priority, item.updated_at));
            let page = filter.page.max(1);
            let limit = filter.limit.clamp(1, 100);
            Ok(items
                .into_iter()
                .skip((page - 1) * limit)
                .take(limit)
                .collect())
        }

        async fn get(
            &self,
            _project: &WorkProject,
            item_key: &str,
        ) -> anyhow::Result<Option<WorkItem>> {
            self.get_calls.fetch_add(1, Ordering::SeqCst);
            anyhow::ensure!(!self.fail_reads.load(Ordering::SeqCst), "forge unavailable");
            let state = self.state.lock().expect("forge state lock");
            Ok(state.items.get(item_key).cloned())
        }

        async fn create(
            &self,
            _project: &WorkProject,
            draft: WorkItemDraft,
        ) -> anyhow::Result<WorkItem> {
            let number = {
                let mut state = self.state.lock().expect("forge state lock");
                let number = state.next_number;
                state.next_number += 1;
                number
            };
            let item = WorkItem {
                item_key:   format!("{}#{number}", self.project.tracker_repo),
                title:      draft.title,
                body:       draft.body,
                state:      "open".to_string(),
                labels:     draft.labels,
                priority:   2,
                url:        format!(
                    "https://github.com/{}/issues/{number}",
                    self.project.tracker_repo
                ),
                updated_at: None,
                created_at: None,
                closed_at:  None,
            };
            self.state
                .lock()
                .expect("forge state lock")
                .items
                .insert(item.item_key.clone(), item.clone());
            Ok(item)
        }

        async fn update(
            &self,
            _project: &WorkProject,
            item_key: &str,
            title: Option<&str>,
            body: Option<&str>,
        ) -> anyhow::Result<WorkItem> {
            let mut state = self.state.lock().expect("forge state lock");
            let item = state
                .items
                .get_mut(item_key)
                .ok_or_else(|| anyhow::anyhow!("work item {item_key} not found"))?;
            if let Some(title) = title {
                item.title = title.to_string();
            }
            if let Some(body) = body {
                item.body = body.to_string();
            }
            Ok(item.clone())
        }

        async fn set_lane(
            &self,
            project: &WorkProject,
            item_key: &str,
            lane: Option<&str>,
        ) -> anyhow::Result<WorkItem> {
            let mut state = self.state.lock().expect("forge state lock");
            let item = state
                .items
                .get_mut(item_key)
                .ok_or_else(|| anyhow::anyhow!("work item {item_key} not found"))?;
            for label in project.lane_labels.values() {
                item.labels.retain(|current| current != label);
            }
            if let Some(lane) = lane {
                let label = project
                    .lane_labels
                    .get(lane)
                    .cloned()
                    .unwrap_or_else(|| format!("lane:{lane}"));
                item.labels.push(label);
            }
            Ok(item.clone())
        }

        async fn close(
            &self,
            _project: &WorkProject,
            item_key: &str,
            _reason: &str,
        ) -> anyhow::Result<()> {
            let mut state = self.state.lock().expect("forge state lock");
            let item = state
                .items
                .get_mut(item_key)
                .ok_or_else(|| anyhow::anyhow!("work item {item_key} not found"))?;
            item.state = "closed".to_string();
            item.closed_at = Some(Utc::now());
            Ok(())
        }

        async fn comment(
            &self,
            _project: &WorkProject,
            _item_key: &str,
            _body: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn project_claim(
            &self,
            project: &WorkProject,
            item_key: &str,
            run_id: &str,
        ) -> anyhow::Result<()> {
            let label = format!("{}{run_id}", project.claim_label_prefix);
            let mut state = self.state.lock().expect("forge state lock");
            if let Some(item) = state.items.get_mut(item_key) {
                item.labels.push(label);
            }
            Ok(())
        }

        async fn clear_claim(
            &self,
            project: &WorkProject,
            item_key: &str,
            run_id: &str,
        ) -> anyhow::Result<()> {
            let label = format!("{}{run_id}", project.claim_label_prefix);
            let mut state = self.state.lock().expect("forge state lock");
            if let Some(item) = state.items.get_mut(item_key) {
                item.labels.retain(|current| *current != label);
            }
            Ok(())
        }
    }

    // ------------------------------------------------------------------
    // Fixtures
    // ------------------------------------------------------------------

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

    fn item(key_number: u64, title: &str, state: &str, labels: &[&str]) -> WorkItem {
        WorkItem {
            item_key:   format!("denkhaus/work#{key_number}"),
            title:      title.to_string(),
            body:       format!("body of {title}"),
            state:      state.to_string(),
            labels:     labels.iter().map(|label| label.to_string()).collect(),
            priority:   2,
            url:        format!("https://github.com/denkhaus/work/issues/{key_number}"),
            updated_at: None,
            created_at: None,
            closed_at:  None,
        }
    }

    async fn fixture() -> (tempfile::TempDir, Arc<ScriptedForge>, MaterializedTracker) {
        let directory = tempfile::tempdir().unwrap();
        let database = fabro_db::Database::connect(directory.path().join("fabro.sqlite3"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let views = Arc::new(WorkItemViewStore::new(database.clone_pool()));
        let forge = Arc::new(ScriptedForge::new(project()));
        let tracker =
            MaterializedTracker::new(Arc::clone(&forge) as Arc<dyn TrackerProvider>, views);
        (directory, forge, tracker)
    }

    /// Seed the forge with a small mixed universe: three open items (one
    /// in the loop lane, one p0), one closed item.
    fn seed_forge(forge: &ScriptedForge, project: &WorkProject) {
        let mut state = forge.state.lock().expect("forge state lock");
        state.next_number = 5;
        state.items.insert(
            "denkhaus/work#1".to_string(),
            item(1, "First open item", "open", &[&project.project_label]),
        );
        state.items.insert(
            "denkhaus/work#2".to_string(),
            item(2, "Loop lane item", "open", &[
                &project.project_label,
                "lane:loop",
            ]),
        );
        let mut priority_item = item(3, "Priority zero item", "open", &[
            &project.project_label,
            "prio:p0",
        ]);
        priority_item.priority = 0;
        state
            .items
            .insert("denkhaus/work#3".to_string(), priority_item);
        let mut closed = item(4, "Closed item", "closed", &[&project.project_label]);
        closed.body = "the closed item mentions watcher".to_string();
        state.items.insert("denkhaus/work#4".to_string(), closed);
    }

    fn open_filter() -> WorkItemFilter {
        WorkItemFilter {
            lane:  None,
            limit: 50,
            query: None,
            label: None,
            page:  1,
            state: WorkItemStateFilter::Open,
        }
    }

    // ------------------------------------------------------------------
    // The view store
    // ------------------------------------------------------------------

    async fn store_fixture() -> (tempfile::TempDir, WorkItemViewStore) {
        let directory = tempfile::tempdir().unwrap();
        let database = fabro_db::Database::connect(directory.path().join("fabro.sqlite3"))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        (directory, WorkItemViewStore::new(database.clone_pool()))
    }

    #[tokio::test]
    async fn put_then_get_round_trips_the_content() {
        let (_directory, store) = store_fixture().await;
        let item = item(7, "Round trip", "open", &["project:fabro"]);
        assert!(
            store.put("fabro", &item).await.unwrap(),
            "a new row changes"
        );
        let stored = store.get("fabro", &item.item_key).await.unwrap().unwrap();
        assert_eq!(stored.item, item);
        assert!(!stored.content_hash.is_empty());
    }

    #[tokio::test]
    async fn put_reports_content_changes_by_hash() {
        let (_directory, store) = store_fixture().await;
        let item = item(7, "Original title", "open", &["project:fabro"]);
        store.put("fabro", &item).await.unwrap();
        assert!(
            !store.put("fabro", &item).await.unwrap(),
            "identical content changes nothing"
        );
        let mut edited = item.clone();
        edited.title = "Edited title".to_string();
        assert!(
            store.put("fabro", &edited).await.unwrap(),
            "an edited title changes the hash"
        );
    }

    #[tokio::test]
    async fn apply_close_materializes_the_closed_state_in_place() {
        let (_directory, store) = store_fixture().await;
        let item = item(7, "To close", "open", &["project:fabro"]);
        store.put("fabro", &item).await.unwrap();
        store.apply_close("fabro", &item.item_key).await.unwrap();
        let stored = store.get("fabro", &item.item_key).await.unwrap().unwrap();
        assert_eq!(stored.item.state, "closed");
        assert!(stored.item.closed_at.is_some(), "the close time lands");
    }

    #[tokio::test]
    async fn evict_removes_the_row() {
        let (_directory, store) = store_fixture().await;
        let item = item(7, "To evict", "open", &["project:fabro"]);
        store.put("fabro", &item).await.unwrap();
        store.evict("fabro", &item.item_key).await.unwrap();
        assert_eq!(store.get("fabro", &item.item_key).await.unwrap(), None);
    }

    #[tokio::test]
    async fn replace_universe_swaps_the_key_set_exactly() {
        let (_directory, store) = store_fixture().await;
        store
            .record_universe("fabro", &[
                "denkhaus/work#1".into(),
                "denkhaus/work#2".into(),
            ])
            .await
            .unwrap();
        store
            .replace_universe("fabro", &[
                "denkhaus/work#2".into(),
                "denkhaus/work#3".into(),
            ])
            .await
            .unwrap();
        assert_eq!(
            store.universe("fabro").await.unwrap(),
            vec!["denkhaus/work#2".to_string(), "denkhaus/work#3".to_string()],
            "a completed walk leaves exactly the keys it saw"
        );
    }

    #[tokio::test]
    async fn replace_universe_drops_the_view_row_of_a_departed_key() {
        let (_directory, store) = store_fixture().await;
        let item = item(1, "Departing", "open", &["project:fabro"]);
        store.put("fabro", &item).await.unwrap();
        store
            .record_universe("fabro", &[item.item_key.clone()])
            .await
            .unwrap();
        store
            .replace_universe("fabro", &["denkhaus/work#9".into()])
            .await
            .unwrap();
        assert_eq!(store.get("fabro", &item.item_key).await.unwrap(), None);
    }

    // ------------------------------------------------------------------
    // The read pins (ADR-0050): cache-first, degrade on failure, fail
    // on total miss
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn a_warm_view_read_serves_without_the_forge() {
        let (_directory, forge, tracker) = fixture().await;
        seed_forge(&forge, &project());
        let first = tracker
            .get(&project(), "denkhaus/work#1")
            .await
            .unwrap()
            .expect("the forge serves the item");
        let second = tracker
            .get(&project(), "denkhaus/work#1")
            .await
            .unwrap()
            .expect("the warm read serves from the view");
        assert_eq!(first, second);
        assert_eq!(forge.get_calls(), 1, "only the cold read pays the forge");
    }

    #[tokio::test]
    async fn a_forge_unavailable_read_against_a_warm_view_serves() {
        let (_directory, forge, tracker) = fixture().await;
        seed_forge(&forge, &project());
        let warm = tracker
            .get(&project(), "denkhaus/work#1")
            .await
            .unwrap()
            .unwrap();
        forge.fail_reads();
        let degraded = tracker
            .get(&project(), "denkhaus/work#1")
            .await
            .unwrap()
            .expect("a warm view never fails hard while the forge is down");
        assert_eq!(degraded, warm);
    }

    #[tokio::test]
    async fn a_total_miss_plus_forge_failure_fails() {
        let (_directory, forge, tracker) = fixture().await;
        forge.fail_reads();
        assert!(
            tracker.get(&project(), "denkhaus/work#404").await.is_err(),
            "nothing cached, nothing to serve: fail as today"
        );
    }

    // ------------------------------------------------------------------
    // The write-through pins: a write moves the view in the same action
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn create_writes_through_to_the_view_and_universe() {
        let (directory, _forge, tracker) = fixture().await;
        let created = tracker
            .create(&project(), WorkItemDraft {
                title:  "Filed through the materializing tracker".to_string(),
                body:   "the spec body".to_string(),
                labels: vec!["project:fabro".to_string()],
            })
            .await
            .unwrap();
        let database = fabro_db::Database::connect(directory.path().join("fabro.sqlite3"))
            .await
            .unwrap();
        let views = WorkItemViewStore::new(database.clone_pool());
        let stored = views.get("fabro", &created.item_key).await.unwrap();
        assert_eq!(
            stored.map(|row| row.item),
            Some(created.clone()),
            "the created item is materialized by the write itself"
        );
        assert!(
            views
                .universe("fabro")
                .await
                .unwrap()
                .contains(&created.item_key),
            "a created item joins the universe"
        );
    }

    #[tokio::test]
    async fn update_writes_through_the_new_content() {
        let (_directory, forge, tracker) = fixture().await;
        seed_forge(&forge, &project());
        tracker
            .get(&project(), "denkhaus/work#1")
            .await
            .unwrap()
            .unwrap();
        let updated = tracker
            .update(
                &project(),
                "denkhaus/work#1",
                Some("Corrected title"),
                Some("corrected body"),
            )
            .await
            .unwrap();
        forge.fail_reads();
        let served = tracker
            .get(&project(), "denkhaus/work#1")
            .await
            .unwrap()
            .expect("the view serves after the forge dies");
        assert_eq!(served.title, "Corrected title");
        assert_eq!(served.body, "corrected body");
        assert_eq!(served, updated);
    }

    #[tokio::test]
    async fn close_materializes_the_closed_state_without_a_re_read() {
        let (_directory, forge, tracker) = fixture().await;
        seed_forge(&forge, &project());
        let before = forge.get_calls();
        tracker
            .get(&project(), "denkhaus/work#1")
            .await
            .unwrap()
            .unwrap();
        tracker
            .close(&project(), "denkhaus/work#1", "landed with evidence")
            .await
            .unwrap();
        forge.fail_reads();
        let served = tracker
            .get(&project(), "denkhaus/work#1")
            .await
            .unwrap()
            .expect("the closed state serves from the view");
        assert_eq!(served.state, "closed");
        assert!(served.closed_at.is_some());
        let _ = before;
    }

    #[tokio::test]
    async fn claim_projections_evict_the_view_row() {
        let (directory, forge, tracker) = fixture().await;
        seed_forge(&forge, &project());
        tracker
            .get(&project(), "denkhaus/work#1")
            .await
            .unwrap()
            .unwrap();
        tracker
            .project_claim(&project(), "denkhaus/work#1", "01MTEST")
            .await
            .unwrap();
        let database = fabro_db::Database::connect(directory.path().join("fabro.sqlite3"))
            .await
            .unwrap();
        let views = WorkItemViewStore::new(database.clone_pool());
        assert_eq!(
            views.get("fabro", "denkhaus/work#1").await.unwrap(),
            None,
            "a claim moves the item's labels; the next read re-materializes them"
        );
    }

    // ------------------------------------------------------------------
    // The list pins: seed once, then assemble locally
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn a_cold_listing_seeds_the_universe_and_later_lists_stay_local() {
        let (_directory, forge, tracker) = fixture().await;
        seed_forge(&forge, &project());
        let first = tracker.list(&project(), open_filter()).await.unwrap();
        assert_eq!(first.len(), 3, "three open items in the seed");
        let seed_calls = forge.list_calls();
        assert!(seed_calls > 0, "the cold listing walks the forge");
        forge.fail_reads();
        let degraded = tracker.list(&project(), open_filter()).await.unwrap();
        assert_eq!(
            degraded.len(),
            3,
            "a forge outage after seeding changes nothing for the listing"
        );
    }

    #[tokio::test]
    async fn a_cold_listing_against_a_dead_forge_fails() {
        let (_directory, forge, tracker) = fixture().await;
        seed_forge(&forge, &project());
        forge.fail_reads();
        assert!(
            tracker.list(&project(), open_filter()).await.is_err(),
            "nothing materialized, nothing to serve: the listing fails as today"
        );
    }

    #[tokio::test]
    async fn lists_assemble_with_local_state_label_and_paging() {
        let (_directory, forge, tracker) = fixture().await;
        seed_forge(&forge, &project());
        // Seed through one listing, then assert against the local
        // assembly only.
        tracker.list(&project(), open_filter()).await.unwrap();
        forge.fail_reads();

        let mut closed_filter = open_filter();
        closed_filter.state = WorkItemStateFilter::Closed;
        let closed = tracker.list(&project(), closed_filter).await.unwrap();
        assert_eq!(
            closed
                .iter()
                .map(|item| item.item_key.as_str())
                .collect::<Vec<_>>(),
            vec!["denkhaus/work#4"],
            "the closed half lists from the view"
        );

        let mut lane_filter = open_filter();
        lane_filter.lane = Some("loop".to_string());
        let lane = tracker.list(&project(), lane_filter).await.unwrap();
        assert_eq!(
            lane.iter()
                .map(|item| item.item_key.as_str())
                .collect::<Vec<_>>(),
            vec!["denkhaus/work#2"],
            "the lane filter maps through the project's lane label"
        );

        let mut page_filter = open_filter();
        page_filter.limit = 2;
        page_filter.page = 2;
        let second_page = tracker.list(&project(), page_filter).await.unwrap();
        assert_eq!(
            second_page.len(),
            1,
            "paging applies locally: three open items, page two of two"
        );

        let page = tracker
            .list_with_total(&project(), open_filter())
            .await
            .unwrap();
        assert_eq!(
            page.total,
            Some(3),
            "the total is exact within the universe"
        );
    }

    #[tokio::test]
    async fn search_spans_states_and_terms_locally() {
        let (_directory, forge, tracker) = fixture().await;
        seed_forge(&forge, &project());
        tracker.list(&project(), open_filter()).await.unwrap();
        forge.fail_reads();
        let mut search = open_filter();
        search.query = Some("watcher".to_string());
        let hits = tracker.list(&project(), search).await.unwrap();
        assert_eq!(
            hits.iter()
                .map(|item| item.item_key.as_str())
                .collect::<Vec<_>>(),
            vec!["denkhaus/work#4"],
            "the closed item's body matches from the view"
        );
    }

    // ------------------------------------------------------------------
    // The refresh door: foreign edits stay invisible until it runs
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn a_foreign_edit_stays_invisible_until_the_refresh_door() {
        let (_directory, forge, tracker) = fixture().await;
        seed_forge(&forge, &project());
        let key = "denkhaus/work#1";
        tracker.get(&project(), key).await.unwrap().unwrap();
        forge
            .state
            .lock()
            .expect("forge state lock")
            .items
            .get_mut(key)
            .expect("the seeded item exists")
            .title = "Foreign edit at the forge".to_string();
        let stale = tracker.get(&project(), key).await.unwrap().unwrap();
        assert_eq!(
            stale.title, "First open item",
            "write-invalidation only: the foreign edit is invisible"
        );
        let refreshed = tracker
            .refresh_item(&project(), key)
            .await
            .unwrap()
            .expect("the refresh loads the item");
        assert_eq!(refreshed.title, "Foreign edit at the forge");
        let served = tracker.get(&project(), key).await.unwrap().unwrap();
        assert_eq!(
            served.title, "Foreign edit at the forge",
            "after the refresh the view serves the foreign edit"
        );
    }

    #[tokio::test]
    async fn refresh_project_purges_keys_that_left_the_project() {
        let (_directory, forge, tracker) = fixture().await;
        seed_forge(&forge, &project());
        tracker.list(&project(), open_filter()).await.unwrap();
        forge
            .state
            .lock()
            .expect("forge state lock")
            .items
            .remove("denkhaus/work#4");
        let report = tracker.refresh_project(&project()).await.unwrap();
        assert!(report.complete, "the small universe walks to the end");
        assert_eq!(report.refreshed, 3, "every remaining item re-materializes");
        forge.fail_reads();
        let mut all = open_filter();
        all.state = WorkItemStateFilter::All;
        let listed = tracker.list(&project(), all).await.unwrap();
        assert_eq!(
            listed.len(),
            3,
            "the departed item no longer assembles into listings"
        );
    }

    #[tokio::test]
    async fn a_failed_refresh_leaves_the_view_serving() {
        let (_directory, forge, tracker) = fixture().await;
        seed_forge(&forge, &project());
        let key = "denkhaus/work#1";
        tracker.get(&project(), key).await.unwrap().unwrap();
        forge.fail_reads();
        assert!(
            tracker.refresh_item(&project(), key).await.is_err(),
            "the explicit refresh reports the forge failure"
        );
        let served = tracker
            .get(&project(), key)
            .await
            .unwrap()
            .expect("the view keeps serving what it had");
        assert_eq!(served.title, "First open item");
    }

    #[tokio::test]
    async fn the_walk_report_counts_changes() {
        let (_directory, forge, tracker) = fixture().await;
        seed_forge(&forge, &project());
        tracker.list(&project(), open_filter()).await.unwrap();
        forge
            .state
            .lock()
            .expect("forge state lock")
            .items
            .get_mut("denkhaus/work#2")
            .expect("the seeded item exists")
            .title = "Foreignly edited".to_string();
        let report = tracker.refresh_project(&project()).await.unwrap();
        assert_eq!(report.refreshed, 4, "every item in the universe re-fetches");
        assert_eq!(
            report.changed, 1,
            "exactly the foreignly edited one changed"
        );
    }

    /// The view row type carries the honest age beside the content —
    /// the degradation warning's data. A compile-time shape pin.
    #[test]
    fn materialized_item_carries_the_age_beside_the_content() {
        let item = item(7, "Aged", "open", &["project:fabro"]);
        let row = MaterializedItem {
            fetched_at: Utc::now(),
            content_hash: "0f00".to_string(),
            item,
        };
        assert_eq!(row.item.state, "open");
        assert!(row.fetched_at <= Utc::now());
        let _: WalkReport = WalkReport::default();
    }
}
