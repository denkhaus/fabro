//! Daytona provider configuration and allocation rules for clients.

use sandbox_driver_docker_config::DockerProviderConfig;
use serde::{Deserialize, Serialize};

/// The effective memory or disk allocation in MiB for a Daytona request.
///
/// Daytona accepts whole GiB. The driver rounds up, with a minimum of one
/// GiB, and caps the result at the API's signed 32-bit GiB limit. Clients
/// can use this same allocation for minimum checks and snapshot identity
/// and comparison. Keep omitted resources omitted by mapping this function
/// over the optional size; it does not supply provider defaults or validate
/// account quotas.
#[must_use]
pub fn allocation_mib(requested_mib: u64) -> u64 {
    requested_mib.div_ceil(1024).clamp(1, i32::MAX as u64) * 1024
}

/// Optional Daytona extensions. Unknown fields are rejected.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DaytonaProviderConfig {
    /// Route outbound HTTP(S) through this proxy. May contain credentials.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outbound_proxy_url: Option<String>,
    /// Run Docker inside the sandbox. Requires `start-docker` and Python 3.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docker:             Option<NestedDockerConfig>,
}

impl DaytonaProviderConfig {
    pub fn into_value(self) -> serde_json::Value {
        serde_json::to_value(self).expect("Daytona configuration is plain serializable data")
    }
}

/// A container owned by the Daytona sandbox. Its workspace is a bind of
/// the VM workspace; no caller filesystem paths cross the connection.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NestedDockerConfig {
    /// Job image for container execution. Process execution does not create
    /// a helper container and ignores this image.
    #[serde(default)]
    pub image:   String,
    #[serde(default)]
    pub target:  DockerExecutionTarget,
    /// User inside the container. The VM bootstrap still runs as its user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user:    Option<String>,
    #[serde(default)]
    pub options: DockerProviderConfig,
}

/// Where ordinary exec and filesystem operations run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockerExecutionTarget {
    /// Ordinary operations and one-shot containers share the job container.
    #[default]
    Container,
    /// Ordinary operations stay in the VM. One-shots use the VM workspace
    /// and host network namespace; Docker starts only when an action needs it.
    VirtualMachine,
}

#[cfg(test)]
mod tests {
    use super::allocation_mib;

    #[test]
    fn allocation_rounds_up_to_the_provider_granularity() {
        for (requested, expected) in [
            (0, 1024),
            (1, 1024),
            (1024, 1024),
            (1025, 2048),
            (3815, 4096),
            (4096, 4096),
            (4097, 5120),
        ] {
            assert_eq!(allocation_mib(requested), expected);
            assert_eq!(allocation_mib(expected), expected);
        }
    }

    #[test]
    fn allocation_respects_the_api_limit_without_overflow() {
        let limit = i32::MAX as u64 * 1024;
        for requested in [limit - 1, limit, limit + 1, u64::MAX] {
            assert_eq!(allocation_mib(requested), limit);
        }
    }
}
