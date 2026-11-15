//! `fabro workflow show <id>` (work#1347): the operator's read door for
//! a stored workflow version — the bundle overview (entrypoint, file
//! list, dependencies) by default, one file's content with `--file`.
//! The `--json` stdout is the stored `WorkflowVersion` record.

use anyhow::{Context as _, Result, anyhow};
use cli_table::format::{Border, Separator};
use cli_table::{Cell, CellStruct, Color, Style, Table};
use fabro_types::{WorkflowVersion, WorkflowVersionId};
use fabro_util::printer::Printer;
use fabro_util::terminal::Styles;

use crate::args::WorkflowShowArgs;
use crate::command_context::CommandContext;
use crate::shared::{color_if, print_json_pretty};

pub(super) async fn show_command(args: &WorkflowShowArgs, base_ctx: &CommandContext) -> Result<()> {
    let id: WorkflowVersionId = args
        .id
        .trim()
        .parse()
        .map_err(|_| anyhow!("`{}` is not a workflow version id (64 hex chars)", args.id))?;
    let ctx = base_ctx.with_target(&args.target)?;
    let client = ctx.server().await?;
    let version = client.get_workflow_version(&id).await?;

    let printer = ctx.printer();
    if let Some(file) = args.file.as_deref() {
        let content = version
            .files()
            .iter()
            .find_map(|(path, content)| (path.as_str() == file).then_some(content.as_str()))
            .ok_or_else(|| {
                anyhow!(
                    "the version stores no file {file}; stored files: {}",
                    version
                        .files()
                        .keys()
                        .map(fabro_types::WorkflowPath::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;
        fabro_util::printout!(printer, "{content}");
        return Ok(());
    }

    if ctx.json_output() {
        print_json_pretty(&version)?;
        return Ok(());
    }

    print_overview(&version, &id, printer)
}

fn print_overview(
    version: &WorkflowVersion,
    id: &WorkflowVersionId,
    printer: Printer,
) -> Result<()> {
    let styles = Styles::detect_stdout();
    let use_color = styles.use_color;
    fabro_util::printout!(
        printer,
        "workflow version {}\nentrypoint: {}\ndependencies: {}",
        styles.bold.apply_to(id),
        version.entrypoint(),
        version.workflow_dependencies().len()
    );

    let title_row: Vec<CellStruct> = vec![
        "FILE".cell().bold(use_color),
        "BYTES".cell().bold(use_color),
    ];
    let rows: Vec<Vec<CellStruct>> = version
        .files()
        .iter()
        .map(|(path, content)| {
            vec![
                path.as_str()
                    .cell()
                    .foreground_color(color_if(use_color, Color::Ansi256(8))),
                content.len().to_string().cell(),
            ]
        })
        .collect();

    let color_choice = if use_color {
        cli_table::ColorChoice::Auto
    } else {
        cli_table::ColorChoice::Never
    };
    let table = rows
        .table()
        .title(title_row)
        .color_choice(color_choice)
        .border(Border::builder().build())
        .separator(Separator::builder().build());
    fabro_util::printout!(
        printer,
        "{}",
        table
            .display()
            .context("rendering the workflow version table")?
    );
    Ok(())
}
