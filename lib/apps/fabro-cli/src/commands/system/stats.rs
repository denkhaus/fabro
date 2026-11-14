use anyhow::Result;
use cli_table::format::{Border, Justify, Separator};
use cli_table::{Cell, CellStruct, Style, Table};
use fabro_api::types;

use crate::args::StatsArgs;
use crate::command_context::CommandContext;
use crate::shared::{format_size, print_json_pretty};

/// The stats door (denkhaus/work#1326): one call answers "which
/// filesystem is full, how full, and what did the last sandbox-reap pass
/// free" — the question the 2026-11-14 `/storage` incident needed
/// host-side forensics for.
pub(super) async fn stats_command(args: &StatsArgs, base_ctx: &CommandContext) -> Result<()> {
    let ctx = base_ctx.with_connection(&args.connection)?;
    let server = ctx.server().await?;
    let output = server.get_system_stats().await?;
    stats_from(&output, ctx.json_output())
}

#[allow(
    clippy::print_stdout,
    reason = "The stats report belongs on stdout for piping."
)]
fn stats_from(output: &types::SystemStatsResponse, json_output: bool) -> Result<()> {
    if json_output {
        print_json_pretty(output)?;
        return Ok(());
    }

    let use_color = console::colors_enabled();
    let color_choice = if use_color {
        cli_table::ColorChoice::Auto
    } else {
        cli_table::ColorChoice::Never
    };

    let title = vec![
        "PATH".cell().bold(use_color),
        "TOTAL".cell().bold(use_color).justify(Justify::Right),
        "USED".cell().bold(use_color).justify(Justify::Right),
        "FREE".cell().bold(use_color).justify(Justify::Right),
        "USE%".cell().bold(use_color).justify(Justify::Right),
    ];
    let rows: Vec<Vec<CellStruct>> = output
        .filesystems
        .iter()
        .map(|row| {
            let bytes = |value: Option<i64>| u64::try_from(value.unwrap_or_default()).unwrap_or(0);
            vec![
                row.path.as_deref().unwrap_or("-").cell(),
                format_size(bytes(row.total_bytes))
                    .cell()
                    .justify(Justify::Right),
                format_size(bytes(row.used_bytes))
                    .cell()
                    .justify(Justify::Right),
                format_size(bytes(row.available_bytes))
                    .cell()
                    .justify(Justify::Right),
                format!("{}%", row.used_percent.unwrap_or_default())
                    .cell()
                    .justify(Justify::Right),
            ]
        })
        .collect();
    let table = rows
        .table()
        .title(title)
        .color_choice(color_choice)
        .border(Border::builder().build())
        .separator(Separator::builder().build());
    println!("{}", table.display()?);
    println!();
    println!("Rows with identical numbers share one filesystem.");

    if let Some(last_reap) = output.last_reap_pass.as_ref() {
        println!();
        println!(
            "Last reap pass: {} reaped, {} already absent, {} failed; freed {}; disk {}%",
            last_reap.reaped.unwrap_or_default(),
            last_reap.absent.unwrap_or_default(),
            last_reap.failed.unwrap_or_default(),
            format_size(u64::try_from(last_reap.freed_bytes.unwrap_or_default()).unwrap_or(0)),
            last_reap
                .disk_used_percent
                .map_or("-".to_string(), |percent| format!("{percent}%")),
        );
        if let Some(finished_at) = last_reap.finished_at {
            println!("  finished at {finished_at}");
        }
    } else {
        println!();
        println!("Last reap pass: none recorded yet (arm disabled or no pass ran)");
    }

    Ok(())
}
