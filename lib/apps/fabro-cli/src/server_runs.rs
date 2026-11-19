use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};
use fabro_types::{Run, RunId, RunStatus};

use crate::server_client::{Client, RunListing, UndecodableRunRow};

#[derive(Debug, Clone)]
pub(crate) struct ServerRunInfo {
    run: Run,
}

impl ServerRunInfo {
    pub(crate) fn from_run(run: Run) -> Self {
        Self { run }
    }

    pub(crate) fn run_id(&self) -> RunId {
        self.run.id
    }

    pub(crate) fn parent_id(&self) -> Option<RunId> {
        self.run.parent_id
    }

    pub(crate) fn workflow_name(&self) -> Option<&str> {
        self.run.workflow.name.as_deref()
    }

    pub(crate) fn workflow_graph_name(&self) -> Option<&str> {
        self.run.workflow.graph_name.as_deref()
    }

    pub(crate) fn workflow_slug(&self) -> Option<&str> {
        self.run.workflow.slug.as_deref()
    }

    pub(crate) fn workflow_display_name(&self) -> String {
        self.run.workflow.display_name().unwrap_or("-").to_string()
    }

    pub(crate) fn workflow_matches(&self, pattern: &str) -> bool {
        [
            self.workflow_name(),
            self.workflow_graph_name(),
            self.workflow_slug(),
        ]
        .into_iter()
        .flatten()
        .any(|value| value.contains(pattern))
    }

    pub(crate) fn status(&self) -> RunStatus {
        self.run.lifecycle.status
    }

    /// Whether the run carries the archived overlay flag. Orthogonal to
    /// the terminal status: an archived run keeps `succeeded`/`failed`
    /// as its status and layers `archived` on top.
    pub(crate) fn archived(&self) -> bool {
        self.run.lifecycle.archived
    }

    pub(crate) fn start_time(&self) -> String {
        self.start_time_dt()
            .map(|time| time.to_rfc3339())
            .unwrap_or_default()
    }

    pub(crate) fn start_time_dt(&self) -> Option<DateTime<Utc>> {
        self.run
            .timestamps
            .started_at
            .or(Some(self.run.id.created_at()))
    }

    pub(crate) fn labels(&self) -> &HashMap<String, String> {
        &self.run.labels
    }

    pub(crate) fn wall_time_ms(&self) -> Option<u64> {
        self.run.timing.as_ref().map(|t| t.wall_time_ms)
    }

    pub(crate) fn total_usd_micros(&self) -> Option<u64> {
        self.run.usage.cost.map(|cost| cost.usd_micros)
    }

    pub(crate) fn source_directory(&self) -> Option<&str> {
        self.run.source_directory.as_deref()
    }

    pub(crate) fn repo_origin_url(&self) -> Option<&str> {
        self.run
            .repository
            .as_ref()
            .and_then(|repository| repository.origin_url.as_deref())
    }

    pub(crate) fn goal(&self) -> String {
        self.run.goal.clone()
    }

    /// The run's current title as the server stores it (JSON rows carry
    /// this verbatim). The server materializes the goal-derived fallback
    /// at creation, so a blank title only means an older server.
    pub(crate) fn title(&self) -> &str {
        self.run.title.as_str()
    }

    /// The title the ps surfaces render: the stored title, falling back
    /// to the goal when the title is blank.
    pub(crate) fn display_title(&self) -> &str {
        if self.run.title.trim().is_empty() {
            &self.run.goal
        } else {
            &self.run.title
        }
    }
}

pub(crate) struct ServerRunLookup {
    runs:        Vec<ServerRunInfo>,
    /// Rows the CLI could not decode (server schema drift, work#1437):
    /// carried alongside the decoded runs so the listing can render
    /// them as placeholders instead of failing whole.
    undecodable: Vec<UndecodableRunRow>,
}

impl ServerRunLookup {
    pub(crate) async fn from_client(client: Arc<Client>) -> Result<Self> {
        let listing = client.list_store_runs_lenient().await?;
        Ok(Self::from_listing(listing))
    }

    pub(crate) async fn from_client_by_parent(
        client: Arc<Client>,
        parent_id: RunId,
    ) -> Result<Self> {
        let listing = client.list_store_runs_lenient_by_parent(parent_id).await?;
        Ok(Self::from_listing(listing))
    }

    fn from_listing(listing: RunListing) -> Self {
        let mut runs = listing
            .runs
            .into_iter()
            .map(ServerRunInfo::from_run)
            .collect::<Vec<_>>();
        runs.sort_by(|a, b| {
            b.start_time_dt()
                .cmp(&a.start_time_dt())
                .then_with(|| b.run_id().cmp(&a.run_id()))
        });
        Self {
            runs,
            undecodable: listing.undecodable,
        }
    }

    pub(crate) fn runs(&self) -> &[ServerRunInfo] {
        &self.runs
    }

    /// The rows the server sent that this CLI could not decode: render
    /// them as placeholders instead of erroring the whole listing.
    pub(crate) fn undecodable_runs(&self) -> &[UndecodableRunRow] {
        &self.undecodable
    }
}

/// Which slice of the run store a listing shows (the `ps` tri-state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RunListScope {
    /// Default: active runs only.
    #[default]
    Active,
    /// `-a`: every run, terminal and archived included.
    All,
    /// `--archived`: only runs carrying the archived overlay flag.
    Archived,
}

pub(crate) fn filter_server_runs(
    runs: &[ServerRunInfo],
    before: Option<&str>,
    workflow: Option<&str>,
    labels: &[(String, String)],
    scope: RunListScope,
) -> Vec<ServerRunInfo> {
    runs.iter()
        .filter(|run| match scope {
            RunListScope::Active => run.status().is_active(),
            RunListScope::All => true,
            RunListScope::Archived => run.archived(),
        })
        .filter(|run| {
            before.is_none_or(|before| {
                let start_time = run.start_time();
                start_time.is_empty() || start_time.as_str() < before
            })
        })
        .filter(|run| workflow.is_none_or(|pattern| run.workflow_matches(pattern)))
        .filter(|run| {
            labels.iter().all(|(key, value)| {
                run.labels()
                    .get(key)
                    .is_some_and(|current| current == value)
            })
        })
        .cloned()
        .collect()
}
