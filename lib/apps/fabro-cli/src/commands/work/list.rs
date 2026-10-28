use anyhow::Result;
use cli_table::{Cell, CellStruct, Style};
use fabro_util::terminal::Styles;

use super::{ensure_known_lane, labels_cell, print_table, project_row};
use crate::args::WorkListArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

pub(super) async fn list_command(args: &WorkListArgs, ctx: &CommandContext) -> Result<()> {
    let client = ctx.server().await?;
    // A lane is a name in the project's registry, not a label a caller may
    // invent: the lane is checked here so a typo fails loudly instead of
    // listing an empty queue the server invented a label for.
    if let Some(lane) = args.lane.as_deref() {
        let project = project_row(&client, &args.project).await?;
        ensure_known_lane(&project, lane)?;
    }
    let printer = ctx.printer();
    let items = client
        .list_work_items(
            &args.project,
            args.lane.as_deref(),
            args.limit,
            args.query.as_deref(),
            args.label.as_deref(),
            args.page,
        )
        .await?;

    if ctx.json_output() {
        print_json_pretty(&items)?;
        return Ok(());
    }

    if items.is_empty() {
        fabro_util::printerr!(printer, "No work items found in project {}.", args.project);
        return Ok(());
    }

    let styles = Styles::detect_stdout();
    let use_color = styles.use_color;

    let title: Vec<CellStruct> = vec![
        "KEY".cell().bold(use_color),
        "PRIO".cell().bold(use_color),
        "STATE".cell().bold(use_color),
        "LABELS".cell().bold(use_color),
        "TITLE".cell().bold(use_color),
    ];

    let rows: Vec<Vec<CellStruct>> = items
        .iter()
        .map(|item| {
            vec![
                item.item_key.clone().cell().bold(use_color),
                item.priority.to_string().cell(),
                item.state.clone().cell(),
                labels_cell(&item.labels).cell(),
                item.title.clone().cell(),
            ]
        })
        .collect();

    print_table(printer, use_color, title, rows)
}
