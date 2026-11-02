//! `fabro run queue` (ADR-0039 stage D, denkhaus/work#1107): the
//! adjudication queue — terminal runs whose latest verdict on the field
//! is not settled (pending or parked; `enabled` and `refused` runs have
//! left). The fast-path script reads its worklist here; each entry
//! carries the work item of the run's latest claim and, for parked
//! runs, the open-escalation marker (stage D part 2).

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
    let workflow = args
        .workflow
        .as_deref()
        .map(str::trim)
        .filter(|w| !w.is_empty());
    let ctx = base_ctx.with_target(&args.server)?;
    let client = ctx.server().await?;
    let queue = client.run_queue(field, workflow).await?;

    let printer = ctx.printer();
    if ctx.json_output() {
        print_json_pretty(&queue)?;
        return Ok(());
    }
    if queue.data.is_empty() {
        fabro_util::printout!(
            printer,
            "The {} queue is empty — every terminal run carries a settled verdict.",
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
        "WORK ITEM".cell().bold(use_color),
        "VERDICT".cell().bold(use_color),
        "TITLE".cell().bold(use_color),
    ];
    let rows: Vec<Vec<CellStruct>> = queue
        .data
        .iter()
        .map(|entry| {
            vec![
                entry.run.id.to_string().cell().bold(use_color),
                entry.run.workflow.slug.clone().unwrap_or_default().cell(),
                entry.run.lifecycle.status.to_string().cell(),
                entry
                    .work_item
                    .clone()
                    .unwrap_or_else(|| "—".to_string())
                    .cell(),
                entry.verdict.to_string().cell(),
                entry.run.title.clone().cell(),
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
