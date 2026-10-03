use anyhow::Result;
use chrono::Utc;
use cli_table::format::{Border, Separator};
use cli_table::{Cell, CellStruct, Style, Table};
use fabro_api::types;
use fabro_automation::AutomationTrigger;
use fabro_util::terminal::Styles;

use super::{next_fire_after, short_revision};
use crate::args::AutomationsShowArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

pub(super) async fn show_command(args: &AutomationsShowArgs, ctx: &CommandContext) -> Result<()> {
    let client = ctx.server().await?;
    let automation = client.retrieve_automation(&args.id).await?;
    if ctx.json_output() {
        print_json_pretty(&automation)?;
        return Ok(());
    }

    let printer = ctx.printer();
    let styles = Styles::detect_stdout();
    let use_color = styles.use_color;

    let title: Vec<CellStruct> = vec![
        "FIELD".cell().bold(use_color),
        "VALUE".cell().bold(use_color),
    ];
    let rows: Vec<Vec<CellStruct>> = trigger_rows(&automation);
    let color_choice = if use_color {
        cli_table::ColorChoice::Auto
    } else {
        cli_table::ColorChoice::Never
    };
    let table = rows
        .table()
        .title(title)
        .color_choice(color_choice)
        .border(Border::builder().build())
        .separator(Separator::builder().build());
    fabro_util::printout!(printer, "{}", table.display()?);

    Ok(())
}

/// Field/value rows for one automation, including per-trigger schedule
/// detail (enabled, expression, computed next fire time, breaker facts) and
/// last-run linkage via the breaker's `last_run_id`.
fn trigger_rows(automation: &types::Automation) -> Vec<Vec<CellStruct>> {
    let styles = Styles::detect_stdout();
    let use_color = styles.use_color;
    let now = Utc::now();

    let mut rows: Vec<Vec<CellStruct>> = vec![
        field_row(use_color, "id", automation.id.to_string()),
        field_row(
            use_color,
            "revision",
            short_revision(automation.revision.as_str()).to_string(),
        ),
        field_row(use_color, "name", automation.name.clone()),
        field_row(
            use_color,
            "description",
            automation
                .description
                .clone()
                .unwrap_or_else(|| "-".to_string()),
        ),
        field_row(
            use_color,
            "environment",
            automation
                .environment_id
                .clone()
                .unwrap_or_else(|| "-".to_string()),
        ),
        field_row(use_color, "workflow", automation.workflow.clone()),
        field_row(
            use_color,
            "on_overlap",
            automation
                .on_overlap
                .map_or_else(|| "skip (default)".to_string(), |policy| policy.to_string()),
        ),
        field_row(
            use_color,
            "last_error",
            automation
                .last_error
                .clone()
                .unwrap_or_else(|| "-".to_string()),
        ),
    ];

    for trigger in &automation.triggers {
        match trigger {
            AutomationTrigger::Api(api) => {
                rows.push(field_row(
                    use_color,
                    &format!("trigger {}", api.id),
                    format!("type=api enabled={}", api.enabled),
                ));
            }
            AutomationTrigger::Schedule(schedule) => {
                let next_fire = if schedule.enabled {
                    next_fire_after(&schedule.expression, now)
                        .ok()
                        .flatten()
                        .map_or_else(|| "unknown".to_string(), |fire| fire.to_rfc3339())
                } else {
                    "-".to_string()
                };
                rows.push(field_row(
                    use_color,
                    &format!("trigger {}", schedule.id),
                    format!(
                        "type=schedule enabled={} expression={}",
                        schedule.enabled, schedule.expression
                    ),
                ));
                rows.push(field_row(use_color, "  next fire", next_fire));
                match &schedule.breaker {
                    Some(breaker) => {
                        rows.push(field_row(
                            use_color,
                            "  breaker",
                            format!(
                                "count={} signature={}",
                                breaker.consecutive_count, breaker.signature
                            ),
                        ));
                        if let Some(paused_at) = breaker.paused_at {
                            rows.push(field_row(
                                use_color,
                                "  breaker paused at",
                                paused_at.to_rfc3339(),
                            ));
                        }
                        rows.push(field_row(
                            use_color,
                            "  breaker last run",
                            breaker.last_run_id.clone(),
                        ));
                    }
                    None => {
                        rows.push(field_row(use_color, "  breaker", "clean".to_string()));
                    }
                }
            }
        }
    }

    rows
}

fn field_row(use_color: bool, field: &str, value: String) -> Vec<CellStruct> {
    vec![field.to_string().cell().bold(use_color), value.cell()]
}
