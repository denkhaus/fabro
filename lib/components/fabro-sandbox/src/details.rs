use std::collections::BTreeMap;

use anyhow::Result;
use chrono::{DateTime, Utc};
use fabro_types::{
    BundledProvider, RunId, RunSandboxInstance, SandboxDetails, SandboxNetwork, SandboxResources,
    SandboxState, SandboxTimestamps,
};

use crate::docker;

/// Inspect the sandbox identified by `record` and return provider-neutral
/// details for control-plane display.
///
/// - `local` always returns a minimal record describing the host.
/// - `docker` describes the managed container through the sandbox driver.
/// - `daytona` reconnects to the SDK sandbox (feature-gated).
#[allow(
    unused_variables,
    reason = "Feature-gated providers consume some parameters only when enabled."
)]
pub async fn sandbox_details(
    record: &RunSandboxInstance,
    daytona_api_key: Option<String>,
    daytona_organization_id: Option<String>,
    run_id: Option<RunId>,
) -> Result<SandboxDetails> {
    match record.provider.bundled() {
        Some(BundledProvider::Local) => Ok(local_details(record)),
        Some(BundledProvider::Docker) => docker_details(record, run_id).await,
        #[cfg(feature = "daytona")]
        Some(BundledProvider::Daytona) => daytona::daytona_details(record, daytona_api_key).await,
        _ => Err(anyhow::anyhow!(
            "Sandbox provider '{}' has no details implementation",
            record.provider
        )),
    }
}

fn local_details(record: &RunSandboxInstance) -> SandboxDetails {
    SandboxDetails {
        sandbox:      record.clone(),
        state:        SandboxState::Running,
        native_state: None,
        region:       None,
        web_url:      None,
        resources:    SandboxResources::default(),
        network:      SandboxNetwork::unknown(),
        labels:       BTreeMap::new(),
        timestamps:   SandboxTimestamps::default(),
    }
}

#[cfg(feature = "daytona")]
fn parse_rfc3339_utc(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Projection of a sandbox-driver [`sandbox_driver::SandboxStatus`] into
/// fabro's inventory shape. The driver reports what a provider exposes
/// through its public facets; fields no facet carries (network policy) stay
/// unknown rather than being read from provider SDK types.
pub(crate) fn info_from_status(
    kind: &fabro_types::SandboxProviderKind,
    status: &sandbox_driver::SandboxStatus,
) -> fabro_types::SandboxInfo {
    let fields = fields_from_status(status);
    fabro_types::SandboxInfo {
        provider:          kind.clone(),
        id:                status.id.to_string(),
        display_name:      status.name.clone().filter(|name| !name.is_empty()),
        state:             fields.state,
        native_state:      fields.native_state,
        image:             status.source.clone(),
        snapshot:          None,
        region:            status.region.clone(),
        web_url:           status.web_url.clone(),
        working_directory: None,
        resources:         fields.resources,
        network:           SandboxNetwork::unknown(),
        labels:            status.labels.clone(),
        timestamps:        fields.timestamps,
    }
}

pub(crate) fn details_from_status(
    record: &RunSandboxInstance,
    status: &sandbox_driver::SandboxStatus,
) -> SandboxDetails {
    let fields = fields_from_status(status);
    SandboxDetails {
        sandbox:      RunSandboxInstance {
            image: status.source.clone().or_else(|| record.image.clone()),
            ..record.clone()
        },
        state:        fields.state,
        native_state: fields.native_state,
        region:       status.region.clone(),
        web_url:      status.web_url.clone(),
        resources:    fields.resources,
        network:      SandboxNetwork::unknown(),
        labels:       status.labels.clone(),
        timestamps:   fields.timestamps,
    }
}

struct StatusFields {
    state:        SandboxState,
    native_state: Option<String>,
    resources:    SandboxResources,
    timestamps:   SandboxTimestamps,
}

fn fields_from_status(status: &sandbox_driver::SandboxStatus) -> StatusFields {
    StatusFields {
        state:        normalize_driver_state(status.state),
        native_state: Some(status.provider_state.clone()).filter(|value| !value.is_empty()),
        resources:    status
            .resources
            .as_ref()
            .map(|resources| SandboxResources {
                cpu_cores:    resources.cpu_cores.map(f64::from),
                memory_bytes: resources.memory_mb.map(|mb| mb * 1024 * 1024),
                disk_bytes:   resources.disk_mb.map(|mb| mb * 1024 * 1024),
            })
            .unwrap_or_default(),
        timestamps:   SandboxTimestamps {
            created_at:       status.created_at.map(DateTime::<Utc>::from),
            last_activity_at: status.updated_at.map(DateTime::<Utc>::from),
        },
    }
}

pub(crate) fn normalize_driver_state(state: sandbox_driver::SandboxState) -> SandboxState {
    use sandbox_driver::SandboxState as Driver;
    match state {
        Driver::Creating | Driver::Forking => SandboxState::Provisioning,
        Driver::Starting | Driver::Resuming => SandboxState::Starting,
        // A sandbox mid-snapshot keeps serving commands.
        Driver::Running | Driver::Snapshotting => SandboxState::Running,
        Driver::Stopping | Driver::Archiving => SandboxState::Stopping,
        Driver::Stopped => SandboxState::Stopped,
        Driver::Pausing | Driver::Paused => SandboxState::Paused,
        Driver::Archived => SandboxState::Archived,
        Driver::Restoring => SandboxState::Restoring,
        Driver::Resizing => SandboxState::Resizing,
        Driver::Deleting => SandboxState::Deleting,
        Driver::Deleted => SandboxState::Deleted,
        Driver::Error => SandboxState::Error,
        _ => SandboxState::Unknown,
    }
}

async fn docker_details(
    record: &RunSandboxInstance,
    run_id: Option<RunId>,
) -> Result<SandboxDetails> {
    let runtime = &record.runtime;
    let sandbox = docker::attach_docker(
        &runtime.id,
        runtime.repo_cloned.unwrap_or(false),
        runtime.working_directory.clone(),
        runtime.clone_origin_url.clone(),
        run_id,
    )
    .await?;
    let status = sandbox.handle()?.describe().await.map_err(|err| {
        anyhow::anyhow!(
            "Failed to describe Docker container '{}': {err}",
            runtime.id
        )
    })?;
    Ok(details_from_status(record, &status))
}

#[cfg(feature = "daytona")]
pub(crate) mod daytona {
    use std::collections::BTreeMap;

    use anyhow::{Context, Result, anyhow};
    use daytona_api_client::models::SandboxState as DaytonaState;
    use fabro_types::{
        RunSandboxInstance, SandboxDetails, SandboxInfo, SandboxNetwork, SandboxNetworkPolicy,
        SandboxProviderKind, SandboxResources, SandboxState, SandboxTimestamps,
    };

    use super::parse_rfc3339_utc;
    use crate::daytona::{DAYTONA_DASHBOARD_SANDBOXES_URL, DaytonaSandbox, WORKING_DIRECTORY};

    pub(super) async fn daytona_details(
        record: &RunSandboxInstance,
        daytona_api_key: Option<String>,
    ) -> Result<SandboxDetails> {
        let runtime = &record.runtime;
        let repo_cloned = runtime
            .repo_cloned
            .context("Daytona run sandbox missing clone metadata")?;

        let sandbox_handle = DaytonaSandbox::reconnect(
            &runtime.id,
            daytona_api_key,
            repo_cloned,
            runtime.working_directory.clone(),
            runtime.clone_origin_url.clone(),
            runtime.clone_branch.clone(),
        )
        .await
        .map_err(anyhow::Error::new)?;
        let sdk_sandbox = sandbox_handle
            .sandbox_handle()
            .ok_or_else(|| anyhow!("Daytona sandbox is not initialized after reconnect"))?;

        Ok(map_daytona_sandbox(sdk_sandbox, record))
    }

    pub(crate) fn daytona_info_from_sdk_sandbox(sandbox: &daytona_sdk::Sandbox) -> SandboxInfo {
        let fields = daytona_fields_from_sdk_sandbox(sandbox);
        SandboxInfo {
            provider:          SandboxProviderKind::DAYTONA,
            id:                sandbox.id.clone(),
            display_name:      Some(sandbox.name.clone()).filter(|name| !name.is_empty()),
            state:             fields.state,
            native_state:      fields.native_state,
            image:             None,
            snapshot:          sandbox.snapshot.clone(),
            region:            fields.region,
            web_url:           Some(daytona_dashboard_url(&sandbox.id)),
            working_directory: Some(WORKING_DIRECTORY.to_string()),
            resources:         fields.resources,
            network:           fields.network,
            labels:            fields.labels,
            timestamps:        fields.timestamps,
        }
    }

    pub(super) fn map_daytona_sandbox(
        sandbox: &daytona_sdk::Sandbox,
        record: &RunSandboxInstance,
    ) -> SandboxDetails {
        let fields = daytona_fields_from_sdk_sandbox(sandbox);
        SandboxDetails {
            sandbox:      RunSandboxInstance {
                snapshot: sandbox.snapshot.clone().or_else(|| record.snapshot.clone()),
                ..record.clone()
            },
            state:        fields.state,
            native_state: fields.native_state,
            region:       fields.region,
            web_url:      Some(daytona_dashboard_url(&sandbox.id)),
            resources:    fields.resources,
            network:      fields.network,
            labels:       fields.labels,
            timestamps:   fields.timestamps,
        }
    }

    struct DaytonaFields {
        state:        SandboxState,
        native_state: Option<String>,
        region:       Option<String>,
        resources:    SandboxResources,
        network:      SandboxNetwork,
        labels:       BTreeMap<String, String>,
        timestamps:   SandboxTimestamps,
    }

    fn daytona_fields_from_sdk_sandbox(sandbox: &daytona_sdk::Sandbox) -> DaytonaFields {
        let normalized_state = sandbox
            .state
            .map_or(SandboxState::Unknown, normalize_daytona_state);
        let native_state = sandbox.state.map(|state| state.to_string());

        let resources = SandboxResources {
            cpu_cores:    Some(sandbox.cpu),
            memory_bytes: gibibytes_to_bytes(sandbox.memory),
            disk_bytes:   gibibytes_to_bytes(sandbox.disk),
        };

        let labels: BTreeMap<String, String> = sandbox
            .labels
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();

        let target = sandbox.target.clone();
        let region = if target.is_empty() {
            None
        } else {
            Some(target)
        };

        DaytonaFields {
            state: normalized_state,
            native_state,
            region,
            resources,
            network: daytona_network(
                sandbox.network_block_all,
                sandbox.network_allow_list.as_deref(),
            ),
            labels,
            timestamps: SandboxTimestamps {
                created_at:       sandbox.created_at.as_deref().and_then(parse_rfc3339_utc),
                last_activity_at: sandbox.updated_at.as_deref().and_then(parse_rfc3339_utc),
            },
        }
    }

    /// The Daytona SDK reports CPU/memory/disk as floats in their respective
    /// SI units (cores, GiB, GiB). Convert mem/disk into bytes.
    fn gibibytes_to_bytes(value: f64) -> Option<u64> {
        if value <= 0.0 || !value.is_finite() {
            return None;
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            reason = "Daytona memory/disk values are well within u64 range and only need approximate byte counts."
        )]
        let bytes = (value * 1024.0 * 1024.0 * 1024.0) as u64;
        Some(bytes)
    }

    fn daytona_dashboard_url(sandbox_id: &str) -> String {
        format!("{DAYTONA_DASHBOARD_SANDBOXES_URL}?sandboxId={sandbox_id}")
    }

    fn daytona_network(
        network_block_all: bool,
        network_allow_list: Option<&str>,
    ) -> SandboxNetwork {
        let egress = if network_block_all {
            SandboxNetworkPolicy::blocked()
        } else {
            let cidrs = network_allow_list
                .into_iter()
                .flat_map(|allow_list| allow_list.split(','))
                .map(str::trim)
                .filter(|cidr| !cidr.is_empty());
            let cidrs: Vec<_> = cidrs.collect();
            if cidrs.is_empty() {
                SandboxNetworkPolicy::open()
            } else {
                SandboxNetworkPolicy::allow_cidrs(cidrs)
            }
        };

        SandboxNetwork {
            egress,
            ingress: SandboxNetworkPolicy::blocked(),
        }
    }

    pub(super) fn normalize_daytona_state(state: DaytonaState) -> SandboxState {
        match state {
            DaytonaState::Creating
            | DaytonaState::PendingBuild
            | DaytonaState::BuildingSnapshot
            | DaytonaState::PullingSnapshot
            | DaytonaState::Forking => SandboxState::Provisioning,
            DaytonaState::Starting | DaytonaState::Resuming => SandboxState::Starting,
            DaytonaState::Started | DaytonaState::Snapshotting => SandboxState::Running,
            DaytonaState::Stopping | DaytonaState::Archiving | DaytonaState::Pausing => {
                SandboxState::Stopping
            }
            DaytonaState::Stopped => SandboxState::Stopped,
            DaytonaState::Paused => SandboxState::Paused,
            DaytonaState::Restoring => SandboxState::Restoring,
            DaytonaState::Resizing => SandboxState::Resizing,
            DaytonaState::Archived => SandboxState::Archived,
            DaytonaState::Destroying => SandboxState::Deleting,
            DaytonaState::Destroyed => SandboxState::Deleted,
            DaytonaState::Error | DaytonaState::BuildFailed => SandboxState::Error,
            DaytonaState::Unknown | DaytonaState::UnknownDefaultOpenApi => SandboxState::Unknown,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn started_normalizes_to_running() {
            assert_eq!(
                normalize_daytona_state(DaytonaState::Started),
                SandboxState::Running
            );
        }

        #[test]
        fn creating_normalizes_to_provisioning() {
            assert_eq!(
                normalize_daytona_state(DaytonaState::Creating),
                SandboxState::Provisioning
            );
        }

        #[test]
        fn building_snapshot_normalizes_to_provisioning() {
            assert_eq!(
                normalize_daytona_state(DaytonaState::BuildingSnapshot),
                SandboxState::Provisioning
            );
        }

        #[test]
        fn stopped_normalizes_to_stopped() {
            assert_eq!(
                normalize_daytona_state(DaytonaState::Stopped),
                SandboxState::Stopped
            );
        }

        #[test]
        fn archived_normalizes_to_archived() {
            assert_eq!(
                normalize_daytona_state(DaytonaState::Archived),
                SandboxState::Archived
            );
        }

        #[test]
        fn destroyed_normalizes_to_deleted() {
            assert_eq!(
                normalize_daytona_state(DaytonaState::Destroyed),
                SandboxState::Deleted
            );
        }

        #[test]
        fn build_failed_normalizes_to_error() {
            assert_eq!(
                normalize_daytona_state(DaytonaState::BuildFailed),
                SandboxState::Error
            );
        }

        #[test]
        fn unknown_normalizes_to_unknown() {
            assert_eq!(
                normalize_daytona_state(DaytonaState::Unknown),
                SandboxState::Unknown
            );
        }

        #[test]
        fn pause_states_normalize_to_fabro_states() {
            assert_eq!(
                normalize_daytona_state(DaytonaState::Pausing),
                SandboxState::Stopping
            );
            assert_eq!(
                normalize_daytona_state(DaytonaState::Paused),
                SandboxState::Paused
            );
            assert_eq!(
                normalize_daytona_state(DaytonaState::Resuming),
                SandboxState::Starting
            );
        }

        #[test]
        fn gibibytes_to_bytes_converts_positive_values() {
            assert_eq!(gibibytes_to_bytes(2.0), Some(2 * 1024 * 1024 * 1024));
        }

        #[test]
        fn gibibytes_to_bytes_returns_none_for_zero() {
            assert_eq!(gibibytes_to_bytes(0.0), None);
        }

        #[test]
        fn daytona_dashboard_url_uses_sandbox_id_query_param() {
            assert_eq!(
                daytona_dashboard_url("ad65029a-2d01-421e-8936-49451653fcd9"),
                "https://app.daytona.io/dashboard/sandboxes?sandboxId=ad65029a-2d01-421e-8936-49451653fcd9",
            );
        }

        #[test]
        fn network_block_all_blocks_egress_and_ingress() {
            let network = daytona_network(true, Some("10.0.0.0/8"));
            assert_eq!(network.egress, SandboxNetworkPolicy::blocked());
            assert_eq!(network.ingress, SandboxNetworkPolicy::blocked());
        }

        #[test]
        fn network_allow_list_maps_to_cidr_allow_list_and_blocks_ingress() {
            let network = daytona_network(false, Some("10.0.0.0/8, 192.168.0.0/16 "));
            assert_eq!(
                network.egress,
                SandboxNetworkPolicy::allow_cidrs(["10.0.0.0/8", "192.168.0.0/16"])
            );
            assert_eq!(network.ingress, SandboxNetworkPolicy::blocked());
        }

        #[test]
        fn empty_network_allow_list_is_open_egress_and_blocked_ingress() {
            let network = daytona_network(false, Some(" , "));
            assert_eq!(network.egress, SandboxNetworkPolicy::open());
            assert_eq!(network.ingress, SandboxNetworkPolicy::blocked());
        }

        #[test]
        fn default_daytona_network_is_open_egress_and_blocked_ingress() {
            let network = daytona_network(false, None);
            assert_eq!(network.egress, SandboxNetworkPolicy::open());
            assert_eq!(network.ingress, SandboxNetworkPolicy::blocked());
        }
    }
}

#[cfg(test)]
mod tests {
    use fabro_types::SandboxProviderKind;
    use sandbox_driver::SandboxId;

    use super::*;

    #[test]
    fn driver_states_map_onto_fabro_states() {
        use sandbox_driver::SandboxState as Driver;
        for (driver, fabro) in [
            (Driver::Creating, SandboxState::Provisioning),
            (Driver::Starting, SandboxState::Starting),
            (Driver::Running, SandboxState::Running),
            (Driver::Snapshotting, SandboxState::Running),
            (Driver::Stopping, SandboxState::Stopping),
            (Driver::Stopped, SandboxState::Stopped),
            (Driver::Paused, SandboxState::Paused),
            (Driver::Archived, SandboxState::Archived),
            (Driver::Deleting, SandboxState::Deleting),
            (Driver::Deleted, SandboxState::Deleted),
            (Driver::Error, SandboxState::Error),
            (Driver::Unknown, SandboxState::Unknown),
        ] {
            assert_eq!(normalize_driver_state(driver), fabro, "{driver:?}");
        }
    }

    #[test]
    fn status_projection_carries_identity_source_and_labels() {
        let mut status = sandbox_driver::SandboxStatus::new(
            SandboxId::try_new("container-abc123").unwrap(),
            sandbox_driver::SandboxState::Running,
        );
        status.name = Some("fabro-run-abc".to_string());
        status.provider_state = "running".to_string();
        status.source = Some("buildpack-deps:noble".to_string());
        status
            .labels
            .insert("sh.fabro.managed".to_string(), "true".to_string());
        let mut resources = sandbox_driver::Resources::default();
        resources.cpu_cores = Some(2);
        resources.memory_mb = Some(2048);
        status.resources = Some(resources);

        let info = info_from_status(&SandboxProviderKind::DOCKER, &status);
        assert_eq!(info.id, "container-abc123");
        assert_eq!(info.display_name.as_deref(), Some("fabro-run-abc"));
        assert_eq!(info.state, SandboxState::Running);
        assert_eq!(info.native_state.as_deref(), Some("running"));
        assert_eq!(info.image.as_deref(), Some("buildpack-deps:noble"));
        assert_eq!(info.resources.cpu_cores, Some(2.0));
        assert_eq!(info.resources.memory_bytes, Some(2_147_483_648));
        assert_eq!(
            info.labels.get("sh.fabro.managed").map(String::as_str),
            Some("true")
        );

        let record = RunSandboxInstance {
            provider: SandboxProviderKind::DOCKER,
            image:    None,
            snapshot: None,
            runtime:  fabro_types::RunSandboxRuntime {
                id:                "container-abc123".to_string(),
                working_directory: "/workspace".to_string(),
                repo_cloned:       Some(true),
                clone_origin_url:  None,
                clone_branch:      None,
                workspace_root:    None,
                repos_root:        None,
                primary_repo_path: None,
                primary_repo_link: None,
            },
        };
        let details = details_from_status(&record, &status);
        assert_eq!(
            details.sandbox.image.as_deref(),
            Some("buildpack-deps:noble")
        );
        assert_eq!(details.sandbox.runtime.id, "container-abc123");
        assert_eq!(details.network, SandboxNetwork::unknown());
    }

    #[test]
    fn local_details_returns_running_with_no_metadata() {
        let record = RunSandboxInstance {
            provider: SandboxProviderKind::LOCAL,
            image:    None,
            snapshot: None,
            runtime:  fabro_types::RunSandboxRuntime {
                id:                "local:01JNQVR7M0EJ5GKAT2SC4ERS1Z".to_string(),
                working_directory: "/Users/client/project".to_string(),
                repo_cloned:       None,
                clone_origin_url:  None,
                clone_branch:      None,
                workspace_root:    None,
                repos_root:        None,
                primary_repo_path: None,
                primary_repo_link: None,
            },
        };
        let details = local_details(&record);
        assert_eq!(details.sandbox.provider, SandboxProviderKind::LOCAL);
        assert_eq!(details.state, SandboxState::Running);
        let runtime = &details.sandbox.runtime;
        assert_eq!(runtime.id, "local:01JNQVR7M0EJ5GKAT2SC4ERS1Z");
        assert_eq!(runtime.working_directory, "/Users/client/project");
        assert!(details.region.is_none());
        assert!(details.sandbox.image.is_none());
        assert!(details.labels.is_empty());
        assert_eq!(details.resources, SandboxResources::default());
        assert_eq!(details.network, SandboxNetwork::unknown());
        assert_eq!(details.timestamps, SandboxTimestamps::default());
    }
}
