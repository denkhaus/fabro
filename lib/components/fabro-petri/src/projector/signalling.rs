//! The in-process wake-up: a run store whose appends signal the projector,
//! for a run that executes in the same process as the projector, over the
//! SQLite store directly, where no append endpoint is there to signal.

use std::sync::Arc;

use fabro_types::RunId;
use petri_execution::{Access, RunKey};
use petri_store::StoreError;

use super::Projector;
use crate::projection;

impl Projector {
    /// A run store whose appends signal this projector: for a run that
    /// executes in the same process as the projector, over the SQLite store
    /// directly, where no append endpoint is there to signal. The signal is
    /// sent after the store's append returned, so the records it covers are
    /// durable before the view sees them.
    pub fn observe_store(
        self: &Arc<Self>,
        inner: Arc<dyn petri_execution::RunStore>,
    ) -> Arc<dyn petri_execution::RunStore> {
        Arc::new(SignallingStore {
            inner,
            projector: Arc::clone(self),
        })
    }
}

/// A run store that signals a projector after each append.
struct SignallingStore {
    inner:     Arc<dyn petri_execution::RunStore>,
    projector: Arc<Projector>,
}

#[async_trait::async_trait]
impl petri_execution::RunStore for SignallingStore {
    async fn open(
        &self,
        key: &RunKey,
        access: Access,
    ) -> Result<Arc<dyn petri_execution::RunLogs>, StoreError> {
        let logs = self.inner.open(key, access).await?;
        Ok(Arc::new(SignallingLogs {
            inner:     logs,
            run_id:    projection::run_id_of(key.as_str()),
            projector: Arc::clone(&self.projector),
        }))
    }
}

struct SignallingLogs {
    inner:     Arc<dyn petri_execution::RunLogs>,
    run_id:    Option<RunId>,
    projector: Arc<Projector>,
}

#[async_trait::async_trait]
impl petri_execution::RunLogs for SignallingLogs {
    fn locator(&self) -> String {
        self.inner.locator()
    }

    async fn append(
        &self,
        log: &petri_execution::LogId,
        records: &[petri_execution::Record],
    ) -> Result<(), StoreError> {
        self.inner.append(log, records).await?;
        if let Some(run_id) = self.run_id {
            self.projector.signal(run_id);
        }
        Ok(())
    }

    async fn read(
        &self,
        log: &petri_execution::LogId,
    ) -> Result<Vec<petri_execution::Record>, StoreError> {
        self.inner.read(log).await
    }

    async fn read_from(
        &self,
        log: &petri_execution::LogId,
        seq: u64,
    ) -> Result<Vec<petri_execution::Record>, StoreError> {
        self.inner.read_from(log, seq).await
    }

    async fn put_blob(&self, bytes: &[u8]) -> Result<petri_store::Digest, StoreError> {
        self.inner.put_blob(bytes).await
    }

    async fn get_blob(&self, digest: petri_store::Digest) -> Result<Option<Vec<u8>>, StoreError> {
        self.inner.get_blob(digest).await
    }
}
