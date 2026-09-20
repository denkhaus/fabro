use anyhow::Result;
use cli_table::format::{Border, Separator};
use cli_table::{Cell, CellStruct, Style, Table};
use fabro_util::terminal::Styles;

use super::short_revision;
use crate::args::EnvShowArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

pub(super) async fn show_command(args: &EnvShowArgs, ctx: &CommandContext) -> Result<()> {
    let client = ctx.server().await?;
    let environment = client.retrieve_environment(&args.id).await?;
    if ctx.json_output() {
        print_json_pretty(&environment)?;
        return Ok(());
    }

    let printer = ctx.printer();
    let styles = Styles::detect_stdout();
    let use_color = styles.use_color;
    let settings = &environment.settings;

    let title: Vec<CellStruct> = vec![
        "FIELD".cell().bold(use_color),
        "VALUE".cell().bold(use_color),
    ];
    let rows: Vec<Vec<CellStruct>> = vec![
        vec![
            "id".cell().bold(use_color),
            environment.id.to_string().cell(),
        ],
        vec![
            "revision".cell().bold(use_color),
            environment.revision.to_string().cell(),
        ],
        vec![
            "provider".cell().bold(use_color),
            settings.provider.to_string().cell(),
        ],
        vec![
            "image".cell().bold(use_color),
            settings
                .image
                .docker
                .clone()
                .unwrap_or_else(|| "-".to_string())
                .cell(),
        ],
        vec![
            "dockerfile".cell().bold(use_color),
            match &settings.image.dockerfile {
                Some(_) => "inline".to_string().cell(),
                None => "-".to_string().cell(),
            },
        ],
        vec![
            "resources".cell().bold(use_color),
            format!(
                "cpu={} memory={} disk={}",
                settings
                    .resources
                    .cpu
                    .map_or_else(|| "-".into(), |cpu| cpu.to_string()),
                settings
                    .resources
                    .memory
                    .as_ref()
                    .map_or_else(|| "-".into(), ToString::to_string),
                settings
                    .resources
                    .disk
                    .as_ref()
                    .map_or_else(|| "-".into(), ToString::to_string),
            )
            .cell(),
        ],
        vec![
            "network".cell().bold(use_color),
            format!(
                "mode={} allow={}",
                settings.network.mode,
                settings.network.allow.join(",")
            )
            .cell(),
        ],
        vec![
            "lifecycle".cell().bold(use_color),
            format!(
                "preserve={} stop_on_terminal={} auto_stop={}",
                settings.lifecycle.preserve,
                settings.lifecycle.stop_on_terminal,
                settings
                    .lifecycle
                    .auto_stop
                    .map_or_else(|| "-".into(), |duration| duration.to_string()),
            )
            .cell(),
        ],
        vec![
            "labels".cell().bold(use_color),
            settings
                .labels
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join(",")
                .cell(),
        ],
        vec![
            "env".cell().bold(use_color),
            settings
                .env
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(",")
                .cell(),
        ],
    ];

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
    fabro_util::printerr!(
        printer,
        "revision {}",
        short_revision(environment.revision.as_str())
    );

    Ok(())
}
