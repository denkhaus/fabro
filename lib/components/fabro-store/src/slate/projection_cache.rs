use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use fabro_types::{RunId, RunProjection};

#[derive(Debug, Clone)]
pub(crate) struct CachedRunProjection {
    pub(crate) run_id:     RunId,
    pub(crate) projection: Arc<RunProjection>,
    pub(crate) last_seq:   u32,
}

impl CachedRunProjection {
    pub(crate) fn from_projection(run_id: RunId, projection: RunProjection, last_seq: u32) -> Self {
        Self {
            run_id,
            projection: Arc::new(projection),
            last_seq,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct RunProjectionCache {
    // Cache operations are bounded in-memory work and never await. Keeping
    // this lock synchronous lets a committed event update both projection
    // caches without introducing a cancellation point.
    state: Mutex<RunProjectionCacheState>,
}

#[derive(Debug, Default)]
struct RunProjectionCacheState {
    entries: HashMap<RunId, CachedRunProjection>,
}

impl RunProjectionCacheState {
    fn replace_all(&mut self, entries: Vec<CachedRunProjection>) {
        self.entries.clear();
        for entry in entries {
            self.insert(entry);
        }
    }

    fn insert(&mut self, entry: CachedRunProjection) {
        self.entries.insert(entry.run_id, entry);
    }

    fn remove(&mut self, run_id: &RunId) {
        self.entries.remove(run_id);
    }
}

impl RunProjectionCache {
    fn lock(&self) -> MutexGuard<'_, RunProjectionCacheState> {
        self.state.lock().expect(
            "run projection cache mutex is never poisoned: no code panics while holding this lock",
        )
    }

    pub(crate) fn replace_all(&self, entries: Vec<CachedRunProjection>) {
        self.lock().replace_all(entries);
    }

    pub(crate) fn replace(&self, entry: CachedRunProjection) {
        self.lock().insert(entry);
    }

    pub(crate) fn projection_snapshot(&self, run_id: &RunId) -> Option<(Arc<RunProjection>, u32)> {
        self.lock()
            .entries
            .get(run_id)
            .map(|entry| (Arc::clone(&entry.projection), entry.last_seq))
    }

    pub(crate) fn remove(&self, run_id: &RunId) {
        self.lock().remove(run_id);
    }
}
