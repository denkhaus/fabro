use super::*;

#[test]
fn system_sandbox_provider_uses_manifest_defaults() {
    let (_environment_temp, environment_store) =
        test_environment_store(Some(SandboxProviderKind::DAYTONA), true);
    let (_mcp_temp, mcp_server_store) = test_mcp_server_store();
    let source = r#"
_version = 1

[run.environment]
id = "default"
"#;
    let manifest_run_settings = resolve_manifest_run_settings_with_catalog(
        &run_manifest::manifest_run_defaults(Some(&manifest_run_defaults_from_toml(source))),
        &environment_store,
        &mcp_server_store,
    );

    assert_eq!(system_sandbox_provider(&manifest_run_settings), "daytona");
}

#[test]
fn system_sandbox_provider_defaults_when_manifest_run_settings_do_not_resolve() {
    let (_environment_temp, environment_store) = test_environment_store(None, true);
    let (_mcp_temp, mcp_server_store) = test_mcp_server_store();
    let source = r#"
_version = 1

[run.environment]
id = "missing"
"#;
    let manifest_run_settings = resolve_manifest_run_settings_with_catalog(
        &run_manifest::manifest_run_defaults(Some(&manifest_run_defaults_from_toml(source))),
        &environment_store,
        &mcp_server_store,
    );

    assert_eq!(
        system_sandbox_provider(&manifest_run_settings),
        SandboxProviderKind::default().to_string()
    );
}

#[test]
fn sandbox_provider_policy_error_reports_disabled_provider() {
    let settings = server_settings_from_toml(
        r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[server.sandbox.providers.daytona]
enabled = false
"#,
    );

    assert_eq!(
        crate::run_manifest::sandbox_provider_policy_error(
            &settings,
            &SandboxProviderKind::DAYTONA
        )
        .as_deref(),
        Some(
            "sandbox provider \"daytona\" is disabled by server.sandbox.providers.daytona.enabled"
        )
    );
}

#[test]
fn clone_sandbox_credentials_are_available_for_clone_based_providers() {
    use fabro_types::SandboxProviderKind;
    assert!(SandboxProviderKind::DOCKER.clones_workspace());
    assert!(SandboxProviderKind::DAYTONA.clones_workspace());
    assert!(!SandboxProviderKind::LOCAL.clones_workspace());
}
