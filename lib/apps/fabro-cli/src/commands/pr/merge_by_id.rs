use anyhow::Result;
use tracing::info;

use crate::args::PrMergeByIdArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

/// The label door's write side (ADR-0040, work#1205): merge one
/// repository pull request by its opaque provider-stable id — the
/// session half of the branch model. The run-scoped `pr merge` stays
/// the authority for run-linked pull requests.
pub(super) async fn merge_by_id_command(
    args: PrMergeByIdArgs,
    base_ctx: &CommandContext,
) -> Result<()> {
    let ctx = base_ctx.with_target(&args.server)?;
    let client = ctx.server().await?;
    let response = client
        .merge_repository_pull_request(&args.repo, &args.id, &args.method)
        .await?;

    info!(
        id = %response.id,
        repo = %args.repo,
        method = %response.method,
        "Merged repository pull request"
    );
    if ctx.json_output() {
        print_json_pretty(&response)?;
    } else {
        fabro_util::printout!(ctx.printer(), "Merged #{} ({})", response.id, response.url);
    }

    Ok(())
}
