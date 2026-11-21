use anyhow::Result;

use super::resolve_item_key;
use crate::args::WorkRefreshArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

/// The refresh door of the read-view materialization (ADR-0050):
/// force-load from the forge and re-materialize — one item via
/// `--item`, the project's whole universe without it. Ordinary reads
/// are write-invalidated only, so this is the door for foreign edits
/// made directly at the forge.
pub(super) async fn refresh_command(args: &WorkRefreshArgs, ctx: &CommandContext) -> Result<()> {
    let client = ctx.server().await?;
    let item_key = match args.item.as_deref() {
        Some(selector) => Some(resolve_item_key(client.as_ref(), &args.project, selector).await?),
        None => None,
    };
    let report = client
        .refresh_work_items(&args.project, item_key.as_deref())
        .await?;

    let printer = ctx.printer();
    if ctx.json_output() {
        print_json_pretty(&report)?;
    } else {
        let walk = if report.complete {
            String::new()
        } else {
            " — walk incomplete, the universe is untouched".to_string()
        };
        fabro_util::printerr!(
            printer,
            "Refreshed {} item(s) from the forge ({} changed{})",
            report.refreshed,
            report.changed,
            walk
        );
    }
    Ok(())
}
