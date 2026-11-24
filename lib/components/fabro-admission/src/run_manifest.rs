use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use anyhow::{Context as _, Result, anyhow, bail};
use fabro_api::types;
use fabro_config::parse::SettingsSource;
use fabro_config::run::resolve_run_goal_from_namespace;
use fabro_config::{
    CliLayer, CliOutputLayer, EnvironmentLayer, MergeMap, RunLayer, SettingsLayer,
    WorkflowSettingsBuilder, parse_input_overrides, parse_labels, project,
};
use fabro_dot::WorkflowGraph;
use fabro_github::token_source::{InstallationTokenSource, ResolvedToken, TokenSnapshot};
use fabro_graphviz::render::apply_direction;
use fabro_petri::check::{Admitted, Launch};
use fabro_petri::run_graph;
use fabro_petri::runtime::RuntimeSpec;
use fabro_proc::ProcessError;
use fabro_static::EnvVars;
use fabro_types::diagnostic::{Diagnostic, Severity};
use fabro_types::settings::cli::OutputVerbosity;
use fabro_types::settings::interp::InterpString;
use fabro_types::settings::run::{McpServerSettings, RunGoal, RunNamespace};
use fabro_types::{
    BundledProvider, ManifestPath, RunGraph, RunId, SandboxProviderKind, ServerSettings,
    WorkflowSettings,
};
use fabro_util::check_report::{CheckDetail, CheckReport, CheckResult, CheckSection, CheckStatus};
use fabro_workflow::Error as WorkflowError;
use fabro_workflow::workflow_bundle::{BundledWorkflow, ParsedWorkflowConfig, WorkflowBundle};
use futures_util::stream::{self, StreamExt};
use lithos_llm::catalog::ProviderId;
use sandbox_driver::{
    GitBackoff, GitCredentials, GitFailure, GitFailureKind, GitRetryPolicy, HealthStatus,
    ProviderHealth,
};
use tokio::process::Command;
use tokio::task;
use tokio_util::sync::CancellationToken;

use crate::context::{AdmissionContext, ProviderHealthSource};
use crate::petri_check::Readiness;
use crate::{petri_check, run_compiler};

#[derive(Clone)]
pub struct PreparedManifest {
    pub git:              Option<types::GitContext>,
    /// The entrypoint's DOT as written: what the render endpoint draws.
    pub root_source:      String,
    pub settings:         WorkflowSettings,
    pub target_path:      ManifestPath,
    pub workflow_bundle:  WorkflowBundle,
    pub source_directory: PathBuf,
}

#[derive(Clone, Debug, Default)]
pub struct ManifestSettingsOverrides {
    pub(crate) run:             Option<RunLayer>,
    pub(crate) cli:             Option<CliLayer>,
    pub(crate) input_overrides: HashMap<String, toml::Value>,
}

pub fn prepare_manifest_with_environment_defaults(
    manifest_run_defaults: &RunLayer,
    manifest_environment_defaults: &MergeMap<EnvironmentLayer>,
    manifest_mcp_server_catalog: &HashMap<String, McpServerSettings>,
    manifest: &types::RunManifest,
) -> Result<PreparedManifest> {
    if manifest.version != 1 {
        bail!("unsupported manifest version {}", manifest.version);
    }

    let cwd = PathBuf::from(&manifest.cwd);
    let target_path = ManifestPath::from_wire(&manifest.target.path)
        .ok_or_else(|| anyhow!("invalid manifest target path: {}", manifest.target.path))?;
    let workflow_bundle = workflow_bundle_from_manifest(&manifest.workflows)?;
    let workflow_input = workflow_bundle
        .workflow(&target_path)
        .cloned()
        .ok_or_else(|| anyhow!("manifest target path is missing from workflows map"))?;
    let root_source = workflow_input.source.clone();

    let args_overrides =
        manifest_args_overrides(manifest.args.as_ref()).context("failed to parse manifest args")?;
    let mut workflow_settings_builder = WorkflowSettingsBuilder::new()
        .server_manifest_defaults(
            manifest_run_defaults.clone(),
            manifest_environment_defaults.clone(),
        )
        .server_mcp_catalog(manifest_mcp_server_catalog.clone());
    if let Some(run) = args_overrides.run {
        workflow_settings_builder = workflow_settings_builder.run_overrides(run);
    }
    if let Some(cli) = args_overrides.cli {
        workflow_settings_builder = workflow_settings_builder.cli_overrides(cli);
    }
    if let Some(config) = workflow_input.config.as_ref() {
        let layer = settings_layer_with_resolved_dockerfiles(
            &config.source,
            &config.path,
            &workflow_input.files,
            SettingsSource::Workflow,
        )?;
        workflow_settings_builder = workflow_settings_builder.workflow_layer(layer);
    }
    for config in manifest
        .configs
        .iter()
        .filter(|config| config.type_ == types::ManifestConfigType::Project)
    {
        if let Some(source) = config.source.as_deref() {
            let config_path = manifest_project_config_path(config, &cwd)?;
            let layer = settings_layer_with_resolved_dockerfiles(
                source,
                &config_path,
                &workflow_input.files,
                SettingsSource::Project,
            )?;
            workflow_settings_builder = workflow_settings_builder.project_layer(layer);
        }
    }
    for config in manifest
        .configs
        .iter()
        .filter(|config| config.type_ == types::ManifestConfigType::User)
    {
        if let Some(source) = config.source.as_deref() {
            workflow_settings_builder = workflow_settings_builder.user_toml(source)?;
        }
    }
    let mut settings = workflow_settings_builder
        .build()
        .context("failed to resolve manifest settings")?;
    settings.run.inputs.extend(args_overrides.input_overrides);
    if let Some(goal) = manifest
        .goal
        .as_ref()
        .filter(|goal| goal.type_ != types::ManifestGoalType::Graph)
    {
        settings.run.goal = Some(RunGoal::Inline(InterpString::parse(&goal.text)));
    }
    manifest
        .title
        .as_ref()
        .map(|title| fabro_types::normalize_explicit_run_title(title.as_str()))
        .transpose()?;
    manifest
        .parent_id
        .as_deref()
        .map(str::parse::<RunId>)
        .transpose()
        .context("invalid parent run ID")?;

    let source_directory = project::resolve_working_directory_from_run(&settings.run, &cwd);
    Ok(PreparedManifest {
        git: manifest.git.clone(),
        root_source,
        settings,
        target_path,
        workflow_bundle,
        source_directory,
    })
}

/// What Petri said about a prepared manifest: the display graph when it
/// admitted the workflow, and every diagnostic, Petri's and Fabro's.
pub struct ManifestCheck {
    pub graph:       Option<RunGraph>,
    pub diagnostics: Vec<Diagnostic>,
}

impl ManifestCheck {
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }
}

/// Petri's check of the whole bundle under `launch` and `runtime`.
/// `Readiness::Server` adds Fabro's readiness rules: the refusal of a model
/// node no provider can run, and the refusal of a stage whose provider has
/// no stored credential (fabro-b46e); `unbound_is_warning` keeps an input
/// nothing binds a warning, for a validation before the run's inputs exist.
/// Blocking: Petri lowers the graph synchronously.
pub fn validate_prepared_manifest(
    prepared: &PreparedManifest,
    vars: &HashMap<String, String>,
    launch: Launch,
    runtime: RuntimeSpec,
    readiness: &Readiness,
    unbound_is_warning: bool,
) -> Result<ManifestCheck, WorkflowError> {
    let request = petri_check::check_request(
        &prepared.workflow_bundle,
        &prepared.target_path,
        &prepared.settings,
        vars,
        launch,
        runtime,
        unbound_is_warning,
    )?;
    let mut checked = petri_check::check(&request, readiness.has_ready_provider())?;
    if let Some(model_readiness) = readiness.as_model_readiness() {
        let requirements = checked
            .admitted
            .as_ref()
            .map(Admitted::model_requirements)
            .unwrap_or_default();
        checked
            .diagnostics
            .extend(petri_check::model_readiness_diagnostics(
                &model_readiness,
                &requirements,
            ));
    }
    Ok(ManifestCheck {
        graph:       checked.admitted.as_ref().map(run_graph::run_graph),
        diagnostics: checked.diagnostics,
    })
}

/// Check a prepared manifest as a run would be admitted: Petri's check with
/// the caller's runtime spec (the app derives it from its state — see
/// `RuntimeSpec`) and the model client over the ready providers, on the
/// blocking pool.
pub async fn check_prepared_manifest(
    ctx: &AdmissionContext,
    runtime: RuntimeSpec,
    prepared: &PreparedManifest,
    vars: HashMap<String, String>,
    ready_providers: &[ProviderId],
) -> Result<ManifestCheck, WorkflowError> {
    let launch = petri_check::launch(
        &ctx.catalog,
        &prepared.settings,
        ready_providers,
        None,
        None,
    );
    let readiness = Readiness::Server {
        catalog: ctx.catalog.clone(),
        ready:   ready_providers.to_vec(),
        force:   false,
    };
    let prepared = prepared.clone();
    task::spawn_blocking(move || {
        validate_prepared_manifest(&prepared, &vars, launch, runtime, &readiness, false)
    })
    .await
    .map_err(|source| WorkflowError::engine_with_source("manifest check task failed", source))?
}

pub async fn run_preflight(
    ctx: &AdmissionContext,
    prepared: &PreparedManifest,
    check: &ManifestCheck,
) -> Result<(types::PreflightResponse, bool)> {
    let shape = workflow_shape(check, prepared);
    let (report, checks_ok) = build_preflight_report(ctx, prepared, check, &shape).await?;
    let preflight_ok = !check.has_errors() && checks_ok;
    Ok((
        types::PreflightResponse {
            ok:       preflight_ok,
            checks:   report_to_api(&report),
            workflow: workflow_summary(check, &shape, prepared.target_path.as_path()),
        },
        preflight_ok,
    ))
}

pub fn validate_response(
    prepared: &PreparedManifest,
    check: &ManifestCheck,
) -> types::ValidateResponse {
    let shape = workflow_shape(check, prepared);
    types::ValidateResponse {
        ok:       !check.has_errors(),
        workflow: workflow_summary(check, &shape, prepared.target_path.as_path()),
    }
}

pub fn graph_source(prepared: &PreparedManifest, direction: Option<&str>) -> String {
    direction.map_or_else(
        || prepared.root_source.clone(),
        |direction| apply_direction(&prepared.root_source, direction).into_owned(),
    )
}

pub fn workflow_bundle_from_manifest(
    workflows: &HashMap<String, types::ManifestWorkflow>,
) -> Result<WorkflowBundle> {
    let mut bundled = HashMap::new();
    let mut workflow_wire_keys = HashMap::new();

    for (wire_key, workflow) in workflows {
        let path = ManifestPath::from_wire(wire_key)
            .ok_or_else(|| anyhow!("invalid manifest workflow key: {wire_key}"))?;
        if let Some(previous) = workflow_wire_keys.get(&path) {
            bail!(
                "duplicate canonical workflow key: {path} (from wire keys {previous:?} and \
                 {wire_key:?})"
            );
        }
        workflow_wire_keys.insert(path.clone(), wire_key.clone());

        let files = workflow_files_from_manifest(&workflow.files)?;
        let config = workflow
            .config
            .as_ref()
            .map(|config| {
                let path = ManifestPath::from_wire(&config.path).ok_or_else(|| {
                    anyhow!("invalid manifest workflow config path: {}", config.path)
                })?;
                Ok::<_, anyhow::Error>(ParsedWorkflowConfig {
                    path,
                    source: config.source.clone(),
                })
            })
            .transpose()?;

        bundled.insert(path.clone(), BundledWorkflow {
            path,
            source: workflow.source.clone(),
            config,
            files,
        });
    }

    Ok(WorkflowBundle::new(bundled))
}

fn workflow_files_from_manifest(
    files: &HashMap<String, types::ManifestFileEntry>,
) -> Result<HashMap<ManifestPath, String>> {
    let mut bundled = HashMap::new();
    let mut file_wire_keys = HashMap::new();

    for (wire_key, entry) in files {
        let path = ManifestPath::from_wire(wire_key)
            .ok_or_else(|| anyhow!("invalid manifest file key: {wire_key}"))?;
        if let Some(previous) = file_wire_keys.get(&path) {
            bail!(
                "duplicate canonical file key: {path} (from wire keys {previous:?} and \
                 {wire_key:?})"
            );
        }
        if let Some(from) = entry.ref_.from.as_deref() {
            ManifestPath::from_wire(from)
                .ok_or_else(|| anyhow!("invalid manifest file ref from: {from}"))?;
        }
        file_wire_keys.insert(path.clone(), wire_key.clone());
        bundled.insert(path, entry.content.clone());
    }

    Ok(bundled)
}

pub fn settings_layer_with_resolved_dockerfiles(
    source: &str,
    config_path: &ManifestPath,
    files: &HashMap<ManifestPath, String>,
    settings_source: SettingsSource,
) -> Result<SettingsLayer> {
    run_compiler::settings_layer_with_resolved_dockerfiles(
        source,
        config_path,
        files,
        settings_source,
    )
    .map_err(anyhow::Error::new)
}

pub(crate) fn manifest_args_overrides(
    args: Option<&types::ManifestArgs>,
) -> Result<ManifestSettingsOverrides> {
    let Some(args) = args else {
        return Ok(ManifestSettingsOverrides::default());
    };

    let run = fabro_manifest::build_sparse_run_overrides(fabro_manifest::RunOverrideInput {
        goal:             None,
        model:            args.model.as_deref(),
        provider:         args.provider.as_deref(),
        environment:      args.environment.as_deref(),
        preserve_sandbox: args.preserve_sandbox,
        dry_run:          args.dry_run,
        auto_approve:     args.auto_approve,
        labels:           parse_labels(&args.label),
    });

    // Verbose is a CLI output concern in v2; route it through cli.output.verbosity.
    let cli = args.verbose.and_then(|verbose| {
        verbose.then(|| CliLayer {
            output: Some(CliOutputLayer {
                verbosity: Some(OutputVerbosity::Verbose),
                ..CliOutputLayer::default()
            }),
            ..CliLayer::default()
        })
    });

    Ok(ManifestSettingsOverrides {
        run,
        cli,
        input_overrides: parse_input_overrides(&args.input)?,
    })
}

fn manifest_project_config_path(
    config: &types::ManifestConfig,
    cwd: &Path,
) -> Result<ManifestPath> {
    let path = config
        .path
        .as_deref()
        .ok_or_else(|| anyhow!("invalid manifest project config path: missing path"))?;
    let path_ref = Path::new(path);
    let manifest_path = if path_ref.is_absolute() {
        ManifestPath::from_absolute(path_ref, cwd)
    } else {
        ManifestPath::from_wire(path)
    };
    manifest_path.ok_or_else(|| anyhow!("invalid manifest project config path: {path}"))
}

async fn build_preflight_report(
    ctx: &AdmissionContext,
    prepared: &PreparedManifest,
    check: &ManifestCheck,
    shape: &WorkflowShape,
) -> Result<(CheckReport, bool)> {
    let mut checks = base_preflight_checks(prepared, shape);
    if check.has_errors() {
        return Ok((
            CheckReport {
                title:    "Run Preflight".into(),
                sections: vec![CheckSection {
                    title: String::new(),
                    checks,
                }],
            },
            true,
        ));
    }

    let resolved_run = prepared.settings.run.clone();
    let server_settings = ctx.server_settings.clone();
    let github_integration = &server_settings.server.integrations.github;
    let sandbox_provider = effective_sandbox_provider(&resolved_run);
    if let Some(error) = sandbox_provider_policy_error(&server_settings, &sandbox_provider) {
        checks.push(CheckResult {
            name:        "Sandbox Provider Policy".into(),
            status:      CheckStatus::Error,
            summary:     error,
            details:     Vec::new(),
            remediation: None,
        });
        return Ok((
            CheckReport {
                title:    "Run Preflight".into(),
                sections: vec![CheckSection {
                    title: String::new(),
                    checks,
                }],
            },
            false,
        ));
    }
    run_environment_capability_check(&mut checks, &resolved_run);
    let needs_github_credentials = sandbox_provider.clones_workspace()
        || resolved_run.integrations.github.is_token_requested();
    let github_app = if needs_github_credentials {
        match (ctx.github_credentials)(github_integration).await {
            Ok(credentials) => credentials,
            // The callback contract: only an unavailable credential store is
            // Err — it fails the whole preflight instead of reading as "no
            // credentials". Every other app-side miss is Ok(None).
            Err(err) => return Err(err),
        }
    } else {
        None
    };

    let sandbox_ok = run_sandbox_check(
        &mut checks,
        &sandbox_provider,
        prepared,
        &resolved_run,
        &ctx.provider_health,
    )
    .await;
    let repository_access_ok = run_repository_access_check(
        &mut checks,
        &sandbox_provider,
        prepared,
        &resolved_run,
        github_app.clone(),
    )
    .await;
    let github_token_ok =
        run_github_token_check(&mut checks, prepared, &resolved_run, github_app).await;

    let checks_ok = sandbox_ok && repository_access_ok && github_token_ok;

    Ok((
        CheckReport {
            title:    "Run Preflight".into(),
            sections: vec![CheckSection {
                title: String::new(),
                checks,
            }],
        },
        checks_ok,
    ))
}

fn base_preflight_checks(prepared: &PreparedManifest, shape: &WorkflowShape) -> Vec<CheckResult> {
    let setup_command_count = prepared.settings.run.prepare.steps.len();
    let repo_summary = prepared.git.as_ref().map_or_else(
        || "unknown".to_string(),
        |git| {
            let https = fabro_github::ssh_url_to_https(&git.origin_url);
            fabro_github::parse_github_owner_repo(&https).map_or_else(
                |_| git.origin_url.clone(),
                |(owner, repo)| format!("{owner}/{repo}"),
            )
        },
    );

    vec![
        CheckResult {
            name:        "Repository".into(),
            status:      CheckStatus::Pass,
            summary:     repo_summary,
            details:     vec![
                CheckDetail::new(format!("Setup commands: {setup_command_count}")),
                CheckDetail {
                    text: format!(
                        "Git: {}",
                        prepared
                            .git
                            .as_ref()
                            .map_or("unknown", |git| match git.dirty {
                                fabro_types::DirtyStatus::Clean => "clean",
                                fabro_types::DirtyStatus::Dirty => "dirty",
                                fabro_types::DirtyStatus::Unknown => "unknown",
                            })
                    ),
                    warn: prepared
                        .git
                        .as_ref()
                        .is_some_and(|git| git.dirty != fabro_types::DirtyStatus::Clean),
                },
            ],
            remediation: None,
        },
        CheckResult {
            name:        "Workflow".into(),
            status:      CheckStatus::Pass,
            summary:     shape.name.clone(),
            details:     vec![
                CheckDetail::new(format!("Nodes: {}", shape.nodes)),
                CheckDetail::new(format!("Edges: {}", shape.edges)),
                CheckDetail::new(format!("Goal: {}", shape.goal)),
            ],
            remediation: None,
        },
    ]
}

pub fn sandbox_provider_policy_error(
    server_settings: &ServerSettings,
    provider: &SandboxProviderKind,
) -> Option<String> {
    let providers = &server_settings.server.sandbox.providers;
    match providers.get(provider) {
        Some(entry) if entry.enabled => None,
        Some(_) => Some(format!(
            "sandbox provider \"{provider}\" is disabled by server.sandbox.providers.{provider}.enabled"
        )),
        None => Some(format!(
            "sandbox provider \"{provider}\" is not configured; add [server.sandbox.providers.{provider}] to settings.toml"
        )),
    }
}

pub fn configured_sandbox_provider(settings: &RunNamespace) -> SandboxProviderKind {
    settings.environment.provider.clone()
}

pub fn effective_sandbox_provider(settings: &RunNamespace) -> SandboxProviderKind {
    configured_sandbox_provider(settings).effective_for(settings.execution.mode)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitRemoteRefCheck {
    pub origin_url: String,
    pub branch:     Option<String>,
}

fn clone_disabled_for_provider(
    provider: &SandboxProviderKind,
    resolved_run: &RunNamespace,
) -> bool {
    provider.clones_workspace() && !resolved_run.clone.enabled
}

fn run_environment_capability_check(checks: &mut Vec<CheckResult>, resolved_run: &RunNamespace) {
    let warnings = environment_capability_warnings(resolved_run);
    if warnings.is_empty() {
        return;
    }
    checks.push(CheckResult {
        name:        "Environment Capabilities".into(),
        status:      CheckStatus::Warning,
        summary:     format!("{} unsupported hint(s) ignored", warnings.len()),
        details:     warnings
            .into_iter()
            .map(|text| CheckDetail { text, warn: true })
            .collect(),
        remediation: None,
    });
}

pub fn environment_capability_warnings(resolved_run: &RunNamespace) -> Vec<String> {
    let environment = &resolved_run.environment;
    let mut warnings = Vec::new();
    match environment.provider.bundled() {
        Some(BundledProvider::Local) => {
            if environment.resources.cpu.is_some()
                || environment.resources.memory.is_some()
                || environment.resources.disk.is_some()
            {
                warnings.push("local provider ignores resource limits".to_string());
            }
            if !environment.labels.is_empty() {
                warnings.push("local provider ignores labels".to_string());
            }
            if environment.lifecycle.auto_stop.is_some() {
                warnings.push("local provider ignores lifecycle.auto_stop".to_string());
            }
        }
        Some(BundledProvider::Docker) => {
            if environment.cwd.is_some() {
                warnings.push("docker provider ignores cwd".to_string());
            }
            if environment.resources.disk.is_some() {
                warnings.push("docker provider ignores disk resource limits".to_string());
            }
            if !environment.labels.is_empty() {
                warnings.push("docker provider ignores labels".to_string());
            }
            if environment.lifecycle.auto_stop.is_some() {
                warnings.push("docker provider ignores lifecycle.auto_stop".to_string());
            }
            // No `image.dockerfile` warning here: the build is LIVE on the
            // docker provider (ADR-0042) — the engine builds the image at
            // scope acquisition over the run's repository as context.
        }
        Some(BundledProvider::Daytona) => {
            if environment.cwd.is_some() {
                warnings.push("daytona provider ignores cwd".to_string());
            }
        }
        None => {
            if environment.cwd.is_some() {
                warnings.push(format!("{} provider ignores cwd", environment.provider));
            }
        }
    }
    warnings
}

fn repository_access_details(request: &GitRemoteRefCheck) -> Vec<CheckDetail> {
    let mut details = vec![CheckDetail::new(format!("Origin: {}", request.origin_url))];
    if let Some(branch) = request.branch.as_ref() {
        details.push(CheckDetail::new(format!("Branch: {branch}")));
    }
    details
}

async fn run_repository_access_check(
    checks: &mut Vec<CheckResult>,
    sandbox_provider: &SandboxProviderKind,
    prepared: &PreparedManifest,
    resolved_run: &RunNamespace,
    github_app: Option<fabro_github::GitHubCredentials>,
) -> bool {
    run_repository_access_check_with(
        checks,
        sandbox_provider,
        prepared,
        resolved_run,
        github_app,
        check_git_remote_ref,
    )
    .await
}

pub async fn run_repository_access_check_with<F, Fut>(
    checks: &mut Vec<CheckResult>,
    sandbox_provider: &SandboxProviderKind,
    prepared: &PreparedManifest,
    resolved_run: &RunNamespace,
    github_app: Option<fabro_github::GitHubCredentials>,
    check_remote_ref: F,
) -> bool
where
    F: FnOnce(GitRemoteRefCheck, Option<fabro_github::GitHubCredentials>) -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    if !sandbox_provider.clones_workspace()
        || clone_disabled_for_provider(sandbox_provider, resolved_run)
    {
        return true;
    }

    let Some(git) = prepared.git.as_ref() else {
        return true;
    };

    let origin_url = fabro_github::normalize_repo_origin_url(&git.origin_url);
    if let Err(err) = fabro_github::parse_github_owner_repo(&origin_url) {
        checks.push(CheckResult {
            name:        "Repository Access".into(),
            status:      CheckStatus::Error,
            summary:     "failed".into(),
            details:     vec![CheckDetail::new(format!("Origin: {origin_url}"))],
            remediation: Some(format!(
                "Clone-based sandboxes currently support GitHub repository origins only: {err}"
            )),
        });
        return false;
    }

    let request = GitRemoteRefCheck {
        origin_url,
        branch: Some(git.branch.clone()).filter(|branch| !branch.trim().is_empty()),
    };
    let details = repository_access_details(&request);

    match check_remote_ref(request, github_app).await {
        Ok(()) => {
            checks.push(CheckResult {
                name: "Repository Access".into(),
                status: CheckStatus::Pass,
                summary: "reachable".into(),
                details,
                remediation: None,
            });
            true
        }
        Err(err) => {
            checks.push(CheckResult {
                name: "Repository Access".into(),
                status: CheckStatus::Error,
                summary: "failed".into(),
                details,
                remediation: Some(format!("Failed to verify repository access: {err}")),
            });
            false
        }
    }
}

async fn check_git_remote_ref(
    request: GitRemoteRefCheck,
    github_app: Option<fabro_github::GitHubCredentials>,
) -> Result<(), String> {
    let auth_url = match github_app.as_ref() {
        Some(creds) => Some(
            fabro_github::resolve_authenticated_url(
                &fabro_github::GitHubContext::new(creds, &fabro_github::github_api_base_url()),
                &request.origin_url,
            )
            .await
            .map_err(|err| format!("Failed to resolve GitHub credentials: {err}"))?,
        ),
        None => None,
    };
    let remote_url = auth_url
        .as_ref()
        .map_or(request.origin_url.as_str(), |url| url.as_raw_url().as_str());

    let mut command = Command::new("git");
    command.env("GIT_TERMINAL_PROMPT", "0").args([
        "ls-remote",
        "--heads",
        "--exit-code",
        remote_url,
    ]);
    if let Some(branch) = request.branch.as_ref() {
        command.arg(branch);
    }

    run_ls_remote(command)
        .await
        .map_err(|message| match &auth_url {
            Some(auth_url) => auth_url.redact_in(&message),
            None => message,
        })
}

/// Run a prepared `git ls-remote` invocation with a 10s timeout, reducing a
/// failure to its most useful message: stderr, then stdout, then the exit
/// status.
pub async fn run_ls_remote(mut command: Command) -> std::result::Result<(), String> {
    let output = fabro_proc::capture(
        &mut command,
        Some(Duration::from_secs(10)),
        &CancellationToken::new(),
        None,
    )
    .await
    .map_err(|err| match err {
        ProcessError::TimedOut => "git ls-remote timed out after 10s".to_string(),
        ProcessError::Io(source) => format!("Failed to run git ls-remote: {source}"),
        ProcessError::Cancelled => "git ls-remote cancelled".to_string(),
    })?
    .output;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Err(if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        format!("git ls-remote exited with status {}", output.status)
    })
}

/// The sandbox check of preflight: the run's provider is reachable and its
/// credential accepted, as the provider's own health check reports. No
/// sandbox is created: Petri creates the run's, in the run's own shape,
/// when the run starts. A `local` environment must also resolve the
/// directory the run would work in.
pub async fn run_sandbox_check(
    checks: &mut Vec<CheckResult>,
    sandbox_provider: &SandboxProviderKind,
    prepared: &PreparedManifest,
    resolved_run: &RunNamespace,
    provider_health: &ProviderHealthSource,
) -> bool {
    if sandbox_provider.bundled() == Some(BundledProvider::Local) {
        if let Err(err) = resolved_run
            .environment
            .local_working_directory(Some(&prepared.source_directory))
        {
            checks.push(CheckResult {
                name:        "Sandbox".into(),
                status:      CheckStatus::Error,
                summary:     "failed".into(),
                details:     vec![CheckDetail::new(format!("Provider: {sandbox_provider}"))],
                remediation: Some(format!(
                    "Failed to resolve local environment working directory: {err}"
                )),
            });
            return false;
        }
    }
    let health = (provider_health)(sandbox_provider)
        .await
        .map_err(|err| format!("{err:#}"));
    let check = sandbox_health_check(sandbox_provider, health);
    let passed = check.status == CheckStatus::Pass;
    checks.push(check);
    passed
}

/// The preflight check for a provider's health report: a connection
/// failure and an unreachable or rejected backend fail with the reason; a
/// healthy backend, or one whose provider has no health check, passes.
pub fn sandbox_health_check(
    sandbox_provider: &SandboxProviderKind,
    health: std::result::Result<ProviderHealth, String>,
) -> CheckResult {
    let details = vec![CheckDetail::new(format!("Provider: {sandbox_provider}"))];
    let failure = |remediation: String| CheckResult {
        name:        "Sandbox".into(),
        status:      CheckStatus::Error,
        summary:     "failed".into(),
        details:     details.clone(),
        remediation: Some(remediation),
    };
    let health = match health {
        Ok(health) => health,
        Err(err) => return failure(err),
    };
    match health.status {
        HealthStatus::Ok | HealthStatus::Unknown => CheckResult {
            name: "Sandbox".into(),
            status: CheckStatus::Pass,
            summary: sandbox_provider.to_string(),
            details,
            remediation: None,
        },
        HealthStatus::Unreachable => failure(format!(
            "{sandbox_provider} backend is unreachable: {}",
            health
                .message
                .unwrap_or_else(|| "the backend did not answer".to_string())
        )),
        HealthStatus::Unauthorized if !health.missing_permissions.is_empty() => failure(format!(
            "{sandbox_provider} credential is missing required permissions: {}",
            health.missing_permissions.join(", ")
        )),
        HealthStatus::Unauthorized => failure(format!(
            "{sandbox_provider} rejected the credential: {}",
            health
                .message
                .unwrap_or_else(|| "the credential was rejected".to_string())
        )),
        _ => failure(format!(
            "{sandbox_provider} reported an unknown health state"
        )),
    }
}

async fn run_github_token_check(
    checks: &mut Vec<CheckResult>,
    prepared: &PreparedManifest,
    resolved_run: &RunNamespace,
    github_app: Option<fabro_github::GitHubCredentials>,
) -> bool {
    run_github_token_check_with(
        checks,
        prepared,
        resolved_run,
        github_app,
        mint_scoped_github_token,
        probe_github_repository,
    )
    .await
}

pub async fn run_github_token_check_with<M, MFut, P, PFut>(
    checks: &mut Vec<CheckResult>,
    prepared: &PreparedManifest,
    resolved_run: &RunNamespace,
    github_app: Option<fabro_github::GitHubCredentials>,
    mint_scoped_token: M,
    probe_repository: P,
) -> bool
where
    M: FnOnce(fabro_github::GitHubRepositoryAccess, fabro_github::GitHubCredentials) -> MFut,
    MFut: Future<Output = std::result::Result<ResolvedToken, String>>,
    P: Fn(fabro_types::GitHubRepositorySlug, ResolvedToken) -> PFut,
    PFut: Future<Output = std::result::Result<(), String>>,
{
    let github = &resolved_run.integrations.github;
    if !github.is_token_requested() {
        return true;
    }

    // Resolve InterpString permission values eagerly for token minting and
    // for display in the preflight report.
    let integration = match github.resolve_integration() {
        Ok(integration) => integration,
        Err(err) => {
            return fail_github_token_check(
                checks,
                Vec::new(),
                "invalid permissions",
                format!("Failed to resolve GitHub permissions: {err}"),
            );
        }
    };
    let perm_details = integration
        .permissions
        .iter()
        .map(|(key, value)| CheckDetail::new(format!("{key}: {value}")))
        .collect::<Vec<_>>();

    if !integration.has_additional_repositories() {
        // Primary-only behavior is unchanged: a mint check when credentials
        // and an origin exist, a warning otherwise, and no Git-content probe
        // (permissions-only workflows may request non-contents permissions).
        return check_primary_only_github_token(
            checks,
            prepared,
            github_app,
            &integration.permissions,
            perm_details,
        )
        .await;
    }

    // `gh` checks GH_TOKEN before GITHUB_TOKEN, so a user-defined GH_TOKEN
    // bypasses the managed scoped token for gh commands. Warn without
    // failing; the value is the workflow author's responsibility.
    if resolved_run.environment.env.contains_key(EnvVars::GH_TOKEN) {
        checks.push(CheckResult {
            name:        "GH_TOKEN Override".into(),
            status:      CheckStatus::Warning,
            summary:     "gh will not use the managed token".into(),
            details:     vec![],
            remediation: Some(
                "The resolved run environment defines GH_TOKEN, which the gh CLI prefers over \
                 the managed GITHUB_TOKEN; gh commands will not use the token scoped to the \
                 declared repositories."
                    .to_string(),
            ),
        });
    }

    let Some(origin_url) = prepared
        .git
        .as_ref()
        .map(|git| git.origin_url.trim())
        .filter(|url| !url.is_empty())
    else {
        return fail_github_token_check(
            checks,
            perm_details,
            "missing origin",
            "run.integrations.github.additional_repositories requires a GitHub run origin, but \
             this run has no repository origin URL"
                .to_string(),
        );
    };
    let Some(creds) = github_app else {
        return fail_github_token_check(
            checks,
            perm_details,
            "missing credentials",
            "run.integrations.github.additional_repositories requires GitHub credentials, but \
             none are configured on the server"
                .to_string(),
        );
    };
    // The same validated access value runtime initialization constructs, so
    // preflight and runtime cannot disagree about the effective set.
    let access = match fabro_github::GitHubRepositoryAccess::new(
        Some(origin_url),
        &integration.additional_repositories,
        integration.permissions.clone(),
    ) {
        Ok(Some(access)) => access,
        // `new` returns `Ok(None)` only when nothing is declared, and the
        // declared set is non-empty here. Fail closed instead of panicking.
        Ok(None) => {
            return fail_github_token_check(
                checks,
                perm_details,
                "missing origin",
                "run.integrations.github.additional_repositories requires a GitHub run origin, \
                 but this run has no repository origin URL"
                    .to_string(),
            );
        }
        Err(err) => {
            return fail_github_token_check(
                checks,
                perm_details,
                "invalid repository set",
                format!("{err:#}"),
            );
        }
    };

    // One mint scoped to the whole effective set. In App mode the minter
    // first resolves every repository's installation so a failure names the
    // repository the App cannot see.
    let token = match mint_scoped_token(access.clone(), creds).await {
        Ok(token) => token,
        Err(err) => {
            return fail_github_token_check(checks, perm_details, "failed", err);
        }
    };
    checks.push(CheckResult {
        name:        "GitHub Token".into(),
        status:      CheckStatus::Pass,
        summary:     "minted".into(),
        details:     perm_details.clone(),
        remediation: None,
    });

    // Probe every effective repository with bounded concurrency, then report
    // in deterministic primary-first order. Possession of a scoped token is
    // not proof of access; the probe also verifies PAT/static credentials.
    let targets: Vec<fabro_types::GitHubRepositorySlug> =
        access.targets().into_iter().cloned().collect();
    let probe_repository = &probe_repository;
    let mut results: Vec<(usize, CheckResult)> =
        stream::iter(targets.into_iter().enumerate().map(|(index, slug)| {
            let token = token.clone();
            let perm_details = perm_details.clone();
            async move {
                let check = match probe_repository(slug.clone(), token).await {
                    Ok(()) => CheckResult {
                        name:        format!("GitHub Repository ({slug})"),
                        status:      CheckStatus::Pass,
                        summary:     "reachable".into(),
                        details:     perm_details,
                        remediation: None,
                    },
                    Err(err) => CheckResult {
                        name:        format!("GitHub Repository ({slug})"),
                        status:      CheckStatus::Error,
                        summary:     "failed".into(),
                        details:     perm_details,
                        remediation: Some(format!("Failed to verify repository access: {err}")),
                    },
                };
                (index, check)
            }
        }))
        .buffer_unordered(REPOSITORY_PROBE_CONCURRENCY)
        .collect()
        .await;
    results.sort_by_key(|(index, _)| *index);

    let mut ok = true;
    for (_, check) in results {
        if check.status != CheckStatus::Pass {
            ok = false;
        }
        checks.push(check);
    }
    ok
}

/// Report one "GitHub Token" preflight failure and fail the check.
fn fail_github_token_check(
    checks: &mut Vec<CheckResult>,
    perm_details: Vec<CheckDetail>,
    summary: &str,
    remediation: String,
) -> bool {
    checks.push(CheckResult {
        name:        "GitHub Token".into(),
        status:      CheckStatus::Error,
        summary:     summary.into(),
        details:     perm_details,
        remediation: Some(remediation),
    });
    false
}

/// Bounded concurrency for per-repository `git ls-remote` probes.
const REPOSITORY_PROBE_CONCURRENCY: usize = 4;

async fn check_primary_only_github_token(
    checks: &mut Vec<CheckResult>,
    prepared: &PreparedManifest,
    github_app: Option<fabro_github::GitHubCredentials>,
    permissions: &HashMap<String, String>,
    perm_details: Vec<CheckDetail>,
) -> bool {
    if let (Some(creds), Some(git)) = (&github_app, prepared.git.as_ref()) {
        match mint_github_token(creds, &git.origin_url, permissions).await {
            Ok(_) => {
                checks.push(CheckResult {
                    name:        "GitHub Token".into(),
                    status:      CheckStatus::Pass,
                    summary:     "minted".into(),
                    details:     perm_details,
                    remediation: None,
                });
                true
            }
            Err(err) => {
                checks.push(CheckResult {
                    name:        "GitHub Token".into(),
                    status:      CheckStatus::Error,
                    summary:     "failed".into(),
                    details:     perm_details,
                    remediation: Some(format!("Failed to mint GitHub token: {err}")),
                });
                false
            }
        }
    } else {
        checks.push(CheckResult {
            name:        "GitHub Token".into(),
            status:      CheckStatus::Warning,
            summary:     "skipped".into(),
            details:     perm_details,
            remediation: Some("No GitHub credentials or origin URL available".to_string()),
        });
        true
    }
}

/// Production minter for the multi-repository path. The source owns the
/// effective set. In App mode its first resolve checks every repository's
/// installation before minting the scoped token.
async fn mint_scoped_github_token(
    access: fabro_github::GitHubRepositoryAccess,
    creds: fabro_github::GitHubCredentials,
) -> std::result::Result<ResolvedToken, String> {
    let source =
        InstallationTokenSource::for_access(&creds, &access).map_err(|err| format!("{err:#}"))?;
    source.resolve().await.map_err(|err| format!("{err:#}"))
}

/// Production per-repository probe: a non-interactive
/// `git ls-remote <https-url> HEAD` authenticated through
/// [`fabro_github::GITHUB_CREDENTIAL_HELPER`] reading `GITHUB_TOKEN` from
/// the child process environment, so the token never appears in the URL,
/// argv, or rendered errors — exactly what the runtime `git_bridge`
/// configures in `fabro-workflow`.
async fn probe_github_repository(
    slug: fabro_types::GitHubRepositorySlug,
    token: ResolvedToken,
) -> std::result::Result<(), String> {
    let url = slug.https_url();
    probe_with_replication_retry(token.snapshot, || run_probe_ls_remote(&url, &token)).await
}

/// Retry auth-shaped failures with the SAME token: replication of a given
/// token only makes progress, while re-minting would restart the replication
/// clock. The driver's git retry owns the decision and the pacing; fabro's
/// probe policy owns the attempt count.
pub async fn probe_with_replication_retry<F, Fut>(
    snapshot: TokenSnapshot,
    run: F,
) -> std::result::Result<(), String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = std::result::Result<(), String>>,
{
    retry_git_messages(
        &repository_probe_policy(),
        Some(&snapshot),
        "repository probe",
        run,
    )
    .await
}

// ── Fabro's retry budget for git operations against GitHub ─────────────────
//
// The driver owns the retry loop and the decision
// (`sandbox_driver::retry_git`): a remote that cannot be reached is retried,
// a rejected credential is retried only while the token is fresh enough to
// still be replicating to GitHub's git endpoints, a static credential fails
// fast, and a command whose outcome is unknown is never replayed. Fabro
// keeps what is policy: how many attempts the host-side repository probe
// gets, how it paces them, and when the credential it runs with was minted.
//
// Retries reuse the same token on purpose. Replication of a given token
// only makes progress, so each attempt strictly improves the odds, while
// re-minting would restart the replication clock.

/// The username GitHub expects with an installation token or PAT.
const GITHUB_TOKEN_USERNAME: &str = "x-access-token";

/// Backoff between attempts: 3s, then 9s.
///
/// GitHub's guidance for token replication is to wait a few seconds and
/// retry with the same token. Sub-second delays land inside the same
/// replication window and spend an attempt for nothing.
fn replication_backoff() -> GitBackoff {
    GitBackoff::new(Duration::from_secs(3), 3.0, Duration::from_secs(10))
}

/// Host-side repository probes get 3 attempts at replication pacing, with
/// no deadline of their own.
pub fn repository_probe_policy() -> GitRetryPolicy {
    GitRetryPolicy::new(3, replication_backoff())
}

/// Credentials carrying only the token's mint time, which is all the
/// driver's decision reads for git that ran outside a sandbox. The token
/// itself never leaves its snapshot.
fn credential_age(snapshot: Option<&TokenSnapshot>) -> Option<GitCredentials> {
    let snapshot = snapshot?;
    let credentials = GitCredentials::new(GITHUB_TOKEN_USERNAME, "");
    Some(match snapshot.minted_at() {
        Some(minted_at) => credentials.minted_at(SystemTime::from(minted_at)),
        None => credentials,
    })
}

/// The driver's failure for a rendered git message, so git that ran
/// outside a sandbox (the host-side repository probe) is classified the
/// same way as git the driver ran.
fn classified_git_failure(operation: &str, message: &str) -> sandbox_driver::Error {
    sandbox_driver::Error::Git(GitFailure::classified(
        operation,
        GitFailureKind::from_message(message),
        None,
    ))
}

/// Runs a host-side git operation that reports failures as rendered
/// messages under `policy`, retrying while the driver's decision says the
/// message is transient for the token behind `snapshot`. The final failure
/// comes back as the operation's own message.
pub async fn retry_git_messages<F, Fut>(
    policy: &GitRetryPolicy,
    snapshot: Option<&TokenSnapshot>,
    operation: &str,
    mut run: F,
) -> std::result::Result<(), String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = std::result::Result<(), String>>,
{
    let credentials = credential_age(snapshot);
    // The operation's own message is kept beside the classified failure the
    // driver decides on, so the caller reads the message it knows.
    let last_message = Mutex::new(None);
    let result = sandbox_driver::retry_git(
        policy,
        credentials.as_ref(),
        operation,
        |_attempt, _timeout| {
            let attempt = run();
            let last_message = &last_message;
            async move {
                attempt.await.map_err(|message| {
                    let error = classified_git_failure(operation, &message);
                    *last_message.lock().unwrap_or_else(PoisonError::into_inner) = Some(message);
                    error
                })
            }
        },
    )
    .await;
    match result {
        Ok(_) => Ok(()),
        Err(failure) => Err(last_message
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner)
            .unwrap_or_else(|| failure.error.to_string())),
    }
}

async fn run_probe_ls_remote(url: &str, token: &ResolvedToken) -> std::result::Result<(), String> {
    let mut command = Command::new("git");
    fabro_github::apply_probe_git_env(&mut command, token.token.expose());
    command.args(["ls-remote", url, "HEAD"]);
    run_ls_remote(command).await
}

async fn mint_github_token(
    creds: &fabro_github::GitHubCredentials,
    origin_url: &str,
    permissions: &HashMap<String, String>,
) -> Result<String> {
    let https_url = fabro_github::ssh_url_to_https(origin_url);
    let (owner, repo) = fabro_github::parse_github_owner_repo(&https_url)?;
    let client = fabro_http::http_client()?;
    let perms_json = serde_json::to_value(permissions)?;
    creds
        .resolve_bearer_token(
            &client,
            &owner,
            &repo,
            &fabro_github::github_api_base_url(),
            perms_json,
        )
        .await
}

/// The workflow as the validate and preflight responses describe it: its
/// name, its size and its goal. From the graph Petri admitted when it did;
/// for a refused workflow, from the DOT as written, so the response still
/// names what was checked. The goal is the run's effective goal, as create
/// materializes it: the settings' `run.goal` when the run has one, else the
/// workflow's own.
pub(crate) struct WorkflowShape {
    name:  String,
    nodes: usize,
    edges: usize,
    goal:  String,
}

pub(crate) fn workflow_shape(check: &ManifestCheck, prepared: &PreparedManifest) -> WorkflowShape {
    workflow_shape_of(
        check,
        &prepared.target_path,
        &prepared.root_source,
        &prepared.settings,
        &prepared.source_directory,
    )
}

pub(crate) fn workflow_shape_of(
    check: &ManifestCheck,
    graph_path: &ManifestPath,
    root_source: &str,
    settings: &WorkflowSettings,
    working_directory: &Path,
) -> WorkflowShape {
    let mut shape = check.graph.as_ref().map_or_else(
        || {
            WorkflowGraph::parse(&graph_path.to_string(), root_source).map_or_else(
                |_| WorkflowShape {
                    name:  String::new(),
                    nodes: 0,
                    edges: 0,
                    goal:  String::new(),
                },
                |graph| WorkflowShape {
                    goal:  graph.goal().unwrap_or_default().to_string(),
                    nodes: graph.node_count(),
                    edges: graph.edge_count(),
                    name:  graph.name().to_string(),
                },
            )
        },
        |graph| WorkflowShape {
            name:  graph.name.clone(),
            nodes: graph.nodes.len(),
            edges: graph.edges.len(),
            goal:  graph.goal().to_string(),
        },
    );
    if let Ok(Some(resolved)) = resolve_run_goal_from_namespace(&settings.run, working_directory) {
        shape.goal = resolved.text;
    }
    shape
}

pub(crate) fn workflow_summary(
    check: &ManifestCheck,
    shape: &WorkflowShape,
    target_path: &Path,
) -> types::PreflightWorkflowSummary {
    types::PreflightWorkflowSummary {
        diagnostics: diagnostics_to_api(&check.diagnostics),
        edges:       i64::try_from(shape.edges).expect("graph edge count should fit in i64"),
        goal:        shape.goal.clone(),
        graph_path:  Some(target_path.display().to_string()),
        name:        shape.name.clone(),
        nodes:       i64::try_from(shape.nodes).expect("graph node count should fit in i64"),
    }
}

fn diagnostics_to_api(diagnostics: &[Diagnostic]) -> Vec<types::WorkflowDiagnostic> {
    diagnostics
        .iter()
        .map(|diagnostic| types::WorkflowDiagnostic {
            column:      diagnostic
                .column
                .and_then(|value| i32::try_from(value).ok()),
            edge:        diagnostic
                .edge
                .as_ref()
                .map(|edge: &(String, String)| [edge.0.clone(), edge.1.clone()]),
            fix:         diagnostic.fix.clone(),
            line:        diagnostic.line.and_then(|value| i32::try_from(value).ok()),
            message:     diagnostic.message.clone(),
            node_id:     diagnostic.node_id.clone(),
            related:     diagnostic
                .related
                .iter()
                .map(|related| types::RelatedWorkflowDiagnostic {
                    column:      related.column.and_then(|value| i32::try_from(value).ok()),
                    line:        related.line.and_then(|value| i32::try_from(value).ok()),
                    message:     related.message.clone(),
                    source_path: related.source_path.clone(),
                })
                .collect(),
            rule:        diagnostic.rule.clone(),
            severity:    match diagnostic.severity {
                Severity::Error => types::WorkflowDiagnosticSeverity::Error,
                Severity::Warning => types::WorkflowDiagnosticSeverity::Warning,
                Severity::Info => types::WorkflowDiagnosticSeverity::Info,
            },
            source_path: diagnostic.source_path.clone(),
            span_len:    diagnostic
                .span_len
                .and_then(|value| i64::try_from(value).ok()),
            span_start:  diagnostic
                .span_start
                .and_then(|value| i64::try_from(value).ok()),
        })
        .collect()
}

fn report_to_api(report: &CheckReport) -> types::PreflightCheckReport {
    types::PreflightCheckReport {
        sections: report
            .sections
            .iter()
            .map(|section| types::PreflightCheckSection {
                checks: section
                    .checks
                    .iter()
                    .map(|check| types::PreflightCheckResult {
                        details:     check
                            .details
                            .iter()
                            .map(|detail| types::PreflightCheckDetail {
                                text: detail.text.clone(),
                                warn: detail.warn,
                            })
                            .collect(),
                        name:        check.name.clone(),
                        remediation: check.remediation.clone(),
                        status:      match check.status {
                            CheckStatus::Pass => types::PreflightCheckResultStatus::Pass,
                            CheckStatus::Warning => types::PreflightCheckResultStatus::Warning,
                            CheckStatus::Error => types::PreflightCheckResultStatus::Error,
                        },
                        summary:     check.summary.clone(),
                    })
                    .collect(),
                title:  section.title.clone(),
            })
            .collect(),
        title:    report.title.clone(),
    }
}
