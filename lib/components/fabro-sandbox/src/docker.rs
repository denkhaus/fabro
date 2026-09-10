//! The `docker` provider kind: what fabro adds to a run's spec for the
//! sandbox-driver Docker provider.
//!
//! The environment's options build the spec once; Docker's overlay fixes the
//! container's working directory at [`WORKING_DIRECTORY`], supplies the
//! default image when the environment names none, and asks the provider to
//! pull a missing image. A cloned repository checks out under
//! [`REPOS_ROOT`] and is linked into the workspace, so the run works in
//! `/workspace/<repo>`.

use sandbox_driver::{HealthStatus, SandboxSource, SandboxSpec as DriverSpec};
use sandbox_driver_docker_config::DockerProviderConfig;

use crate::driver::ProviderAccess;
use crate::driver_sandbox::WorkspaceLayout;
use crate::options::SandboxOptions;
use crate::provider_sandbox;

pub const WORKING_DIRECTORY: &str = "/workspace";
pub const REPOS_ROOT: &str = "/repos";
/// The image a Docker environment gets when it names none.
pub const DEFAULT_IMAGE: &str = "buildpack-deps:noble";

/// The workspace layout every Docker sandbox uses.
pub(crate) fn layout() -> WorkspaceLayout {
    WorkspaceLayout {
        workspace_root: WORKING_DIRECTORY.to_string(),
        repos_root:     REPOS_ROOT.to_string(),
    }
}

/// The image a Docker sandbox runs: the environment's, or the default.
pub(crate) fn effective_image(options: &SandboxOptions) -> String {
    options
        .image
        .clone()
        .unwrap_or_else(|| DEFAULT_IMAGE.to_string())
}

/// Docker's additions to the base spec, and the image it will run.
pub(crate) fn overlay(spec: DriverSpec, options: &SandboxOptions) -> (DriverSpec, String) {
    let image = effective_image(options);
    let mut spec = spec;
    spec.source = SandboxSource::Image {
        reference: image.clone(),
    };
    let spec = spec.working_directory(WORKING_DIRECTORY).provider_config(
        DockerProviderConfig {
            auto_pull: true,
            ..DockerProviderConfig::default()
        }
        .into_value(),
    );
    (spec, image)
}

/// Whether the Docker daemon answers. Used by `fabro doctor`.
pub async fn check_docker_daemon() -> crate::Result<()> {
    let provider = provider_sandbox::connect_bundled_docker(&ProviderAccess::default()).await?;
    let health = provider
        .health()
        .await
        .map_err(|error| crate::Error::context("Docker health check failed", error))?;
    match health.status {
        HealthStatus::Ok | HealthStatus::Unknown => Ok(()),
        HealthStatus::Unreachable | HealthStatus::Unauthorized => {
            Err(crate::Error::message(health.message.unwrap_or_else(|| {
                "Failed to reach Docker daemon".to_string()
            })))
        }
        _ => Err(crate::Error::message(
            "Docker daemon reported an unknown health state",
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use fabro_types::RunId;
    use sandbox_driver::NetworkPolicy;

    use super::*;
    use crate::options::base_spec;

    #[test]
    fn overlay_fixes_the_workspace_and_pulls_the_named_image() {
        let run_id: RunId = "01HY0000000000000000000000".parse().unwrap();
        let options = SandboxOptions {
            image: Some("ghcr.io/acme/dev:1".to_string()),
            env: BTreeMap::from([("FOO".to_string(), "bar".to_string())]),
            memory_bytes: Some(4_000_000_000),
            cpu: Some(2),
            network: NetworkPolicy::Block,
            ..SandboxOptions::default()
        };
        let (spec, image) = overlay(base_spec(&options, Some(&run_id)), &options);

        assert_eq!(image, "ghcr.io/acme/dev:1");
        assert!(matches!(
            &spec.source,
            SandboxSource::Image { reference } if reference == "ghcr.io/acme/dev:1"
        ));
        assert_eq!(
            spec.name.as_deref(),
            Some("fabro-run-01HY0000000000000000000000")
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
        assert_eq!(spec.env.get("FOO").map(String::as_str), Some("bar"));
        assert_eq!(spec.resources.cpu_cores, Some(2));
        assert_eq!(spec.resources.memory_mb, Some(3815));
        assert!(matches!(spec.network, NetworkPolicy::Block));
        assert_eq!(spec.provider_config["auto_pull"], true);
    }

    #[test]
    fn overlay_supplies_the_default_image_when_the_environment_names_none() {
        let options = SandboxOptions::default();
        let (spec, image) = overlay(base_spec(&options, None), &options);
        assert_eq!(image, DEFAULT_IMAGE);
        assert!(matches!(
            &spec.source,
            SandboxSource::Image { reference } if reference == DEFAULT_IMAGE
        ));
        assert!(spec.name.is_none());
        assert!(!spec.labels.contains_key("sh.fabro.run_id"));
    }
}
