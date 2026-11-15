//! Fork-only system stats door (denkhaus/work#1326): one API call that
//! answers "which filesystem is full, how full, and what did the last
//! sandbox-reap pass free" — the question the 2026-11-14 `/storage`
//! incident needed host-side forensics (ssh `df`, `docker system df`) to
//! answer, with Rootprint blind because the retention supervisor logs
//! its disk reads at INFO.
//!
//! The per-filesystem rows go through the SAME `statvfs` math the disk
//! brake reads (`fork_run_retention::percent_used`), so the door's
//! numbers and the brake's decision can never drift. The last-reap
//! memory is written by the sandbox-reap arm after every executed pass
//! and read verbatim here; it is process state, deliberately not a
//! durable record — a restart simply answers `null` until the next pass.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use fabro_api::types::{SystemFilesystemUsage, SystemStatsResponseLastReapPass};

use super::fork_run_retention::percent_used;
use super::to_i64;

/// The Docker data root probed beside the storage directory. On the
/// production host the docker root lives on the same filesystem as the
/// storage dir (the `/dev/vda3` of the 2026-11-14 incident); probing both
/// paths makes the sharing (or a future split) visible in one answer —
/// two rows with identical numbers ARE one filesystem.
const DOCKER_DATA_ROOT: &str = "/var/lib/docker";

/// What the last sandbox-reap pass did, as the stats door remembers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LastReapPass {
    pub finished_at:       DateTime<Utc>,
    /// Sandboxes reaped by the pass.
    pub reaped:            usize,
    /// Candidates whose sandbox was already absent — the idempotent case.
    pub absent:            usize,
    /// Candidates whose reap failed and retries next tick.
    pub failed:            usize,
    /// Estimated freed bytes: the reaped runs' host workspace sizes,
    /// summed before their prune.
    pub freed_bytes:       u64,
    /// Storage-filesystem usage percent sampled after the pass; `None`
    /// when `statvfs` failed.
    pub disk_used_percent: Option<u64>,
    /// Whether the disk brake widened the pass's window.
    pub brake:             bool,
}

/// The AppState-shared cell the reap arm writes and the stats handler
/// reads. Std `RwLock`: both sides are short, await-free critical
/// sections.
pub(crate) type LastReapPassCell = std::sync::RwLock<Option<LastReapPass>>;

/// One probed path's filesystem usage. `None` when `statvfs` fails (the
/// row is omitted, never zero-filled — a zero row would read as "plenty
/// of space").
pub(crate) fn filesystem_usage(path: &Path) -> Option<SystemFilesystemUsage> {
    let stats = fs2::statvfs(path).ok()?;
    let total = stats.total_space();
    let available = stats.available_space();
    let used_percent = percent_used(total, available)?;
    let used = total.saturating_sub(available);
    Some(SystemFilesystemUsage {
        path:            Some(path.display().to_string()),
        total_bytes:     Some(to_i64(total)),
        used_bytes:      Some(to_i64(used)),
        available_bytes: Some(to_i64(available)),
        used_percent:    Some(to_i64(used_percent)),
    })
}

/// The paths the door probes: the storage directory, plus the Docker
/// data root when it exists (absent on a non-Docker host — nothing is
/// served by a `/var/lib/docker` row that is only the storage
/// filesystem's numbers under a wrong name).
pub(crate) fn probed_paths(storage_dir: &Path) -> Vec<PathBuf> {
    let mut paths = vec![storage_dir.to_path_buf()];
    let docker_root = Path::new(DOCKER_DATA_ROOT);
    if docker_root.is_dir() {
        paths.push(docker_root.to_path_buf());
    }
    paths
}

/// The wire shape of one remembered pass.
pub(crate) fn last_reap_pass_to_api(last: &LastReapPass) -> SystemStatsResponseLastReapPass {
    SystemStatsResponseLastReapPass {
        finished_at:       Some(last.finished_at),
        reaped:            Some(to_i64(last.reaped)),
        absent:            Some(to_i64(last.absent)),
        failed:            Some(to_i64(last.failed)),
        freed_bytes:       Some(to_i64(last.freed_bytes)),
        disk_used_percent: last.disk_used_percent.map(to_i64),
        brake:             Some(last.brake),
    }
}
