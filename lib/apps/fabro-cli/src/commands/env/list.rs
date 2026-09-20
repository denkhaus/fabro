use anyhow::Result;
use cli_table::format::{Border, Separator};
use cli_table::{Cell, CellStruct, Style, Table};
use fabro_util::terminal::Styles;

use super::short_revision;
use crate::args::EnvListArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

pub(super) async fn list_command(_args: &EnvListArgs, ctx: &CommandContext) -> Result<()> {
    let client = ctx.server().await?;
    let printer = ctx.printer();
    let environments = client.list_environments().await?;
    if ctx.json_output() {
        print_json_pretty(&environments)?;
        return Ok(());
    }

    if environments.is_empty() {
        fabro_util::printerr!(printer, "No environments found.");
        return Ok(());
    }

    let styles = Styles::detect_stdout();
    let use_color = styles.use_color;

    let title: Vec<CellStruct> = vec![
        "ID".cell().bold(use_color),
        "PROVIDER".cell().bold(use_color),
        "IMAGE".cell().bold(use_color),
        "REVISION".cell().bold(use_color),
    ];

    let rows: Vec<Vec<CellStruct>> = environments
        .iter()
        .map(|environment| {
            vec![
                environment.id.to_string().cell().bold(use_color),
                environment.settings.provider.to_string().cell(),
                environment
                    .settings
                    .image
                    .docker
                    .clone()
                    .unwrap_or_default()
                    .cell(),
                short_revision(environment.revision.as_str()).cell(),
            ]
        })
        .collect();

    let color_choice = if use_color {
        cli_table::ColorChoice::Auto
    } else {
        cli_table::ColorChoice::Never
    };
    let table = rows
        .table()
        .title(title)
        .color_choice(color_choice)
        .border(Border::builder().build())
        .separator(Separator::builder().build());
    fabro_util::printout!(printer, "{}", table.display()?);

    Ok(())
}
