//! Fork seam anchors (denkhaus line).
//!
//! Fork features are implemented in fork-owned files (`reclaim.rs` in
//! fabro-sandbox, `sandbox_gc.rs` here) so upstream merges cannot
//! overwrite them. This module is the anchor: every fork feature is
//! pinned by a test here, so a merge that silently drops or breaks fork
//! behavior fails this suite, not production. Declared `#[cfg(test)]` in
//! `lib.rs` — the deterministic tester step owns execution.
//!
//! Anchored features:
//! - Sandbox reclaim — `fabro_sandbox::reclaim` (fabro-44d8)
//! - Sandbox GC — `crate::sandbox_gc` (fabro-44d8)

use std::collections::BTreeSet;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use fabro_sandbox::SandboxInventory;
use fabro_sandbox::test_support::{
    ScriptedSandbox, managed_scripted_sandbox, managed_scripted_sandbox_for_run,
    scripted_inventory_provider,
};
use fabro_types::{RunStatus, SandboxProviderKind, SuccessReason};
use sandbox_driver::SandboxState;

use crate::sandbox_gc::{
    GcImage, GcRun, GcSandbox, SandboxGcConfig, ToolchainImageRegistry, image_tags_to_remove,
    sandboxes_to_delete, sweep,
};

// ---------------------------------------------------------------------------
// Feature: sandbox reclaim (fabro-44d8) — `fabro_sandbox::reclaim`
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reclaim_deletes_a_managed_sandbox_by_native_id() {
    let run_id = "01HY0000000000000000000000";
    let inventory = SandboxInventory::empty().with_connected(scripted_inventory_provider(
        SandboxProviderKind::DOCKER,
        vec![
            managed_scripted_sandbox_for_run("docker-1", run_id),
            managed_scripted_sandbox("docker-2"),
        ],
    ));

    fabro_sandbox::reclaim::delete_managed_by_native_id(&inventory, "docker-1")
        .await
        .expect("managed sandbox deletes");

    let listed = inventory.list_managed().await;
    let ids: Vec<&str> = listed
        .data
        .iter()
        .map(|info| info.status.id.as_str())
        .collect();
    assert_eq!(ids, ["docker-2"]);
}

#[tokio::test]
async fn reclaim_is_idempotent_for_an_unknown_id() {
    let inventory = SandboxInventory::empty().with_connected(scripted_inventory_provider(
        SandboxProviderKind::DOCKER,
        vec![managed_scripted_sandbox("docker-1")],
    ));

    fabro_sandbox::reclaim::delete_managed_by_native_id(&inventory, "never-existed")
        .await
        .expect("an unknown managed id deletes as a no-op");
}

#[tokio::test]
async fn reclaim_refuses_a_sandbox_fabro_does_not_own() {
    let foreign = ScriptedSandbox::with_id_and_working_dir("someone-elses", "/work")
        .state(SandboxState::Running);
    let inventory = SandboxInventory::empty().with_connected(scripted_inventory_provider(
        SandboxProviderKind::DOCKER,
        vec![std::sync::Arc::new(foreign)],
    ));

    let error = fabro_sandbox::reclaim::delete_managed_by_native_id(&inventory, "someone-elses")
        .await
        .expect_err("the ownership scope refuses foreign sandboxes");

    assert!(matches!(
        error,
        fabro_sandbox::SandboxLookupError::ProviderUnavailable { .. }
    ));
}

// ---------------------------------------------------------------------------
// Feature: sandbox GC (fabro-44d8) — `crate::sandbox_gc`
// ---------------------------------------------------------------------------

fn config() -> SandboxGcConfig {
    SandboxGcConfig {
        workflow_slug:    "develop".to_string(),
        max_age:          chrono::Duration::days(7),
        keep_most_recent: 2,
        sweep_interval:   Duration::from_hours(1),
    }
}

/// Two distinct, well-formed workflow version ids (64 hex characters).
const VERSION_NEW: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const VERSION_OLD: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn run(
    run_id: &str,
    created_at: chrono::DateTime<Utc>,
    status: RunStatus,
    version: Option<&str>,
) -> GcRun {
    GcRun {
        run_id: run_id.parse().unwrap(),
        status,
        workflow_slug: Some("develop".to_string()),
        workflow_version_id: version.map(|v| v.parse().unwrap()),
        created_at,
        sandbox_image: None,
    }
}

fn sandbox(sandbox_id: &str, run_id: &str) -> GcSandbox {
    GcSandbox {
        sandbox_id: sandbox_id.to_string(),
        run_id:     Some(run_id.parse().unwrap()),
        image:      None,
    }
}

fn at(day: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, day, 12, 0, 0).unwrap()
}

fn succeeded() -> RunStatus {
    RunStatus::Succeeded {
        reason: SuccessReason::Completed,
    }
}

#[test]
fn gc_retires_a_terminal_run_on_stale_evidence_beyond_the_carve_out() {
    let now = at(19);
    let runs = vec![
        // Newest run names the current develop version (`new`).
        run(
            "01HY0000000000000000000002",
            at(19),
            succeeded(),
            Some(VERSION_NEW),
        ),
        run(
            "01HY0000000000000000000001",
            at(18),
            succeeded(),
            Some(VERSION_NEW),
        ),
        // A third terminal run on the superseded version: beyond the
        // K=2 carve-out and stale evidence — collectable.
        run(
            "01HY0000000000000000000000",
            at(17),
            RunStatus::Failed {
                reason: fabro_types::FailureReason::WorkflowError,
            },
            Some(VERSION_OLD),
        ),
    ];
    let sandboxes = vec![
        sandbox("fabro-run-0", "01HY0000000000000000000000"),
        sandbox("fabro-run-1", "01HY0000000000000000000001"),
        sandbox("fabro-run-2", "01HY0000000000000000000002"),
    ];

    let removed = sandboxes_to_delete(&runs, &sandboxes, now, &config());

    assert_eq!(removed, ["fabro-run-0".to_string()]);
}

#[test]
fn gc_carve_out_protects_stale_runs_inside_k_most_recent() {
    let now = at(19);
    // Exactly K=2 terminal runs, the older one on a stale version: the
    // carve-out wins — a version refresh must not strip every sandbox
    // before a revisor has seen the new version once.
    let runs = vec![
        run(
            "01HY0000000000000000000002",
            at(19),
            succeeded(),
            Some(VERSION_NEW),
        ),
        run(
            "01HY0000000000000000000001",
            at(18),
            succeeded(),
            Some(VERSION_OLD),
        ),
    ];
    let sandboxes = vec![
        sandbox("fabro-run-1", "01HY0000000000000000000001"),
        sandbox("fabro-run-2", "01HY0000000000000000000002"),
    ];

    let removed = sandboxes_to_delete(&runs, &sandboxes, now, &config());

    assert!(removed.is_empty());
}

#[test]
fn gc_keeps_fresh_matching_version_runs_revisable() {
    let now = at(19);
    let runs = vec![
        run(
            "01HY0000000000000000000001",
            at(19),
            succeeded(),
            Some(VERSION_NEW),
        ),
        run(
            "01HY0000000000000000000002",
            at(18),
            succeeded(),
            Some(VERSION_NEW),
        ),
        run(
            "01HY0000000000000000000003",
            at(17),
            succeeded(),
            Some(VERSION_NEW),
        ),
    ];
    let sandboxes = vec![
        sandbox("fabro-run-1", "01HY0000000000000000000001"),
        sandbox("fabro-run-2", "01HY0000000000000000000002"),
        sandbox("fabro-run-3", "01HY0000000000000000000003"),
    ];

    let removed = sandboxes_to_delete(&runs, &sandboxes, now, &config());

    assert!(removed.is_empty());
}

#[test]
fn gc_collects_an_aged_out_run_even_on_the_current_version() {
    let now = at(19);
    let runs = vec![
        run(
            "01HY0000000000000000000001",
            at(1),
            succeeded(),
            Some(VERSION_NEW),
        ),
        run(
            "01HY0000000000000000000002",
            at(19),
            succeeded(),
            Some(VERSION_NEW),
        ),
        run(
            "01HY0000000000000000000003",
            at(18),
            succeeded(),
            Some(VERSION_NEW),
        ),
    ];
    let sandboxes = vec![
        sandbox("fabro-run-1", "01HY0000000000000000000001"),
        sandbox("fabro-run-2", "01HY0000000000000000000002"),
        sandbox("fabro-run-3", "01HY0000000000000000000003"),
    ];

    let removed = sandboxes_to_delete(&runs, &sandboxes, now, &config());

    assert_eq!(removed, ["fabro-run-1".to_string()]);
}

#[test]
fn gc_never_collects_a_non_terminal_run_sandbox() {
    let now = at(19);
    let runs = vec![
        run(
            "01HY0000000000000000000001",
            at(1),
            RunStatus::Running,
            Some(VERSION_OLD),
        ),
        run(
            "01HY0000000000000000000002",
            at(19),
            succeeded(),
            Some(VERSION_NEW),
        ),
    ];
    let sandboxes = vec![
        sandbox("fabro-run-1", "01HY0000000000000000000001"),
        sandbox("fabro-run-2", "01HY0000000000000000000002"),
    ];

    let removed = sandboxes_to_delete(&runs, &sandboxes, now, &config());

    assert!(removed.is_empty());
}

#[test]
fn gc_leaves_sandboxes_without_a_terminal_run_alone() {
    let now = at(19);
    let runs = vec![run(
        "01HY0000000000000000000002",
        at(19),
        succeeded(),
        Some(VERSION_NEW),
    )];
    let sandboxes = vec![
        GcSandbox {
            sandbox_id: "orphan".to_string(),
            run_id:     None,
            image:      None,
        },
        GcSandbox {
            sandbox_id: "unknown-run".to_string(),
            run_id:     Some("01HZZ000000000000000000009".parse().unwrap()),
            image:      None,
        },
    ];

    let removed = sandboxes_to_delete(&runs, &sandboxes, now, &config());

    assert!(removed.is_empty());
}

#[test]
fn gc_image_pass_keeps_pinned_latest_newest_and_local_cache_tags() {
    let images = vec![
        GcImage {
            id:         "sha:old".to_string(),
            tags:       vec!["ghcr.io/denkhaus/fabro-toolchain:aaaa".to_string()],
            created_at: 100,
            size_bytes: 7_000_000_000,
        },
        GcImage {
            id:         "sha:current".to_string(),
            tags:       vec![
                "ghcr.io/denkhaus/fabro-toolchain:bbbb".to_string(),
                "fabro-toolchain:noble".to_string(),
            ],
            created_at: 50,
            size_bytes: 7_700_000_000,
        },
        GcImage {
            id:         "sha:newest".to_string(),
            tags:       vec![
                "ghcr.io/denkhaus/fabro-toolchain:cccc".to_string(),
                "ghcr.io/denkhaus/fabro-toolchain:latest".to_string(),
            ],
            created_at: 200,
            size_bytes: 8_000_000_000,
        },
    ];
    let keep_refs = BTreeSet::from(["ghcr.io/denkhaus/fabro-toolchain:bbbb".to_string()]);

    let removed = image_tags_to_remove(&images, &keep_refs);

    // Only the old, unreferenced, registry-qualified tag goes. The pinned
    // tag, the local build-cache tag, `latest`, and the newest image all
    // survive — the 30-minute toolchain build cache is never invalidated.
    assert_eq!(removed, [(
        "ghcr.io/denkhaus/fabro-toolchain:aaaa".to_string(),
        7_000_000_000
    )]);
}

/// An in-memory image registry double: no Docker daemon in tests.
struct ScriptedImages {
    images:   Vec<GcImage>,
    removed:  Mutex<Vec<String>>,
    fail_tag: Option<&'static str>,
}

impl ScriptedImages {
    fn new(images: Vec<GcImage>) -> Self {
        Self {
            images,
            removed: Mutex::new(Vec::new()),
            fail_tag: None,
        }
    }
}

#[async_trait]
impl ToolchainImageRegistry for ScriptedImages {
    async fn list_toolchain_images(&self) -> anyhow::Result<Vec<GcImage>> {
        Ok(self.images.clone())
    }

    async fn remove_image_tag(&self, tag: &str) -> anyhow::Result<()> {
        if self.fail_tag == Some(tag) {
            anyhow::bail!("scripted failure for {tag}");
        }
        self.removed.lock().unwrap().push(tag.to_string());
        Ok(())
    }
}

fn toolchain_images() -> Vec<GcImage> {
    vec![
        GcImage {
            id:         "sha:old".to_string(),
            tags:       vec!["ghcr.io/denkhaus/fabro-toolchain:aaaa".to_string()],
            created_at: 100,
            size_bytes: 7_000_000_000,
        },
        GcImage {
            id:         "sha:newest".to_string(),
            tags:       vec!["ghcr.io/denkhaus/fabro-toolchain:cccc".to_string()],
            created_at: 200,
            size_bytes: 8_000_000_000,
        },
    ]
}

/// The full sweep over sandbox test_support doubles: a scripted docker
/// provider behind the real inventory, and the scripted image registry.
/// Pins the revisable-to-GC transition end to end, without a Docker
/// daemon.
#[tokio::test]
async fn sweep_deletes_non_revisable_sandboxes_and_stale_tags_with_a_summary() {
    let now = at(19);
    let runs = vec![
        run(
            "01HY0000000000000000000000",
            at(1),
            RunStatus::Failed {
                reason: fabro_types::FailureReason::WorkflowError,
            },
            Some(VERSION_OLD),
        ),
        run(
            "01HY0000000000000000000001",
            at(18),
            succeeded(),
            Some(VERSION_NEW),
        ),
        run(
            "01HY0000000000000000000002",
            at(19),
            succeeded(),
            Some(VERSION_NEW),
        ),
    ];
    let kept = managed_scripted_sandbox_for_run(
        "fabro-run-01HY0000000000000000000002",
        "01HY0000000000000000000002",
    );
    let inventory = SandboxInventory::empty().with_connected(scripted_inventory_provider(
        SandboxProviderKind::DOCKER,
        vec![
            managed_scripted_sandbox_for_run(
                "fabro-run-01HY0000000000000000000000",
                "01HY0000000000000000000000",
            ),
            managed_scripted_sandbox_for_run(
                "fabro-run-01HY0000000000000000000001",
                "01HY0000000000000000000001",
            ),
            kept.clone(),
        ],
    ));
    let images = ScriptedImages::new(toolchain_images());

    let report = sweep(&inventory, &images, &runs, now, &config())
        .await
        .expect("sweep completes");

    // The aged-out run 0 is deleted; runs 1 and 2 sit inside the
    // carve-out and BOTH remain listed as managed sandboxes. The old toolchain tag
    // is untagged, its bytes freed.
    assert_eq!(report.sandboxes_removed, 1);
    assert_eq!(report.image_tags_removed, 1);
    assert_eq!(report.freed_bytes, 7_000_000_000);
    assert_eq!(images.removed.lock().unwrap().as_slice(), [
        "ghcr.io/denkhaus/fabro-toolchain:aaaa"
    ]);
    let listed = inventory.list_managed().await;
    let ids: Vec<&str> = listed
        .data
        .iter()
        .map(|info| info.status.id.as_str())
        .collect();
    assert_eq!(ids, [
        "fabro-run-01HY0000000000000000000001",
        "fabro-run-01HY0000000000000000000002"
    ]);
    // The surviving sandbox was never touched.
    assert_eq!(kept.current_state(), SandboxState::Running);
}

/// A failing image untag is skipped, not fatal: the sweep still reports
/// what it reclaimed.
#[tokio::test]
async fn sweep_survives_an_image_untag_failure() {
    let now = at(19);
    let runs = vec![run(
        "01HY0000000000000000000002",
        at(19),
        succeeded(),
        Some(VERSION_NEW),
    )];
    let inventory = SandboxInventory::empty().with_connected(scripted_inventory_provider(
        SandboxProviderKind::DOCKER,
        vec![managed_scripted_sandbox_for_run(
            "fabro-run-01HY0000000000000000000002",
            "01HY0000000000000000000002",
        )],
    ));
    let mut images = ScriptedImages::new(toolchain_images());
    images.fail_tag = Some("ghcr.io/denkhaus/fabro-toolchain:aaaa");

    let report = sweep(&inventory, &images, &runs, now, &config())
        .await
        .expect("sweep completes despite the untag failure");

    assert_eq!(report.sandboxes_removed, 0);
    assert_eq!(report.image_tags_removed, 0);
}
