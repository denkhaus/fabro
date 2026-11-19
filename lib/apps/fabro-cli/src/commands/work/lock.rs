use anyhow::Result;

use super::resolve_item_key;
use crate::args::WorkLockArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

/// Places the operator lock (denkhaus/work#1420): a locked item is
/// refused by the claim pool and the grant door — "do not touch this
/// until I say so" — while its lane stays untouched, so re-routing
/// decisions stay visible. The server refuses a worker principal (the
/// lock is the human surface) and a live claim (the message names it).
pub(super) async fn lock_command(args: &WorkLockArgs, ctx: &CommandContext) -> Result<()> {
    let client = ctx.server().await?;
    let item_key = resolve_item_key(client.as_ref(), &args.project, &args.selector).await?;
    let item = client.lock_work_item(&args.project, &item_key).await?;

    let printer = ctx.printer();
    if ctx.json_output() {
        print_json_pretty(&item)?;
    } else {
        fabro_util::printerr!(
            printer,
            "Locked {} — claiming is refused until you unlock it; the lane is untouched.",
            item.item_key
        );
    }
    Ok(())
}

/// Lifts the operator lock (denkhaus/work#1420): the item becomes
/// grantable again. Unlocking an unlocked item succeeds — the server
/// just reports the item unlocked.
pub(super) async fn unlock_command(args: &WorkLockArgs, ctx: &CommandContext) -> Result<()> {
    let client = ctx.server().await?;
    let item_key = resolve_item_key(client.as_ref(), &args.project, &args.selector).await?;
    let item = client.unlock_work_item(&args.project, &item_key).await?;

    let printer = ctx.printer();
    if ctx.json_output() {
        print_json_pretty(&item)?;
    } else {
        fabro_util::printerr!(
            printer,
            "Unlocked {} — the item is grantable again.",
            item.item_key
        );
    }
    Ok(())
}
