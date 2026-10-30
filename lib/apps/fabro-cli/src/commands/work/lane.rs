use anyhow::{Result, bail};

use super::{lane_label, project_row, resolve_item_key};
use crate::args::WorkLaneArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

/// Assigns a work item to a lane, or clears its lane with `--lane none`
/// — the tranche model's session move (work#1065). The lane name is
/// validated against the project's declared lanes exactly like `--lane`
/// on create, so a typo is refused here instead of silently unlanning
/// the item.
pub(super) async fn lane_command(args: &WorkLaneArgs, ctx: &CommandContext) -> Result<()> {
    let lane = args.lane.trim();
    if lane.is_empty() {
        bail!("the lane must not be empty — pass a lane name or `none` to clear the lane");
    }

    let client = ctx.server().await?;
    // `none` clears the lane: no registry lookup, the request travels
    // without one. Every other name resolves through the project's lane
    // labels (unknown lanes refused, declared lanes named) — the lookup
    // validates only; the wire carries the NAME and the server maps it
    // again through the same registry.
    let lane = if lane.eq_ignore_ascii_case("none") {
        None
    } else {
        let project = project_row(&client, &args.project).await?;
        lane_label(&project, lane)?;
        Some(lane)
    };
    let item_key = resolve_item_key(client.as_ref(), &args.project, &args.selector).await?;
    let item = client
        .lane_work_item(&args.project, &item_key, lane)
        .await?;

    let printer = ctx.printer();
    if ctx.json_output() {
        print_json_pretty(&item)?;
    } else {
        fabro_util::printerr!(
            printer,
            "Moved {} to {}.",
            item.item_key,
            lane.unwrap_or("none")
        );
    }
    Ok(())
}
