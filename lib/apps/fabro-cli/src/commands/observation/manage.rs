//! `fabro observation manage` (ADR-0044): the triage decision door's
//! CLI facade — `no-filing` records that the triager (or the operator)
//! judged a painpoint needs no work item; the record then derives
//! `reviewed` in the feed, never `untriaged`. Door-symmetric with the
//! `fabro_observation_manage` agent tool.

use anyhow::Result;

use crate::args::ObservationManageArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

pub(super) async fn manage_command(
    args: &ObservationManageArgs,
    ctx: &CommandContext,
) -> Result<()> {
    let run_id: fabro_types::RunId = args
        .run_id
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("`{}` is not a run id", args.run_id))?;
    let reason = args.reason.trim();
    anyhow::ensure!(
        !reason.is_empty(),
        "the decision reason must not be blank — a decision without a rationale teaches nothing"
    );
    let action = args.action.to_string();
    let project = args
        .project
        .as_deref()
        .map(str::trim)
        .filter(|project| !project.is_empty());
    let item_key = args
        .item_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty());
    if action == "link_existing" && (project.is_none() || item_key.is_none()) {
        anyhow::bail!(
            "link-existing needs the item half beside the observation half — pass --project and \
             --item-key"
        );
    }
    let client = ctx.server().await?;
    let response = client
        .manage_observation(
            &run_id.to_string(),
            args.seq,
            &action,
            reason,
            project,
            item_key,
        )
        .await?;

    let printer = ctx.printer();
    if ctx.json_output() {
        print_json_pretty(&response)?;
    } else {
        let item = response
            .item_key
            .as_deref()
            .map(|item_key| format!(" → {item_key}"))
            .unwrap_or_default();
        fabro_util::printout!(
            printer,
            "{} decided on {} seq {}: {}{}",
            response.action,
            response.run_id,
            response.seq,
            response.reason,
            item
        );
    }
    Ok(())
}
