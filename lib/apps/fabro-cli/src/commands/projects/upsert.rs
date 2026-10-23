use anyhow::{Result, anyhow, bail};
use fabro_api::types::WorkProjectUpsertRequest;
use fabro_config::{SettingsLayer, WorkLayer};

use crate::args::ProjectsUpsertArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

/// Upsert every work project the settings declare (fabro-f1a9). The server
/// heals its registry to the declared rows at startup; this command is the
/// runbook hammer for applying the same declaration without a restart. The
/// server stays the writer: the CLI only projects the settings onto
/// `PUT /api/v1/projects/{id}`.
pub(super) async fn upsert_command(args: &ProjectsUpsertArgs, ctx: &CommandContext) -> Result<()> {
    let settings: SettingsLayer =
        fabro_config::load_config_file(args.config.as_deref(), "settings.toml")?;
    let work = settings.work.unwrap_or_default();
    work.validate().map_err(anyhow::Error::msg)?;
    if work.projects.is_empty() {
        bail!(
            "no work projects declared: add a [work.projects.<id>] section to {}",
            args.config.as_deref().map_or_else(
                || "~/.fabro/settings.toml".to_owned(),
                |path| path.display().to_string()
            ),
        );
    }

    let mut project_ids: Vec<&String> = work.projects.keys().collect();
    project_ids.sort();

    if args.dry_run {
        let printer = ctx.printer();
        for project_id in project_ids {
            fabro_util::printerr!(printer, "would upsert work project {project_id}");
        }
        return Ok(());
    }

    let client = ctx.server().await?;
    let printer = ctx.printer();
    for project_id in project_ids {
        let request = upsert_request(project_id, &work)?;
        let project = client
            .upsert_work_project(project_id.as_str(), request)
            .await
            .map_err(|error| anyhow!("upserting work project {project_id}: {error}"))?;
        fabro_util::printerr!(
            printer,
            "Upserted work project {} (tracker {}/{}).",
            project.project_id,
            project.tracker_provider,
            project.tracker_repo
        );
        if ctx.json_output() {
            print_json_pretty(&project)?;
        }
    }
    Ok(())
}

fn upsert_request(project_id: &str, work: &WorkLayer) -> Result<WorkProjectUpsertRequest> {
    let row = work
        .projects
        .get(project_id)
        .ok_or_else(|| anyhow!("work.projects.{project_id} vanished between read and write"))?;
    Ok(WorkProjectUpsertRequest {
        display_name:             row.display_name.clone().unwrap_or_default(),
        tracker_provider:         row.tracker_provider.clone().unwrap_or_default(),
        tracker_repo:             row.tracker_repo.clone().unwrap_or_default(),
        target_repo:              row.target_repo.clone(),
        project_label:            row.project_label.clone().unwrap_or_default(),
        lane_labels:              row
            .lane_labels
            .clone()
            .unwrap_or_default()
            .into_iter()
            .collect(),
        claim_label_prefix:       row.claim_label_prefix.clone(),
        claim_comment_enabled:    row.claim_comment_enabled,
        backpressure_open_pr_cap: row.backpressure_open_pr_cap,
    })
}

#[cfg(test)]
mod tests {
    use fabro_config::{SettingsLayer, WorkLayer};

    use super::upsert_request;

    fn layer(toml: &str) -> WorkLayer {
        toml::from_str::<SettingsLayer>(toml)
            .expect("settings parse")
            .work
            .expect("work section present")
    }

    /// The settings row projects onto the wire request one-for-one; an
    /// omitted optional field stays absent so the server default applies.
    #[test]
    fn a_declared_row_projects_onto_the_upsert_request() {
        let work = layer(
            r#"
[work.projects.fabro]
display_name = "Fabro"
tracker_provider = "github"
tracker_repo = "denkhaus/work"
project_label = "project:fabro"
"#,
        );
        work.validate().expect("a complete row validates");

        let request = upsert_request("fabro", &work).expect("declared row projects");
        assert_eq!(request.display_name, "Fabro");
        assert_eq!(request.tracker_provider, "github");
        assert_eq!(request.tracker_repo, "denkhaus/work");
        assert_eq!(request.project_label, "project:fabro");
        assert_eq!(request.target_repo, None);
        assert_eq!(request.claim_label_prefix, None);
        assert_eq!(request.backpressure_open_pr_cap, None);
    }

    /// A half-declared row is refused before any request is built, naming
    /// the project and the missing fields.
    #[test]
    fn a_half_declared_row_is_refused_naming_project_and_fields() {
        let work = layer(
            r#"
[work.projects.docs]
display_name = "Docs"
tracker_provider = "github"
"#,
        );
        let error = work.validate().expect_err("an incomplete row is refused");
        assert!(error.contains("work.projects.docs"), "{error}");
        assert!(error.contains("tracker_repo"), "{error}");
        assert!(error.contains("project_label"), "{error}");
        assert!(!error.contains("claim_label_prefix"), "{error}");
    }
}
