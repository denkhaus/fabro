//! Fork feature (fabro-f1a9, ADR-0032 Phase 6): the declared work-project
//! registry. The `[work.projects.<id>]` settings section is the desired
//! state of the server's work-projects registry; every startup (and
//! settings reload) upserts each declared project, healing its
//! tracker-wiring fields to the declared values. The database stays the
//! runtime authority for claims — this module never touches claims.
//!
//! A half-declared row fails loudly at startup instead of upserting a
//! project the claim path would die on (the overnight tracker-binding
//! outage class: every scheduled run burned against a dead binding).

use fabro_config::{WorkLayer, WorkProjectLayer};
use fabro_work::{DEFAULT_CLAIM_LABEL_PREFIX, WorkProject, WorkStore};
use tracing::info;

/// Convert the declared layer into registry rows. Completeness is the
/// settings layer's own rule (`WorkLayer::validate`), so the CLI hammer and
/// the startup materializer refuse the same rows for the same reasons.
pub(crate) fn declared_projects(layer: &WorkLayer) -> Result<Vec<WorkProject>, String> {
    layer.validate()?;
    let mut declared = Vec::with_capacity(layer.projects.len());
    for (project_id, row) in &layer.projects.0 {
        declared.push(declared_project(project_id, row));
    }
    declared.sort_by(|left, right| left.project_id.cmp(&right.project_id));
    Ok(declared)
}

/// A row that passed [`WorkLayer::validate`] always carries the required
/// fields; the defaults apply to the optional ones.
fn declared_project(project_id: &str, row: &WorkProjectLayer) -> WorkProject {
    WorkProject {
        project_id:               project_id.to_owned(),
        display_name:             row.display_name.clone().unwrap_or_default(),
        tracker_provider:         row.tracker_provider.clone().unwrap_or_default(),
        tracker_repo:             row.tracker_repo.clone().unwrap_or_default(),
        target_repo:              row.target_repo.clone(),
        project_label:            row.project_label.clone().unwrap_or_default(),
        lane_labels:              row.lane_labels.clone().unwrap_or_default(),
        claim_label_prefix:       row
            .claim_label_prefix
            .clone()
            .unwrap_or_else(|| DEFAULT_CLAIM_LABEL_PREFIX.to_owned()),
        claim_comment_enabled:    row.claim_comment_enabled.unwrap_or(false),
        backpressure_open_pr_cap: row.backpressure_open_pr_cap.unwrap_or(0),
    }
}

/// Upsert every declared project. Idempotent: a stored row that already
/// matches writes nothing new; a drifted row heals to the declared values.
/// Projects absent from the section are left alone — the section declares
/// row CONTENT, not the registry's membership.
pub(crate) async fn materialize(store: &WorkStore, layer: &WorkLayer) -> Result<(), String> {
    let declared = declared_projects(layer)?;
    for project in declared {
        info!(
            project = %project.project_id,
            tracker = %project.tracker_repo,
            "work registry: ensuring declared project"
        );
        store.upsert_project(&project).await.map_err(|error| {
            format!(
                "work.projects.{}: upsert failed: {error}",
                project.project_id
            )
        })?;
    }
    Ok(())
}

/// A refusal the scheduler's fire gate logs and honors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BindingRefusal {
    /// The automation names no work project (a row that predates the
    /// reference and was not edited yet).
    Unbound,
    /// The named project is absent from the registry.
    Unknown(String),
    /// The project exists but carries no tracker wiring to claim against.
    Unconfigured(String),
    /// The registry could not be read; refusing beats firing blind.
    Unreadable(String),
}

impl std::fmt::Display for BindingRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unbound => f.write_str("automation names no work_project"),
            Self::Unknown(project) => write!(f, "work project `{project}` is not registered"),
            Self::Unconfigured(project) => {
                write!(f, "work project `{project}` has no tracker wiring")
            }
            Self::Unreadable(error) => write!(f, "work registry unreadable: {error}"),
        }
    }
}

/// The scheduler's cheap binding health gate (fabro-f1a9, layered with the
/// in-run tracker guard): a fire whose project binding is dead would burn a
/// run that dies at the claim stage — the overnight tracker-binding outage
/// class. Finding a claimable item stays the run's job; this only proves the
/// binding is usable. A store error refuses (fail-safe), never fires blind.
pub(crate) async fn binding_ready(
    store: &WorkStore,
    work_project: Option<&str>,
) -> Result<(), BindingRefusal> {
    let Some(project_id) = work_project else {
        return Err(BindingRefusal::Unbound);
    };
    match store.project(project_id).await {
        Ok(Some(project))
            if !project.tracker_provider.trim().is_empty()
                && !project.tracker_repo.trim().is_empty() =>
        {
            Ok(())
        }
        Ok(Some(_)) => Err(BindingRefusal::Unconfigured(project_id.to_owned())),
        Ok(None) => Err(BindingRefusal::Unknown(project_id.to_owned())),
        Err(error) => Err(BindingRefusal::Unreadable(error.to_string())),
    }
}
