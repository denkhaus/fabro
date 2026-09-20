use anyhow::Result;
use cli_table::format::{Border, Separator};
use cli_table::{Cell, CellStruct, Style, Table};
use fabro_util::terminal::Styles;

use crate::args::AutomationsRunsArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

pub(super) async fn runs_command(args: &AutomationsRunsArgs, ctx: &CommandContext) -> Result<()> {
    let client = ctx.server().await?;
    let printer = ctx.printer();
    let runs = client.list_automation_runs(&args.id).await?;
    if ctx.json_output() {
        print_json_pretty(&runs)?;
        return Ok(());
    }

    if runs.is_empty() {
        fabro_util::printerr!(printer, "No runs recorded for automation {}.", args.id);
        return Ok(());
    }

    let styles = Styles::detect_stdout();
    let use_color = styles.use_color;

    let title: Vec<CellStruct> = vec![
        "RUN".cell().bold(use_color),
        "STATUS".cell().bold(use_color),
        "CREATED".cell().bold(use_color),
        "GOAL".cell().bold(use_color),
    ];

    let rows: Vec<Vec<CellStruct>> = runs
        .iter()
        .map(|run| {
            vec![
                run.id.to_string().cell().bold(use_color),
                run.lifecycle.status.to_string().cell(),
                run.timestamps.created_at.to_rfc3339().cell(),
                run.goal.clone().cell(),
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
