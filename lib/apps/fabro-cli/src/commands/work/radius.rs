//! `fabro work radius <selector>` (ADR-0038 read door, ADR-0039 stage
//! D): a work item's closure radius — the open work items linked to the
//! observations of its runs, the same computation the close gate
//! enforces. The adjudicator's fast path demands an empty radius for
//! the trivial case; this verb is the script-facing read.

use anyhow::Result;
use cli_table::{Cell, CellStruct, Style};
use fabro_util::terminal::Styles;

use super::{print_table, resolve_item_key};
use crate::args::WorkRadiusArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

pub(super) async fn radius_command(args: &WorkRadiusArgs, ctx: &CommandContext) -> Result<()> {
    let client = ctx.server().await?;
    let item_key = resolve_item_key(&client, &args.project, &args.selector).await?;
    let radius = client.work_item_radius(&args.project, &item_key).await?;

    let printer = ctx.printer();
    if ctx.json_output() {
        print_json_pretty(&radius)?;
        return Ok(());
    }
    if radius.radius.is_empty() {
        fabro_util::printout!(
            printer,
            "{} has an empty closure radius — nothing blocks its close.",
            radius.item.item_key
        );
        return Ok(());
    }

    let styles = Styles::detect_stdout();
    let use_color = styles.use_color;
    let title: Vec<CellStruct> = vec![
        "ITEM".cell().bold(use_color),
        "STATE".cell().bold(use_color),
        "TITLE".cell().bold(use_color),
    ];
    let rows: Vec<Vec<CellStruct>> = radius
        .radius
        .iter()
        .map(|item| {
            vec![
                item.item_key.clone().cell().bold(use_color),
                item.state.clone().cell(),
                item.title.clone().cell(),
            ]
        })
        .collect();
    fabro_util::printout!(
        printer,
        "The open closure radius of {} ({} item(s)):",
        radius.item.item_key,
        radius.radius.len()
    );
    print_table(printer, use_color, title, rows)
}
