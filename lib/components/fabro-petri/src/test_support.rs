//! Petri's test kit, for Fabro crates that check a store implementation
//! against Petri's contract from their own tests, and an in-memory platform
//! record store for tests of the hooks and recovery. Compiled only with the
//! `test-support` feature, which a dev-dependency turns on.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use fabro_store::platform_records::now_ms;
use fabro_store::{PlatformRecord, PlatformRecordKind, StagePosition, StoredPlatformRecord};
use fabro_types::RunId;
pub use petri_testkit::run_store;

use crate::platform_records::{PlatformRecordError, PlatformRecords};

/// Platform records kept in memory, per run, in seq order.
#[derive(Debug, Default)]
pub struct MemoryPlatformRecords {
    runs: Mutex<HashMap<RunId, Vec<StoredPlatformRecord>>>,
}

impl MemoryPlatformRecords {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Every record of the run, in seq order.
    #[must_use]
    pub fn records(&self, run_id: &RunId) -> Vec<StoredPlatformRecord> {
        lock(&self.runs).get(run_id).cloned().unwrap_or_default()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[async_trait]
impl PlatformRecords for MemoryPlatformRecords {
    async fn append(
        &self,
        run_id: &RunId,
        record: &PlatformRecord,
        position: Option<StagePosition>,
    ) -> Result<StoredPlatformRecord, PlatformRecordError> {
        let mut runs = lock(&self.runs);
        let records = runs.entry(*run_id).or_default();
        let stored = StoredPlatformRecord {
            seq: records.len() as u64 + 1,
            recorded_at: now_ms(),
            record: record.clone(),
            position,
        };
        records.push(stored.clone());
        Ok(stored)
    }

    async fn read_kind(
        &self,
        run_id: &RunId,
        kind: PlatformRecordKind,
    ) -> Result<Vec<StoredPlatformRecord>, PlatformRecordError> {
        Ok(self
            .records(run_id)
            .into_iter()
            .filter(|record| record.record.kind() == kind)
            .collect())
    }
}
