//! Admission-path tests that run through the server's app state: the moved
//! admission API (fabro-admission, work#1672) is exercised the same way the
//! endpoints drive it — an `AppState`, its derived `AdmissionContext`, and
//! the server's `RuntimeSpec`. The bodies are the original `run_manifest`
//! test mod, unchanged apart from the imports and the provider-health
//! callback swap.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use fabro_admission::petri_check::{self, Readiness};
use fabro_admission::run_manifest::*;
use fabro_api::types;
use fabro_config::{EnvironmentLayer, MergeMap, RunLayer};
use fabro_static::EnvVars;
use fabro_types::diagnostic::Severity;
use fabro_workflow::Error as WorkflowError;
use lithos_llm::catalog::ProviderId;

use crate::server::AppState;

/// The manifest run defaults helper the preflight fixtures share (was the
/// `run_manifest` cfg(test) helper before the extraction, work#1672).
pub(crate) fn manifest_run_defaults(run: Option<&RunLayer>) -> RunLayer {
    run.cloned().unwrap_or_default()
}

// The fixtures below are the pure-admission fixture set the AppState-driven
// tests share; the same fixtures live in fabro-admission's own test mod
// (review split, work#1672) — duplicated deliberately so neither side
// imports the other's test-only surface.
fn minimal_manifest() -> types::RunManifest {
    types::RunManifest {
        args: None,

        configs: Vec::new(),

        cwd: "/tmp/project".to_string(),

        git: None,

        goal: None,

        parent_id: None,

        title: None,

        target: types::ManifestTarget {
            path: "workflow.fabro".to_string(),
        },

        version: 1,

        workflows: HashMap::from([("workflow.fabro".to_string(), types::ManifestWorkflow {
            config: None,

            files: HashMap::new(),

            source: "digraph Demo { start [shape=Mdiamond] exit [shape=Msquare] start -> exit }"
                .to_string(),
        })]),
    }
}

fn invalid_manifest() -> types::RunManifest {
    types::RunManifest {
        workflows: HashMap::from([("workflow.fabro".to_string(), types::ManifestWorkflow {
            config: None,

            files: HashMap::new(),

            source: "digraph Invalid { exit [shape=Msquare] orphan exit -> orphan }".to_string(),
        })]),

        ..minimal_manifest()
    }
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
    fabro_admission::run_manifest::prepare_manifest_with_environment_defaults(
        manifest_run_defaults,
        &environment_defaults_fixture(),
        &HashMap::new(),
        manifest,
    )
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
}
