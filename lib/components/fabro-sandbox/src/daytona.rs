//! The `daytona` provider kind: what fabro adds to a run's spec for the
//! sandbox-driver Daytona provider.
//!
//! The environment's options build the spec once; Daytona's overlay creates
//! sandboxes from a snapshot (built from the environment's image or
//! Dockerfile and named by an HMAC of its inputs, or Daytona's default when
//! the environment names neither), fixes the working directory, and sets
//! the lifecycle timers. The run works in `/home/daytona/workspace`, with a
//! cloned repository checked out under `/home/daytona/repos` and linked
//! into the workspace.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use fabro_types::settings::server::ServerSandboxProviderSettings;
use fabro_types::{RunId, SandboxProviderKind};
use sandbox_driver::{
    EventContext, HealthStatus, LifecycleTimers, Resources, SandboxProvider, SandboxSource,
    SandboxSpec as DriverSpec, SnapshotId, SnapshotSource, SnapshotSpec,
};
use tokio::time;

pub use crate::driver::DaytonaCredentials;
use crate::driver::{ProviderConnectOptions, connect_provider};
use crate::driver_sandbox::{CreatePlan, PreparedCreate, WorkspaceLayout};
use crate::options::SandboxOptions;

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
/// leaked by a dead worker. An explicit zero disables auto-stop entirely.
const DEFAULT_AUTO_STOP: Duration = Duration::from_hours(2);

/// Scopes a Daytona API key needs for fabro's snapshot and sandbox flow, in
/// the order the remediation text lists them.
pub const REQUIRED_DAYTONA_SCOPES: &[&str] = &[
    "write:snapshots",
    "delete:snapshots",
    "write:sandboxes",
    "delete:sandboxes",
];

/// What a custom snapshot is built from: the environment's image or
/// Dockerfile and its resources in whole gigabytes, the units Daytona
/// sizes snapshots in and the values the snapshot's name is derived from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotInputs<'a> {
    pub source:    SnapshotInput<'a>,
    pub cpu:       Option<i32>,
    pub memory_gb: Option<i32>,
    pub disk_gb:   Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotInput<'a> {
    /// A pullable image reference such as `ubuntu:24.04`.
    Image(&'a str),
    /// A Dockerfile Daytona builds into the snapshot.
    Dockerfile(&'a str),
}

/// The snapshot `options` ask for, or `None` when the environment names no
/// image or Dockerfile and the sandbox comes from Daytona's default.
pub fn snapshot_inputs(options: &SandboxOptions) -> Option<SnapshotInputs<'_>> {
    let source = match (&options.image, &options.dockerfile) {
        (Some(image), _) => SnapshotInput::Image(image),
        (None, Some(dockerfile)) => SnapshotInput::Dockerfile(dockerfile),
        (None, None) => return None,
    };
    Some(SnapshotInputs {
        source,
        cpu: options.cpu.and_then(|cpu| i32::try_from(cpu).ok()),
        memory_gb: options.memory_bytes.map(bytes_to_gb),
        disk_gb: options.disk_bytes.map(bytes_to_gb),
    })
}

/// Whole decimal gigabytes, the unit Daytona sizes snapshots in.
fn bytes_to_gb(bytes: u64) -> i32 {
    i32::try_from(bytes / 1_000_000_000).unwrap_or(i32::MAX)
}

pub mod snapshot_identity {
    use hmac::{Hmac, Mac};
    use serde::Serialize;
    use sha2::{Digest, Sha256};
    use uuid::Uuid;

    use super::{SnapshotInput, SnapshotInputs};

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

    /// The name of the snapshot built from `inputs`: a UUIDv8 derived from an
    /// HMAC of the build inputs keyed by the API key, so the same inputs reuse
    /// the same snapshot and a rotated key never collides with another
    /// tenant's.
    pub fn snapshot_name(api_key: &str, inputs: &SnapshotInputs<'_>) -> crate::Result<String> {
        let manifest = canonical_manifest(inputs)?;
        let mut mac = HmacSha256::new_from_slice(api_key.as_bytes())
            .expect("HMAC-SHA256 accepts keys of any length");
        mac.update(&manifest);
        let digest = mac.finalize().into_bytes();
        let mut bytes = [0_u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        Ok(format!("fabro-{}", Uuid::new_v8(bytes)))
    }

    fn canonical_manifest(inputs: &SnapshotInputs<'_>) -> crate::Result<Vec<u8>> {
        let source = match inputs.source {
            SnapshotInput::Image(image) => SourceManifest::Image(image),
            SnapshotInput::Dockerfile(text) => {
                SourceManifest::DockerfileSha256(hex::encode(Sha256::digest(text.as_bytes())))
            }
        };
        let manifest = SnapshotManifest {
            identity_version: IDENTITY_VERSION,
            provider: PROVIDER,
            tenant: TENANT,
            source,
            cpu: inputs.cpu,
            memory_gb: inputs.memory_gb,
            disk_gb: inputs.disk_gb,
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

/// Daytona's additions to the base spec: the snapshot the sandbox is created
/// from, the fixed working directory, the run's Daytona name, and the
/// lifecycle timers.
pub(crate) fn overlay(
    spec: DriverSpec,
    options: &SandboxOptions,
    run_id: Option<&RunId>,
    snapshot: &SnapshotId,
) -> DriverSpec {
    let mut spec = spec.working_directory(WORKING_DIRECTORY);
    spec.source = SandboxSource::Snapshot {
        id: snapshot.clone(),
    };
    spec.name = run_id.map(|run_id| format!("fabro-{run_id}"));
    let mut timers = LifecycleTimers::default();
    // An explicit zero disables auto-stop; the driver encodes
    // `Duration::ZERO` as that wire value.
    timers.auto_stop_after_idle = Some(options.auto_stop.unwrap_or(DEFAULT_AUTO_STOP));
    // Run sandboxes are never deleted on stop: the run record may need
    // them again on resume, and `fabro system prune` reclaims them.
    timers.auto_delete_after_stop = Some(Duration::ZERO);
    spec.timers(timers)
}

/// Ensures the snapshot `inputs` describe exists and is active, building
/// it when Daytona does not have it. Returns the snapshot to create
/// sandboxes from.
async fn ensure_snapshot(
    provider: &dyn SandboxProvider,
    api_key: &str,
    inputs: &SnapshotInputs<'_>,
    events: Option<EventContext>,
) -> crate::Result<(SnapshotId, String)> {
    let name = snapshot_identity::snapshot_name(api_key, inputs)?;
    let snapshots = provider.snapshots().ok_or_else(|| {
        crate::Error::message("The Daytona provider does not expose snapshot management")
    })?;
    let id = snapshots
        .ensure(
            &snapshot_spec(&name, inputs),
            DAYTONA_SNAPSHOT_ACTIVE_TIMEOUT,
            events,
        )
        .await
        .map_err(|error| {
            crate::Error::context(format!("Failed to ensure snapshot '{name}'"), error)
        })?;
    Ok((id, name))
}

fn snapshot_spec(name: &str, inputs: &SnapshotInputs<'_>) -> SnapshotSpec {
    let source = match inputs.source {
        SnapshotInput::Image(image) => SnapshotSource::Image {
            reference: image.to_string(),
        },
        SnapshotInput::Dockerfile(content) => SnapshotSource::Dockerfile {
            content: content.to_string(),
        },
    };
    let mut resources = Resources::default();
    resources.cpu_cores = inputs.cpu.and_then(|cpu| u32::try_from(cpu).ok());
    resources.memory_mb = inputs
        .memory_gb
        .and_then(|gb| u64::try_from(gb).ok())
        .map(|gb| gb * 1024);
    resources.disk_mb = inputs
        .disk_gb
        .and_then(|gb| u64::try_from(gb).ok())
        .map(|gb| gb * 1024);
    SnapshotSpec::new(source).name(name).resources(resources)
}

/// Prepares a Daytona create: the snapshot first, then the spec naming it.
pub(crate) struct DaytonaCreatePlan {
    provider: Arc<dyn SandboxProvider>,
    api_key:  String,
    base:     DriverSpec,
    options:  SandboxOptions,
    run_id:   Option<RunId>,
}

/// The create plan for a run on Daytona: `base` is the spec the
/// environment's options built, which the plan completes with the snapshot
/// once it exists.
pub(crate) fn create_plan(
    provider: Arc<dyn SandboxProvider>,
    api_key: String,
    base: DriverSpec,
    options: SandboxOptions,
    run_id: Option<RunId>,
) -> DaytonaCreatePlan {
    DaytonaCreatePlan {
        provider,
        api_key,
        base,
        options,
        run_id,
    }
}

#[async_trait]
impl CreatePlan for DaytonaCreatePlan {
    async fn prepare(&self, events: Option<EventContext>) -> crate::Result<PreparedCreate> {
        let (snapshot_id, snapshot_name) = match snapshot_inputs(&self.options) {
            // The driver finds, activates, builds, or waits for the snapshot
            // as needed, and reports that work through `events`.
            Some(inputs) => {
                ensure_snapshot(self.provider.as_ref(), &self.api_key, &inputs, events).await?
            }
            None => (
                SnapshotId::try_new(DEFAULT_SNAPSHOT).expect("the default snapshot name is valid"),
                DEFAULT_SNAPSHOT.to_string(),
            ),
        };
        Ok(PreparedCreate {
            spec:     overlay(
                self.base.clone(),
                &self.options,
                self.run_id.as_ref(),
                &snapshot_id,
            ),
            snapshot: Some(snapshot_name),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use sandbox_driver::NetworkPolicy;

    use super::*;
    use crate::options::base_spec;

    fn run_id() -> RunId {
        "01HY0000000000000000000000".parse().unwrap()
    }

    fn dockerfile_inputs(dockerfile: &str) -> SnapshotInputs<'_> {
        SnapshotInputs {
            source:    SnapshotInput::Dockerfile(dockerfile),
            cpu:       Some(2),
            memory_gb: Some(4),
            disk_gb:   Some(10),
        }
    }

    #[test]
    fn snapshot_inputs_come_from_the_image_or_dockerfile_in_whole_gigabytes() {
        assert!(snapshot_inputs(&SandboxOptions::default()).is_none());

        let options = SandboxOptions {
            image: Some("ubuntu:24.04".to_string()),
            cpu: Some(2),
            memory_bytes: Some(4_000_000_000),
            disk_bytes: Some(10_500_000_000),
            ..SandboxOptions::default()
        };
        assert_eq!(
            snapshot_inputs(&options),
            Some(SnapshotInputs {
                source:    SnapshotInput::Image("ubuntu:24.04"),
                cpu:       Some(2),
                memory_gb: Some(4),
                disk_gb:   Some(10),
            })
        );

        let options = SandboxOptions {
            dockerfile: Some("FROM ubuntu".to_string()),
            ..SandboxOptions::default()
        };
        assert_eq!(
            snapshot_inputs(&options).map(|inputs| inputs.source),
            Some(SnapshotInput::Dockerfile("FROM ubuntu"))
        );
    }

    #[test]
    fn overlay_names_the_run_and_carries_fabro_labels_and_timers() {
        let options = SandboxOptions {
            labels: BTreeMap::from([("team".to_string(), "platform".to_string())]),
            network: NetworkPolicy::CidrAllowList {
                cidrs: vec!["10.0.0.0/8".to_string()],
            },
            ..SandboxOptions::default()
        };
        let snapshot = SnapshotId::try_new("snap-1").unwrap();
        let spec = overlay(
            base_spec(&options, Some(&run_id())),
            &options,
            Some(&run_id()),
            &snapshot,
        );

        assert!(matches!(&spec.source, SandboxSource::Snapshot { id } if id == &snapshot));
        assert_eq!(
            spec.name.as_deref(),
            Some("fabro-01HY0000000000000000000000")
        );
        assert_eq!(spec.working_directory.as_deref(), Some(WORKING_DIRECTORY));
        // Fabro's ownership labels are stamped by the scope the provider is
        // connected through, not by the spec.
        assert!(!spec.labels.contains_key("sh.fabro.managed"));
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
    fn overlay_passes_explicit_auto_stop_through_and_zero_disables() {
        let snapshot = SnapshotId::try_new(DEFAULT_SNAPSHOT).unwrap();
        let options = SandboxOptions {
            auto_stop: Some(Duration::from_mins(45)),
            network: NetworkPolicy::Block,
            ..SandboxOptions::default()
        };
        let explicit = overlay(base_spec(&options, None), &options, None, &snapshot);
        assert_eq!(
            explicit.timers.auto_stop_after_idle,
            Some(Duration::from_mins(45))
        );
        assert!(matches!(explicit.network, NetworkPolicy::Block));
        assert!(explicit.name.is_none());

        let options = SandboxOptions {
            auto_stop: Some(Duration::ZERO),
            ..SandboxOptions::default()
        };
        let disabled = overlay(base_spec(&options, None), &options, None, &snapshot);
        assert_eq!(disabled.timers.auto_stop_after_idle, Some(Duration::ZERO));
    }

    #[test]
    fn snapshot_spec_maps_sources_and_gigabyte_resources() {
        let inputs = SnapshotInputs {
            source:    SnapshotInput::Image("ubuntu:24.04"),
            cpu:       Some(2),
            memory_gb: Some(4),
            disk_gb:   Some(10),
        };
        let spec = snapshot_spec("fabro-x", &inputs);
        assert_eq!(spec.name.as_deref(), Some("fabro-x"));
        assert!(matches!(
            &spec.source,
            SnapshotSource::Image { reference } if reference == "ubuntu:24.04"
        ));
        assert_eq!(spec.resources.cpu_cores, Some(2));
        assert_eq!(spec.resources.memory_mb, Some(4096));
        assert_eq!(spec.resources.disk_mb, Some(10_240));

        let dockerfile = snapshot_spec("fabro-y", &SnapshotInputs {
            source: SnapshotInput::Dockerfile("FROM ubuntu"),
            ..inputs
        });
        assert!(matches!(
            &dockerfile.source,
            SnapshotSource::Dockerfile { content } if content == "FROM ubuntu"
        ));
    }

    #[test]
    fn computed_snapshot_identity_is_deterministic_and_keyed() {
        let inputs = dockerfile_inputs("FROM ubuntu:24.04\nRUN apt-get update");

        let first = snapshot_identity::snapshot_name("dtn_secret", &inputs).unwrap();
        let second = snapshot_identity::snapshot_name("dtn_secret", &inputs).unwrap();
        let rotated_key = snapshot_identity::snapshot_name("dtn_rotated", &inputs).unwrap();

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
        let base = dockerfile_inputs("FROM ubuntu:24.04");
        let base_name = snapshot_identity::snapshot_name("dtn_secret", &base).unwrap();

        let cases = [
            SnapshotInputs {
                source: SnapshotInput::Dockerfile("FROM ubuntu:24.04\n# roll cache"),
                ..base.clone()
            },
            SnapshotInputs {
                cpu: Some(4),
                ..base.clone()
            },
            SnapshotInputs {
                memory_gb: Some(8),
                ..base.clone()
            },
            SnapshotInputs {
                disk_gb: Some(20),
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
        let inputs = SnapshotInputs {
            source:    SnapshotInput::Dockerfile(
                "FROM private.example.com/secret-image\nRUN echo raw-secret",
            ),
            cpu:       None,
            memory_gb: None,
            disk_gb:   None,
        };

        let name = snapshot_identity::snapshot_name("dtn_super_secret_key", &inputs).unwrap();

        assert!(name.starts_with("fabro-"));
        assert!(!name.contains("private.example.com"));
        assert!(!name.contains("raw-secret"));
        assert!(!name.contains("dtn_super_secret_key"));
    }

    #[test]
    fn computed_snapshot_identity_changes_for_image_reference() {
        let inputs = SnapshotInputs {
            source:    SnapshotInput::Image("ubuntu:24.04"),
            cpu:       Some(2),
            memory_gb: Some(4),
            disk_gb:   Some(10),
        };
        let first = snapshot_identity::snapshot_name("dtn_secret", &inputs).unwrap();
        let changed = snapshot_identity::snapshot_name("dtn_secret", &SnapshotInputs {
            source: SnapshotInput::Image("ubuntu:24.10"),
            ..inputs
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
    use sandbox_driver::SandboxProvider;
    use sandbox_driver_protocol::{PluginProvider, serve};
    use tokio::io::{duplex, split};

    use super::*;
    use crate::driver_sandbox::{LayoutSource, RepoWorkspace, RunSandbox};
    use crate::options::base_spec;

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
        let options = SandboxOptions::default();
        let spec = overlay(base_spec(&options, None), &options, None, &snapshot);
        let sandbox = RunSandbox::pending(SandboxProviderKind::DAYTONA, remote, spec, workspace);
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
}
