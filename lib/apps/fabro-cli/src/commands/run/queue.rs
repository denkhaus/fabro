//! `fabro run queue` (ADR-0039 stage D, denkhaus/work#1107): the
//! adjudication queue — terminal runs without a verdict on the field.
//! The fast-path script reads its worklist here; a run leaves the queue
//! the moment a verdict record for the field exists.

use anyhow::Result;
use cli_table::{Cell, CellStruct, Style, Table};
use fabro_types::OrchestrationField;
use fabro_util::printer::Printer;
use fabro_util::terminal::Styles;

use crate::args::RunQueueArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

pub(super) async fn queue_command(args: &RunQueueArgs, base_ctx: &CommandContext) -> Result<()> {
    let field = match args.field.trim() {
        "merge" => OrchestrationField::Merge,
        "closable" => OrchestrationField::Closable,
        other => anyhow::bail!(
            "`{other}` is not an orchestration field — the fields are merge (the \
             adjudicator's pass-one queue) and closable (the merger's)"
        ),
    };
    let ctx = base_ctx.with_target(&args.server)?;
    let client = ctx.server().await?;
    let runs = client.run_queue(field).await?;

    let printer = ctx.printer();
    if ctx.json_output() {
        print_json_pretty(&runs)?;
        return Ok(());
    }
    if runs.is_empty() {
        fabro_util::printout!(
            printer,
            "The {} queue is empty — every terminal run carries a verdict.",
            field
        );
        return Ok(());
    }

    let styles = Styles::detect_stdout();
    let use_color = styles.use_color;
    let title: Vec<CellStruct> = vec![
        "RUN".cell().bold(use_color),
        "WORKFLOW".cell().bold(use_color),
        "STATUS".cell().bold(use_color),
        "TITLE".cell().bold(use_color),
    ];
    let rows: Vec<Vec<CellStruct>> = runs
        .iter()
        .map(|run| {
            vec![
                run.id.to_string().cell().bold(use_color),
                run.workflow.slug.clone().unwrap_or_default().cell(),
                run.lifecycle.status.to_string().cell(),
                run.title.clone().cell(),
            ]
        })
        .collect();
    print_run_table(printer, use_color, title, rows)
}

/// Render the queue table (the work list's print_table shape, local to
/// the run facade).
fn print_run_table(
    printer: Printer,
    use_color: bool,
    title: Vec<CellStruct>,
    rows: Vec<Vec<CellStruct>>,
) -> Result<()> {
    let color_choice = if use_color {
        cli_table::ColorChoice::Auto
    } else {
        cli_table::ColorChoice::Never
    };
    let table = rows.table().title(title).color_choice(color_choice);
    fabro_util::printout!(printer, "{}", table.display()?);
    Ok(())
}
