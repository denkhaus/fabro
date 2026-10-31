use anyhow::Result;
use fabro_api::types::WorkObservationLinkOutcome;

use super::resolve_item_key;
use crate::args::WorkLinkArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

pub(super) async fn link_command(args: &WorkLinkArgs, ctx: &CommandContext) -> Result<()> {
    let client = ctx.server().await?;
    let item_key = resolve_item_key(client.as_ref(), &args.project, &args.selector).await?;
    let response = client
        .link_work_item_observation(&args.project, &item_key, &args.run, args.seq)
        .await?;

    let printer = ctx.printer();
    if ctx.json_output() {
        print_json_pretty(&response)?;
    } else {
        let outcome = match response.outcome {
            WorkObservationLinkOutcome::Linked => "linked",
            WorkObservationLinkOutcome::AlreadyLinked => "already linked (deduped at the link)",
        };
        fabro_util::printerr!(
            printer,
            "{}: {} to observation ({}, seq {}).",
            outcome,
            response.item_key,
            response.run_id,
            response.seq
        );
    }
    Ok(())
}
