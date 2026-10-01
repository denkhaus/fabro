//! Retrying starts a new execution from the original spec. It does not
//! require Git checkpoints or reconstruct the previous workspace.

use std::path::Path;

use fabro_store::platform_records::RunCreatedRecord;
use fabro_store::{Database, RunProjection};
use fabro_types::{RunId, RunProvenance};

use super::fork;
use crate::error::Error;

pub fn ensure_retryable(source: &RunProjection, run_id: &RunId) -> Result<(), Error> {
    fork::ensure_forkable(source, run_id)?;
    fork::ensure_terminal(source, run_id, "retry")
}

pub async fn persist_retried_run(
    store: &Database,
    source: &RunProjection,
    run_id: RunId,
    run_dir: &Path,
    provenance: RunProvenance,
    web_url: Option<String>,
) -> Result<(), Error> {
    let mut spec = source.spec.clone();
    spec.run_id = run_id;
    spec.fork_source_ref = None;
    spec.provenance = provenance;
    let created = RunCreatedRecord {
        spec,
        title: Some(source.title().into_owned()),
        parent_id: source.parent_id,
        retried_from: Some(source.spec.run_id),
        web_url,
    };
    fork::persist_new_run(store, run_id, run_dir, created).await
}
