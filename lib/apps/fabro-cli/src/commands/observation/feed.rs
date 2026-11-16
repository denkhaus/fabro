//! `fabro observation feed` (ADR-0044): the global observations feed —
//! every run's records with the DERIVED triage state, the triager
//! gear's worklist read. Door-symmetric with the `fabro_observation_feed`
//! agent tool. The `--json` stdout is the `GlobalObservationList`
//! RECORD (`.observations` + `.total`), never a bare array — the sweep
//! script parses off this stdout.

use anyhow::Result;
use cli_table::{Cell, CellStruct, Style, Table};
use fabro_util::terminal::Styles;

use crate::args::{ObservationCategoryArg, ObservationFeedArgs};
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

pub(super) async fn feed_command(args: &ObservationFeedArgs, ctx: &CommandContext) -> Result<()> {
    let category = args.category.map(|category| match category {
        ObservationCategoryArg::Painpoint => "painpoint".to_string(),
        ObservationCategoryArg::Note => "note".to_string(),
    });
    let triage = args
        .triage
        .as_deref()
        .map(str::trim)
        .filter(|triage| !triage.is_empty());
    let run_id = args
        .run_id
        .as_deref()
        .map(str::trim)
        .filter(|run| !run.is_empty());
    let query = args
        .query
        .as_deref()
        .map(str::trim)
        .filter(|query| !query.is_empty());
    let sort = args
        .sort
        .as_deref()
        .map(str::trim)
        .filter(|sort| !sort.is_empty());

    let client = ctx.server().await?;
    let response = client
        .list_global_observations(
            category.as_deref(),
            triage,
            run_id,
            query,
            sort,
            args.limit.unwrap_or(25),
            args.page.unwrap_or(1).max(1),
        )
        .await?;
    let observations = &response.observations;

    let printer = ctx.printer();
    if ctx.json_output() {
        print_json_pretty(&response)?;
        return Ok(());
    }
    if observations.is_empty() {
        fabro_util::printout!(printer, "The observation feed page is empty.");
        return Ok(());
    }

    let styles = Styles::detect_stdout();
    let use_color = styles.use_color;
    let title: Vec<CellStruct> = vec![
        "RUN".cell().bold(use_color),
        "SEQ".cell().bold(use_color),
        "CATEGORY".cell().bold(use_color),
        "TRIAGE".cell().bold(use_color),
        "RECORDED".cell().bold(use_color),
        "TEXT".cell().bold(use_color),
    ];
    let rows: Vec<Vec<CellStruct>> = observations
        .iter()
        .map(|observation| {
            let recorded = match observation.recorded_at {
                Some(at) => at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                None => "-".to_string(),
            };
            let head = observation.text.lines().next().unwrap_or_default();
            vec![
                observation.run_id.clone().cell(),
                observation.seq.to_string().cell(),
                observation.category.to_string().cell(),
                observation.triage_state.to_string().cell(),
                recorded.cell(),
                head.cell(),
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
    fabro_util::printout!(
        printer,
        "Page of {} — {} total.",
        observations.len(),
        response.total
    );
    Ok(())
}
