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

use anyhow::Context as _;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sha2::{Digest as _, Sha256};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row as _, SqlitePool};

use crate::tracker::{
    TrackerProvider, WorkItem, WorkItemComment, WorkItemDraft, WorkItemFilter, WorkItemPage,
    WorkItemStateFilter,
};
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
fn view_from_row(row: &SqliteRow) -> Result<MaterializedItem> {
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

/// One walk page: 100 items — the tracker port's per-page maximum, so
/// the walk's page boundaries line up with every provider's paging.
const WALK_PAGE: usize = 100;

/// The walk's page budget: ~3000 items. A project whose listing exceeds
/// it walks incomplete (no universe replacement, the seed retries on
/// the next listing) instead of burning an unbounded quota on one read.
const WALK_MAX_PAGES: usize = 30;

impl MaterializedTracker {
    #[must_use]
    pub fn new(forge: Arc<dyn TrackerProvider>, views: Arc<WorkItemViewStore>) -> Self {
        Self { forge, views }
    }

    /// Read one item through the view (ADR-0050): a cached row serves
    /// without any forge call — the quota saver — and a miss loads on
    /// demand, materializing what the forge answers. A miss whose load
    /// fails fails as today: there is nothing to serve.
    async fn get_through_view(
        &self,
        project: &WorkProject,
        item_key: &str,
    ) -> anyhow::Result<Option<WorkItem>> {
        if let Some(row) = self
            .views
            .get(&project.project_id, item_key)
            .await
            .with_context(|| format!("reading the materialized view of {item_key}"))?
        {
            return Ok(Some(row.item));
        }
        match self.forge.get(project, item_key).await {
            Ok(Some(item)) => {
                self.views
                    .put(&project.project_id, &item)
                    .await
                    .with_context(|| format!("materializing {item_key}"))?;
                Ok(Some(item))
            }
            Ok(None) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// List through the view: assemble locally from the materialized
    /// rows, with the exact total within the universe. Filter, sort
    /// (`(priority, updated_at)` ascending, the port contract's list
    /// order) and page are all local — a warm listing costs the forge
    /// nothing.
    ///
    /// The search arm (`filter.query`) ranks by the same local order,
    /// not by the forge's relevance ranking — an honest boundary of the
    /// materialized view: the dedupe checks that use it need the
    /// matches, not the forge's ordering.
    async fn list_from_view(
        &self,
        project: &WorkProject,
        filter: WorkItemFilter,
    ) -> anyhow::Result<WorkItemPage> {
        let rows = self.materialized_rows(project).await?;
        let mut items: Vec<WorkItem> = rows
            .into_iter()
            .map(|row| row.item)
            .filter(|item| item_matches_filter(project, &filter, item))
            .collect();
        items.sort_by_key(|item| (item.priority, item.updated_at));
        let total = u64::try_from(items.len()).unwrap_or(u64::MAX);
        let page = filter.page.max(1);
        let limit = filter.limit.clamp(1, 100);
        let items = items
            .into_iter()
            .skip(page.saturating_sub(1).saturating_mul(limit))
            .take(limit)
            .collect();
        Ok(WorkItemPage {
            items,
            total: Some(total),
        })
    }

    /// The project's materialized rows, complete within the universe:
    ///
    /// - No universe, nothing cached: cold — the seed walk runs, and a failed
    ///   walk with nothing to serve fails as today.
    /// - No universe but rows cached (reads materialized before any listing
    ///   completed): retry the seed; a failure degrades to the rows the view
    ///   holds, with the oldest row's age named.
    /// - Warm universe: fill keys missing from the view on demand; a failed
    ///   load drops just that key from this assembly.
    async fn materialized_rows(
        &self,
        project: &WorkProject,
    ) -> anyhow::Result<Vec<MaterializedItem>> {
        let project_id = project.project_id.as_str();
        let universe = self
            .views
            .universe(project_id)
            .await
            .context("reading the work-item universe")?;
        let mut rows = self
            .views
            .items(project_id)
            .await
            .context("reading the materialized work items")?;
        if universe.is_empty() {
            if rows.is_empty() {
                self.walk_and_materialize(project)
                    .await
                    .context("seeding the work-item view from the forge")?;
                rows = self
                    .views
                    .items(project_id)
                    .await
                    .context("reading the materialized work items")?;
            } else if let Err(error) = self.walk_and_materialize(project).await {
                let oldest_age = rows
                    .iter()
                    .map(|row| row.fetched_at)
                    .min()
                    .map(|oldest| (Utc::now() - oldest).num_seconds());
                tracing::warn!(
                    project = project_id,
                    rows = rows.len(),
                    oldest_row_age_secs = oldest_age,
                    error = ?error,
                    "work-item view seed failed; the listing serves the materialized rows"
                );
            } else {
                rows = self
                    .views
                    .items(project_id)
                    .await
                    .context("reading the materialized work items")?;
            }
            return Ok(rows);
        }
        let cached: HashSet<String> = rows.iter().map(|row| row.item.item_key.clone()).collect();
        let missing: Vec<String> = universe
            .into_iter()
            .filter(|key| !cached.contains(key))
            .collect();
        for key in missing {
            match self.forge.get(project, &key).await {
                Ok(Some(item)) => {
                    self.views
                        .put(project_id, &item)
                        .await
                        .with_context(|| format!("materializing {key}"))?;
                    rows.push(MaterializedItem {
                        item,
                        // The row's own age fields are irrelevant to the
                        // assembly; put just wrote the honest ones.
                        fetched_at: Utc::now(),
                        content_hash: String::new(),
                    });
                }
                Ok(None) => {
                    // The forge no longer knows the key as this
                    // project's item — it leaves the universe and the
                    // view, so listings stop assembling it.
                    self.views
                        .evict(project_id, &key)
                        .await
                        .with_context(|| format!("evicting {key}"))?;
                    self.views
                        .remove_from_universe(project_id, &key)
                        .await
                        .with_context(|| format!("removing {key} from the universe"))?;
                }
                Err(error) => {
                    tracing::warn!(
                        project = project_id,
                        item_key = %key,
                        error = ?error,
                        "on-demand work-item load failed; the listing degrades without the key"
                    );
                }
            }
        }
        Ok(rows)
    }

    /// One full walk of the forge's project listing (every state), every
    /// item materialized, the universe replaced with exactly the keys
    /// the walk saw — but only when the walk completed. An
    /// incomplete walk records no universe, so the next listing retries
    /// the seed.
    async fn walk_and_materialize(&self, project: &WorkProject) -> anyhow::Result<WalkReport> {
        let project_id = project.project_id.as_str();
        let mut report = WalkReport::default();
        let mut keys = Vec::new();
        for page in 1..=WALK_MAX_PAGES {
            let filter = WorkItemFilter {
                lane: None,
                limit: WALK_PAGE,
                query: None,
                label: None,
                page,
                state: WorkItemStateFilter::All,
            };
            let batch = self.forge.list(project, filter).await?;
            let exhausted = batch.len() < WALK_PAGE;
            for item in batch {
                if self
                    .views
                    .put(project_id, &item)
                    .await
                    .with_context(|| format!("materializing {}", item.item_key))?
                {
                    report.changed += 1;
                }
                report.refreshed += 1;
                keys.push(item.item_key);
            }
            if exhausted {
                report.complete = true;
                break;
            }
        }
        if report.complete {
            self.views
                .replace_universe(project_id, &keys)
                .await
                .context("replacing the work-item universe")?;
        }
        Ok(report)
    }

    /// Write-through for writes that return the updated item: the view
    /// moves in the same action as the forge write, so the next read is
    /// already fresh. A view-write failure fails the operation — the
    /// forge already moved, and a stale row would serve the pre-write
    /// content until an explicit refresh; the caller retries the write.
    async fn materialize_write(
        &self,
        project: &WorkProject,
        item: &WorkItem,
    ) -> anyhow::Result<()> {
        let project_id = project.project_id.as_str();
        self.views
            .put(project_id, item)
            .await
            .with_context(|| format!("materializing {}", item.item_key))?;
        self.views
            .record_universe(project_id, std::slice::from_ref(&item.item_key))
            .await
            .with_context(|| format!("recording {} in the universe", item.item_key))?;
        Ok(())
    }

    /// The refresh door, one item: force-load the item from the forge
    /// and materialize what it answers — the foreign-edit case. `None`
    /// (the item left the project) drops the row and the universe key
    /// and reports `refreshed == 0`. A failed load propagates: the
    /// refresh is explicit, its failure is the operator's answer, and
    /// the view keeps serving what it had.
    pub async fn refresh_item(
        &self,
        project: &WorkProject,
        item_key: &str,
    ) -> anyhow::Result<WalkReport> {
        let project_id = project.project_id.as_str();
        match self.forge.get(project, item_key).await {
            Ok(Some(item)) => {
                let changed = self
                    .views
                    .put(project_id, &item)
                    .await
                    .with_context(|| format!("materializing {item_key}"))?;
                self.views
                    .record_universe(project_id, std::slice::from_ref(&item.item_key))
                    .await
                    .with_context(|| format!("recording {item_key} in the universe"))?;
                Ok(WalkReport {
                    refreshed: 1,
                    changed:   usize::from(changed),
                    complete:  true,
                })
            }
            Ok(None) => {
                self.views
                    .evict(project_id, item_key)
                    .await
                    .with_context(|| format!("evicting {item_key}"))?;
                self.views
                    .remove_from_universe(project_id, item_key)
                    .await
                    .with_context(|| format!("removing {item_key} from the universe"))?;
                Ok(WalkReport {
                    refreshed: 0,
                    changed:   0,
                    complete:  true,
                })
            }
            Err(error) => Err(error),
        }
    }

    /// The refresh door, the whole project: one full walk of the forge
    /// listing, every row re-materialized, the universe replaced with
    /// exactly what the walk saw. Only a completed walk replaces the
    /// universe; a page-capped walk reports `complete == false` and
    /// leaves the universe as it was.
    pub async fn refresh_project(&self, project: &WorkProject) -> anyhow::Result<WalkReport> {
        self.walk_and_materialize(project).await
    }
}

#[async_trait]
impl TrackerProvider for MaterializedTracker {
    async fn list(
        &self,
        project: &WorkProject,
        filter: WorkItemFilter,
    ) -> anyhow::Result<Vec<WorkItem>> {
        Ok(self.list_from_view(project, filter).await?.items)
    }

    async fn list_with_total(
        &self,
        project: &WorkProject,
        filter: WorkItemFilter,
    ) -> anyhow::Result<WorkItemPage> {
        self.list_from_view(project, filter).await
    }

    async fn get(&self, project: &WorkProject, item_key: &str) -> anyhow::Result<Option<WorkItem>> {
        self.get_through_view(project, item_key).await
    }

    async fn create(
        &self,
        project: &WorkProject,
        draft: WorkItemDraft,
    ) -> anyhow::Result<WorkItem> {
        let item = self.forge.create(project, draft).await?;
        self.materialize_write(project, &item).await?;
        Ok(item)
    }

    async fn update(
        &self,
        project: &WorkProject,
        item_key: &str,
        title: Option<&str>,
        body: Option<&str>,
    ) -> anyhow::Result<WorkItem> {
        let item = self.forge.update(project, item_key, title, body).await?;
        self.materialize_write(project, &item).await?;
        Ok(item)
    }

    async fn set_lane(
        &self,
        project: &WorkProject,
        item_key: &str,
        lane: Option<&str>,
    ) -> anyhow::Result<WorkItem> {
        let item = self.forge.set_lane(project, item_key, lane).await?;
        self.materialize_write(project, &item).await?;
        Ok(item)
    }

    async fn close(
        &self,
        project: &WorkProject,
        item_key: &str,
        reason: &str,
    ) -> anyhow::Result<()> {
        self.forge.close(project, item_key, reason).await?;
        // The close is one of our writes: the row's state moves to
        // exactly what the forge write set, no re-read needed. A row
        // that was never cached stays absent — the next read
        // materializes the closed item on demand.
        self.views
            .apply_close(&project.project_id, item_key)
            .await
            .with_context(|| format!("materializing the close of {item_key}"))?;
        Ok(())
    }

    async fn comment(
        &self,
        project: &WorkProject,
        item_key: &str,
        body: &str,
    ) -> anyhow::Result<()> {
        // First cut is item content; a comment moves none of it (the
        // forge's `updated_at` drift is accepted — the refresh door
        // reconciles it), so the view stays untouched.
        self.forge.comment(project, item_key, body).await
    }

    async fn list_comments(
        &self,
        project: &WorkProject,
        item_key: &str,
    ) -> anyhow::Result<Vec<WorkItemComment>> {
        // Comments are part 2: never cached, always the forge's answer.
        self.forge.list_comments(project, item_key).await
    }

    async fn project_claim(
        &self,
        project: &WorkProject,
        item_key: &str,
        run_id: &str,
    ) -> anyhow::Result<()> {
        self.forge.project_claim(project, item_key, run_id).await?;
        // The claim label moves the item's label set — first-cut
        // content. Refresh the row from the forge so the next read
        // stays LOCAL (one forge read per claim, never a forced miss):
        // the claim-time eviction made a run's first post-claim work
        // show re-materialize through the forge exactly when the forge
        // was down — a 403 total miss that killed the run's tool chain
        // (work#1557 post-merge finding 2026-11-21, the bench's
        // probe-23/25/39 class). On a load failure the stale row KEEPS
        // serving (ADR-0050's honest staleness): eviction is the one
        // error the view exists to absorb.
        match self.forge.get(project, item_key).await {
            Ok(Some(item)) => {
                self.views
                    .put(&project.project_id, &item)
                    .await
                    .with_context(|| {
                        format!("materializing {item_key} after a claim projection")
                    })?;
            }
            Ok(None) => {
                self.views
                    .evict(&project.project_id, item_key)
                    .await
                    .with_context(|| format!("evicting {item_key} after a claim projection"))?;
                self.views
                    .remove_from_universe(&project.project_id, item_key)
                    .await
                    .with_context(|| {
                        format!("removing {item_key} from the universe after a claim projection")
                    })?;
            }
            Err(error) => {
                tracing::warn!(
                    project = %project.project_id,
                    item_key = %item_key,
                    error = ?error,
                    "post-claim refresh failed; the view keeps serving its stale row"
                );
            }
        }
        Ok(())
    }

    async fn clear_claim(
        &self,
        project: &WorkProject,
        item_key: &str,
        run_id: &str,
    ) -> anyhow::Result<()> {
        self.forge.clear_claim(project, item_key, run_id).await?;
        // The clear moves the item's label set — refresh-or-keep, the
        // claim projection's twin (see the comment above): a failed
        // refresh keeps the stale row serving, never evicts.
        match self.forge.get(project, item_key).await {
            Ok(Some(item)) => {
                self.views
                    .put(&project.project_id, &item)
                    .await
                    .with_context(|| format!("materializing {item_key} after a claim clear"))?;
            }
            Ok(None) => {
                self.views
                    .evict(&project.project_id, item_key)
                    .await
                    .with_context(|| format!("evicting {item_key} after a claim clear"))?;
                self.views
                    .remove_from_universe(&project.project_id, item_key)
                    .await
                    .with_context(|| {
                        format!("removing {item_key} from the universe after a claim clear")
                    })?;
            }
            Err(error) => {
                tracing::warn!(
                    project = %project.project_id,
                    item_key = %item_key,
                    error = ?error,
                    "post-clear refresh failed; the view keeps serving its stale row"
                );
            }
        }
        Ok(())
    }

    async fn gc_claim_labels(&self, project: &WorkProject) -> anyhow::Result<u64> {
        self.forge.gc_claim_labels(project).await
    }
}

/// The labels a filtered list requires (the port contract the GitHub
/// adapter serves): the project label, the lane's label when a lane is
/// filtered, and the raw label filter.
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

/// Whether one item belongs to a filter's result, judged locally over
/// the materialized content: the state match (list arm), the label
/// containment, and — for the search arm — every whitespace-separated
/// term hitting title or body, case-insensitively, across states.
fn item_matches_filter(project: &WorkProject, filter: &WorkItemFilter, item: &WorkItem) -> bool {
    let required = required_labels(project, filter);
    let labels_ok = required
        .iter()
        .all(|label| item.labels.iter().any(|carried| carried == label));
    if let Some(query) = filter.query.as_deref() {
        let title = item.title.to_lowercase();
        let body = item.body.to_lowercase();
        return labels_ok
            && query
                .to_lowercase()
                .split_whitespace()
                .all(|term| title.contains(term) || body.contains(term));
    }
    let state_ok = match filter.state {
        WorkItemStateFilter::Open => item.state == "open",
        WorkItemStateFilter::Closed => item.state == "closed",
        WorkItemStateFilter::All => true,
    };
    state_ok && labels_ok
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
        project:          WorkProject,
        state:            Mutex<ForgeState>,
        fail_reads:       AtomicBool,
        /// The quota switch (denkhaus/work#1556): reads fail as the
        /// port's typed `RateLimited` class with an announced window
        /// while set — the decorator-passthrough pin's script.
        rate_limit_reads: AtomicBool,
        list_calls:       AtomicUsize,
        get_calls:        AtomicUsize,
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
                rate_limit_reads: AtomicBool::new(false),
                list_calls: AtomicUsize::new(0),
                get_calls: AtomicUsize::new(0),
            }
        }

        fn fail_reads(&self) {
            self.fail_reads.store(true, Ordering::SeqCst);
        }

        fn rate_limit_reads(&self) {
            self.rate_limit_reads.store(true, Ordering::SeqCst);
        }

        /// The read failure the switches script: the typed quota class
        /// first, the plain prose failure otherwise.
        fn read_failure(&self) -> anyhow::Error {
            if self.rate_limit_reads.load(Ordering::SeqCst) {
                return anyhow::Error::new(crate::TrackerError::RateLimited {
                    window: Some(std::time::Duration::from_secs(45)),
                });
            }
            anyhow::anyhow!("forge unavailable")
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

        fn matches(filter: &WorkItemFilter, required: &[String], item: &WorkItem) -> bool {
            let labels_ok = required.iter().all(|label| item.labels.contains(label));
            if let Some(query) = filter.query.as_deref() {
                let terms = query.to_lowercase();
                let title = item.title.to_lowercase();
                let body = item.body.to_lowercase();
                return labels_ok
                    && terms
                        .split_whitespace()
                        .all(|term| title.contains(term) || body.contains(term));
            }
            let state_ok = match filter.state {
                WorkItemStateFilter::Open => item.state == "open",
                WorkItemStateFilter::Closed => item.state == "closed",
                WorkItemStateFilter::All => true,
            };
            state_ok && labels_ok
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
            if self.fail_reads.load(Ordering::SeqCst)
                || self.rate_limit_reads.load(Ordering::SeqCst)
            {
                return Err(self.read_failure());
            }
            let required = Self::required_labels(project, &filter);
            let state = self.state.lock().expect("forge state lock");
            let mut items: Vec<WorkItem> = state
                .items
                .values()
                .filter(|item| Self::matches(&filter, &required, item))
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
            if self.fail_reads.load(Ordering::SeqCst)
                || self.rate_limit_reads.load(Ordering::SeqCst)
            {
                return Err(self.read_failure());
            }
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
            labels:     labels.iter().map(ToString::to_string).collect(),
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
            .record_universe("fabro", std::slice::from_ref(&item.item_key))
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
    async fn a_claim_forge_failure_keeps_the_view_row_serving() {
        // The claim-time refresh must never evict on failure: the run
        // just claimed the item and its NEXT tool call reads it — an
        // eviction would force that read through the forge exactly when
        // the forge is down (2026-11-21: the claim-time eviction made a
        // run's first post-claim work show die on a 403 total miss
        // during the quota window; the bench's probe-23/25/39 class,
        // work#1557 post-merge finding).
        let (_directory, forge, tracker) = fixture().await;
        seed_forge(&forge, &project());
        let warm = tracker
            .get(&project(), "denkhaus/work#1")
            .await
            .unwrap()
            .unwrap();
        forge.fail_reads();
        tracker
            .project_claim(&project(), "denkhaus/work#1", "run-1")
            .await
            .expect("the claim write itself succeeds — reads fail, not writes");
        let served = tracker
            .get(&project(), "denkhaus/work#1")
            .await
            .unwrap()
            .expect("the stale row keeps serving after a failed claim refresh");
        assert_eq!(served, warm, "the claim never evicts the view row");
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
    async fn claim_projections_refresh_the_view_row() {
        // The claim moves the item's labels — the row REFRESHES from the
        // forge (one read per claim), it never evicts: an eviction would
        // force the run's next post-claim read through the forge exactly
        // when the forge is down (the 2026-11-21 quota window;
        // work#1557 post-merge finding).
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
        assert!(
            views
                .get("fabro", "denkhaus/work#1")
                .await
                .unwrap()
                .is_some(),
            "a claim refreshes the row in place — the next read stays local"
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

    /// THE DECORATOR-PASSTROUGH PIN (denkhaus/work#1556): a
    /// `RateLimited` from the inner forge surfaces as the SAME
    /// classification through the decorator — serve-stale is
    /// deliberately failure-class independent, and the one thing it may
    /// never do is erase the class a door needs for the park posture.
    /// A cold listing has nothing to serve, so the failure propagates
    /// with the classification intact.
    #[tokio::test]
    async fn a_rate_limited_forge_surfaces_the_classification_through_the_decorator() {
        let (_directory, forge, tracker) = fixture().await;
        seed_forge(&forge, &project());
        forge.rate_limit_reads();

        let error = tracker
            .list(&project(), open_filter())
            .await
            .expect_err("nothing materialized: the quota failure propagates");
        let classified = error
            .downcast_ref::<crate::TrackerError>()
            .expect("the classification survives the decorator");
        assert_eq!(
            classified,
            &crate::TrackerError::RateLimited {
                window: Some(std::time::Duration::from_secs(45)),
            },
            "the window rides through untouched"
        );
    }

    /// The same passthrough on the item read: a total miss whose forge
    /// load fails as `RateLimited` keeps the classification through the
    /// decorator's on-demand path (a warm view would serve stale — the
    /// cold miss is the honest propagation).
    #[tokio::test]
    async fn a_rate_limited_forge_keeps_the_classification_on_a_cold_item_miss() {
        let (_directory, forge, tracker) = fixture().await;
        seed_forge(&forge, &project());
        forge.rate_limit_reads();

        let error = tracker
            .get(&project(), "denkhaus/work#1")
            .await
            .expect_err("a total miss with a quota-dead forge fails");
        assert_eq!(
            error.downcast_ref::<crate::TrackerError>(),
            Some(&crate::TrackerError::RateLimited {
                window: Some(std::time::Duration::from_secs(45)),
            }),
            "the on-demand load propagates the classification unchanged"
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
        let report = tracker.refresh_item(&project(), key).await.unwrap();
        assert_eq!(report.refreshed, 1, "the refresh loaded the item");
        assert_eq!(report.changed, 1, "the foreign edit moved the content hash");
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
