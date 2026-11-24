//! Admission test fixtures: in-memory stores, no server state.

use std::sync::Arc;

use fabro_store::{ArtifactStore, Database, test_support as store_test_support};
use object_store::memory::InMemory as MemoryObjectStore;

#[cfg(test)]
pub(crate) fn test_store_bundle() -> (Arc<Database>, ArtifactStore) {
    let object_store: Arc<dyn object_store::ObjectStore> = Arc::new(MemoryObjectStore::new());
    let store = Arc::new(store_test_support::test_database());
    let artifact_store = ArtifactStore::new(object_store, "artifacts");
    (store, artifact_store)
}
