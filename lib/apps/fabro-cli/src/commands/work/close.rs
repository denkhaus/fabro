use anyhow::Result;

use super::resolve_item_key;
use crate::args::WorkCloseArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

pub(super) async fn close_command(args: &WorkCloseArgs, ctx: &CommandContext) -> Result<()> {
    // A close without evidence is no close (retro finding 2026-10-26: a
    // raced write closed an item with an EMPTY reason and nothing went
    // red — the boundary refuses empty/whitespace before any server call,
    // mirroring the update verb's empty-edit refusal).
    let reason = args.reason.trim();
    if reason.is_empty() {
        anyhow::bail!(
            "a close reason is required — pass --reason \"<evidence>\" (run ids, PR, commit)"
        );
    }
    let client = ctx.server().await?;
    let item_key = resolve_item_key(client.as_ref(), &args.project, &args.selector).await?;
    let item = client
        .close_work_item(&args.project, &item_key, reason)
        .await?;

    let printer = ctx.printer();
    if ctx.json_output() {
        print_json_pretty(&item)?;
    } else {
        fabro_util::printerr!(printer, "Closed {} ({}).", item.item_key, item.state);
    }
    Ok(())
}
