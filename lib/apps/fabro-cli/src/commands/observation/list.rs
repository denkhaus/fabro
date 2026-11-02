//! `fabro observation list --run-id <id>` (ADR-0039 stage D,
//! denkhaus/work#1107): a run's observation records in seq order — the
//! read door the adjudicator's fast path (counting painpoints) and the
//! agent pass (reading the facts) share. The `--json` stdout is the
//! `RunObservationList` RECORD, never a bare array: the fast path
//! parses `.observations` off this stdout, and a bare array silently
//! reads as zero facts.

use anyhow::Result;
use cli_table::{Cell, CellStruct, Style, Table};
use fabro_types::RunId;
use fabro_util::terminal::Styles;

use crate::args::ObservationListArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

pub(super) async fn list_command(args: &ObservationListArgs, ctx: &CommandContext) -> Result<()> {
    let run_id: RunId = args
        .run_id
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("`{}` is not a run id", args.run_id))?;
    let client = ctx.server().await?;
    let response = client.list_run_observations(&run_id).await?;
    let observations = &response.observations;

    let printer = ctx.printer();
    if ctx.json_output() {
        print_json_pretty(&response)?;
        return Ok(());
    }
    if observations.is_empty() {
        fabro_util::printout!(printer, "Run {} recorded no observations.", run_id);
        return Ok(());
    }

    let styles = Styles::detect_stdout();
    let use_color = styles.use_color;
    let title: Vec<CellStruct> = vec![
        "SEQ".cell().bold(use_color),
        "CATEGORY".cell().bold(use_color),
        "POSITION".cell().bold(use_color),
        "TEXT".cell().bold(use_color),
    ];
    let rows: Vec<Vec<CellStruct>> = observations
        .iter()
        .map(|observation| {
            let position = match (observation.execution, observation.firing) {
                (Some(execution), Some(firing)) => format!("{execution}/{firing}"),
                _ => "-".to_string(),
            };
            vec![
                observation.seq.to_string().cell(),
                observation.category.to_string().cell(),
                position.cell(),
                observation.text.clone().cell(),
            ]
        })
        .collect();
    let color_choice = if use_color {
        cli_table::ColorChoice::Auto
    } else {
        cli_table::ColorChoice::Never
    };
    let table = rows.table().title(title).color_choice(color_choice);
    fabro_util::printout!(printer, "{}", table.display()?);
    Ok(())
}
