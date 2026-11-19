use std::path::Path;

use anyhow::Result;
use chrono::Utc;
use cli_table::format::{Border, Separator};
use cli_table::{Cell, CellStruct, Color, Style, Table};
use fabro_types::RunStatus;
use fabro_util::terminal::Styles;
use fabro_util::text::strip_goal_decoration;

use super::short_run_id;
use crate::args::RunsListArgs;
use crate::command_context::CommandContext;
use crate::commands::resolve_run_id;
use crate::server_client::UndecodableRunRow;
use crate::server_runs::{RunListScope, ServerRunLookup, filter_server_runs};
use crate::shared::{color_if, format_duration_ms, run_status_kind, tilde_path};

pub(crate) async fn list_command(
    args: &RunsListArgs,
    styles: &Styles,
    base_ctx: &CommandContext,
) -> Result<()> {
    let ctx = base_ctx.with_target(&args.server)?;
    let printer = ctx.printer();
    let client = ctx.server().await?;
    let parent_id = match args.parent.as_deref() {
        Some(selector) => Some(resolve_run_id(client.as_ref(), selector).await?),
        None => None,
    };
    let filtered_by_parent = parent_id.is_some();
    let lookup = match parent_id {
        Some(parent_id) => ServerRunLookup::from_client_by_parent(client, parent_id).await?,
        None => ServerRunLookup::from_client(client).await?,
    };
    let label_filters = parse_label_filters(&args.filter.label);
    // Tri-state scope: default shows active runs, -a shows everything
    // (archived included), --archived narrows to archived runs only.
    let scope = if args.archived {
        RunListScope::Archived
    } else if args.all {
        RunListScope::All
    } else {
        RunListScope::Active
    };
    let filtered = filter_server_runs(
        lookup.runs(),
        args.filter.before.as_deref(),
        args.filter.workflow.as_deref(),
        &label_filters,
        scope,
    );

    // work#1437: rows the server sent that this CLI cannot decode
    // render as placeholders instead of killing the whole listing.
    // Warn loudly, once, before any rendering — and point at the
    // self-update door, since the matching CLI ships with the server's
    // own release.
    if !lookup.undecodable_runs().is_empty() {
        fabro_util::printerr!(
            printer,
            "WARNING: {}",
            undecodable_skew_warning(lookup.undecodable_runs())
        );
        fabro_util::printerr!(printer, "{SELF_UPDATE_HINT}");
    }

    if ctx.json_output() {
        let mut json_rows: Vec<_> = filtered
            .iter()
            .map(|run| {
                serde_json::json!({
                    "run_id": run.run_id(),
                    "title": run.title(),
                    "parent_id": run.parent_id(),
                    "workflow_name": run.workflow_name(),
                    "workflow_graph_name": run.workflow_graph_name(),
                    "workflow_slug": run.workflow_slug(),
                    "status": run.status(),
                    "archived": run.archived(),
                    "start_time": run.start_time(),
                    "labels": run.labels(),
                    "wall_time_ms": run.wall_time_ms(),
                    "total_usd_micros": run.total_usd_micros(),
                    "source_directory": run.source_directory(),
                    "repo_origin_url": run.repo_origin_url(),
                    "goal": run.goal(),
                })
            })
            .collect();
        json_rows.extend(lookup.undecodable_runs().iter().map(|row| {
            serde_json::json!({
                "run_id": row.run_id(),
                "undecodable": true,
                "error": row.error,
                "raw": row.raw,
            })
        }));
        fabro_util::printout!(printer, "{}", serde_json::to_string_pretty(&json_rows)?);
        return Ok(());
    }

    if args.quiet {
        for run in &filtered {
            fabro_util::printout!(printer, "{}", run.run_id());
        }
        for row in lookup.undecodable_runs() {
            if let Some(run_id) = row.run_id() {
                fabro_util::printout!(printer, "{run_id}");
            }
        }
        return Ok(());
    }

    if filtered.is_empty() && lookup.undecodable_runs().is_empty() {
        if args.archived {
            fabro_util::printerr!(printer, "No archived runs found.");
        } else if args.all {
            fabro_util::printerr!(printer, "No runs found.");
        } else {
            fabro_util::printerr!(
                printer,
                "No running processes found. Use -a to show all runs (including archived)."
            );
        }
        return Ok(());
    }

    let mut display_runs = filtered;
    display_runs.reverse();
    let show_parent_column =
        !filtered_by_parent && display_runs.iter().any(|run| run.parent_id().is_some());

    let use_color = styles.use_color;
    let now = Utc::now();
    let mut title = vec!["RUN ID".cell().bold(use_color)];
    if show_parent_column {
        title.push("PARENT".cell().bold(use_color));
    }
    title.extend([
        "WORKFLOW".cell().bold(use_color),
        "STATUS".cell().bold(use_color),
        "DIRECTORY".cell().bold(use_color),
        "DURATION".cell().bold(use_color),
        "TITLE".cell().bold(use_color),
    ]);

    let rows: Vec<Vec<CellStruct>> = display_runs
        .iter()
        .map(|run| {
            let duration_display = match run.wall_time_ms() {
                Some(ms) => format_duration_ms(ms),
                None => match run.start_time_dt() {
                    Some(start) => {
                        let elapsed = now.signed_duration_since(start);
                        format_duration_ms(elapsed.num_milliseconds().max(0).cast_unsigned())
                    }
                    None => "-".to_string(),
                },
            };
            let dir_display = run
                .source_directory()
                .map_or_else(|| "-".to_string(), |p| tilde_path(Path::new(p)));
            let run_id = run.run_id().to_string();

            let mut row = vec![
                run_id
                    .clone()
                    .cell()
                    .foreground_color(color_if(use_color, Color::Ansi256(8))),
            ];
            if show_parent_column {
                let parent_display = run.parent_id().map_or_else(
                    || "-".to_string(),
                    |parent_id| short_run_id(&parent_id.to_string()).to_string(),
                );
                row.push(
                    parent_display
                        .cell()
                        .foreground_color(color_if(use_color, Color::Ansi256(8))),
                );
            }
            row.extend([
                run.workflow_display_name().cell(),
                status_cell(run.status(), run.archived(), use_color),
                dir_display.cell(),
                duration_display.cell(),
                truncate_goal(run.display_title(), 50)
                    .cell()
                    .foreground_color(color_if(use_color, Color::Ansi256(8))),
            ]);
            row
        })
        .chain(
            lookup
                .undecodable_runs()
                .iter()
                .map(|row| undecodable_row_cells(row, show_parent_column, use_color)),
        )
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

    fabro_util::printerr!(
        printer,
        "\n{} run(s) listed.",
        display_runs.len() + lookup.undecodable_runs().len()
    );
    Ok(())
}

/// The STATUS cell: the status bucket, with the archived overlay
/// surfaced as an `archived/<status>` marker so maintenance sweeps can
/// read archive state straight off the table.
fn status_cell(status: RunStatus, archived: bool, use_color: bool) -> CellStruct {
    let kind = run_status_kind(status);
    let text = if archived {
        format!("archived/{kind}")
    } else {
        kind.to_string()
    };
    let color = match status {
        RunStatus::Succeeded { .. } => Some(Color::Green),
        RunStatus::Failed { .. } => Some(Color::Red),
        RunStatus::Running | RunStatus::Starting | RunStatus::Runnable => Some(Color::Cyan),
        RunStatus::Submitted | RunStatus::Pending { .. } | RunStatus::Dead => {
            Some(Color::Ansi256(8))
        }
        RunStatus::Blocked { .. } | RunStatus::Removing | RunStatus::HumanNeeded => {
            Some(Color::Yellow)
        }
        RunStatus::Paused { .. } => Some(Color::Magenta),
    };
    text.cell()
        .bold(use_color && color != Some(Color::Ansi256(8)))
        .foreground_color(color_if(use_color, color.unwrap_or(Color::Ansi256(8))))
}

/// The one-line self-update pointer printed under the skew warning
/// (work#1437): the matching CLI ships inside the server's own release
/// image, so upgrading the CLI converges it with the server.
const SELF_UPDATE_HINT: &str = "upgrade the CLI: the release carries it — `fabro upgrade` (or the mise github backend installs it)";

/// The drift warning for rows this CLI cannot decode: names how many
/// rows drifted and what the server's unknown shape looks like (the
/// status kind when the raw row carries one), capped at five details.
fn undecodable_skew_warning(runs: &[UndecodableRunRow]) -> String {
    let details = runs
        .iter()
        .take(5)
        .map(|row| {
            row.status_kind().map_or_else(
                || "unknown shape".to_string(),
                |kind| format!("status_kind={kind}"),
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let more = runs.len().saturating_sub(5);
    let more_note = if more > 0 {
        format!(" ...and {more} more")
    } else {
        String::new()
    };
    format!(
        "{} run row(s) this CLI cannot decode — the server sent a field or variant this CLI \
         does not know ({details}{more_note}). They render as placeholder rows.",
        runs.len()
    )
}

/// The title a placeholder row carries: the task's suggested shape,
/// naming the unknown status kind when the raw row exposes one.
fn undecodable_title(row: &UndecodableRunRow) -> String {
    match row.status_kind() {
        Some(kind) => format!("<undecodable run: status_kind={kind}>"),
        None => "<undecodable run>".to_string(),
    }
}

/// One table row for a run this CLI cannot decode: the run id and
/// status kind off the raw JSON where present, dashes elsewhere, and
/// the `<undecodable run>` placeholder as the title.
fn undecodable_row_cells(
    row: &UndecodableRunRow,
    show_parent_column: bool,
    use_color: bool,
) -> Vec<CellStruct> {
    let gray = color_if(use_color, Color::Ansi256(8));
    let mut cells = vec![row.run_id().unwrap_or("-").cell().foreground_color(gray)];
    if show_parent_column {
        cells.push("-".cell().foreground_color(gray));
    }
    cells.extend([
        "-".cell(),
        row.status_kind()
            .unwrap_or("undecodable")
            .cell()
            .bold(use_color)
            .foreground_color(color_if(use_color, Color::Yellow)),
        "-".cell(),
        "-".cell(),
        truncate_goal(&undecodable_title(row), 50)
            .cell()
            .foreground_color(gray),
    ]);
    cells
}

fn parse_label_filters(label_args: &[String]) -> Vec<(String, String)> {
    label_args
        .iter()
        .filter_map(|s| s.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn truncate_goal(goal: &str, max_len: usize) -> String {
    truncate_str(strip_goal_decoration(goal), max_len)
}

fn truncate_str(s: &str, max_len: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_len {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_len - 3).collect();
    format!("{truncated}...")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_goal_strips_markdown_headings() {
        assert_eq!(truncate_goal("## Fix bug", 50), "Fix bug");
        assert_eq!(truncate_goal("# Title", 50), "Title");
        assert_eq!(truncate_goal("### Deep heading", 50), "Deep heading");
    }

    #[test]
    fn truncate_goal_strips_plan_prefix() {
        assert_eq!(truncate_goal("Plan: do stuff", 50), "do stuff");
    }

    #[test]
    fn truncate_goal_strips_heading_and_plan_prefix() {
        assert_eq!(truncate_goal("## Plan: migrate DB", 50), "migrate DB");
    }

    #[test]
    fn truncate_goal_plain_text_unchanged() {
        assert_eq!(truncate_goal("Fix the login bug", 50), "Fix the login bug");
    }

    #[test]
    fn truncate_goal_still_truncates_after_stripping() {
        assert_eq!(
            truncate_goal("## A long goal description", 10),
            "A long ..."
        );
    }

    fn undecodable_row(raw: serde_json::Value) -> UndecodableRunRow {
        UndecodableRunRow {
            raw,
            error: "unknown variant `human_needed`".to_string(),
        }
    }

    #[test]
    fn undecodable_title_carries_the_status_kind() {
        let row = undecodable_row(serde_json::json!({
            "id": "01JNQVR7M0EJ5GKAT2SC4ERS1Z",
            "lifecycle": {"status": {"kind": "human_needed"}}
        }));
        assert_eq!(
            undecodable_title(&row),
            "<undecodable run: status_kind=human_needed>"
        );
        assert_eq!(
            undecodable_title(&undecodable_row(serde_json::json!({}))),
            "<undecodable run>"
        );
    }

    #[test]
    fn skew_warning_names_the_drift_and_caps_details() {
        let rows: Vec<_> = (0..7)
            .map(|_| {
                undecodable_row(serde_json::json!({
                    "lifecycle": {"status": {"kind": "human_needed"}}
                }))
            })
            .collect();
        let warning = undecodable_skew_warning(&rows);
        assert!(warning.contains("7 run row(s)"), "{warning}");
        assert!(
            warning.contains("a field or variant this CLI does not know"),
            "{warning}"
        );
        assert!(warning.contains("status_kind=human_needed"), "{warning}");
        assert!(warning.contains("...and 2 more"), "{warning}");

        let single = undecodable_skew_warning(&rows[..1]);
        assert!(!single.contains("more"), "{single}");
    }
}
