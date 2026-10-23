//! `[work]` settings layer: the desired-state registry of work-item
//! projects (fabro-f1a9, ADR-0032 Phase 6).
//!
//! The section declares what the server's work-projects registry must
//! contain; the server upserts every declared project at startup, healing
//! the tracker-wiring fields to their declared values (settings are the
//! desired state of the REGISTRY; the database stays the runtime authority
//! for claims). Field names mirror `fabro_work::WorkProject`.
//!
//! ```toml
//! [work.projects.fabro]
//! display_name = "Fabro"
//! tracker_provider = "github"
//! tracker_repo = "denkhaus/work"
//! target_repo = "denkhaus/fabro"          # optional fallback for manual runs
//! project_label = "project:fabro"
//! lane_labels = { loop = "lane:loop" }
//! claim_label_prefix = "fabro:run:"   # optional, this is the default
//! claim_comment_enabled = false
//! backpressure_open_pr_cap = 0
//! ```
//!
//! A declared row is SPARSE like every settings layer; the startup
//! materializer requires `display_name`, `tracker_provider`, `tracker_repo`,
//! and `project_label` to be present (a half-declared project fails loudly
//! at startup) and applies the declared values over the stored row.
//! `target_repo` defaults to explicit-only, `lane_labels` to empty,
//! `claim_label_prefix` to `fabro:run:`, `claim_comment_enabled` to false,
//! `backpressure_open_pr_cap` to 0 (fire only when drained).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::maps::MergeMap;

/// Top-level `[work]` settings layer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, fabro_macros::Combine)]
pub struct WorkLayer {
    /// The desired registry, keyed by project id. Declared projects are
    /// upserted at startup; projects absent here are left alone (the
    /// registry is declarative for its rows' CONTENT, not a pruning list).
    #[serde(default, skip_serializing_if = "MergeMap::is_empty")]
    pub projects: MergeMap<WorkProjectLayer>,
}

/// One `[work.projects.<id>]` row: a project's desired tracker wiring.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, fabro_macros::Combine)]
pub struct WorkProjectLayer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name:             Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracker_provider:         Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracker_repo:             Option<String>,
    /// The code repository this project's runs work on; absent keeps the
    /// project explicit-only (manual runs must name it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_repo:              Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_label:            Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane_labels:              Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_label_prefix:       Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_comment_enabled:    Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backpressure_open_pr_cap: Option<i64>,
}

impl WorkLayer {
    /// Validate that every declared row names the fields the registry needs
    /// to function. A half-declared row is a configuration error, never a
    /// silently partial project: the operator reads the project and the
    /// missing field names, the way they see the settings file.
    ///
    /// # Errors
    ///
    /// Returns the first problem, naming `work.projects.<id>` and the
    /// missing fields.
    pub fn validate(&self) -> Result<(), String> {
        for (project_id, row) in &self.projects.0 {
            let mut missing = Vec::new();
            if row.display_name.is_none() {
                missing.push("display_name");
            }
            if row.tracker_provider.is_none() {
                missing.push("tracker_provider");
            }
            if row.tracker_repo.is_none() {
                missing.push("tracker_repo");
            }
            if row.project_label.is_none() {
                missing.push("project_label");
            }
            if !missing.is_empty() {
                return Err(format!(
                    "work.projects.{project_id}: missing {} — a declared project names these \
                     fields or it does not ship",
                    missing.join(", ")
                ));
            }
        }
        Ok(())
    }
}
