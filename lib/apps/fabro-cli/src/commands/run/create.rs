use std::path::Path;

use anyhow::{Context as _, anyhow, bail};
use fabro_config::project;
use fabro_server::manifest_validation;
use fabro_types::settings::run::EnvironmentProvider;
use fabro_types::{DirtyStatus, RunId, RunIntent, RunTarget};
use fabro_util::terminal::Styles;

use super::output::print_workflow_summary;
use super::overrides::prepare_intent_overrides;
use crate::args::RunArgs;
use crate::command_context::CommandContext;
use crate::commands::resolve_run_id;
use crate::user_config::{RunSettingsKeyPresence, read_project_run_settings_key_presence};

pub(crate) struct CreatedRun {
    pub(crate) run_id: RunId,
}

/// Create a workflow run: allocate run directory, persist RunSpec, return
/// (run_id, run_dir).
///
/// This does NOT execute the workflow — it only prepares the run directory.
pub(crate) async fn create_run(
    ctx: &CommandContext,
    args: &RunArgs,
    styles: &Styles,
    quiet: bool,
) -> anyhow::Result<CreatedRun> {
    let workflow_path = args
        .workflow
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("--workflow is required"))?;
    let canonical_cwd = ctx.cwd().canonicalize().with_context(|| {
        format!(
            "failed to canonicalize caller working directory {}",
            ctx.cwd().display()
        )
    })?;
    let user_workflows_root = fabro_util::Home::from_env().workflows_dir();
    let package = fabro_manifest::resolve_local_workflow_package(
        workflow_path,
        &canonical_cwd,
        Some(&user_workflows_root),
    )
    .map_err(anyhow::Error::new)?;
    let prepared = prepare_intent_overrides(args, &canonical_cwd)?;
    let validation = manifest_validation::validate_collected_workflow(
        package.closure(),
        Some(&prepared.run_layer),
        &prepared.input_overrides,
    )?;
    if !quiet {
        print_workflow_summary(
            &validation.workflow,
            Some(&package.workflow_location().graph),
            styles,
            ctx.printer(),
        );
    }
    if !validation.ok {
        bail!("Validation failed");
    }

    warn_untransmitted_settings(
        ctx,
        styles,
        ctx.base_config_path(),
        *ctx.run_settings_key_presence(),
    );
    let project_config = project::discover_project_config(&package.workflow_location().dir)?;
    if let Some(path) = project_config.as_deref() {
        warn_untransmitted_settings(
            ctx,
            styles,
            path,
            read_project_run_settings_key_presence(path)?,
        );
    }

    let client = ctx.server().await?;
    let parent_id = match args.parent.as_deref() {
        Some(parent_selector) => Some(resolve_run_id(client.as_ref(), parent_selector).await?),
        None => None,
    };
    let environment_id = args.environment.as_deref().unwrap_or("default");
    let environment = client
        .retrieve_environment(environment_id)
        .await
        .with_context(|| format!("could not retrieve environment `{environment_id}`"))?;
    let target =
        run_target_for_environment(environment.settings.provider, &canonical_cwd, ctx, styles)?;
    let workflow_version_id = package.closure().root_id();
    client
        .register_workflow_versions(
            package
                .closure()
                .versions()
                .map(|(_, validated)| validated.version()),
        )
        .await
        .context("could not register workflow versions")?;
    let created_run_id = client
        .create_run_from_intent(RunIntent {
            workflow_version_id,
            target,
            args: prepared.intent_args,
            environment_id: Some(environment.id.to_string()),
            parent_id,
            title: None,
            goal: prepared.goal,
        })
        .await
        .context("could not create run")?;

    Ok(CreatedRun {
        run_id: created_run_id,
    })
}

fn warn_untransmitted_settings(
    ctx: &CommandContext,
    styles: &Styles,
    path: &Path,
    presence: RunSettingsKeyPresence,
) {
    let keys = presence.key_paths();
    if keys.is_empty() {
        return;
    }
    fabro_util::printerr!(
        ctx.printer(),
        "{} {} contains {}; `fabro run` and `fabro create` do not transmit these settings. Move workflow-owned run behavior, including `run.pull_request`, to `workflow.toml`; configure placement with server-managed environments.",
        styles.yellow.apply_to("Warning:"),
        path.display(),
        keys.join(", "),
    );
}

fn run_target_for_environment(
    provider: EnvironmentProvider,
    canonical_cwd: &Path,
    ctx: &CommandContext,
    styles: &Styles,
) -> anyhow::Result<RunTarget> {
    match provider {
        EnvironmentProvider::Local => {
            let path = canonical_cwd.to_str().ok_or_else(|| {
                anyhow!(
                    "caller working directory is not valid UTF-8: {}",
                    canonical_cwd.display()
                )
            })?;
            Ok(RunTarget::Folder {
                path: path.to_string(),
            })
        }
        EnvironmentProvider::Docker | EnvironmentProvider::Daytona => {
            let Some(observation) = fabro_manifest::observe_git_run_target(canonical_cwd, None)
            else {
                return Ok(RunTarget::None {});
            };
            if observation.legacy_git_context.dirty == DirtyStatus::Dirty {
                fabro_util::printerr!(
                    ctx.printer(),
                    "{} the caller Git working tree is dirty; uncommitted changes are not included in the run target.",
                    styles.yellow.apply_to("Warning:"),
                );
            }
            let target = observation.run_target.ok_or_else(|| {
                anyhow!(
                    "the caller Git checkout cannot be represented as a canonical GitHub run target"
                )
            })?;
            Ok(RunTarget::Git(target))
        }
    }
}
