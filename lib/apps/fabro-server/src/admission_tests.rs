//! Admission-path tests that run through the server's app state: the moved
//! admission API (fabro-admission, work#1672) is exercised the same way the
//! endpoints drive it — an `AppState`, its derived `AdmissionContext`, and
//! the server's `RuntimeSpec`. The bodies are the original `run_manifest`
//! test mod, unchanged apart from the imports and the provider-health
//! callback swap.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use fabro_admission::petri_check::{self, Readiness};
use fabro_admission::run_manifest::*;
use fabro_api::types;
use fabro_config::{EnvironmentLayer, MergeMap, RunLayer};
use fabro_github::token_source::TokenSnapshot;
use fabro_static::EnvVars;
use fabro_types::SandboxProviderKind;
use fabro_types::diagnostic::Severity;
use fabro_types::settings::InterpString;
use fabro_types::settings::run::RunNamespace;
use fabro_util::check_report::CheckStatus;
use fabro_workflow::Error as WorkflowError;
use lithos_llm::catalog::ProviderId;
use sandbox_driver::{HealthStatus, ProviderHealth};
use tokio::process::Command;
use tokio::time;

use crate::server::AppState;

/// The manifest run defaults helper the preflight fixtures share (was the
/// `run_manifest` cfg(test) helper before the extraction, work#1672).
pub(crate) fn manifest_run_defaults(run: Option<&RunLayer>) -> RunLayer {
    run.cloned().unwrap_or_default()
}

/// The same call the app wires: the server's provider-health machinery over
/// default (empty) provider access.
fn default_provider_health() -> fabro_admission::ProviderHealthSource {
    Arc::new(|kind| {
        let kind = kind.clone();
        Box::pin(async move {
            crate::sandbox_access::provider_health(
                &kind,
                &crate::sandbox_access::ProviderAccess::default(),
            )
            .await
        })
    })
}

#[cfg(test)]
mod tests {

    use super::*;

    /// Validate as the endpoints do: Petri's check with the state's model
    /// client over `ready_providers`.
    fn validate_for_test(
        state: &AppState,
        prepared: &PreparedManifest,
        ready_providers: &[ProviderId],
    ) -> Result<ManifestCheck, WorkflowError> {
        let launch = petri_check::launch(
            &state.catalog(),
            &prepared.settings,
            ready_providers,
            None,
            None,
        );
        let runtime = crate::server::petri_runs::runtime_spec(
            state,
            ready_providers,
            false,
            state.sandbox_provider_config(None),
            // Admission installs no hook-secrets provider (fabro-225f).
            None,
        );
        validate_prepared_manifest(
            prepared,
            &HashMap::new(),
            launch,
            runtime,
            &Readiness::Server {
                catalog: state.catalog(),
                ready:   ready_providers.to_vec(),
                force:   false,
            },
            false,
        )
    }

    /// Validate with every provider of the state's catalog eligible, as the
    /// legacy catalog validation judged a manifest.
    fn validate_with_catalog(
        state: &AppState,
        prepared: &PreparedManifest,
    ) -> Result<ManifestCheck, WorkflowError> {
        let providers = state
            .catalog()
            .enabled_provider_ids()
            .into_iter()
            .collect::<Vec<_>>();
        validate_for_test(state, prepared, &providers)
    }

    #[tokio::test]
    async fn ls_remote_capture_keeps_diagnostic_precedence_and_unlimited_output() {
        for (script, expected) in [
            ("printf stdout; printf stderr >&2; exit 7", "stderr"),
            ("printf stdout; exit 7", "stdout"),
        ] {
            let mut command = Command::new("sh");
            command.args(["-c", script]);
            assert_eq!(super::run_ls_remote(command).await.unwrap_err(), expected);
        }
        let mut command = Command::new("sh");
        command.args(["-c", "head -c 200000 /dev/zero | tr '\\0' x >&2; exit 7"]);
        assert_eq!(
            super::run_ls_remote(command).await.unwrap_err().len(),
            200_000
        );
        let mut command = Command::new("sh");
        command.args(["-c", "exit 7"]);
        assert!(
            super::run_ls_remote(command)
                .await
                .unwrap_err()
                .starts_with("git ls-remote exited with status")
        );
    }

    fn minimal_manifest() -> types::RunManifest {
        types::RunManifest {
            args:      None,
            configs:   Vec::new(),
            cwd:       "/tmp/project".to_string(),
            git:       None,
            goal:      None,
            parent_id: None,
            title:     None,
            target:    types::ManifestTarget {
                path: "workflow.fabro".to_string(),
            },
            version:   1,
            workflows: HashMap::from([("workflow.fabro".to_string(), types::ManifestWorkflow {
                config: None,
                files:  HashMap::new(),
                source:
                    "digraph Demo { start [shape=Mdiamond] exit [shape=Msquare] start -> exit }"
                        .to_string(),
            })]),
        }
    }

    fn invalid_manifest() -> types::RunManifest {
        types::RunManifest {
            workflows: HashMap::from([("workflow.fabro".to_string(), types::ManifestWorkflow {
                config: None,
                files:  HashMap::new(),
                source: "digraph Invalid { exit [shape=Msquare] orphan exit -> orphan }"
                    .to_string(),
            })]),
            ..minimal_manifest()
        }
    }

    fn server_settings_fixture(source: &str) -> RunLayer {
        let mut document: toml::Table = source.parse().expect("v2 fixture should parse");
        document
            .remove("run")
            .map(toml::Value::try_into::<RunLayer>)
            .transpose()
            .expect("run settings should parse")
            .unwrap_or_default()
    }

    fn default_settings_fixture() -> RunLayer {
        RunLayer::default()
    }

    fn environment_defaults_fixture() -> MergeMap<EnvironmentLayer> {
        MergeMap::from(HashMap::from([
            ("default".to_string(), EnvironmentLayer {
                provider: Some("local".to_string()),
                ..EnvironmentLayer::default()
            }),
            ("local".to_string(), EnvironmentLayer {
                provider: Some("local".to_string()),
                ..EnvironmentLayer::default()
            }),
            ("daytona".to_string(), EnvironmentLayer {
                provider: Some("daytona".to_string()),
                ..EnvironmentLayer::default()
            }),
            ("selected".to_string(), EnvironmentLayer {
                provider: Some("docker".to_string()),
                ..EnvironmentLayer::default()
            }),
            ("cloud".to_string(), EnvironmentLayer {
                provider: Some("daytona".to_string()),
                ..EnvironmentLayer::default()
            }),
        ]))
    }

    fn prepare_manifest(
        manifest_run_defaults: &RunLayer,
        manifest: &types::RunManifest,
    ) -> Result<PreparedManifest> {
        super::prepare_manifest_with_environment_defaults(
            manifest_run_defaults,
            &environment_defaults_fixture(),
            &HashMap::new(),
            manifest,
        )
    }

    fn ready_moonshot_and_openrouter_state(
        server: &httpmock::MockServer,
    ) -> Arc<crate::server::AppState> {
        let moonshot_url = server.url("/moonshot/v1");
        let openrouter_url = server.url("/openrouter/v1");
        crate::test_support::TestAppStateBuilder::new()
            .llm_overlay_toml(&format!(
                r#"
[providers.moonshot]
base_url = "{moonshot_url}"

[providers.openrouter]
base_url = "{openrouter_url}"
enabled = true

"#
            ))
            .vault_entries([
                (EnvVars::KIMI_API_KEY, "test-moonshot-key"),
                (EnvVars::OPENROUTER_API_KEY, "test-openrouter-key"),
            ])
            .build()
    }

    /// Preflight for a workflow naming `model`, which Petri refuses.
    async fn preflight_for_refused_model(
        state: &Arc<crate::server::AppState>,
        model: &str,
    ) -> (types::PreflightResponse, bool) {
        let (_, ready_providers) = state.resolve_llm_client_with_ready_ids().await;
        let mut manifest = minimal_manifest();
        manifest.workflows.get_mut("workflow.fabro").unwrap().source = format!(
            r#"
digraph Demo {{
    start [shape=Mdiamond]
    exit  [shape=Msquare]
    work  [prompt="Do work", model="{model}"]
    start -> work -> exit
}}
"#
        );
        let prepared = prepare_manifest(
            &manifest_run_defaults(Some(&default_settings_fixture())),
            &manifest,
        )
        .unwrap();
        let validated = validate_for_test(state, &prepared, &ready_providers).unwrap();
        assert!(validated.has_errors(), "{:?}", validated.diagnostics);

        run_preflight(&state.admission_context(), &prepared, &validated)
            .await
            .unwrap()
    }

    async fn resolve_and_run_preflight(
        state: &Arc<AppState>,
        prepared: &PreparedManifest,
        check: &ManifestCheck,
    ) -> Result<(types::PreflightResponse, bool)> {
        run_preflight(&state.admission_context(), prepared, check).await
    }

    fn manifest_workflow() -> types::ManifestWorkflow {
        types::ManifestWorkflow {
            config: None,
            files:  HashMap::new(),
            source: "digraph Demo { start [shape=Mdiamond] exit [shape=Msquare] start -> exit }"
                .to_string(),
        }
    }

    fn manifest_file(content: &str) -> types::ManifestFileEntry {
        types::ManifestFileEntry {
            content: content.to_string(),
            ref_:    types::ManifestFileRef {
                from:     Some("workflow.fabro".to_string()),
                original: "prompt.md".to_string(),
                type_:    types::ManifestFileRefType::FileInline,
            },
        }
    }

    fn git_context(origin_url: &str, branch: &str) -> types::GitContext {
        types::GitContext {
            origin_url: origin_url.to_string(),
            branch:     branch.to_string(),
            sha:        None,
            dirty:      fabro_types::DirtyStatus::Clean,
        }
    }

    fn prepared_and_resolved_for_sandbox(
        provider: &SandboxProviderKind,
        clone_enabled: bool,
        git: Option<types::GitContext>,
    ) -> (PreparedManifest, RunNamespace) {
        let mut manifest = minimal_manifest();
        manifest.git = git;
        manifest.configs.push(types::ManifestConfig {
            path:   Some("/tmp/project/.fabro/project.toml".to_string()),
            source: Some(format!(
                r#"
_version = 1

[run.environment]
id = "selected"

[run.clone]
enabled = {clone_enabled}
"#
            )),
            type_:  types::ManifestConfigType::Project,
        });
        let mut environment_defaults = environment_defaults_fixture();
        environment_defaults.insert("selected".to_string(), EnvironmentLayer {
            provider: Some(provider.to_string()),
            ..EnvironmentLayer::default()
        });

        let prepared = super::prepare_manifest_with_environment_defaults(
            &manifest_run_defaults(Some(&default_settings_fixture())),
            &environment_defaults,
            &HashMap::new(),
            &manifest,
        )
        .unwrap();
        let resolved = prepared.settings.run.clone();

        (prepared, resolved)
    }

    #[test]
    fn docker_environment_cwd_is_reported_as_ignored() {
        let mut resolved = RunNamespace::default();
        resolved.environment.provider = SandboxProviderKind::DOCKER;
        resolved.environment.cwd = Some("/workspace/custom".to_string());

        assert_eq!(environment_capability_warnings(&resolved), vec![
            "docker provider ignores cwd".to_string()
        ]);
    }

    #[test]
    fn daytona_environment_cwd_is_reported_as_ignored() {
        let mut resolved = RunNamespace::default();
        resolved.environment.provider = SandboxProviderKind::DAYTONA;
        resolved.environment.cwd = Some("/home/daytona/workspace/custom".to_string());

        assert_eq!(environment_capability_warnings(&resolved), vec![
            "daytona provider ignores cwd".to_string()
        ]);
    }

    #[test]
    fn prepare_manifest_accepts_project_environment_catalog_definitions() {
        let mut manifest = minimal_manifest();
        manifest.configs.push(types::ManifestConfig {
            path:   Some(".fabro/project.toml".to_string()),
            source: Some(
                r#"_version = 1

[run.environment]
id = "cloud"

[environments.cloud]
provider = "local"
"#
                .to_string(),
            ),
            type_:  types::ManifestConfigType::Project,
        });

        let prepared = prepare_manifest(
            &manifest_run_defaults(Some(&default_settings_fixture())),
            &manifest,
        )
        .expect("project environment catalog should resolve");

        assert_eq!(
            prepared.settings.run.environment.provider,
            SandboxProviderKind::LOCAL
        );
    }

    #[tokio::test]
    async fn repository_access_check_skips_when_clone_is_disabled() {
        let (prepared, resolved) = prepared_and_resolved_for_sandbox(
            &SandboxProviderKind::DOCKER,
            false,
            Some(git_context("https://github.com/acme/widgets", "main")),
        );
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let calls_for_check = Arc::clone(&calls);
        let mut checks = Vec::new();

        let ok = run_repository_access_check_with(
            &mut checks,
            &SandboxProviderKind::DOCKER,
            &prepared,
            &resolved,
            None,
            move |request, _github_app| {
                calls_for_check.lock().unwrap().push(request);
                async { Ok(()) }
            },
        )
        .await;

        assert!(ok);
        assert!(checks.is_empty());
        assert!(calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn repository_access_check_rejects_non_github_origins_before_remote_probe() {
        let (prepared, resolved) = prepared_and_resolved_for_sandbox(
            &SandboxProviderKind::DOCKER,
            true,
            Some(git_context("https://gitlab.com/acme/widgets", "main")),
        );
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let calls_for_check = Arc::clone(&calls);
        let mut checks = Vec::new();

        let ok = run_repository_access_check_with(
            &mut checks,
            &SandboxProviderKind::DOCKER,
            &prepared,
            &resolved,
            None,
            move |request, _github_app| {
                calls_for_check.lock().unwrap().push(request);
                async { Ok(()) }
            },
        )
        .await;

        assert!(!ok);
        assert!(calls.lock().unwrap().is_empty());
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].name, "Repository Access");
        assert_eq!(checks[0].status, CheckStatus::Error);
        assert!(
            checks[0]
                .remediation
                .as_deref()
                .unwrap_or_default()
                .contains("GitHub repository origins only")
        );
    }

    #[tokio::test]
    async fn repository_access_check_probes_normalized_github_branch() {
        let (prepared, resolved) = prepared_and_resolved_for_sandbox(
            &SandboxProviderKind::DOCKER,
            true,
            Some(git_context(
                "git@github.com:acme/widgets.git",
                "feature/demo",
            )),
        );
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let calls_for_check = Arc::clone(&calls);
        let mut checks = Vec::new();

        let ok = run_repository_access_check_with(
            &mut checks,
            &SandboxProviderKind::DOCKER,
            &prepared,
            &resolved,
            None,
            move |request, _github_app| {
                calls_for_check.lock().unwrap().push(request);
                async { Ok(()) }
            },
        )
        .await;

        assert!(ok);
        assert_eq!(calls.lock().unwrap().as_slice(), [GitRemoteRefCheck {
            origin_url: "https://github.com/acme/widgets".to_string(),
            branch:     Some("feature/demo".to_string()),
        }]);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].name, "Repository Access");
        assert_eq!(checks[0].status, CheckStatus::Pass);
    }

    #[tokio::test]
    async fn repository_access_check_surfaces_remote_probe_failure() {
        let (prepared, resolved) = prepared_and_resolved_for_sandbox(
            &SandboxProviderKind::DOCKER,
            true,
            Some(git_context("https://github.com/acme/widgets", "missing")),
        );
        let mut checks = Vec::new();

        let ok = run_repository_access_check_with(
            &mut checks,
            &SandboxProviderKind::DOCKER,
            &prepared,
            &resolved,
            None,
            |_request, _github_app| async { Err("remote branch not found".to_string()) },
        )
        .await;

        assert!(!ok);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].name, "Repository Access");
        assert_eq!(checks[0].status, CheckStatus::Error);
        assert!(
            checks[0]
                .remediation
                .as_deref()
                .unwrap_or_default()
                .contains("remote branch not found")
        );
    }

    fn health(status: HealthStatus, message: Option<&str>) -> ProviderHealth {
        let mut health = ProviderHealth::new(status);
        health.message = message.map(str::to_string);
        health
    }

    #[test]
    fn a_healthy_or_uncheckable_provider_passes_the_sandbox_check() {
        for status in [HealthStatus::Ok, HealthStatus::Unknown] {
            let check =
                sandbox_health_check(&SandboxProviderKind::DOCKER, Ok(health(status, None)));
            assert_eq!(check.status, CheckStatus::Pass, "{status:?}");
            assert_eq!(check.summary, "docker");
            assert_eq!(check.details[0].text, "Provider: docker");
            assert!(check.remediation.is_none());
        }
    }

    #[test]
    fn an_unreachable_backend_fails_the_sandbox_check_with_its_reason() {
        let check = sandbox_health_check(
            &SandboxProviderKind::DOCKER,
            Ok(health(
                HealthStatus::Unreachable,
                Some("connection refused on /var/run/docker.sock"),
            )),
        );
        assert_eq!(check.status, CheckStatus::Error);
        assert_eq!(check.summary, "failed");
        assert_eq!(
            check.remediation.as_deref(),
            Some("docker backend is unreachable: connection refused on /var/run/docker.sock")
        );
    }

    #[test]
    fn a_rejected_credential_fails_the_sandbox_check_naming_the_missing_scopes() {
        let mut unauthorized = health(HealthStatus::Unauthorized, Some("forbidden"));
        unauthorized.missing_permissions = vec!["write:sandboxes".to_string()];
        let check = sandbox_health_check(&SandboxProviderKind::DAYTONA, Ok(unauthorized));
        assert_eq!(check.status, CheckStatus::Error);
        assert_eq!(
            check.remediation.as_deref(),
            Some("daytona credential is missing required permissions: write:sandboxes")
        );

        let check = sandbox_health_check(
            &SandboxProviderKind::DAYTONA,
            Ok(health(HealthStatus::Unauthorized, Some("bad key"))),
        );
        assert_eq!(
            check.remediation.as_deref(),
            Some("daytona rejected the credential: bad key")
        );
    }

    #[test]
    fn a_provider_that_cannot_connect_fails_the_sandbox_check_with_the_connect_error() {
        let check = sandbox_health_check(
            &SandboxProviderKind::DAYTONA,
            Err("Daytona sandboxes require DAYTONA_API_KEY in the vault".to_string()),
        );
        assert_eq!(check.status, CheckStatus::Error);
        assert_eq!(
            check.remediation.as_deref(),
            Some("Daytona sandboxes require DAYTONA_API_KEY in the vault")
        );
    }

    #[tokio::test]
    async fn the_local_sandbox_check_passes_through_the_host_providers_health() {
        let (mut prepared, resolved) =
            prepared_and_resolved_for_sandbox(&SandboxProviderKind::LOCAL, false, None);
        // The local check resolves the run's working directory from the
        // manifest's source directory, which must exist on this server.
        let source = tempfile::tempdir().expect("a source directory");
        prepared.source_directory = source.path().to_path_buf();
        let mut checks = Vec::new();
        let passed = run_sandbox_check(
            &mut checks,
            &SandboxProviderKind::LOCAL,
            &prepared,
            &resolved,
            &default_provider_health(),
        )
        .await;
        assert!(passed, "{checks:?}");
        assert_eq!(checks[0].name, "Sandbox");
        assert_eq!(checks[0].status, CheckStatus::Pass);
    }

    #[tokio::test]
    async fn the_daytona_sandbox_check_fails_without_a_vault_key() {
        let (prepared, resolved) =
            prepared_and_resolved_for_sandbox(&SandboxProviderKind::DAYTONA, false, None);
        let mut checks = Vec::new();
        let passed = run_sandbox_check(
            &mut checks,
            &SandboxProviderKind::DAYTONA,
            &prepared,
            &resolved,
            &default_provider_health(),
        )
        .await;
        assert!(!passed);
        assert_eq!(checks[0].status, CheckStatus::Error);
        assert!(
            checks[0]
                .remediation
                .as_deref()
                .unwrap_or_default()
                .contains("DAYTONA_API_KEY"),
            "{checks:?}"
        );
    }

    fn snapshot(age: Duration) -> TokenSnapshot {
        let now = chrono::Utc::now();
        TokenSnapshot {
            generation: 1,
            provenance: fabro_github::token_source::TokenProvenance::Minted {
                minted_at:  now - chrono::Duration::from_std(age).unwrap(),
                expires_at: now + chrono::Duration::hours(1),
            },
        }
    }

    fn static_snapshot() -> TokenSnapshot {
        TokenSnapshot {
            generation: 0,
            provenance: fabro_github::token_source::TokenProvenance::Static,
        }
    }

    #[test]
    fn probe_backoff_paces_at_replication_intervals() {
        let backoff = repository_probe_policy().backoff;
        assert_eq!(backoff.delay_after(1), Duration::from_secs(3));
        assert_eq!(backoff.delay_after(2), Duration::from_secs(9));
    }

    #[tokio::test(start_paused = true)]
    async fn host_side_retries_keep_the_operations_own_message() {
        let calls = Mutex::new(0_u32);
        let result = retry_git_messages(
            &repository_probe_policy(),
            Some(&snapshot(Duration::from_secs(1))),
            "repository probe",
            || {
                let attempt = {
                    let mut calls = calls.lock().unwrap();
                    *calls += 1;
                    *calls
                };
                async move {
                    if attempt < 3 {
                        Err(format!("remote: Repository not found. (attempt {attempt})"))
                    } else {
                        Ok(())
                    }
                }
            },
        )
        .await;
        assert_eq!(result, Ok(()));
        assert_eq!(*calls.lock().unwrap(), 3);

        let permanent = retry_git_messages(
            &repository_probe_policy(),
            Some(&static_snapshot()),
            "repository probe",
            || async { Err("remote: Repository not found.".to_owned()) },
        )
        .await;
        assert_eq!(permanent, Err("remote: Repository not found.".to_owned()));
    }

    #[test]
    fn workflow_bundle_rejects_duplicate_canonical_workflow_keys() {
        let workflows = HashMap::from([
            ("bar.fabro".to_string(), manifest_workflow()),
            ("./foo/../bar.fabro".to_string(), manifest_workflow()),
        ]);

        let error = workflow_bundle_from_manifest(&workflows).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("duplicate canonical workflow key: bar.fabro")
        );
    }

    #[test]
    fn workflow_bundle_rejects_duplicate_canonical_file_keys() {
        let mut workflow = manifest_workflow();
        workflow.files = HashMap::from([
            ("prompts/hello.md".to_string(), manifest_file("first")),
            ("./prompts/./hello.md".to_string(), manifest_file("second")),
        ]);
        let workflows = HashMap::from([("workflow.fabro".to_string(), workflow)]);

        let error = workflow_bundle_from_manifest(&workflows).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("duplicate canonical file key: prompts/hello.md")
        );
    }

    #[test]
    fn workflow_bundle_rejects_invalid_workflow_key() {
        let workflows = HashMap::from([("/abs/path.fabro".to_string(), manifest_workflow())]);

        let error = workflow_bundle_from_manifest(&workflows).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("invalid manifest workflow key: /abs/path.fabro")
        );
    }

    #[test]
    fn workflow_bundle_rejects_invalid_file_key() {
        let mut workflow = manifest_workflow();
        workflow.files = HashMap::from([("~/foo.md".to_string(), manifest_file("content"))]);
        let workflows = HashMap::from([("workflow.fabro".to_string(), workflow)]);

        let error = workflow_bundle_from_manifest(&workflows).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("invalid manifest file key: ~/foo.md")
        );
    }

    #[test]
    fn prepare_manifest_preserves_explicit_manifest_dry_run() {
        let server_settings = manifest_run_defaults(Some(&server_settings_fixture(
            r#"
_version = 1

[run.execution]
mode = "dry_run"

[server.storage]
root = "/srv/fabro"
"#,
        )));
        let mut manifest = minimal_manifest();
        manifest.args = Some(types::ManifestArgs {
            auto_approve:     None,
            dry_run:          Some(true),
            label:            Vec::new(),
            model:            None,
            preserve_sandbox: None,
            provider:         None,
            environment:      None,
            input:            Vec::new(),
            verbose:          None,
        });

        let prepared = prepare_manifest(&server_settings, &manifest).unwrap();

        assert_eq!(
            prepared.settings.run.execution.mode,
            fabro_types::settings::run::RunMode::DryRun
        );
    }

    #[test]
    fn prepare_manifest_applies_input_args_as_sparse_overrides() {
        let server_settings = manifest_run_defaults(Some(&server_settings_fixture(
            r#"
_version = 1

[run.inputs]
keep = "server"
override = "server"
"#,
        )));
        let mut manifest = minimal_manifest();
        manifest.args = Some(types::ManifestArgs {
            auto_approve:     None,
            dry_run:          None,
            label:            Vec::new(),
            model:            None,
            preserve_sandbox: None,
            provider:         None,
            environment:      None,
            input:            vec!["override=cli".to_string()],
            verbose:          None,
        });

        let prepared = prepare_manifest(&server_settings, &manifest).unwrap();

        assert_eq!(
            prepared.settings.run.inputs.get("keep"),
            Some(&toml::Value::String("server".to_string()))
        );
        assert_eq!(
            prepared.settings.run.inputs.get("override"),
            Some(&toml::Value::String("cli".to_string()))
        );
    }

    #[test]
    fn prepare_manifest_prefers_bundled_settings_without_duplication() {
        let server_settings = manifest_run_defaults(Some(&server_settings_fixture(
            r#"
_version = 1

[server.storage]
root = "/srv/fabro"

[[run.prepare.steps]]
script = "cli-setup"

[server.integrations.github]
app_id = "fixture-app-id"
"#,
        )));

        let mut manifest = minimal_manifest();
        manifest.workflows.get_mut("workflow.fabro").unwrap().config =
            Some(types::ManifestWorkflowConfig {
                path:   "workflow.toml".to_string(),
                source: r#"
_version = 1

[[run.prepare.steps]]
script = "workflow-setup"
"#
                .to_string(),
            });
        manifest.configs.push(types::ManifestConfig {
            path:   Some("/tmp/home/.fabro/settings.toml".to_string()),
            source: Some(
                r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[[run.prepare.steps]]
script = "cli-setup"

[server.integrations.github]
app_id = "fixture-app-id"
"#
                .to_string(),
            ),
            type_:  types::ManifestConfigType::User,
        });

        let prepared = prepare_manifest(&server_settings, &manifest).unwrap();
        let settings_json = serde_json::to_value(&prepared.settings).unwrap();

        // run.prepare.steps replaces the whole list across layers, so the
        // higher-precedence workflow layer wins over cli.
        assert_eq!(prepared.settings.run.prepare.steps.len(), 1);
        assert_eq!(
            prepared.settings.run.prepare.steps[0].to_shell_command(),
            "workflow-setup"
        );
        assert!(settings_json.pointer("/server").is_none());
    }

    #[test]
    fn prepare_manifest_preserves_bundled_workflow_metadata() {
        let mut manifest = minimal_manifest();
        manifest.workflows.get_mut("workflow.fabro").unwrap().config =
            Some(types::ManifestWorkflowConfig {
                path:   "workflow.toml".to_string(),
                source: r#"
_version = 1

[workflow]
name = "Ship feature"
description = "Move the feature through review"

[workflow.metadata]
team = "platform"
priority = "high"

[run.environment]
id = "local"
"#
                .to_string(),
            });

        let prepared = prepare_manifest(
            &manifest_run_defaults(Some(&default_settings_fixture())),
            &manifest,
        )
        .unwrap();

        assert_eq!(
            prepared.settings.workflow.name.as_deref(),
            Some("Ship feature")
        );
        assert_eq!(
            prepared.settings.workflow.description.as_deref(),
            Some("Move the feature through review")
        );
        assert_eq!(
            prepared
                .settings
                .workflow
                .metadata
                .get("team")
                .map(String::as_str),
            Some("platform")
        );
        assert_eq!(
            prepared
                .settings
                .workflow
                .metadata
                .get("priority")
                .map(String::as_str),
            Some("high")
        );
    }

    #[test]
    fn prepare_manifest_keeps_missing_metadata_names_absent() {
        let mut manifest = minimal_manifest();
        manifest.workflows.get_mut("workflow.fabro").unwrap().source = r"
digraph GraphName {
    start [shape=Mdiamond]
    exit [shape=Msquare]
    start -> exit
}
"
        .to_string();
        manifest.configs.push(types::ManifestConfig {
            path:   Some(".fabro/project.toml".to_string()),
            source: Some(
                r#"_version = 1

[project.metadata]
team = "platform"
"#
                .to_string(),
            ),
            type_:  types::ManifestConfigType::Project,
        });

        let prepared = prepare_manifest(
            &manifest_run_defaults(Some(&default_settings_fixture())),
            &manifest,
        )
        .unwrap();

        assert_eq!(prepared.settings.workflow.name, None);
        assert_eq!(prepared.settings.project.name, None);
    }

    #[test]
    fn prepare_manifest_preserves_explicit_project_name() {
        let mut manifest = minimal_manifest();
        manifest.configs.push(types::ManifestConfig {
            path:   Some(".fabro/project.toml".to_string()),
            source: Some(
                r#"_version = 1

[project]
name = "Control Plane"
"#
                .to_string(),
            ),
            type_:  types::ManifestConfigType::Project,
        });

        let prepared = prepare_manifest(
            &manifest_run_defaults(Some(&default_settings_fixture())),
            &manifest,
        )
        .unwrap();

        assert_eq!(
            prepared.settings.project.name.as_deref(),
            Some("Control Plane")
        );
    }

    #[tokio::test]
    async fn invalid_preflight_returns_diagnostics_without_runtime_checks() {
        let state = crate::test_support::test_app_state();
        let prepared = prepare_manifest(
            &manifest_run_defaults(Some(&default_settings_fixture())),
            &invalid_manifest(),
        )
        .unwrap();
        let validated = validate_with_catalog(&state, &prepared).unwrap();

        assert!(validated.has_errors());

        let (response, ok) = resolve_and_run_preflight(&state, &prepared, &validated)
            .await
            .unwrap();

        assert!(!ok);
        assert_eq!(response.workflow.name, "Invalid");
        assert!(!response.workflow.diagnostics.is_empty());
        assert_eq!(response.checks.title, "Run Preflight");
        assert_eq!(response.checks.sections.len(), 1);
        assert_eq!(response.checks.sections[0].checks.len(), 2);
    }

    #[tokio::test]
    async fn preflight_runs_github_token_check_when_run_level_permissions_declared() {
        // When a workflow declares `[run.integrations.github.permissions]`,
        // `run_github_token_check` is invoked and surfaces a "GitHub Token"
        // entry in the preflight report. With no configured GitHub App
        // credentials in the test fixture, the check status is
        // Warning/skipped — the important assertion is that the entry
        // *exists*, proving the gate now reads from run-level config.
        let state = crate::test_support::test_app_state();
        let mut manifest = minimal_manifest();
        manifest.workflows.get_mut("workflow.fabro").unwrap().config =
            Some(types::ManifestWorkflowConfig {
                path:   "workflow.toml".to_string(),
                source: r#"_version = 1

[run.environment]
id = "local"

[run.integrations.github.permissions]
issues = "read"
"#
                .to_string(),
            });

        let prepared = prepare_manifest(
            &manifest_run_defaults(Some(&default_settings_fixture())),
            &manifest,
        )
        .unwrap();
        let validated = validate_with_catalog(&state, &prepared).unwrap();
        assert!(!validated.has_errors(), "{:?}", validated.diagnostics);

        let (response, _ok) = resolve_and_run_preflight(&state, &prepared, &validated)
            .await
            .unwrap();

        assert!(
            response.checks.sections[0]
                .checks
                .iter()
                .any(|check| check.name == "GitHub Token"),
            "expected GitHub Token check to run when run-level permissions are set; \
             checks were {:?}",
            response.checks.sections[0]
                .checks
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn preflight_rejects_unresolved_github_permissions() {
        let state = crate::test_support::test_app_state();
        let mut manifest = minimal_manifest();
        manifest.workflows.get_mut("workflow.fabro").unwrap().config =
            Some(types::ManifestWorkflowConfig {
                path:   "workflow.toml".to_string(),
                source: r#"_version = 1

[run.environment]
id = "local"

[run.integrations.github.permissions]
issues = "{{ env.GITHUB_ISSUES_PERMISSION }}"
"#
                .to_string(),
            });

        let prepared = prepare_manifest(
            &manifest_run_defaults(Some(&default_settings_fixture())),
            &manifest,
        )
        .unwrap();
        let validated = validate_with_catalog(&state, &prepared).unwrap();
        assert!(!validated.has_errors(), "{:?}", validated.diagnostics);

        let (response, ok) = resolve_and_run_preflight(&state, &prepared, &validated)
            .await
            .unwrap();
        let github_token_check = response.checks.sections[0]
            .checks
            .iter()
            .find(|check| check.name == "GitHub Token")
            .expect("GitHub Token check should report invalid permissions");

        assert!(!ok);
        assert_eq!(
            github_token_check.status,
            types::PreflightCheckResultStatus::Error
        );
        assert_eq!(github_token_check.summary, "invalid permissions");
        assert!(
            github_token_check
                .remediation
                .as_deref()
                .is_some_and(|message| message.contains("GITHUB_ISSUES_PERMISSION"))
        );
    }

    #[tokio::test]
    async fn preflight_allows_pull_request_enabled_without_github_credentials() {
        let state = crate::test_support::test_app_state();
        let source_dir = tempfile::tempdir().unwrap();
        let mut manifest = minimal_manifest();
        manifest.cwd = source_dir.path().to_string_lossy().into_owned();
        manifest.configs.push(types::ManifestConfig {
            path:   Some("/tmp/project/.fabro/project.toml".to_string()),
            source: Some(
                r#"
_version = 1

[run.pull_request]
enabled = true

[run.environment]
id = "local"
"#
                .to_string(),
            ),
            type_:  types::ManifestConfigType::Project,
        });

        let prepared = prepare_manifest(
            &manifest_run_defaults(Some(&default_settings_fixture())),
            &manifest,
        )
        .unwrap();
        let validated = validate_with_catalog(&state, &prepared).unwrap();

        assert!(!validated.has_errors(), "{:?}", validated.diagnostics);

        let (response, ok) = resolve_and_run_preflight(&state, &prepared, &validated)
            .await
            .unwrap();

        assert!(ok);
        assert!(response.workflow.diagnostics.is_empty());
        assert!(
            response.checks.sections[0]
                .checks
                .iter()
                .all(|check| check.name != "GitHub Token")
        );
    }

    #[test]
    fn prepare_manifest_does_not_backfill_missing_project_and_workflow_names() {
        let mut manifest = minimal_manifest();
        manifest.workflows.get_mut("workflow.fabro").unwrap().config =
            Some(types::ManifestWorkflowConfig {
                path:   "workflow.toml".to_string(),
                source: "_version = 1\n".to_string(),
            });
        manifest.configs.push(types::ManifestConfig {
            path:   Some("/tmp/project/.fabro/project.toml".to_string()),
            source: Some("_version = 1\n".to_string()),
            type_:  types::ManifestConfigType::Project,
        });

        let prepared = prepare_manifest(
            &manifest_run_defaults(Some(&default_settings_fixture())),
            &manifest,
        )
        .unwrap();

        assert_eq!(prepared.settings.project.name, None);
        assert_eq!(prepared.settings.workflow.name, None);
    }

    #[test]
    fn prepare_manifest_preserves_explicit_project_and_workflow_names() {
        let mut manifest = minimal_manifest();
        manifest.workflows.get_mut("workflow.fabro").unwrap().config =
            Some(types::ManifestWorkflowConfig {
                path:   "workflow.toml".to_string(),
                source: r#"
_version = 1

[workflow]
name = "Workflow Config Name"
"#
                .to_string(),
            });
        manifest.configs.push(types::ManifestConfig {
            path:   Some("/tmp/project/.fabro/project.toml".to_string()),
            source: Some(
                r#"
_version = 1

[project]
name = "Project Config Name"
"#
                .to_string(),
            ),
            type_:  types::ManifestConfigType::Project,
        });

        let prepared = prepare_manifest(
            &manifest_run_defaults(Some(&default_settings_fixture())),
            &manifest,
        )
        .unwrap();

        assert_eq!(
            prepared.settings.project.name.as_deref(),
            Some("Project Config Name")
        );
        assert_eq!(
            prepared.settings.workflow.name.as_deref(),
            Some("Workflow Config Name")
        );
    }

    #[tokio::test]
    async fn preflight_daytona_without_github_credentials_returns_report() {
        let state = crate::test_support::test_app_state();
        let mut manifest = minimal_manifest();
        manifest.configs.push(types::ManifestConfig {
            path:   Some("/tmp/project/.fabro/project.toml".to_string()),
            source: Some(
                r#"
_version = 1

[run.environment]
id = "daytona"
"#
                .to_string(),
            ),
            type_:  types::ManifestConfigType::Project,
        });

        let prepared = prepare_manifest(
            &manifest_run_defaults(Some(&default_settings_fixture())),
            &manifest,
        )
        .unwrap();
        let validated = validate_with_catalog(&state, &prepared).unwrap();

        let (response, _ok) = resolve_and_run_preflight(&state, &prepared, &validated)
            .await
            .unwrap();

        assert!(response.workflow.diagnostics.is_empty());
        assert!(
            response.checks.sections[0]
                .checks
                .iter()
                .any(|check| check.name == "Sandbox")
        );
    }

    #[tokio::test]
    async fn preflight_refuses_an_unknown_unqualified_model_before_any_check() {
        let server = httpmock::MockServer::start_async().await;
        let any_provider_call = server
            .mock_async(|when, then| {
                when.any_request();
                then.status(500);
            })
            .await;
        let state = ready_moonshot_and_openrouter_state(&server);

        // Petri admits no model its catalog lacks, so the workflow is refused
        // at its check: no passthrough of an unknown model to a provider.
        let (response, ok) = preflight_for_refused_model(&state, "provider-private-preview").await;

        assert!(!ok);
        assert!(response.workflow.diagnostics.iter().any(|diagnostic| {
            diagnostic.rule == "attractor.model.unknown"
                && diagnostic.message.contains("provider-private-preview")
        }));
        assert!(
            response.checks.sections[0]
                .checks
                .iter()
                .all(|check| check.name != "LLM"),
            "no LLM check runs on a refused workflow"
        );
        assert_eq!(any_provider_call.calls_async().await, 0);
    }

    #[tokio::test]
    async fn preflight_refuses_a_stage_whose_provider_has_no_stored_credential() {
        let server = httpmock::MockServer::start_async().await;
        let any_provider_call = server
            .mock_async(|when, then| {
                when.any_request();
                then.status(500);
            })
            .await;
        // moonshot is enabled but carries no credential in this state; the
        // workflow needs it, so the validation refuses the stage at its
        // check instead of letting a fire die inside it (fabro-b46e).
        let moonshot_url = server.url("/moonshot/v1");
        let openrouter_url = server.url("/openrouter/v1");
        let state = crate::test_support::TestAppStateBuilder::new()
            .llm_overlay_toml(&format!(
                r#"
[providers.moonshot]
base_url = "{moonshot_url}"

[providers.openrouter]
base_url = "{openrouter_url}"
enabled = true

"#
            ))
            .vault_entries([(EnvVars::OPENROUTER_API_KEY, "test-openrouter-key")])
            .build();
        let (_, ready_providers) = state.resolve_llm_client_with_ready_ids().await;
        let mut manifest = minimal_manifest();
        manifest.workflows.get_mut("workflow.fabro").unwrap().source = r#"
digraph Demo {
    start [shape=Mdiamond]
    exit  [shape=Msquare]
    work  [prompt="Do work", model="kimi-k3", provider="moonshot"]
    start -> work -> exit
}
"#
        .to_string();
        let prepared = prepare_manifest(
            &manifest_run_defaults(Some(&default_settings_fixture())),
            &manifest,
        )
        .unwrap();
        let validated = validate_for_test(&state, &prepared, &ready_providers).unwrap();

        let refusal = validated
            .diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.rule == fabro_admission::petri_check::PROVIDER_NOT_READY_RULE
            })
            .expect("the not-ready provider should refuse the workflow");
        assert!(refusal.severity == Severity::Error);
        assert!(
            refusal.message.contains("moonshot") && refusal.message.contains("kimi-k3"),
            "{refusal:?}"
        );
        assert_eq!(any_provider_call.calls_async().await, 0);
    }

    #[test]
    fn static_validation_rejects_unknown_llm_provider() {
        let state = crate::test_support::test_app_state();
        let mut manifest = minimal_manifest();
        manifest.workflows.get_mut("workflow.fabro").unwrap().source = r#"
digraph Demo {
    start [shape=Mdiamond]
    exit  [shape=Msquare]
    work  [prompt="Do work", model="missing-model", provider="missing-provider"]
    start -> work -> exit
}
"#
        .to_string();
        let prepared = prepare_manifest(
            &manifest_run_defaults(Some(&default_settings_fixture())),
            &manifest,
        )
        .unwrap();
        let validated = validate_with_catalog(&state, &prepared).unwrap();

        // Petri refuses the model node: the provider is not in the catalog.
        let refusal = validated
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.severity == Severity::Error)
            .expect("unknown provider should fail static validation");
        assert_eq!(refusal.rule, "attractor.model.unknown", "{refusal:?}");
        assert!(refusal.message.contains("missing-provider"), "{refusal:?}");
    }

    #[tokio::test]
    async fn preflight_resolves_model_aliases_from_app_state_catalog() {
        let state = crate::test_support::TestAppStateBuilder::new()
            .llm_overlay_toml(
                r#"
[providers.acme]
display_name = "Acme"
base_url = "https://api.acme.test/v1"
auth = { type = "bearer" }
default_model = "acme-large"

[providers.acme.metadata.agent]
profile = "openai"

[providers.acme.models."acme-large"]
display_name = "Acme Large"
aliases = ["vl"]
api_model = "acme-large"
limits = { context_tokens = 128000, max_output_tokens = 8192 }
capabilities = { text = true, tools = true }
"#,
            )
            .build();
        let mut manifest = minimal_manifest();
        manifest.workflows.get_mut("workflow.fabro").unwrap().source = r#"
digraph Demo {
    start [shape=Mdiamond]
    exit  [shape=Msquare]
    work  [prompt="Do work", model="vl"]
    start -> work -> exit
}
"#
        .to_string();
        let prepared = prepare_manifest(
            &manifest_run_defaults(Some(&default_settings_fixture())),
            &manifest,
        )
        .unwrap();
        let (_, ready_providers) = state.resolve_llm_client_with_ready_ids().await;
        assert!(ready_providers.is_empty());

        // With no provider ready there is no model client to resolve the
        // alias against, so Fabro refuses the model node before any check
        // runs: the run could not pick a model at create either.
        let validated = validate_for_test(&state, &prepared, &ready_providers).unwrap();
        assert!(validated.has_errors());
        assert!(
            validated
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.rule == petri_check::NO_READY_PROVIDER_RULE),
            "{:?}",
            validated.diagnostics
        );

        let (response, ok) = run_preflight(&state.admission_context(), &prepared, &validated)
            .await
            .unwrap();
        assert!(!ok);
        assert!(
            response
                .checks
                .sections
                .iter()
                .flat_map(|section| section.checks.iter())
                .all(|check| check.name != "LLM"),
            "no LLM check runs on a refused workflow"
        );
    }

    mod settings_layer_with_resolved_dockerfiles_tests {
        //! `settings_layer_with_resolved_dockerfiles` parses bundled
        //! workflow.toml through the strict `SettingsLayer` schema, so
        //! unknown fields anywhere in the document trip
        //! `deny_unknown_fields`.

        use fabro_config::parse::SettingsSource;
        use fabro_types::ManifestPath;
        use fabro_workflow::workflow_bundle::{BundledWorkflow, ParsedWorkflowConfig};

        use super::super::settings_layer_with_resolved_dockerfiles;

        fn workflow_with_config(source: &str) -> BundledWorkflow {
            BundledWorkflow {
                path:   ManifestPath::from_wire("workflow.fabro").expect("path should be valid"),
                source: "digraph G {}".to_string(),
                config: Some(ParsedWorkflowConfig {
                    path:   ManifestPath::from_wire("workflow.toml")
                        .expect("config path should be valid"),
                    source: source.to_string(),
                }),
                files:  std::collections::HashMap::new(),
            }
        }

        #[test]
        fn parses_run_integrations_github_permissions() {
            let workflow = workflow_with_config(
                r#"_version = 1

[run.integrations.github.permissions]
issues = "read"
"#,
            );

            let layer = settings_layer_with_resolved_dockerfiles(
                &workflow.config.as_ref().unwrap().source,
                &workflow.config.as_ref().unwrap().path,
                &workflow.files,
                SettingsSource::Workflow,
            )
            .expect("workflow.toml should parse");
            let run = layer.run.expect("run layer should be present");
            let github = run
                .integrations
                .as_ref()
                .and_then(|integrations| integrations.github.as_ref())
                .expect("integrations.github should be present");
            let permissions = github
                .permissions
                .as_ref()
                .expect("permissions should be present");
            assert_eq!(permissions.len(), 1);
            assert!(permissions.contains_key("issues"));
        }

        #[test]
        fn rejects_stale_server_integrations_github_permissions() {
            let workflow = workflow_with_config(
                r#"_version = 1

[server.integrations.github.permissions]
issues = "read"
"#,
            );

            let err = settings_layer_with_resolved_dockerfiles(
                &workflow.config.as_ref().unwrap().source,
                &workflow.config.as_ref().unwrap().path,
                &workflow.files,
                SettingsSource::Workflow,
            )
            .expect_err("stale [server.integrations.github.permissions] should be rejected");
            let message = format!("{err:#}");
            assert!(
                message.contains("permissions") || message.contains("unknown field"),
                "expected unknown-field error, got: {message}"
            );
        }

        #[test]
        fn accepts_workflow_block_and_version() {
            let workflow = workflow_with_config(
                r#"_version = 1

[workflow]
name = "demo"

[run.integrations.github.permissions]
contents = "read"
"#,
            );

            let layer = settings_layer_with_resolved_dockerfiles(
                &workflow.config.as_ref().unwrap().source,
                &workflow.config.as_ref().unwrap().path,
                &workflow.files,
                SettingsSource::Workflow,
            )
            .expect("workflow + run blocks should parse");
            let run = layer.run.expect("run layer should be present");
            assert!(run.integrations.is_some());
        }

        #[test]
        fn inlines_dockerfile_path_in_environments_catalog() {
            use fabro_config::EnvironmentDockerfileLayer;

            let source = r#"_version = 1

[environments.fabro-dev]
provider = "daytona"

[environments.fabro-dev.image]
dockerfile = { path = "Dockerfile" }
"#;
            let config_path =
                ManifestPath::from_wire("project.toml").expect("config path should be valid");
            let mut files = std::collections::HashMap::new();
            files.insert(
                ManifestPath::from_wire("Dockerfile").expect("dockerfile path should be valid"),
                "FROM ubuntu:24.04\n".to_string(),
            );

            let layer = settings_layer_with_resolved_dockerfiles(
                source,
                &config_path,
                &files,
                SettingsSource::Project,
            )
            .expect("project config should parse");

            let dockerfile = layer
                .environments
                .get("fabro-dev")
                .expect("fabro-dev environment should be present")
                .image
                .as_ref()
                .and_then(|image| image.dockerfile.as_ref())
                .expect("dockerfile should be present");
            assert_eq!(
                *dockerfile,
                EnvironmentDockerfileLayer::Inline("FROM ubuntu:24.04\n".to_string()),
                "catalog dockerfile path should be inlined, got: {dockerfile:?}"
            );
        }
    }

    mod github_additional_repository_checks {
        //! Seam-injected tests for the declared-additional-repositories
        //! preflight path: one scoped mint, per-repository probes with
        //! deterministic primary-first reporting, GH_TOKEN warning, and the
        //! replication-lag retry policy.

        use std::sync::Mutex as StdMutex;
        use std::sync::atomic::{AtomicU64, Ordering};

        use fabro_github::token_source::{
            ResolvedToken, SecretString, TokenProvenance, TokenSnapshot,
        };
        use fabro_types::settings::run::RunIntegrationsGithubSettings;

        use super::*;

        fn static_token(secret: &str) -> ResolvedToken {
            ResolvedToken {
                token:          SecretString::new(secret.to_string()),
                snapshot:       TokenSnapshot {
                    generation: 0,
                    provenance: TokenProvenance::Static,
                },
                refresh_failed: false,
            }
        }

        fn fresh_minted_snapshot() -> TokenSnapshot {
            let now = chrono::Utc::now();
            TokenSnapshot {
                generation: 1,
                provenance: TokenProvenance::Minted {
                    minted_at:  now,
                    expires_at: now + chrono::Duration::minutes(60),
                },
            }
        }

        fn declared(origin: &str, additional: &[&str]) -> (PreparedManifest, RunNamespace) {
            let (prepared, mut resolved) = prepared_and_resolved_for_sandbox(
                &SandboxProviderKind::LOCAL,
                true,
                Some(git_context(origin, "main")),
            );
            resolved.integrations.github = RunIntegrationsGithubSettings {
                permissions:             HashMap::from([(
                    "contents".to_string(),
                    InterpString::parse("read"),
                )]),
                additional_repositories: additional
                    .iter()
                    .map(|value| value.parse().expect("test slug should parse"))
                    .collect(),
            };
            (prepared, resolved)
        }

        fn pat_creds() -> fabro_github::GitHubCredentials {
            fabro_github::GitHubCredentials::Pat("ghp_test".to_string())
        }

        #[tokio::test(start_paused = true)]
        async fn reports_each_repository_primary_first_despite_probe_completion_order() {
            let (prepared, resolved) = declared("https://github.com/acme/widgets", &[
                "acme/zeta",
                "acme/alpha",
            ]);
            let minted = Arc::new(StdMutex::new(Vec::new()));
            let minted_for_seam = Arc::clone(&minted);
            let mut checks = Vec::new();

            let ok = run_github_token_check_with(
                &mut checks,
                &prepared,
                &resolved,
                Some(pat_creds()),
                move |access, _creds| {
                    minted_for_seam.lock().unwrap().push(access);
                    async { Ok(static_token("scoped-token")) }
                },
                |slug, _token| async move {
                    // Invert completion order: the primary finishes last.
                    let delay = match slug.repo() {
                        "widgets" => 30,
                        "alpha" => 20,
                        _ => 10,
                    };
                    time::sleep(Duration::from_millis(delay)).await;
                    Ok(())
                },
            )
            .await;

            assert!(ok);
            // One mint listing every repository with the shared permissions.
            let minted = minted.lock().unwrap();
            assert_eq!(minted.len(), 1);
            assert_eq!(minted[0].repository_names(), vec![
                "widgets", "alpha", "zeta"
            ]);
            assert_eq!(
                minted[0].permissions().get("contents").map(String::as_str),
                Some("read")
            );

            let names: Vec<&str> = checks.iter().map(|check| check.name.as_str()).collect();
            assert_eq!(names, vec![
                "GitHub Token",
                "GitHub Repository (acme/widgets)",
                "GitHub Repository (acme/alpha)",
                "GitHub Repository (acme/zeta)",
            ]);
            assert!(checks.iter().all(|check| check.status == CheckStatus::Pass));
            // The token never reaches check output.
            for check in &checks {
                let rendered = format!("{check:?}");
                assert!(!rendered.contains("scoped-token"), "{rendered}");
            }
        }

        #[tokio::test]
        async fn installation_resolution_failure_names_only_the_inaccessible_repository() {
            let (prepared, resolved) =
                declared("https://github.com/acme/widgets", &["acme/keystone"]);
            let probes = Arc::new(AtomicU64::new(0));
            let probes_for_seam = Arc::clone(&probes);
            let mut checks = Vec::new();

            let ok = run_github_token_check_with(
                &mut checks,
                &prepared,
                &resolved,
                Some(pat_creds()),
                |_access, _creds| async {
                    Err(
                        "the GitHub App installation cannot see repository acme/keystone; add \
                         it to the installation's repository access"
                            .to_string(),
                    )
                },
                move |_slug, _token| {
                    probes_for_seam.fetch_add(1, Ordering::SeqCst);
                    async { Ok(()) }
                },
            )
            .await;

            assert!(!ok);
            assert_eq!(
                probes.load(Ordering::SeqCst),
                0,
                "no probes after a failed mint"
            );
            assert_eq!(checks.last().unwrap().name, "GitHub Token");
            assert_eq!(checks.last().unwrap().status, CheckStatus::Error);
            let remediation = checks.last().unwrap().remediation.as_deref().unwrap();
            assert!(remediation.contains("acme/keystone"), "{remediation}");
            assert!(!remediation.contains("acme/widgets"), "{remediation}");
        }

        #[tokio::test]
        async fn successful_mint_with_failed_probe_still_fails() {
            let (prepared, resolved) =
                declared("https://github.com/acme/widgets", &["acme/keystone"]);
            let mut checks = Vec::new();

            let ok = run_github_token_check_with(
                &mut checks,
                &prepared,
                &resolved,
                Some(pat_creds()),
                |_access, _creds| async { Ok(static_token("scoped-token")) },
                |slug, _token| async move {
                    if slug.repo() == "keystone" {
                        Err("remote: Repository not found.".to_string())
                    } else {
                        Ok(())
                    }
                },
            )
            .await;

            assert!(!ok);
            let keystone = checks
                .iter()
                .find(|check| check.name == "GitHub Repository (acme/keystone)")
                .expect("keystone probe result should be reported");
            assert_eq!(keystone.status, CheckStatus::Error);
            assert!(
                !keystone
                    .remediation
                    .as_deref()
                    .unwrap_or_default()
                    .contains("scoped-token")
            );
            let widgets = checks
                .iter()
                .find(|check| check.name == "GitHub Repository (acme/widgets)")
                .expect("primary probe result should be reported");
            assert_eq!(widgets.status, CheckStatus::Pass);
        }

        #[tokio::test]
        async fn resolved_gh_token_warns_without_failing() {
            let (prepared, mut resolved) =
                declared("https://github.com/acme/widgets", &["acme/keystone"]);
            resolved
                .environment
                .env
                .insert("GH_TOKEN".to_string(), InterpString::parse("user-token"));
            let mut checks = Vec::new();

            let ok = run_github_token_check_with(
                &mut checks,
                &prepared,
                &resolved,
                Some(pat_creds()),
                |_access, _creds| async { Ok(static_token("scoped-token")) },
                |_slug, _token| async { Ok(()) },
            )
            .await;

            assert!(ok, "a GH_TOKEN override warns but does not fail preflight");
            let warning = checks
                .iter()
                .find(|check| check.name == "GH_TOKEN Override")
                .expect("GH_TOKEN warning should be reported");
            assert_eq!(warning.status, CheckStatus::Warning);
        }

        #[tokio::test]
        async fn missing_origin_fails_for_declared_repositories() {
            let (prepared, resolved) = {
                let (mut prepared, resolved) =
                    declared("https://github.com/acme/widgets", &["acme/keystone"]);
                prepared.git = None;
                (prepared, resolved)
            };
            let mut checks = Vec::new();

            let ok = run_github_token_check_with(
                &mut checks,
                &prepared,
                &resolved,
                Some(pat_creds()),
                |_access, _creds| async { Ok(static_token("scoped-token")) },
                |_slug, _token| async { Ok(()) },
            )
            .await;

            assert!(!ok);
            let check = checks.last().unwrap();
            assert_eq!(check.summary, "missing origin");
            assert!(
                check
                    .remediation
                    .as_deref()
                    .unwrap_or_default()
                    .contains("requires a GitHub run origin"),
                "{:?}",
                check.remediation
            );
        }

        #[tokio::test]
        async fn missing_credentials_fail_for_declared_repositories() {
            let (prepared, resolved) =
                declared("https://github.com/acme/widgets", &["acme/keystone"]);
            let mut checks = Vec::new();

            let ok = run_github_token_check_with(
                &mut checks,
                &prepared,
                &resolved,
                None,
                |_access, _creds| async { Ok(static_token("scoped-token")) },
                |_slug, _token| async { Ok(()) },
            )
            .await;

            assert!(!ok);
            assert_eq!(checks.last().unwrap().summary, "missing credentials");
        }

        #[tokio::test(start_paused = true)]
        async fn replication_lag_failure_retries_with_the_same_token_and_succeeds() {
            let attempts = Arc::new(AtomicU64::new(0));
            let attempts_for_run = Arc::clone(&attempts);

            let result = probe_with_replication_retry(fresh_minted_snapshot(), move || {
                let attempts = Arc::clone(&attempts_for_run);
                async move {
                    if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                        Err("remote: Repository not found.".to_string())
                    } else {
                        Ok(())
                    }
                }
            })
            .await;

            assert!(result.is_ok());
            assert_eq!(attempts.load(Ordering::SeqCst), 2);
        }

        #[tokio::test(start_paused = true)]
        async fn static_credential_auth_failures_do_not_retry() {
            let attempts = Arc::new(AtomicU64::new(0));
            let attempts_for_run = Arc::clone(&attempts);
            let static_snapshot = TokenSnapshot {
                generation: 0,
                provenance: TokenProvenance::Static,
            };

            let result = probe_with_replication_retry(static_snapshot, move || {
                let attempts = Arc::clone(&attempts_for_run);
                async move {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    Err("remote: Repository not found.".to_string())
                }
            })
            .await;

            assert!(result.is_err());
            assert_eq!(
                attempts.load(Ordering::SeqCst),
                1,
                "a 404 with a static credential cannot become valid by waiting"
            );
        }
    }
}
