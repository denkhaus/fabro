use anyhow::Result;
use tracing::info;

use crate::args::PrListArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

/// The label door's read side (ADR-0040, work#1205): a repository's pull
/// requests through the server's forge integration, optionally narrowed
/// by label. The merger's label arm reads its session-PR worklist here.
pub(super) async fn list_command(args: PrListArgs, base_ctx: &CommandContext) -> Result<()> {
    let ctx = base_ctx.with_target(&args.server)?;
    let client = ctx.server().await?;
    let listing = client
        .list_pull_requests(&args.repo, &args.state, args.label.as_deref())
        .await?;

    info!(
        repo = %args.repo,
        state = %args.state,
        label = args.label.as_deref().unwrap_or(""),
        count = listing.data.len(),
        "Listing pull requests"
    );

    if ctx.json_output() {
        print_json_pretty(&listing)?;
        return Ok(());
    }

    let printer = ctx.printer();
    if listing.data.is_empty() {
        fabro_util::printout!(printer, "No pull requests match.");
        return Ok(());
    }
    fabro_util::printout!(printer, "ID      STATE  LABELS                 TITLE");
    for pull_request in &listing.data {
        let labels = pull_request.labels.join(",");
        fabro_util::printout!(
            printer,
            "{:<7} {:<7} {:<22} {}",
            format!("#{}", pull_request.id),
            pull_request.state,
            labels,
            pull_request.title
        );
    }

    Ok(())
}
