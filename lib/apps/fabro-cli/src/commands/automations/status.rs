use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use cli_table::format::{Border, Separator};
use cli_table::{Cell, CellStruct, Style, Table};
use fabro_api::types;
use fabro_types::{Run, RunStatus};
use fabro_util::terminal::Styles;
use tokio::time::sleep;

use super::{cron_interval, schedule_triggers};
use crate::args::AutomationsStatusArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

/// Re-check interval for `--watch`.
const WATCH_INTERVAL_SECS: u64 = 30;

/// Fire-drift multiplier: an enabled schedule with no successful fire within
/// 2x its cron interval has drifted.
const DRIFT_INTERVALS: i64 = 2;

pub(super) async fn status_command(
    args: &AutomationsStatusArgs,
    ctx: &CommandContext,
) -> Result<()> {
    loop {
        let drift = status_snapshot(ctx).await?;
        if ctx.json_output() {
            let rows: Vec<serde_json::Value> = drift
                .iter()
                .map(|row| {
                    serde_json::json!({
                        "id": row.id,
                        "schedule": row.schedule,
                        "enabled": row.enabled,
                        "last_success": row.last_success,
                        "drift": row.flags,
                    })
                })
                .collect();
            print_json_pretty(&rows)?;
        } else {
            render_table(ctx, &drift)?;
        }

        if !args.watch {
            return Ok(());
        }
        fabro_util::printerr!(
            ctx.printer(),
            "Watching for fire drift; re-checking in {WATCH_INTERVAL_SECS}s (Ctrl-C to exit)."
        );
        sleep(Duration::from_secs(WATCH_INTERVAL_SECS)).await;
    }
}

/// One automation's drift summary row.
struct StatusRow {
    id:           String,
    schedule:     String,
    enabled:      bool,
    last_success: Option<DateTime<Utc>>,
    flags:        Vec<String>,
}

async fn status_snapshot(ctx: &CommandContext) -> Result<Vec<StatusRow>> {
    let client = ctx.server().await?;
    let automations = client.list_automations().await?;
    let now = Utc::now();
    let mut rows = Vec::new();
    for automation in &automations {
        let schedules: Vec<_> = schedule_triggers(automation).collect();
        let schedule = schedules
            .iter()
            .map(|schedule| schedule.expression.clone())
            .collect::<Vec<_>>()
            .join(" | ");
        let enabled = schedules.iter().any(|schedule| schedule.enabled);

        let runs = if enabled {
            client.list_automation_runs(automation.id.as_str()).await?
        } else {
            Vec::new()
        };

        rows.push(StatusRow {
            id: automation.id.to_string(),
            schedule,
            enabled,
            last_success: last_success(&runs),
            flags: drift_flags(automation, &runs, now),
        });
    }
    Ok(rows)
}

/// Deterministic fire-drift rule (no LLM): an enabled schedule with no
/// successful fire within 2x its cron interval has drifted; a non-zero
/// breaker count or a non-null `last_error` flags drift too.
fn drift_flags(automation: &types::Automation, runs: &[Run], now: DateTime<Utc>) -> Vec<String> {
    let mut flags = Vec::new();

    for schedule in schedule_triggers(automation).filter(|schedule| schedule.enabled) {
        if let Ok(Some(interval)) = cron_interval(&schedule.expression, now) {
            let threshold = interval * DRIFT_INTERVALS;
            let last_success = last_success(runs);
            let stalled = match last_success {
                Some(success) => (now - success).num_seconds() > threshold,
                None => true,
            };
            if stalled {
                flags.push(format!(
                    "fire drift: schedule {} enabled but no successful fire within {}s (last: \
                     {})",
                    schedule.expression,
                    threshold,
                    last_success
                        .map_or_else(|| "never".to_string(), |success| success.to_rfc3339())
                ));
            }
        }
    }

    for schedule in schedule_triggers(automation).filter_map(|schedule| {
        schedule
            .breaker
            .as_ref()
            .filter(|breaker| breaker.consecutive_count > 0)
    }) {
        flags.push(format!(
            "breaker count {} (signature {})",
            schedule.consecutive_count, schedule.signature
        ));
    }

    if let Some(last_error) = &automation.last_error {
        flags.push(format!("last_error: {last_error}"));
    }

    flags
}

/// Creation time of the most recent succeeded run, if any.
fn last_success(runs: &[Run]) -> Option<DateTime<Utc>> {
    runs.iter()
        .filter(|run| matches!(run.lifecycle.status, RunStatus::Succeeded { .. }))
        .map(|run| run.timestamps.created_at)
        .max()
}

fn render_table(ctx: &CommandContext, rows: &[StatusRow]) -> Result<()> {
    let printer = ctx.printer();
    if rows.is_empty() {
        fabro_util::printerr!(printer, "No automations found.");
        return Ok(());
    }

    let styles = Styles::detect_stdout();
    let use_color = styles.use_color;

    let title: Vec<CellStruct> = vec![
        "ID".cell().bold(use_color),
        "SCHEDULE".cell().bold(use_color),
        "ENABLED".cell().bold(use_color),
        "LAST SUCCESS".cell().bold(use_color),
        "STATUS".cell().bold(use_color),
    ];

    let table_rows: Vec<Vec<CellStruct>> = rows
        .iter()
        .map(|row| {
            vec![
                row.id.clone().cell().bold(use_color),
                row.schedule.clone().cell(),
                row.enabled.to_string().cell(),
                row.last_success
                    .map_or_else(|| "never".to_string(), |success| success.to_rfc3339())
                    .cell(),
                if row.flags.is_empty() {
                    "ok".to_string().cell()
                } else {
                    row.flags.join("; ").cell()
                },
            ]
        })
        .collect();

    let color_choice = if use_color {
        cli_table::ColorChoice::Auto
    } else {
        cli_table::ColorChoice::Never
    };
    let table = table_rows
        .table()
        .title(title)
        .color_choice(color_choice)
        .border(Border::builder().build())
        .separator(Separator::builder().build());
    fabro_util::printout!(printer, "{}", table.display()?);

    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use fabro_api::types::Automation;

    use super::{drift_flags, last_success};

    fn automation_json_triggers(last_error: Option<&str>) -> Automation {
        serde_json::from_value(serde_json::json!({
            "id": "nightly-deps",
            "revision": "1111111111111111111111111111111111111111111111111111111111111111",
            "name": "Nightly",
            "description": null,
            "environment_id": "toolchain",
            "last_error": last_error,
            "target": { "kind": "git", "repo": "fabro-sh/fabro", "branch": "main" },
            "workflow": "dependency-update",
            "triggers": [
                { "id": "manual", "type": "api", "enabled": true },
                {
                    "id": "nightly",
                    "type": "schedule",
                    "enabled": true,
                    "expression": "0 3 * * *",
                    "breaker": {
                        "signature": "park|dependency-update|boundary",
                        "consecutive_count": 2,
                        "last_run_id": "01M2ZSYQR4ZM8N6HD67TT3AT3X",
                        "paused_at": null
                    }
                }
            ]
        }))
        .unwrap()
    }

    fn run_json(created_at: &str, status: &serde_json::Value) -> fabro_types::Run {
        serde_json::from_value(serde_json::json!({
            "id": "01M2ZSYQR4ZM8N6HD67TT3AT3X",
            "title": "t",
            "goal": "g",
            "workflow": { "slug": "dependency-update" },
            "created_by": {
                "kind": "user",
                "identity": { "issuer": "https://github.com", "subject": "123" },
                "login": "octocat",
                "auth_method": "github"
            },
            "origin": { "kind": "api" },
            "labels": {},
            "lifecycle": { "status": status, "error": null, "archived": false },
            "models": [],
            "timestamps": { "created_at": created_at },
            "links": { "web": null }
        }))
        .unwrap()
    }

    #[test]
    fn drift_flags_combine_breaker_last_error_and_stale_success() {
        let now = Utc.with_ymd_and_hms(2026, 9, 20, 12, 0, 0).unwrap();
        let automation = automation_json_triggers(Some("scheduler failed"));
        let runs = vec![run_json(
            "2026-09-10T03:00:00Z",
            &serde_json::json!({ "kind": "succeeded", "reason": "completed" }),
        )];

        let flags = drift_flags(&automation, &runs, now);

        assert_eq!(flags.len(), 3);
        assert!(flags[0].contains("fire drift"));
        assert!(flags[1].contains("breaker count 2"));
        assert!(flags[2].contains("last_error: scheduler failed"));
    }

    #[test]
    fn drift_flags_clean_when_recent_success_and_no_breaker_or_error() {
        let now = Utc.with_ymd_and_hms(2026, 9, 20, 12, 0, 0).unwrap();
        let automation = serde_json::from_value::<Automation>(serde_json::json!({
            "id": "nightly-deps",
            "revision": "1111111111111111111111111111111111111111111111111111111111111111",
            "name": "Nightly",
            "description": null,
            "environment_id": "toolchain",
            "last_error": null,
            "target": { "kind": "git", "repo": "fabro-sh/fabro", "branch": "main" },
            "workflow": "dependency-update",
            "triggers": [
                { "id": "manual", "type": "api", "enabled": true },
                { "id": "nightly", "type": "schedule", "enabled": true, "expression": "0 3 * * *" }
            ]
        }))
        .unwrap();
        let runs = vec![run_json(
            "2026-09-20T03:00:00Z",
            &serde_json::json!({ "kind": "succeeded", "reason": "completed" }),
        )];

        assert!(drift_flags(&automation, &runs, now).is_empty());
    }

    #[test]
    fn last_success_ignores_non_succeeded_runs() {
        let runs = vec![
            run_json(
                "2026-09-20T04:00:00Z",
                &serde_json::json!({ "kind": "failed", "reason": "workflow_error" }),
            ),
            run_json(
                "2026-09-20T03:00:00Z",
                &serde_json::json!({ "kind": "succeeded", "reason": "completed" }),
            ),
        ];
        assert_eq!(
            last_success(&runs).unwrap().to_rfc3339(),
            "2026-09-20T03:00:00+00:00"
        );
    }
}
