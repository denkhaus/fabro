use std::collections::BTreeMap;

use anyhow::Result;
use chrono::{DateTime, Utc};
use fabro_types::{
    BundledProvider, RunId, RunSandboxInstance, SandboxDetails, SandboxNetwork,
    SandboxProviderKind, SandboxResources, SandboxState, SandboxTimestamps,
};

use crate::driver::ProviderAccess;
use crate::reconnect;

/// Inspect the sandbox identified by `record` and return provider-neutral
/// details for control-plane display.
///
/// `local` always returns a minimal record describing the host; every other
/// provider is described through the sandbox driver.
pub async fn sandbox_details(
    record: &RunSandboxInstance,
    access: &ProviderAccess,
    run_id: Option<RunId>,
) -> Result<SandboxDetails> {
    if record.provider.bundled() == Some(BundledProvider::Local) {
        return Ok(local_details(record));
    }
    let sandbox = reconnect::reconnect_driver_for_run(record, access, run_id, None).await?;
    let status = sandbox.handle()?.describe().await.map_err(|err| {
        anyhow::anyhow!(
            "Failed to describe {} sandbox '{}': {err}",
            record.provider,
            record.runtime.id
        )
    })?;
    Ok(details_from_status(record, &status))
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
            image: (record.provider == SandboxProviderKind::DOCKER)
                .then(|| status.source.clone())
                .flatten()
                .or_else(|| record.image.clone()),
            snapshot: (record.provider == SandboxProviderKind::DAYTONA)
                .then(|| status.source.clone())
                .flatten()
                .or_else(|| record.snapshot.clone()),
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

#[cfg(test)]
mod tests {
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
