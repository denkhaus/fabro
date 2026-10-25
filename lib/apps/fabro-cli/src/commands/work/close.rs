use anyhow::Result;

use super::resolve_item_key;
use crate::args::WorkCloseArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

pub(super) async fn close_command(args: &WorkCloseArgs, ctx: &CommandContext) -> Result<()> {
    let client = ctx.server().await?;
    let item_key = resolve_item_key(client.as_ref(), &args.project, &args.selector).await?;
    let item = client
        .close_work_item(&args.project, &item_key, &args.reason)
        .await?;

    let printer = ctx.printer();
    if ctx.json_output() {
        print_json_pretty(&item)?;
    } else {
        fabro_util::printerr!(printer, "Closed {} ({}).", item.item_key, item.state);
    }
    Ok(())
}
