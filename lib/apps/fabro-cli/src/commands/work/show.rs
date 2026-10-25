use anyhow::Result;
use cli_table::{Cell, CellStruct, Style};
use fabro_util::terminal::Styles;

use super::{labels_cell, print_table, resolve_item_key};
use crate::args::WorkShowArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

/// One item: a head table with the fields a caller needs to decide what to do
/// next, then the body verbatim. `--json` carries the same item as structure.
pub(super) async fn show_command(args: &WorkShowArgs, ctx: &CommandContext) -> Result<()> {
    let client = ctx.server().await?;
    let item_key = resolve_item_key(&client, &args.project, &args.selector).await?;
    let item = client.retrieve_work_item(&args.project, &item_key).await?;

    if ctx.json_output() {
        print_json_pretty(&item)?;
        return Ok(());
    }

    let printer = ctx.printer();
    let styles = Styles::detect_stdout();
    let use_color = styles.use_color;

    let title: Vec<CellStruct> = vec![
        "FIELD".cell().bold(use_color),
        "VALUE".cell().bold(use_color),
    ];
    let rows: Vec<Vec<CellStruct>> = vec![
        field_row(use_color, "key", item.item_key.clone()),
        field_row(use_color, "state", item.state.clone()),
        field_row(use_color, "priority", item.priority.to_string()),
        field_row(use_color, "labels", labels_cell(&item.labels)),
        field_row(use_color, "title", item.title.clone()),
        field_row(use_color, "url", item.url.clone()),
        field_row(
            use_color,
            "updated",
            item.updated_at
                .map_or_else(|| "-".to_string(), |updated| updated.to_rfc3339()),
        ),
    ];
    print_table(printer, use_color, title, rows)?;
    fabro_util::printout!(printer, "\n{}", item.body);

    Ok(())
}

fn field_row(use_color: bool, field: &str, value: String) -> Vec<CellStruct> {
    vec![field.to_string().cell().bold(use_color), value.cell()]
}
