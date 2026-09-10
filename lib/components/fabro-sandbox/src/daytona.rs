//! The `daytona` provider kind: fabro's environment mapping onto the
//! sandbox-driver Daytona provider.
//!
//! Fabro decides the snapshot (built from the environment's image or
//! Dockerfile and named by an HMAC of its inputs), the lifecycle timers,
//! labels, network policy, and workspace layout; the driver creates and
//! drives the sandbox. The run works in `/home/daytona/workspace`, with a
//! cloned repository checked out under `/home/daytona/repos` and linked into
//! the workspace.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use fabro_github::GitHubCredentials;
use fabro_types::settings::server::ServerSandboxProviderSettings;
use fabro_types::{RunId, SandboxProviderKind};
use sandbox_driver::{
    HealthStatus, LifecycleTimers, NetworkPolicy, Resources, SandboxId, SandboxProvider,
    SandboxSource, SandboxSpec as DriverSpec, SnapshotFilter, SnapshotId, SnapshotProvider,
    SnapshotSource, SnapshotSpec, SnapshotState,
};
use tokio::time;

pub use crate::config::{
    DaytonaNetwork, DaytonaSettings as DaytonaConfig,
    DaytonaSnapshotSettings as DaytonaSnapshotConfig, DaytonaSnapshotSource, DockerfileSource,
};
pub use crate::driver::DaytonaCredentials;
use crate::driver::{ProviderConnectOptions, connect_provider};
use crate::driver_sandbox::{
    CreatePlan, DriverSandbox, LayoutSource, PreparedCreate, RepoWorkspace, WorkspaceLayout,
};
use crate::managed_labels;
use crate::sandbox::SandboxEvent;

pub(crate) const WORKING_DIRECTORY: &str = "/home/daytona/workspace";
pub(crate) const REPOS_ROOT: &str = "/home/daytona/repos";
const DEFAULT_SNAPSHOT: &str = "daytona-medium";
pub const DEFAULT_DAYTONA_API_URL: &str = "https://app.daytona.io/api";
/// Budget for the credential probe `fabro doctor` and the install flow run.
pub const DAYTONA_CREDENTIAL_PROBE_TIMEOUT: Duration = Duration::from_secs(20);
/// Budget for a custom snapshot to reach Daytona's active state.
const DAYTONA_SNAPSHOT_ACTIVE_TIMEOUT: Duration = Duration::from_mins(30);
/// Auto-stop applied when `lifecycle.auto_stop` is unset. Omitting the timer
/// would inherit Daytona's server-side default of 15 idle minutes, which is
/// shorter than a single long inference call and stops the sandbox mid-run;
/// 120 minutes clears any realistic call while still reclaiming sandboxes
/// leaked by a dead worker. An explicit `0` disables auto-stop entirely.
const DEFAULT_AUTO_STOP_INTERVAL_MINUTES: i32 = 120;

/// Scopes a Daytona API key needs for fabro's snapshot and sandbox flow, in
/// the order the remediation text lists them.
pub const REQUIRED_DAYTONA_SCOPES: &[&str] = &[
    "write:snapshots",
    "delete:snapshots",
    "write:sandboxes",
    "delete:sandboxes",
];

pub mod snapshot_identity {
    use hmac::{Hmac, Mac};
    use serde::Serialize;
    use sha2::{Digest, Sha256};
    use uuid::Uuid;

    use super::{DaytonaSnapshotConfig, DaytonaSnapshotSource, DockerfileSource};

    const IDENTITY_VERSION: u8 = 1;
    const PROVIDER: &str = "daytona";
    const TENANT: &str = "single-tenant";

    type HmacSha256 = Hmac<Sha256>;

    /// The snapshot source as it appears in the identity manifest. Each
    /// variant flattens into a single `"<key>": "<value>"` entry.
    #[derive(Serialize)]
    #[serde(rename_all = "snake_case")]
    enum SourceManifest<'a> {
        DockerfileSha256(String),
        Image(&'a str),
    }

    #[derive(Serialize)]
    struct SnapshotManifest<'a> {
        identity_version: u8,
        provider:         &'static str,
        tenant:           &'static str,
        #[serde(flatten)]
        source:           SourceManifest<'a>,
        cpu:              Option<i32>,
        memory_gb:        Option<i32>,
        disk_gb:          Option<i32>,
        /// Nothing sets an entrypoint yet. The field stays because removing
        /// it would rename every existing snapshot under `IDENTITY_VERSION` 1.
        entrypoint:       Option<&'static str>,
    }

    /// The name of the snapshot built from `config`: a UUIDv8 derived from an
    /// HMAC of the build inputs keyed by the API key, so the same inputs reuse
    /// the same snapshot and a rotated key never collides with another
    /// tenant's.
    pub fn snapshot_name(api_key: &str, config: &DaytonaSnapshotConfig) -> crate::Result<String> {
        let manifest = canonical_manifest(config)?;
        let mut mac = HmacSha256::new_from_slice(api_key.as_bytes())
            .expect("HMAC-SHA256 accepts keys of any length");
        mac.update(&manifest);
        let digest = mac.finalize().into_bytes();
        let mut bytes = [0_u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        Ok(format!("fabro-{}", Uuid::new_v8(bytes)))
    }

    fn canonical_manifest(config: &DaytonaSnapshotConfig) -> crate::Result<Vec<u8>> {
        let source = match &config.source {
            DaytonaSnapshotSource::Image(image) => SourceManifest::Image(image),
            DaytonaSnapshotSource::Dockerfile(DockerfileSource::Inline(text)) => {
                SourceManifest::DockerfileSha256(hex::encode(Sha256::digest(text.as_bytes())))
            }
            DaytonaSnapshotSource::Dockerfile(DockerfileSource::Path { .. }) => {
                return Err(crate::Error::message(
                    "Daytona snapshot dockerfile path should have been resolved to inline content before sandbox creation",
                ));
            }
        };
        let manifest = SnapshotManifest {
            identity_version: IDENTITY_VERSION,
            provider: PROVIDER,
            tenant: TENANT,
            source,
            cpu: config.cpu,
            memory_gb: config.memory,
            disk_gb: config.disk,
            entrypoint: None,
        };
        serde_json::to_vec(&manifest).map_err(|err| {
            crate::Error::context("Failed to serialize Daytona snapshot identity", err)
        })
    }
}

/// Outcome of probing a Daytona credential through the provider's health
/// check.
#[derive(Debug)]
pub struct DaytonaKeyCheck {
    /// Scopes the key lacks, in Daytona's wire names.
    pub missing: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("Daytona credential probe timed out after {timeout:?}")]
pub struct DaytonaCredentialProbeTimeout {
    timeout: Duration,
}

impl DaytonaCredentialProbeTimeout {
    #[must_use]
    pub const fn new(timeout: Duration) -> Self {
        Self { timeout }
    }

    #[must_use]
    pub const fn timeout(&self) -> Duration {
        self.timeout
    }
}

impl DaytonaKeyCheck {
    #[must_use]
    pub fn ok(&self) -> bool {
        self.missing.is_empty()
    }

    #[must_use]
    pub fn missing_display(&self) -> String {
        self.missing.join(", ")
    }

    #[must_use]
    pub fn missing_message(&self) -> String {
        format!(
            "Daytona API key is missing required scopes: {}. Regenerate the key with all \
             snapshot and sandbox scopes.",
            self.missing_display()
        )
    }
}

#[must_use]
pub fn required_perms_display() -> String {
    REQUIRED_DAYTONA_SCOPES.join(", ")
}

/// Whether `credentials` reach Daytona, are accepted, and carry the scopes
/// fabro needs. Reachability and authentication failures are errors; a key
/// that authenticates but lacks scopes is an `Ok` check that is not `ok()`.
pub async fn check_daytona_api_key(
    credentials: &DaytonaCredentials,
    probe_timeout: Duration,
) -> anyhow::Result<DaytonaKeyCheck> {
    let probe = async {
        let provider = connect(credentials).await?;
        let health = provider
            .health()
            .await
            .map_err(|error| anyhow::Error::new(error).context("Daytona health check failed"))?;
        match health.status {
            HealthStatus::Ok | HealthStatus::Unknown => Ok(DaytonaKeyCheck {
                missing: Vec::new(),
            }),
            HealthStatus::Unauthorized if !health.missing_permissions.is_empty() => {
                Ok(DaytonaKeyCheck {
                    missing: ordered_scopes(&health.missing_permissions),
                })
            }
            HealthStatus::Unauthorized => Err(anyhow::anyhow!(
                "failed to authenticate with Daytona: {}",
                health
                    .message
                    .unwrap_or_else(|| "the credential was rejected".to_string())
            )),
            _ => Err(anyhow::anyhow!(
                "failed to reach Daytona: {}",
                health
                    .message
                    .unwrap_or_else(|| "the control plane did not answer".to_string())
            )),
        }
    };
    match time::timeout(probe_timeout, probe).await {
        Ok(result) => result,
        Err(_) => Err(anyhow::Error::new(DaytonaCredentialProbeTimeout::new(
            probe_timeout,
        ))),
    }
}

/// The scopes fabro requires, in fabro's documented order, followed by any
/// other scope the provider reported missing.
fn ordered_scopes(missing: &[String]) -> Vec<String> {
    let mut ordered: Vec<String> = REQUIRED_DAYTONA_SCOPES
        .iter()
        .filter(|scope| missing.iter().any(|reported| reported == *scope))
        .map(|scope| (*scope).to_string())
        .collect();
    for scope in missing {
        if !ordered.contains(scope) {
            ordered.push(scope.clone());
        }
    }
    ordered
}

async fn connect(credentials: &DaytonaCredentials) -> anyhow::Result<Arc<dyn SandboxProvider>> {
    connect_provider(
        &SandboxProviderKind::DAYTONA,
        &ServerSandboxProviderSettings::default(),
        &ProviderConnectOptions {
            host_registry_root: None,
            daytona:            Some(credentials.clone()),
        },
    )
    .await
    .map(|connected| connected.provider)
    .map_err(|error| anyhow::Error::new(error).context("Failed to connect to Daytona"))
}

/// The workspace layout every Daytona sandbox uses.
pub(crate) fn layout() -> WorkspaceLayout {
    WorkspaceLayout {
        workspace_root: WORKING_DIRECTORY.to_string(),
        repos_root:     REPOS_ROOT.to_string(),
    }
}

/// The driver spec for a fabro Daytona sandbox created from `snapshot`.
pub(crate) fn driver_spec(
    config: &DaytonaConfig,
    run_id: Option<&RunId>,
    snapshot: &SnapshotId,
) -> DriverSpec {
    let mut spec = DriverSpec::new(SandboxSource::Snapshot {
        id: snapshot.clone(),
    })
    .working_directory(WORKING_DIRECTORY)
    .network(match &config.network {
        Some(DaytonaNetwork::Block) => NetworkPolicy::Block,
        Some(DaytonaNetwork::AllowAll) => NetworkPolicy::AllowAll,
        Some(DaytonaNetwork::AllowList(cidrs)) => NetworkPolicy::CidrAllowList {
            cidrs: cidrs.clone(),
        },
        None => NetworkPolicy::ProviderDefault,
    });
    if let Some(run_id) = run_id {
        spec = spec.name(format!("fabro-{run_id}"));
    }
    let mut labels: Vec<(String, String)> =
        managed_labels::merge_for_run(config.labels.as_ref(), run_id)
            .into_iter()
            .collect();
    labels.sort();
    for (key, value) in labels {
        spec = spec.label(key, value);
    }
    let mut timers = LifecycleTimers::default();
    // An explicit zero disables auto-stop; the driver encodes
    // `Duration::ZERO` as that wire value.
    timers.auto_stop_after_idle = Some(minutes_to_duration(
        config
            .auto_stop_interval
            .unwrap_or(DEFAULT_AUTO_STOP_INTERVAL_MINUTES),
    ));
    // Run sandboxes are never deleted on stop: the run record may need
    // them again on resume, and `fabro system prune` reclaims them.
    timers.auto_delete_after_stop = Some(Duration::ZERO);
    spec.timers(timers)
}

fn minutes_to_duration(minutes: i32) -> Duration {
    Duration::from_mins(u64::try_from(minutes).unwrap_or(0))
}

/// Ensures the snapshot `config` describes exists and is active, building
/// it when Daytona does not have it. Returns the snapshot to create
/// sandboxes from.
async fn ensure_snapshot(
    provider: &dyn SandboxProvider,
    api_key: &str,
    config: &DaytonaSnapshotConfig,
    emit: &(dyn Fn(SandboxEvent) + Send + Sync),
) -> crate::Result<(SnapshotId, String)> {
    let name = snapshot_identity::snapshot_name(api_key, config)?;
    let snapshots = provider.snapshots().ok_or_else(|| {
        crate::Error::message("The Daytona provider does not expose snapshot management")
    })?;
    let mut filter = SnapshotFilter::default();
    filter.name = Some(name.clone());
    let existing = snapshots
        .list(&filter)
        .await
        .map_err(|error| {
            crate::Error::context(format!("Failed to look up snapshot '{name}'"), error)
        })?
        .into_iter()
        .find(|status| status.name.as_deref() == Some(name.as_str()));
    let id = if let Some(status) = existing {
        match status.state {
            SnapshotState::Active => return Ok((status.id, name)),
            SnapshotState::Error => {
                return Err(crate::Error::message(format!(
                    "Snapshot '{name}' is in an error state: {}",
                    status.error_reason.unwrap_or_default()
                )));
            }
            SnapshotState::Inactive => {
                emit(SandboxEvent::SnapshotCreating { name: name.clone() });
                snapshots
                    .activate(&status.id, None)
                    .await
                    .map_err(|error| {
                        crate::Error::context(
                            format!("Failed to activate snapshot '{name}'"),
                            error,
                        )
                    })?;
                status.id
            }
            _ => {
                emit(SandboxEvent::SnapshotCreating { name: name.clone() });
                status.id
            }
        }
    } else {
        emit(SandboxEvent::SnapshotCreating { name: name.clone() });
        let spec = snapshot_spec(&name, config)?;
        snapshots.create(&spec, None).await.map_err(|error| {
            crate::Error::context(format!("Failed to create snapshot '{name}'"), error)
        })?
    };
    wait_for_active_snapshot(snapshots, &id, &name).await?;
    Ok((id, name))
}

fn snapshot_spec(name: &str, config: &DaytonaSnapshotConfig) -> crate::Result<SnapshotSpec> {
    let source = match &config.source {
        DaytonaSnapshotSource::Image(image) => SnapshotSource::Image {
            reference: image.clone(),
        },
        DaytonaSnapshotSource::Dockerfile(DockerfileSource::Inline(content)) => {
            SnapshotSource::Dockerfile {
                content: content.clone(),
            }
        }
        DaytonaSnapshotSource::Dockerfile(DockerfileSource::Path { .. }) => {
            return Err(crate::Error::message(format!(
                "Snapshot '{name}': dockerfile path should have been resolved to inline content before sandbox creation"
            )));
        }
    };
    let mut resources = Resources::default();
    resources.cpu_cores = config.cpu.and_then(|cpu| u32::try_from(cpu).ok());
    resources.memory_mb = config
        .memory
        .and_then(|gb| u64::try_from(gb).ok())
        .map(|gb| gb * 1024);
    resources.disk_mb = config
        .disk
        .and_then(|gb| u64::try_from(gb).ok())
        .map(|gb| gb * 1024);
    Ok(SnapshotSpec::new(source).name(name).resources(resources))
}

/// Polls a snapshot until it is active, with exponential back-off, or fails
/// when it errors or the budget runs out.
async fn wait_for_active_snapshot(
    snapshots: &dyn SnapshotProvider,
    id: &SnapshotId,
    name: &str,
) -> crate::Result<()> {
    let mut delay = Duration::from_secs(2);
    let max_delay = Duration::from_secs(30);
    let deadline = time::Instant::now() + DAYTONA_SNAPSHOT_ACTIVE_TIMEOUT;
    while time::Instant::now() < deadline {
        time::sleep(delay).await;
        let status = snapshots.get(id).await.map_err(|error| {
            crate::Error::context(format!("Failed to poll snapshot '{name}'"), error)
        })?;
        match status.state {
            SnapshotState::Active => return Ok(()),
            SnapshotState::Error | SnapshotState::Deleting => {
                return Err(crate::Error::message(format!(
                    "Snapshot '{name}' failed: {}",
                    status.error_reason.unwrap_or_default()
                )));
            }
            _ => delay = (delay * 2).min(max_delay),
        }
    }
    Err(crate::Error::message(format!(
        "Timed out waiting for snapshot '{name}' to become active"
    )))
}

/// Prepares a Daytona create: the snapshot first, then the spec naming it.
struct DaytonaCreatePlan {
    provider: Arc<dyn SandboxProvider>,
    api_key:  String,
    config:   DaytonaConfig,
    run_id:   Option<RunId>,
}

#[async_trait]
impl CreatePlan for DaytonaCreatePlan {
    async fn prepare(
        &self,
        emit: &(dyn Fn(SandboxEvent) + Send + Sync),
    ) -> crate::Result<PreparedCreate> {
        let (snapshot_id, snapshot_name) = match &self.config.snapshot {
            Some(snapshot) => {
                let started = time::Instant::now();
                let result =
                    ensure_snapshot(self.provider.as_ref(), &self.api_key, snapshot, emit).await;
                match result {
                    Ok((id, name)) => {
                        emit(SandboxEvent::SnapshotReady {
                            name:        name.clone(),
                            duration_ms: u64::try_from(started.elapsed().as_millis())
                                .unwrap_or(u64::MAX),
                        });
                        (id, name)
                    }
                    Err(error) => {
                        let name = snapshot_identity::snapshot_name(&self.api_key, snapshot)
                            .unwrap_or_default();
                        emit(SandboxEvent::SnapshotFailed {
                            name,
                            error: error.to_string(),
                            causes: error.causes(),
                        });
                        return Err(error);
                    }
                }
            }
            None => (
                SnapshotId::try_new(DEFAULT_SNAPSHOT).expect("the default snapshot name is valid"),
                DEFAULT_SNAPSHOT.to_string(),
            ),
        };
        Ok(PreparedCreate {
            spec:     driver_spec(&self.config, self.run_id.as_ref(), &snapshot_id),
            source:   Some(snapshot_name.clone()),
            snapshot: Some(snapshot_name),
        })
    }
}

/// A Daytona sandbox for a run. The sandbox is created by `initialize`;
/// construction validates the clone request and connects the provider, so
/// a bad spec or missing credential fails before any control-plane call.
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors SandboxSpec::Daytona; clone inputs are validated together"
)]
pub async fn daytona_sandbox(
    config: DaytonaConfig,
    github_app: Option<&GitHubCredentials>,
    run_id: Option<RunId>,
    clone_origin_url: Option<String>,
    clone_branch: Option<String>,
    clone_tag: Option<String>,
    clone_commit_sha: Option<String>,
    credentials: &DaytonaCredentials,
) -> crate::Result<DriverSandbox> {
    let workspace = RepoWorkspace::plan(
        LayoutSource::Fixed(layout()),
        config.skip_clone,
        clone_origin_url.as_deref(),
        clone_branch.as_deref(),
        clone_tag.as_deref(),
        clone_commit_sha.as_deref(),
        config
            .clone_depth
            .and_then(|depth| u32::try_from(depth).ok()),
        github_app,
    )?;
    let provider = connect(credentials)
        .await
        .map_err(|error| crate::Error::context_anyhow("Failed to connect to Daytona", error))?;
    let plan = DaytonaCreatePlan {
        provider: Arc::clone(&provider),
        api_key: credentials.api_key.clone(),
        config,
        run_id,
    };
    Ok(DriverSandbox::pending_with_plan(
        SandboxProviderKind::DAYTONA,
        provider,
        Box::new(plan),
        workspace,
    ))
}

/// Reattach to a run's Daytona sandbox by its persisted id.
///
/// The sandbox must carry fabro's managed label and, when a run id is
/// known, the matching run label: fabro never operates on a sandbox it did
/// not create, even inside its own organization.
pub async fn attach_daytona(
    sandbox_id: &str,
    repo_cloned: bool,
    working_directory: String,
    clone_origin_url: Option<String>,
    run_id: Option<RunId>,
    credentials: &DaytonaCredentials,
) -> crate::Result<DriverSandbox> {
    let provider = connect(credentials)
        .await
        .map_err(|error| crate::Error::context_anyhow("Failed to connect to Daytona", error))?;
    let id = SandboxId::try_new(sandbox_id)
        .map_err(|error| crate::Error::context("Invalid Daytona sandbox id", error))?;
    let handle = provider.attach(&id, None).await.map_err(|error| {
        crate::Error::context(
            format!("Failed to reconnect Daytona sandbox '{sandbox_id}'"),
            error,
        )
    })?;
    let status = handle.describe().await?;
    managed_labels::verify_managed(
        &SandboxProviderKind::DAYTONA,
        sandbox_id,
        &status.labels,
        run_id.as_ref(),
    )?;
    let workspace = RepoWorkspace::attached(
        LayoutSource::Fixed(layout()),
        repo_cloned,
        working_directory,
        clone_origin_url,
    );
    let sandbox = DriverSandbox::attached(SandboxProviderKind::DAYTONA, handle, workspace);
    if let Some(snapshot) = status.source {
        sandbox.set_snapshot(snapshot);
    }
    Ok(sandbox)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn run_id() -> RunId {
        "01HY0000000000000000000000".parse().unwrap()
    }

    #[test]
    fn daytona_config_defaults() {
        let config = DaytonaConfig::default();
        assert!(config.snapshot.is_none());
        assert!(config.auto_stop_interval.is_none());
        assert!(config.labels.is_none());
        assert!(config.clone_depth.is_none());
    }

    #[test]
    fn driver_spec_names_the_run_and_carries_fabro_labels_and_timers() {
        let config = DaytonaConfig {
            labels: Some(HashMap::from([(
                "team".to_string(),
                "platform".to_string(),
            )])),
            network: Some(DaytonaNetwork::AllowList(vec!["10.0.0.0/8".to_string()])),
            ..DaytonaConfig::default()
        };
        let snapshot = SnapshotId::try_new("snap-1").unwrap();
        let spec = driver_spec(&config, Some(&run_id()), &snapshot);

        assert!(matches!(&spec.source, SandboxSource::Snapshot { id } if id == &snapshot));
        assert_eq!(
            spec.name.as_deref(),
            Some("fabro-01HY0000000000000000000000")
        );
        assert_eq!(spec.working_directory.as_deref(), Some(WORKING_DIRECTORY));
        assert_eq!(
            spec.labels.get("sh.fabro.managed").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            spec.labels.get("sh.fabro.run_id").map(String::as_str),
            Some("01HY0000000000000000000000")
        );
        assert_eq!(
            spec.labels.get("team").map(String::as_str),
            Some("platform")
        );
        assert!(matches!(
            &spec.network,
            NetworkPolicy::CidrAllowList { cidrs } if cidrs == &["10.0.0.0/8".to_string()]
        ));
        assert_eq!(
            spec.timers.auto_stop_after_idle,
            Some(Duration::from_hours(2)),
            "an unset auto-stop gets fabro's explicit default, never Daytona's 15 minutes"
        );
        assert_eq!(spec.timers.auto_delete_after_stop, Some(Duration::ZERO));
        assert!(!spec.ephemeral);
    }

    #[test]
    fn driver_spec_passes_explicit_auto_stop_through_and_zero_disables() {
        let snapshot = SnapshotId::try_new(DEFAULT_SNAPSHOT).unwrap();
        let explicit = driver_spec(
            &DaytonaConfig {
                auto_stop_interval: Some(45),
                network: Some(DaytonaNetwork::Block),
                ..DaytonaConfig::default()
            },
            None,
            &snapshot,
        );
        assert_eq!(
            explicit.timers.auto_stop_after_idle,
            Some(Duration::from_mins(45))
        );
        assert!(matches!(explicit.network, NetworkPolicy::Block));
        assert!(explicit.name.is_none());
        assert!(!explicit.labels.contains_key("sh.fabro.run_id"));

        let disabled = driver_spec(
            &DaytonaConfig {
                auto_stop_interval: Some(0),
                ..DaytonaConfig::default()
            },
            None,
            &snapshot,
        );
        assert_eq!(disabled.timers.auto_stop_after_idle, Some(Duration::ZERO));
    }

    #[test]
    fn snapshot_spec_maps_sources_and_gigabyte_resources() {
        let config = DaytonaSnapshotConfig {
            cpu:    Some(2),
            memory: Some(4),
            disk:   Some(10),
            source: DaytonaSnapshotSource::Image("ubuntu:24.04".to_string()),
        };
        let spec = snapshot_spec("fabro-x", &config).unwrap();
        assert_eq!(spec.name.as_deref(), Some("fabro-x"));
        assert!(matches!(
            &spec.source,
            SnapshotSource::Image { reference } if reference == "ubuntu:24.04"
        ));
        assert_eq!(spec.resources.cpu_cores, Some(2));
        assert_eq!(spec.resources.memory_mb, Some(4096));
        assert_eq!(spec.resources.disk_mb, Some(10_240));

        let dockerfile = snapshot_spec("fabro-y", &DaytonaSnapshotConfig {
            source: DaytonaSnapshotSource::Dockerfile(DockerfileSource::Inline(
                "FROM ubuntu".to_string(),
            )),
            ..config.clone()
        })
        .unwrap();
        assert!(matches!(
            &dockerfile.source,
            SnapshotSource::Dockerfile { content } if content == "FROM ubuntu"
        ));

        let unresolved = snapshot_spec("fabro-z", &DaytonaSnapshotConfig {
            source: DaytonaSnapshotSource::Dockerfile(DockerfileSource::Path {
                path: "Dockerfile".to_string(),
            }),
            ..config
        })
        .unwrap_err();
        assert!(
            unresolved
                .to_string()
                .contains("resolved to inline content")
        );
    }

    #[test]
    fn computed_snapshot_identity_is_deterministic_and_keyed() {
        let config = DaytonaSnapshotConfig {
            cpu:    Some(2),
            memory: Some(4),
            disk:   Some(10),
            source: DaytonaSnapshotSource::Dockerfile(DockerfileSource::Inline(
                "FROM ubuntu:24.04\nRUN apt-get update".to_string(),
            )),
        };

        let first = snapshot_identity::snapshot_name("dtn_secret", &config).unwrap();
        let second = snapshot_identity::snapshot_name("dtn_secret", &config).unwrap();
        let rotated_key = snapshot_identity::snapshot_name("dtn_rotated", &config).unwrap();

        assert_eq!(first, second);
        assert_eq!(first, "fabro-e607185f-c7ab-88c9-bf9d-d70addba9298");
        assert_ne!(first, rotated_key);
        let uuid = first
            .strip_prefix("fabro-")
            .and_then(|raw| uuid::Uuid::parse_str(raw).ok())
            .expect("snapshot name should be fabro-<uuid>");
        assert_eq!(uuid.get_version_num(), 8);
        assert_eq!(uuid.get_variant(), uuid::Variant::RFC4122);
    }

    #[test]
    fn computed_snapshot_identity_changes_for_generation_inputs() {
        let base = DaytonaSnapshotConfig {
            cpu:    Some(2),
            memory: Some(4),
            disk:   Some(10),
            source: DaytonaSnapshotSource::Dockerfile(DockerfileSource::Inline(
                "FROM ubuntu:24.04".to_string(),
            )),
        };
        let base_name = snapshot_identity::snapshot_name("dtn_secret", &base).unwrap();

        let cases = [
            DaytonaSnapshotConfig {
                source: DaytonaSnapshotSource::Dockerfile(DockerfileSource::Inline(
                    "FROM ubuntu:24.04\n# roll cache".to_string(),
                )),
                ..base.clone()
            },
            DaytonaSnapshotConfig {
                cpu: Some(4),
                ..base.clone()
            },
            DaytonaSnapshotConfig {
                memory: Some(8),
                ..base.clone()
            },
            DaytonaSnapshotConfig {
                disk: Some(20),
                ..base.clone()
            },
        ];

        for changed in cases {
            let changed_name = snapshot_identity::snapshot_name("dtn_secret", &changed).unwrap();
            assert_ne!(base_name, changed_name);
        }
    }

    #[test]
    fn computed_snapshot_identity_excludes_raw_dockerfile_and_key_material() {
        let config = DaytonaSnapshotConfig {
            cpu:    None,
            memory: None,
            disk:   None,
            source: DaytonaSnapshotSource::Dockerfile(DockerfileSource::Inline(
                "FROM private.example.com/secret-image\nRUN echo raw-secret".to_string(),
            )),
        };

        let name = snapshot_identity::snapshot_name("dtn_super_secret_key", &config).unwrap();

        assert!(name.starts_with("fabro-"));
        assert!(!name.contains("private.example.com"));
        assert!(!name.contains("raw-secret"));
        assert!(!name.contains("dtn_super_secret_key"));
    }

    #[test]
    fn computed_snapshot_identity_changes_for_image_reference() {
        let config = DaytonaSnapshotConfig {
            cpu:    Some(2),
            memory: Some(4),
            disk:   Some(10),
            source: DaytonaSnapshotSource::Image("ubuntu:24.04".to_string()),
        };
        let first = snapshot_identity::snapshot_name("dtn_secret", &config).unwrap();
        let changed = snapshot_identity::snapshot_name("dtn_secret", &DaytonaSnapshotConfig {
            source: DaytonaSnapshotSource::Image("ubuntu:24.10".to_string()),
            ..config
        })
        .unwrap();

        assert_eq!(first, "fabro-5d23a023-d7ff-8d68-b3ca-e6286f4211d9");
        assert_ne!(first, changed);
    }

    #[test]
    fn missing_scopes_render_in_documented_order() {
        let check = DaytonaKeyCheck {
            missing: ordered_scopes(&[
                "write:sandboxes".to_string(),
                "write:snapshots".to_string(),
                "manage:secrets".to_string(),
            ]),
        };
        assert!(!check.ok());
        assert_eq!(
            check.missing_display(),
            "write:snapshots, write:sandboxes, manage:secrets"
        );
        assert_eq!(
            check.missing_message(),
            "Daytona API key is missing required scopes: write:snapshots, write:sandboxes, \
             manage:secrets. Regenerate the key with all snapshot and sandbox scopes."
        );
        assert_eq!(
            required_perms_display(),
            "write:snapshots, delete:snapshots, write:sandboxes, delete:sandboxes"
        );
    }

    #[tokio::test]
    async fn credential_probe_reports_configured_timeout() {
        let credentials = DaytonaCredentials {
            api_key:         "dtn_test".to_string(),
            // A non-routable address: the probe cannot finish within the budget.
            api_url:         Some("http://10.255.255.1:1/api".to_string()),
            organization_id: None,
            target:          None,
            http_client:     None,
        };
        let err = check_daytona_api_key(&credentials, Duration::from_millis(1))
            .await
            .expect_err("probe should time out");
        let timeout = err
            .downcast_ref::<DaytonaCredentialProbeTimeout>()
            .expect("timeout should preserve its type");
        assert_eq!(timeout.timeout(), Duration::from_millis(1));
        assert_eq!(
            err.to_string(),
            "Daytona credential probe timed out after 1ms"
        );
    }
}

/// The git clone contract over the plugin wire against live Daytona.
///
/// Host and Docker derive their git facet from `Exec`, so only Daytona
/// exercises the driver's native clone through the JSON-RPC protocol. The
/// provider is served over an in-process duplex pipe exactly as a plugin
/// executable would serve it on stdio.
#[cfg(test)]
mod wire_gate {
    use std::sync::Arc;

    use fabro_static::EnvVars;
    use fabro_types::SandboxProviderKind;
    use sandbox_driver::{SandboxProvider, SandboxSource, SandboxSpec as DriverSpec};
    use sandbox_driver_protocol::{PluginProvider, serve};
    use tokio::io::{duplex, split};

    use super::*;
    use crate::Sandbox as _;
    use crate::driver_sandbox::{DriverSandbox, LayoutSource, RepoWorkspace};

    #[expect(
        clippy::disallowed_methods,
        reason = "the live gate takes Daytona credentials from the developer's environment"
    )]
    fn live_credentials() -> Option<DaytonaCredentials> {
        let api_key = std::env::var(EnvVars::DAYTONA_API_KEY).ok()?;
        Some(DaytonaCredentials {
            api_key,
            api_url: std::env::var(EnvVars::DAYTONA_API_URL)
                .or_else(|_| std::env::var(EnvVars::DAYTONA_SERVER_URL))
                .ok(),
            organization_id: std::env::var(EnvVars::DAYTONA_ORGANIZATION_ID).ok(),
            target: None,
            http_client: None,
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires live Daytona credentials and provisions a sandbox"]
    async fn native_clone_over_the_wire_lays_out_the_repository() {
        let credentials = live_credentials().expect("DAYTONA_API_KEY must be set");
        let in_process = connect(&credentials).await.expect("connect to Daytona");

        let (host_side, plugin_side) = duplex(1024 * 1024);
        let (host_read, host_write) = split(host_side);
        let (plugin_read, plugin_write) = split(plugin_side);
        tokio::spawn(serve(Arc::clone(&in_process), plugin_read, plugin_write));
        let remote = PluginProvider::connect(host_read, host_write)
            .await
            .expect("protocol handshake");
        assert_eq!(remote.kind().as_str(), "daytona");
        let remote: Arc<dyn SandboxProvider> = Arc::new(remote);

        let workspace = RepoWorkspace::plan(
            LayoutSource::Fixed(layout()),
            false,
            Some("https://github.com/brynary/rack-test"),
            None,
            None,
            None,
            Some(100),
            None,
        )
        .expect("clone plan");
        let snapshot = SnapshotId::try_new(DEFAULT_SNAPSHOT).expect("snapshot id");
        let spec = DriverSpec::new(SandboxSource::Snapshot { id: snapshot });
        let spec = driver_spec(&DaytonaConfig::default(), None, &snapshot_id_of(&spec));
        let sandbox = DriverSandbox::pending(
            SandboxProviderKind::DAYTONA,
            remote,
            spec,
            Some(DEFAULT_SNAPSHOT.to_string()),
            workspace,
        );
        sandbox
            .initialize()
            .await
            .expect("initialize over the wire");

        let checks = async {
            assert_eq!(
                sandbox.working_directory(),
                "/home/daytona/workspace/rack-test"
            );
            let result = sandbox
                .exec_command(
                    "test -d /home/daytona/repos/brynary/rack-test/.git && \
                     test -L /home/daytona/workspace/rack-test && \
                     git rev-parse --is-inside-work-tree",
                    30_000,
                    None,
                    None,
                    None,
                )
                .await
                .expect("layout check");
            assert!(result.is_success(), "{result:?}");
            assert!(result.stdout.contains("true"));
            let layout = sandbox.workspace_layout().expect("layout record");
            assert_eq!(
                layout.primary_repo_path.as_deref(),
                Some("/home/daytona/repos/brynary/rack-test")
            );
        };
        checks.await;
        sandbox.cleanup().await.expect("cleanup");
    }

    fn snapshot_id_of(spec: &DriverSpec) -> SnapshotId {
        match &spec.source {
            SandboxSource::Snapshot { id } => id.clone(),
            _ => unreachable!("the gate builds a snapshot source"),
        }
    }
}
