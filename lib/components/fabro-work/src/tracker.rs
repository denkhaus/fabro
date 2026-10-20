//! The tracker port: work-item content lives in a forge, assignment lives in
//! the work store (ADR-0032).
//!
//! The port is deliberately forge-agnostic — no GitHub, Forgejo or vendor
//! vocabulary in its surface. An adapter maps the port onto one forge:
//! `fabro-github` ships the GitHub adapter, a Forgejo adapter can be added
//! later without touching this port or the store. `item_key` is an opaque
//! provider-stable identity (GitHub and Forgejo both spell it
//! `owner/repo#number`); `project_claim` projects a claim in whatever way the
//! forge shows "a run is working on this item" (a run-id label for both
//! GitHub and Forgejo).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One work item as its forge sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItem {
    /// Opaque provider-stable identity (GitHub/Forgejo: `owner/repo#number`).
    pub item_key:   String,
    pub title:      String,
    pub body:       String,
    pub state:      String,
    pub labels:     Vec<String>,
    pub priority:   i64,
    pub url:        String,
    pub updated_at: Option<DateTime<Utc>>,
}

/// A work item to file in the project's forge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItemDraft {
    pub title:  String,
    pub body:   String,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkItemFilter {
    pub lane:  Option<String>,
    pub limit: usize,
}

/// The forge-agnostic tracker port. Implemented per forge; the work store
/// and the server never name a forge.
#[async_trait]
pub trait TrackerProvider: Send + Sync {
    async fn list_open(
        &self,
        project: &crate::WorkProject,
        filter: WorkItemFilter,
    ) -> anyhow::Result<Vec<WorkItem>>;

    async fn get(
        &self,
        project: &crate::WorkProject,
        item_key: &str,
    ) -> anyhow::Result<Option<WorkItem>>;

    async fn create(
        &self,
        project: &crate::WorkProject,
        draft: WorkItemDraft,
    ) -> anyhow::Result<WorkItem>;

    async fn close(
        &self,
        project: &crate::WorkProject,
        item_key: &str,
        reason: &str,
    ) -> anyhow::Result<()>;

    async fn comment(
        &self,
        project: &crate::WorkProject,
        item_key: &str,
        body: &str,
    ) -> anyhow::Result<()>;

    /// Project the claim in the forge (the run-id label on GitHub and
    /// Forgejo). Best called after the claim committed in the store: the
    /// store is the authority, the forge projection is eventual.
    async fn project_claim(
        &self,
        project: &crate::WorkProject,
        item_key: &str,
        run_id: &str,
    ) -> anyhow::Result<()>;

    /// Remove the claim projection and clean up its forge resource.
    async fn clear_claim(
        &self,
        project: &crate::WorkProject,
        item_key: &str,
        run_id: &str,
    ) -> anyhow::Result<()>;
}

/// A provider for projects without a forge: no items, no writes. The claim
/// ledger still works; the planner simply has nothing to claim.
pub struct NullTracker;

#[async_trait]
impl TrackerProvider for NullTracker {
    async fn list_open(
        &self,
        _project: &crate::WorkProject,
        _filter: WorkItemFilter,
    ) -> anyhow::Result<Vec<WorkItem>> {
        Ok(Vec::new())
    }

    async fn get(
        &self,
        _project: &crate::WorkProject,
        _item_key: &str,
    ) -> anyhow::Result<Option<WorkItem>> {
        Ok(None)
    }

    async fn create(
        &self,
        _project: &crate::WorkProject,
        _draft: WorkItemDraft,
    ) -> anyhow::Result<WorkItem> {
        anyhow::bail!("this project has no tracker provider")
    }

    async fn close(
        &self,
        _project: &crate::WorkProject,
        _item_key: &str,
        _reason: &str,
    ) -> anyhow::Result<()> {
        anyhow::bail!("this project has no tracker provider")
    }

    async fn comment(
        &self,
        _project: &crate::WorkProject,
        _item_key: &str,
        _body: &str,
    ) -> anyhow::Result<()> {
        anyhow::bail!("this project has no tracker provider")
    }

    async fn project_claim(
        &self,
        _project: &crate::WorkProject,
        _item_key: &str,
        _run_id: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn clear_claim(
        &self,
        _project: &crate::WorkProject,
        _item_key: &str,
        _run_id: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}
