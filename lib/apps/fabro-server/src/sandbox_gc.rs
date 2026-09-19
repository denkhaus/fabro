//! Engine-side garbage collection for run sandboxes and stale toolchain
//! image tags (fabro-44d8).
//!
//! The revisable-runs contract (ADR-0011) keeps a terminal develop run's
//! sandbox alive so a revisor pass can inspect it. Nothing previously
//! removed those sandboxes once the run could no longer be revised, so a
//! docker host accumulates `fabro-run-*` containers and one
//! multi-gigabyte `ghcr.io/denkhaus/fabro-toolchain:<sha>` image per
//! content-hash refresh until it runs out of disk.
//!
//! This module closes that loop on the server, which owns the docker
//! socket:
//!
//! - A terminal run's sandbox is deleted once the run is no longer revisable:
//!   its `workflow_version_id` no longer matches the current registered version
//!   of its workflow (the stale-evidence definition in `docs/lab/CONTEXT.md`),
//!   or the run is older than the age threshold — whichever comes first. The K
//!   most recent terminal-run sandboxes always survive, so a version refresh
//!   never strips every sandbox before a revisor has seen the new version once.
//! - Stale toolchain image tags (every image carrying the
//!   `sh.fabro.toolchain.sha256` label, the content-hash semantics of
//!   `scripts/run-images.nu`) are untagged once a newer tag exists. The tag a
//!   fresh run still pins, the newest image (the tag a refresh just landed),
//!   `latest`, and every local-only tag (the build cache, e.g.
//!   `fabro-toolchain:noble`) survive, so the toolchain build cache is never
//!   invalidated.
//!
//! "A revision was filed" (the revisor consumed the run's evidence) has
//! no engine-visible producer today: revisions are filed in the seeds
//! tracker inside a run sandbox, invisible to the server. Such a run
//! stays until the version or age rule retires it — the conservative
//! direction: the GC never removes a sandbox a revisor could still use.
//! The retention supervisor (fabro-3377) is the natural seam to close
//! this: once it observes filed revisions, it can feed them in as an
//! additional retirement reason.
//!
//! Container deletion goes through [`SandboxInventory`], so the driver's
//! ownership scope refuses anything fabro did not create. Image untagging
//! talks to the Docker API directly through bollard — the driver does not
//! model image tags.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bollard::image::ListImagesOptions;
use chrono::{DateTime, Utc};
use fabro_sandbox::reclaim::delete_managed_by_native_id;
use fabro_sandbox::{RUN_ID_LABEL, SandboxInventory};
use fabro_types::{RunId, RunStatus, WorkflowVersionId};
use tokio::task::JoinHandle;
use tokio::time::{MissedTickBehavior, interval};
use tracing::{Instrument, debug, info, info_span, warn};

use crate::server::AppState;

/// The label `scripts/run-images.nu` stamps on every toolchain image it
/// builds, carrying the content hash that gates rebuilds.
const TOOLCHAIN_SHA_LABEL: &str = "sh.fabro.toolchain.sha256";

/// How the GC decides what to collect. Defaults match the revisor
/// contract: a week of revisability and the five most recent terminal
/// sandboxes always kept.
#[derive(Clone, Debug)]
pub(crate) struct SandboxGcConfig {
    /// The workflow whose runs a revisor may revisit. Only runs of this
    /// slug retire on the stale-evidence (version mismatch) rule; every
    /// workflow's runs retire on age.
    pub workflow_slug:    String,
    /// A terminal run older than this is no longer revisable, whatever
    /// its version.
    pub max_age:          chrono::Duration,
    /// The K most recent terminal-run sandboxes always survive.
    pub keep_most_recent: usize,
    /// How often the supervisor sweeps.
    pub sweep_interval:   Duration,
}

impl Default for SandboxGcConfig {
    fn default() -> Self {
        Self {
            workflow_slug:    "develop".to_string(),
            max_age:          chrono::Duration::days(7),
            keep_most_recent: 5,
            sweep_interval:   Duration::from_hours(1),
        }
    }
}

/// What the GC needs to know about a run. A projection of the run
/// summary, plain data so the revisable-to-GC transition is testable
/// without stores or providers.
#[derive(Clone, Debug)]
pub(crate) struct GcRun {
    pub run_id:              RunId,
    pub status:              RunStatus,
    pub workflow_slug:       Option<String>,
    pub workflow_version_id: Option<WorkflowVersionId>,
    pub created_at:          DateTime<Utc>,
    /// The image the run's sandbox instance records, when any: fresh runs
    /// pin the environment's current image, which image GC must keep.
    pub sandbox_image:       Option<String>,
}

impl GcRun {
    fn terminal(&self) -> bool {
        self.status.is_terminal()
    }
}

/// A managed sandbox the inventory listed, flattened to what the decision
/// needs.
#[derive(Clone, Debug)]
pub(crate) struct GcSandbox {
    pub sandbox_id: String,
    pub run_id:     Option<RunId>,
    pub image:      Option<String>,
}

/// A toolchain image the registry listed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GcImage {
    pub id:         String,
    pub tags:       Vec<String>,
    /// Unix seconds, as the Docker API reports image creation time.
    pub created_at: i64,
    pub size_bytes: u64,
}

/// One sweep's outcome, for the operator-facing summary log line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SandboxGcReport {
    pub sandboxes_removed:  usize,
    pub image_tags_removed: usize,
    pub freed_bytes:        u64,
}

impl SandboxGcReport {
    fn removed_anything(self) -> bool {
        self.sandboxes_removed > 0 || self.image_tags_removed > 0
    }
}

/// The workflow version a revisor would currently see: the newest run of
/// the workflow names the version the current registration serves.
pub(crate) fn current_workflow_version(runs: &[GcRun], slug: &str) -> Option<WorkflowVersionId> {
    runs.iter()
        .filter(|run| run.workflow_slug.as_deref() == Some(slug))
        .max_by_key(|run| run.created_at)
        .and_then(|run| run.workflow_version_id)
}

/// Whether a terminal run can no longer be revised: its version is stale
/// evidence, or it has aged out. Non-terminal runs are never eligible
/// (their sandbox is the run's live workspace).
fn non_revisable(
    run: &GcRun,
    current_version: Option<WorkflowVersionId>,
    now: DateTime<Utc>,
    config: &SandboxGcConfig,
) -> bool {
    if !run.terminal() {
        return false;
    }
    if run.created_at <= now - config.max_age {
        return true;
    }
    // Only the revisor's workflow has a stale-evidence rule; a version
    // mismatch on another workflow just means that workflow was
    // re-registered, which says nothing about revisability.
    if run.workflow_slug.as_deref() == Some(&config.workflow_slug) {
        if let (Some(run_version), Some(current)) = (run.workflow_version_id, current_version) {
            return run_version != current;
        }
    }
    false
}

/// The sandboxes to delete this sweep: every listed sandbox of a
/// non-revisable run, minus the K most recent terminal-run sandboxes
/// (the carve-out that always survives).
pub(crate) fn sandboxes_to_delete(
    runs: &[GcRun],
    sandboxes: &[GcSandbox],
    now: DateTime<Utc>,
    config: &SandboxGcConfig,
) -> Vec<String> {
    let by_run: BTreeMap<RunId, &GcRun> = runs.iter().map(|run| (run.run_id, run)).collect();

    // The carve-out: terminal-run sandboxes, newest run first.
    let mut terminal_runs: Vec<&GcRun> = sandboxes
        .iter()
        .filter_map(|sandbox| {
            sandbox
                .run_id
                .as_ref()
                .and_then(|id| by_run.get(id).copied())
        })
        .filter(|run| run.terminal())
        .collect();
    terminal_runs.sort_by_key(|run| std::cmp::Reverse(run.created_at));
    let protected: BTreeSet<RunId> = terminal_runs
        .into_iter()
        .take(config.keep_most_recent)
        .map(|run| run.run_id)
        .collect();

    let current = current_workflow_version(runs, &config.workflow_slug);
    sandboxes
        .iter()
        .filter(|sandbox| {
            sandbox.run_id.is_some_and(|run_id| {
                !protected.contains(&run_id)
                    && by_run
                        .get(&run_id)
                        .is_some_and(|run| non_revisable(run, current, now, config))
            })
        })
        .map(|sandbox| sandbox.sandbox_id.clone())
        .collect()
}

/// The toolchain image tags to untag: registry-qualified tags of images
/// that are neither the newest build (the tag a refresh just landed),
/// still pinned by a live reference (`keep_refs`: images of live
/// sandboxes and of fresh runs), nor local-only builds (the toolchain
/// build cache, e.g. `fabro-toolchain:noble`). `latest` always survives.
/// Returns the removable tags with the image's size, for freed-byte
/// accounting.
pub(crate) fn image_tags_to_remove(
    images: &[GcImage],
    keep_refs: &BTreeSet<String>,
) -> Vec<(String, u64)> {
    let Some(newest) = images.iter().max_by_key(|image| image.created_at) else {
        return Vec::new();
    };
    let mut removals = Vec::new();
    for image in images {
        // The newest image is the tag a refresh just landed: it always
        // survives, so a sweep right after a refresh cannot strip the
        // current toolchain before anything pins it.
        if image.id == newest.id {
            continue;
        }
        let registry_tags: Vec<&String> = image
            .tags
            .iter()
            .filter(|tag| {
                // A tag without a registry prefix is a local build-cache
                // tag; the toolchain cache is never invalidated.
                tag.contains('/') && !tag.ends_with(":latest") && !keep_refs.contains(tag.as_str())
            })
            .collect();
        // Untag only when no tag of the image is protected: an image that
        // also carries a kept or local tag stays (removing the other tag
        // would free nothing and can only confuse the build cache).
        let any_kept = image.tags.iter().any(|tag| {
            !tag.contains('/') || tag.ends_with(":latest") || keep_refs.contains(tag.as_str())
        });
        if any_kept || registry_tags.is_empty() {
            continue;
        }
        for tag in registry_tags {
            removals.push((tag.clone(), image.size_bytes));
        }
    }
    removals
}

/// The Docker image-tag surface the driver does not model. A trait so the
/// sweep is testable without a Docker daemon.
#[async_trait]
pub(crate) trait ToolchainImageRegistry: Send + Sync {
    /// Every local image carrying the toolchain content-hash label.
    async fn list_toolchain_images(&self) -> anyhow::Result<Vec<GcImage>>;
    /// Untags one image reference. Untagging never removes shared layers
    /// while another tag references them.
    async fn remove_image_tag(&self, tag: &str) -> anyhow::Result<()>;
}

/// The production registry view: the local Docker daemon, through the
/// same environment (`DOCKER_HOST`, socket) the docker provider uses.
pub(crate) struct DockerImageRegistry {
    docker: bollard::Docker,
}

impl DockerImageRegistry {
    /// Connects through the local Docker defaults; errors surface per
    /// sweep, never at startup.
    pub(crate) fn connect() -> anyhow::Result<Self> {
        Ok(Self {
            docker: bollard::Docker::connect_with_local_defaults()?,
        })
    }
}

#[async_trait]
impl ToolchainImageRegistry for DockerImageRegistry {
    async fn list_toolchain_images(&self) -> anyhow::Result<Vec<GcImage>> {
        let mut filters: HashMap<String, Vec<String>> = HashMap::new();
        filters.insert("label".to_string(), vec![TOOLCHAIN_SHA_LABEL.to_string()]);
        let options = ListImagesOptions {
            all: false,
            filters,
            digests: false,
        };
        let images = self.docker.list_images(Some(options)).await?;
        Ok(images
            .into_iter()
            .map(|summary| GcImage {
                id:         summary.id,
                tags:       summary.repo_tags,
                created_at: summary.created,
                size_bytes: summary.size.max(0).cast_unsigned(),
            })
            .collect())
    }

    async fn remove_image_tag(&self, tag: &str) -> anyhow::Result<()> {
        self.docker
            .remove_image(tag, None, None)
            .await
            .map(|_| ())
            .map_err(|error| anyhow::anyhow!("removing image tag {tag}: {error}"))
    }
}

/// Builds the GC's run projection from run summaries.
pub(crate) fn gc_runs_from_summaries(runs: &[fabro_types::Run]) -> Vec<GcRun> {
    runs.iter()
        .map(|run| GcRun {
            run_id:              run.id,
            status:              run.lifecycle.status,
            workflow_slug:       run.workflow.slug.clone(),
            workflow_version_id: run.workflow.workflow_version_id,
            created_at:          run.timestamps.created_at,
            sandbox_image:       run.sandbox.as_ref().and_then(|sandbox| {
                sandbox
                    .instance()
                    .and_then(|instance| instance.image.clone())
            }),
        })
        .collect()
}

/// Runs one sweep: lists managed sandboxes, deletes the non-revisable
/// ones through the inventory (ownership-scoped), untags stale toolchain
/// images, and reports counts and freed bytes. Individual removal
/// failures are logged and skipped — one wedged container must not block
/// reclaiming the rest.
pub(crate) async fn sweep(
    inventory: &SandboxInventory,
    images: &dyn ToolchainImageRegistry,
    runs: &[GcRun],
    now: DateTime<Utc>,
    config: &SandboxGcConfig,
) -> anyhow::Result<SandboxGcReport> {
    let mut report = SandboxGcReport::default();

    let listed = inventory.list_managed().await;
    if !listed.meta.provider_errors.is_empty() {
        anyhow::bail!(
            "sandbox providers failed to list: {}",
            listed
                .meta
                .provider_errors
                .iter()
                .map(|error| format!("{}: {}", error.provider, error.message))
                .collect::<Vec<_>>()
                .join("; ")
        );
    }
    let sandboxes: Vec<GcSandbox> = listed
        .data
        .iter()
        .map(|info| GcSandbox {
            sandbox_id: info.status.id.as_str().to_string(),
            run_id:     info
                .status
                .labels
                .get(RUN_ID_LABEL)
                .and_then(|value| value.parse().ok()),
            image:      info.status.image.clone(),
        })
        .collect();

    for sandbox_id in sandboxes_to_delete(runs, &sandboxes, now, config) {
        match delete_managed_by_native_id(inventory, &sandbox_id).await {
            Ok(()) => report.sandboxes_removed += 1,
            Err(error) => {
                warn!(sandbox_id = %sandbox_id, error = %error, "sandbox gc failed to delete a non-revisable run sandbox");
            }
        }
    }

    let listed_images = match images.list_toolchain_images().await {
        Ok(images) => images,
        Err(error) => {
            warn!(error = %error, "sandbox gc failed to list toolchain images; skipping image pass");
            Vec::new()
        }
    };

    // Live references: images of every remaining sandbox, plus the images
    // fresh runs (within the age threshold) pin for their next runs.
    let mut keep_refs: BTreeSet<String> = sandboxes
        .iter()
        .filter_map(|sandbox| sandbox.image.clone())
        .collect();
    let age_cutoff = now - config.max_age;
    for run in runs {
        if run.created_at > age_cutoff {
            if let Some(image) = &run.sandbox_image {
                keep_refs.insert(image.clone());
            }
        }
    }

    for (tag, size_bytes) in image_tags_to_remove(&listed_images, &keep_refs) {
        match images.remove_image_tag(&tag).await {
            Ok(()) => {
                report.image_tags_removed += 1;
                report.freed_bytes += size_bytes;
            }
            Err(error) => {
                warn!(image_tag = %tag, error = %error, "sandbox gc failed to untag a stale toolchain image");
            }
        }
    }

    Ok(report)
}

/// The periodic driver: one sweep per interval, racing shutdown. Spawned
/// from `serve`; a sweep that errors is logged and retried next tick.
pub(crate) fn spawn_sandbox_gc_supervisor(state: Arc<AppState>) -> JoinHandle<()> {
    tokio::spawn(
        async move {
            let config = SandboxGcConfig::default();
            let shutdown = state.shutdown_token();
            let mut timer = interval(config.sweep_interval);
            timer.set_missed_tick_behavior(MissedTickBehavior::Delay);
            timer.tick().await; // the first tick fires immediately; skip it
            loop {
                tokio::select! {
                    () = shutdown.cancelled() => break,
                    _ = timer.tick() => {}
                }
                let now = Utc::now();
                let summaries = match state.stores.run_summaries.list_all(now).await {
                    Ok(summaries) => summaries,
                    Err(error) => {
                        warn!(error = %error, "sandbox gc failed to list run summaries");
                        continue;
                    }
                };
                let registry = match DockerImageRegistry::connect() {
                    Ok(registry) => registry,
                    Err(error) => {
                        debug!(error = %error, "sandbox gc found no docker daemon; skipping sweep");
                        continue;
                    }
                };
                match sweep(
                    state.sandbox_inventory(),
                    &registry,
                    &gc_runs_from_summaries(&summaries),
                    now,
                    &config,
                )
                .await
                {
                    Ok(report) if report.removed_anything() => {
                        info!(
                            sandboxes_removed = report.sandboxes_removed,
                            image_tags_removed = report.image_tags_removed,
                            freed_bytes = report.freed_bytes,
                            "sandbox gc sweep reclaimed non-revisable run sandboxes and stale toolchain image tags"
                        );
                    }
                    Ok(_) => {}
                    Err(error) => {
                        warn!(error = %error, "sandbox gc sweep failed");
                    }
                }
            }
        }
        .instrument(info_span!("sandbox_gc_supervisor")),
    )
}
