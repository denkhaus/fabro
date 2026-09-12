use sandbox_driver::SandboxStatus;
use serde::{Deserialize, Serialize};

use crate::SandboxProviderKind;

/// One sandbox of fabro's inventory: the provider fabro connected it
/// through, and the status the sandbox driver reports for it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxInfo {
    pub provider: SandboxProviderKind,
    pub status:   SandboxStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxProviderLookupError {
    pub provider: SandboxProviderKind,
    pub message:  String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SandboxListMeta {
    #[serde(default)]
    pub provider_errors: Vec<SandboxProviderLookupError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxListResponse {
    pub data: Vec<SandboxInfo>,
    pub meta: SandboxListMeta,
}

/// Narrow live view for run-scoped callers (fabro-8d30a): the run ids
/// whose sandboxes still exist on this host. Stopped counts, removed
/// does not. An incomplete provider view fails the request instead of
/// returning a set that could read as "removed".
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RunSandboxAvailability {
    /// Sorted, deduplicated run ids (canonical string form).
    pub run_ids: Vec<String>,
}
