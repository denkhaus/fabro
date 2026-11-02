//! `fabro work comments <selector>` (ADR-0039 stage D part 2,
//! denkhaus/work#1107): the comment READ verb — a work item's comments,
//! oldest first. The answer channel a parked run's fork travels back
//! on: the adjudicator parks with a `parked-fork:` question, the
//! operator answers on the work item, and the next pass (or this verb)
//! reads the answer. Writing stays the human's surface; reading is open
//! to every worker of the project (the radius read's authorization).

use anyhow::Result;
use fabro_types::WorkItemComment;
use fabro_util::printer::Printer;

use super::resolve_item_key;
use crate::args::WorkCommentsArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

pub(super) async fn comments_command(args: &WorkCommentsArgs, ctx: &CommandContext) -> Result<()> {
    let client = ctx.server().await?;
    let item_key = resolve_item_key(&client, &args.project, &args.selector).await?;
    let comments = client.work_item_comments(&args.project, &item_key).await?;

    let printer = ctx.printer();
    if ctx.json_output() {
        print_json_pretty(&comments)?;
        return Ok(());
    }
    print_comments_text(printer, &item_key, &comments.comments);
    Ok(())
}

/// The human read: each comment as `author (when)` with its body
/// indented underneath — bodies are free prose (multi-line), so a table
/// cell would mangle them.
fn print_comments_text(printer: Printer, item_key: &str, comments: &[WorkItemComment]) {
    if comments.is_empty() {
        fabro_util::printout!(
            printer,
            "No comments on {} yet — the answer channel is empty.",
            item_key
        );
        return;
    }
    fabro_util::printout!(
        printer,
        "Comments on {} (oldest first, {}):",
        item_key,
        plural_comments(comments.len())
    );
    for (index, comment) in comments.iter().enumerate() {
        let when = comment
            .created_at
            .map_or_else(|| "unknown time".to_string(), |at| at.to_rfc3339());
        fabro_util::printout!(printer, "[{}] {} ({when})", index + 1, comment.author);
        for line in comment.body.lines() {
            fabro_util::printout!(printer, "    {line}");
        }
        fabro_util::printout!(printer, "");
    }
}

/// "1 comment" / "2 comments" for the header line.
fn plural_comments(count: usize) -> String {
    if count == 1 {
        "1 comment".to_string()
    } else {
        format!("{count} comments")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_counts_comments_in_grammar() {
        assert_eq!(plural_comments(1), "1 comment");
        assert_eq!(plural_comments(2), "2 comments");
    }
}
