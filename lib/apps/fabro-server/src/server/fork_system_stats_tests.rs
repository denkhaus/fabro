//! Fork-only pins for the system stats door (denkhaus/work#1326): the
//! presence pin proving the door survives upstream merges, the
//! shared-math pin (the door's percents ARE the disk brake's percents),
//! and the last-reap memory's wire shape.

use std::path::{Path, PathBuf};

use chrono::Utc;

use super::fork_run_retention::percent_used;
use super::fork_system_stats::{
    LastReapPass, filesystem_usage, last_reap_pass_to_api, probed_paths,
};
use super::to_i64;

fn temp_dir(name: &str) -> PathBuf {
    tempfile::tempdir()
        .expect("tempdir should create")
        .path()
        .join(name)
}

#[test]
fn filesystem_usage_reuses_the_disk_brakes_percent_math() {
    let dir = temp_dir("usage");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let row = filesystem_usage(&dir).expect("statvfs on a fresh temp dir");
    let path = Path::new(&dir);
    // The door's percent must be the brake's number over the same
    // probe — a drift here means the door and the brake disagree about
    // whether the disk is full.
    let stats = fs2::statvfs(path).expect("statvfs");
    let expected = percent_used(stats.total_space(), stats.available_space())
        .expect("nonzero total on a temp dir");
    assert_eq!(row.used_percent, Some(to_i64(expected)));
    assert_eq!(row.total_bytes, Some(to_i64(stats.total_space())));
    assert_eq!(row.available_bytes, Some(to_i64(stats.available_space())));
    assert_eq!(
        row.used_bytes,
        Some(to_i64(
            stats.total_space().saturating_sub(stats.available_space())
        )),
    );
    assert_eq!(
        row.path.as_deref(),
        Some(dir.display().to_string().as_str())
    );
}

#[test]
fn filesystem_usage_is_none_when_statvfs_fails() {
    let missing = temp_dir("missing").join("nope");
    assert!(filesystem_usage(&missing).is_none());
}

#[test]
fn probed_paths_lead_with_storage_dir_and_add_docker_root_only_when_present() {
    let storage = temp_dir("storage");
    std::fs::create_dir_all(&storage).expect("mkdir");
    let paths = probed_paths(&storage);
    assert_eq!(
        paths.first(),
        Some(&storage),
        "storage dir is always probed"
    );
    // /var/lib/docker exists on this sandbox's Docker host or not; either
    // way the list only ever names paths that exist.
    assert!(paths.iter().all(|path| path.is_dir()));
    assert!(
        !paths.iter().any(|path| path.ends_with("/var/lib/docker") && !Path::new(
            "/var/lib/docker"
        )
        .is_dir()),
        "a non-existent docker root is never probed"
    );
}

#[test]
fn last_reap_pass_maps_every_field_to_the_wire() {
    let finished_at = Utc::now();
    let last = LastReapPass {
        finished_at,
        reaped: 12,
        absent: 3,
        failed: 1,
        freed_bytes: 4096,
        disk_used_percent: Some(86),
        brake: true,
    };
    let api = last_reap_pass_to_api(&last);
    assert_eq!(api.reaped, Some(12));
    assert_eq!(api.absent, Some(3));
    assert_eq!(api.failed, Some(1));
    assert_eq!(api.freed_bytes, Some(4096));
    assert_eq!(api.disk_used_percent, Some(86));
    assert_eq!(api.brake, Some(true));
    assert_eq!(api.finished_at, Some(finished_at));
}
