use anyhow::Result;
use cli_table::format::{Border, Separator};
use cli_table::{Cell, CellStruct, Style, Table};
use fabro_api::types::AutomationTrigger;
use fabro_util::terminal::Styles;

use super::schedule_triggers;
use crate::args::AutomationsListArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

pub(super) async fn list_command(_args: &AutomationsListArgs, ctx: &CommandContext) -> Result<()> {
    let client = ctx.server().await?;
    let printer = ctx.printer();
    let automations = client.list_automations().await?;
    if ctx.json_output() {
        print_json_pretty(&automations)?;
        return Ok(());
    }

    if automations.is_empty() {
        fabro_util::printerr!(printer, "No automations found.");
        return Ok(());
    }

    let styles = Styles::detect_stdout();
    let use_color = styles.use_color;

    let title: Vec<CellStruct> = vec![
        "ID".cell().bold(use_color),
        "NAME".cell().bold(use_color),
        "SCHEDULE".cell().bold(use_color),
        "ENABLED".cell().bold(use_color),
        "BREAKER".cell().bold(use_color),
        "WINDOW".cell().bold(use_color),
        "LAST ERROR".cell().bold(use_color),
    ];

    let rows: Vec<Vec<CellStruct>> = automations
        .iter()
        .map(|automation| {
            let schedules: Vec<String> = schedule_triggers(automation)
                .map(|schedule| {
                    format!(
                        "{} {}",
                        if schedule.enabled { "on" } else { "off" },
                        schedule.expression
                    )
                })
                .collect();
            let breaker = schedule_triggers(automation)
                .filter_map(|schedule| schedule.breaker.as_ref())
                .map(|breaker| breaker.consecutive_count.to_string())
                .max()
                .unwrap_or_else(|| "-".to_string());
            // The provider-window gate's hold, when it is holding this
            // schedule's fires (fabro-b869): one closed provider is enough.
            let window = schedule_triggers(automation)
                .filter_map(|schedule| schedule.provider_window.as_ref())
                .flat_map(|window| &window.providers)
                .filter(|fact| fact.window == fabro_automation::ProviderWindowKind::Closed)
                .map(|fact| fact.provider.clone())
                .collect::<Vec<_>>();
            let window = if window.is_empty() {
                "-".to_string()
            } else {
                window.join(",")
            };
            let enabled = if automation.triggers.iter().all(AutomationTrigger::enabled) {
                "all"
            } else if automation.triggers.iter().any(AutomationTrigger::enabled) {
                "partial"
            } else {
                "none"
            };
            vec![
                automation.id.to_string().cell().bold(use_color),
                automation.name.clone().cell(),
                schedules.join(" | ").cell(),
                enabled.cell(),
                breaker.cell(),
                window.cell(),
                automation
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "-".to_string())
                    .cell(),
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
        .title(title)
        .color_choice(color_choice)
        .border(Border::builder().build())
        .separator(Separator::builder().build());
    fabro_util::printout!(printer, "{}", table.display()?);

    Ok(())
}
