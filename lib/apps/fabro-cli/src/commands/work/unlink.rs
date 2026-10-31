use anyhow::Result;

use super::resolve_item_key;
use crate::args::WorkUnlinkArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

pub(super) async fn unlink_command(args: &WorkUnlinkArgs, ctx: &CommandContext) -> Result<()> {
    let client = ctx.server().await?;
    let item_key = resolve_item_key(client.as_ref(), &args.project, &args.selector).await?;
    let response = client
        .unlink_work_item_observation(&args.project, &item_key, &args.run, args.seq)
        .await?;

    let printer = ctx.printer();
    if ctx.json_output() {
        print_json_pretty(&response)?;
    } else {
        let outcome = if response.removed {
            "removed"
        } else {
            "no link existed (nothing removed)"
        };
        fabro_util::printerr!(
            printer,
            "{}: {} from observation ({}, seq {}).",
            outcome,
            item_key,
            args.run,
            args.seq
        );
    }
    Ok(())
}
