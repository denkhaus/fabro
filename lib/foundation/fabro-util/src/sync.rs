//! Locking helpers shared across crates.

use std::sync::{Mutex, MutexGuard, PoisonError};

/// Lock a mutex, recovering the guard when another holder panicked. The
/// state such a mutex guards is bookkeeping (a cache, a set of ids, a
/// counter) that stays usable after a panic elsewhere, so the poison is
/// cleared rather than propagated.
pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
