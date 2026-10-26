use anyhow::{Result, bail};

use super::{read_optional_body, resolve_item_key};
use crate::args::WorkUpdateArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

/// Edits a work item's title and/or body — the wayfinding append path
/// (ADR-0032: a resolver appends the answer to the item body; the
/// endpoint predates this verb, work#1008). At least one input is
/// required; the refusal happens before any server call. Tracker text
/// travels through stdin/file only (`--body-file -`), never a command
/// line.
pub(super) async fn update_command(args: &WorkUpdateArgs, ctx: &CommandContext) -> Result<()> {
    let title = args
        .title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let body = read_optional_body(args.body.as_deref(), args.body_file.as_deref())?
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if title.is_none() && body.is_none() {
        bail!("an update needs a title or a body — pass --title, --body, or --body-file - (stdin)");
    }

    let client = ctx.server().await?;
    let item_key = resolve_item_key(client.as_ref(), &args.project, &args.selector).await?;
    let item = client
        .update_work_item(&args.project, &item_key, title, body.as_deref())
        .await?;

    let printer = ctx.printer();
    if ctx.json_output() {
        print_json_pretty(&item)?;
    } else {
        fabro_util::printerr!(printer, "Updated {}.", item.item_key);
    }
    Ok(())
}
