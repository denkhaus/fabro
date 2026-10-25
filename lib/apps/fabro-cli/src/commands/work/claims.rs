use anyhow::Result;
use cli_table::{Cell, CellStruct, Style};
use fabro_util::terminal::Styles;

use super::print_table;
use crate::args::WorkClaimsArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

/// The live claims of one project: the queue view the host reads instead of
/// scanning run PRs (granted, active, awaiting_merge).
pub(super) async fn claims_command(args: &WorkClaimsArgs, ctx: &CommandContext) -> Result<()> {
    let client = ctx.server().await?;
    let printer = ctx.printer();
    let claims = client.list_work_claims(&args.project).await?;

    if ctx.json_output() {
        print_json_pretty(&claims)?;
        return Ok(());
    }

    if claims.is_empty() {
        fabro_util::printerr!(printer, "No live claims in project {}.", args.project);
        return Ok(());
    }

    let styles = Styles::detect_stdout();
    let use_color = styles.use_color;

    let title: Vec<CellStruct> = vec![
        "CLAIM".cell().bold(use_color),
        "ITEM".cell().bold(use_color),
        "RUN".cell().bold(use_color),
        "LANE".cell().bold(use_color),
        "STATE".cell().bold(use_color),
        "GRANTED".cell().bold(use_color),
        "PR".cell().bold(use_color),
    ];

    let rows: Vec<Vec<CellStruct>> = claims
        .iter()
        .map(|claim| {
            vec![
                claim.claim_id.to_string().cell(),
                claim.item_key.clone().cell().bold(use_color),
                claim.run_id.clone().cell(),
                claim.lane.clone().cell(),
                claim.state.to_string().cell(),
                claim.granted_at.to_rfc3339().cell(),
                claim
                    .pr_url
                    .clone()
                    .unwrap_or_else(|| "-".to_string())
                    .cell(),
            ]
        })
        .collect();

    print_table(printer, use_color, title, rows)
}
