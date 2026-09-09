//! The construction function serves a non-bundled kind through a plugin
//! executable, and a sandbox created through one plugin generation is
//! reachable by persisted id from a fresh connection.

use std::collections::BTreeMap;

use fabro_sandbox::driver::{ProviderConnectOptions, connect_provider};
use fabro_types::SandboxProviderKind;
use fabro_types::settings::server::{SandboxPluginSettings, ServerSandboxProviderSettings};
use sandbox_driver::{ExecSpec, SandboxId, SandboxSource, SandboxSpec};

const HOST_PLUGIN: &str = env!("CARGO_BIN_EXE_fabro-sandbox-host");

fn host_plugin_settings(registry: &std::path::Path) -> ServerSandboxProviderSettings {
    ServerSandboxProviderSettings {
        enabled: true,
        plugin:  Some(SandboxPluginSettings {
            path:        Some(HOST_PLUGIN.to_string()),
            sha256:      None,
            dev:         true,
            args:        Vec::new(),
            env:         BTreeMap::from([(
                "SANDBOX_DRIVER_HOST_REGISTRY".to_string(),
                registry.display().to_string(),
            )]),
            inherit_env: Vec::new(),
        }),
    }
}

#[tokio::test]
async fn host_plugin_under_a_non_bundled_kind_creates_and_reattaches_by_persisted_id() {
    let registry = tempfile::tempdir().expect("registry tempdir");
    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let kind = SandboxProviderKind::try_new("host").expect("host is a valid kind");
    assert_eq!(
        kind.bundled(),
        None,
        "host is not one of fabro's bundled kinds"
    );
    let settings = host_plugin_settings(registry.path());

    let persisted_id: SandboxId = {
        let connected = connect_provider(&kind, &settings, &ProviderConnectOptions::default())
            .await
            .expect("plugin launches");
        assert_eq!(connected.kind, kind);
        assert_eq!(connected.provider.kind().as_str(), "host");
        let spec = SandboxSpec::new(SandboxSource::HostDirectory)
            .working_directory(workspace.path().display().to_string())
            .label("sh.fabro.managed", "true");
        let sandbox = connected
            .provider
            .create(&spec, None)
            .await
            .expect("create over the wire");
        let result = sandbox
            .exec()
            .run(&ExecSpec::bash(
                "printf hello > marker.txt && cat marker.txt",
            ))
            .await
            .expect("exec over the wire");
        assert!(result.success(), "{result:?}");
        assert_eq!(result.stdout_lossy(), "hello");
        sandbox.id().clone()
    };

    // A fresh connection is a new plugin process; the id alone must be
    // enough to find the sandbox again, exactly as run reconnect will do.
    let connected = connect_provider(&kind, &settings, &ProviderConnectOptions::default())
        .await
        .expect("plugin relaunches");
    let sandbox = connected
        .provider
        .attach(&persisted_id, None)
        .await
        .expect("attach by persisted id");
    let content = sandbox
        .fs()
        .read("marker.txt")
        .await
        .expect("file survives across plugin generations");
    assert_eq!(content, b"hello");
    assert!(workspace.path().join("marker.txt").is_file());
    sandbox.delete().await.expect("delete releases the handle");
    assert!(
        workspace.path().is_dir(),
        "designated directories are never removed by delete"
    );
}

#[tokio::test]
async fn a_plugin_that_declares_another_kind_is_rejected() {
    let registry = tempfile::tempdir().expect("registry tempdir");
    let kind = SandboxProviderKind::try_new("e2b").expect("valid kind");
    let error = connect_provider(
        &kind,
        &host_plugin_settings(registry.path()),
        &ProviderConnectOptions::default(),
    )
    .await
    .err()
    .expect("the host executable declares `host`, not `e2b`");
    let rendered = format!(
        "{error}: {:?}",
        std::error::Error::source(&error).map(ToString::to_string)
    );
    assert!(rendered.contains("e2b"), "{rendered}");
}
