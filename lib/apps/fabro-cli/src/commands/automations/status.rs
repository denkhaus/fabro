use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use cli_table::format::{Border, Separator};
use cli_table::{Cell, CellStruct, Style, Table};
use fabro_api::types;
use fabro_automation::{ProviderWindowKind, ScheduleTrigger};
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
        // The wedge alarm rides stderr in BOTH modes so the JSON stream
        // stays parseable while an exterior probe still sees the line.
        if let Some(warning) = wedge_warning(&drift) {
            fabro_util::printerr!(ctx.printer(), "{warning}");
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
    /// Structured drift signal, derived ONCE beside the flag rendering —
    /// consumers never re-grep the prose (a reword would silently disarm
    /// them).
    fire_drift:   bool,
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

        let flags = drift_flags(automation, &runs, now);
        // The single derivation site: the structured signal is taken from
        // the same flags the table renders.
        let fire_drift = flags.iter().any(|flag| flag.starts_with("fire drift"));
        rows.push(StatusRow {
            id: automation.id.to_string(),
            schedule,
            enabled,
            last_success: last_success(&runs),
            flags,
            fire_drift,
        });
    }
    Ok(rows)
}

/// Deterministic fire-drift rule (no LLM): an enabled schedule with no
/// successful fire within 2x its cron interval has drifted; a non-zero
/// breaker count or a non-null `last_error` flags drift too. A schedule
/// the provider-window gate is holding gets its own hold state, distinct
/// from drift (denkhaus/work#1411).
fn drift_flags(automation: &types::Automation, runs: &[Run], now: DateTime<Utc>) -> Vec<String> {
    let mut flags = Vec::new();

    for schedule in schedule_triggers(automation).filter(|schedule| schedule.enabled) {
        // Provider-window hold first (denkhaus/work#1411): a closed window
        // holds fires with no run, no error, and no drift signal — the
        // quiet has a known cause, so it is surfaced as its own state and
        // never read as (or counted toward) scheduler drift.
        if let Some(held) = closed_provider_window_flag(schedule) {
            flags.push(held);
            continue;
        }
        if let Ok(Some(interval)) = cron_interval(&schedule.expression, now) {
            let threshold = interval * DRIFT_INTERVALS;
            let last_success = last_success(runs);
            let stalled = match last_success {
                Some(success) => (now - success).num_seconds() > threshold,
                None => true,
            };
            if stalled {
                flags.push(stalled_fire_flag(
                    schedule,
                    runs,
                    now,
                    threshold,
                    last_success,
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

/// The provider-window hold state (denkhaus/work#1411): a closed window
/// holds the trigger's scheduled fires — no run is created, no
/// `last_error` is recorded, and the API surface stays fully "armed". The
/// gate's per-trigger read model names the closed providers, so the quiet
/// is surfaced as an explicit hold, deliberately NOT prefixed `fire drift`
/// (a held line never counts toward the scheduler-wedge signature).
fn closed_provider_window_flag(schedule: &ScheduleTrigger) -> Option<String> {
    let closed: Vec<_> = schedule
        .provider_window
        .as_ref()?
        .providers
        .iter()
        .filter(|fact| fact.window == ProviderWindowKind::Closed)
        .collect();
    if closed.is_empty() {
        return None;
    }
    let providers = closed
        .iter()
        .map(|fact| fact.provider.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let next_probe = closed
        .iter()
        .map(|fact| fact.next_probe_at)
        .min()
        .map_or_else(|| "unknown".to_string(), |at| at.to_rfc3339());
    Some(format!(
        "provider window closed: schedule {} fires held (never attempted) by {providers}; next \
         probe {next_probe}",
        schedule.expression,
    ))
}

/// One drifted schedule's quiet, classified (denkhaus/work#1411): a run
/// inside the drift window that did not succeed is a fire ATTEMPTED and
/// failed; no run in the window means no fire was ever attempted (a held
/// or silently skipped schedule).
fn stalled_fire_flag(
    schedule: &ScheduleTrigger,
    runs: &[Run],
    now: DateTime<Utc>,
    threshold_seconds: i64,
    last_success: Option<DateTime<Utc>>,
) -> String {
    let window_start = now - chrono::Duration::seconds(threshold_seconds);
    let newest_attempt = runs
        .iter()
        .filter(|run| run.timestamps.created_at >= window_start)
        .max_by_key(|run| run.timestamps.created_at);
    if let Some(run) = newest_attempt {
        return format!(
            "fire drift: schedule {} enabled but fire attempted and failed (last run {} at {})",
            schedule.expression,
            run.lifecycle.status.kind(),
            run.timestamps.created_at.to_rfc3339(),
        );
    }
    format!(
        "fire drift: schedule {} enabled but no fire attempted within {threshold_seconds}s (last \
         success {})",
        schedule.expression,
        last_success.map_or_else(|| "never".to_string(), |success| success.to_rfc3339()),
    )
}

/// Creation time of the most recent succeeded run, if any.
fn last_success(runs: &[Run]) -> Option<DateTime<Utc>> {
    runs.iter()
        .filter(|run| matches!(run.lifecycle.status, RunStatus::Succeeded { .. }))
        .map(|run| run.timestamps.created_at)
        .max()
}

/// The scheduler-wedge signature (denkhaus/work#1143): two or more
/// enabled automations drifting together is the scheduler itself, not
/// the schedules. A row WITHOUT drift is evidence against the wedge
/// only when it fired AFTER the newest drifting fire (the scheduler
/// served it post-silence); a flagless row whose last success is older
/// is simply not due yet (a fresh row, a long cadence) — it neither
/// confirms nor denies. Per-row drift stays in the table; this line is
/// the alarm an exterior probe reads.
fn wedge_warning(rows: &[StatusRow]) -> Option<String> {
    /// Below this count, simultaneous drift is not a scheduler signature.
    const MIN_WEDGE_ROWS: usize = 2;

    let enabled: Vec<&StatusRow> = rows.iter().filter(|row| row.enabled).collect();
    let drifting: Vec<&StatusRow> = enabled
        .iter()
        .copied()
        .filter(|row| row.fire_drift)
        .collect();
    if drifting.len() < MIN_WEDGE_ROWS {
        return None;
    }
    let newest_drift = drifting.iter().filter_map(|row| row.last_success).max();
    let counterexample = enabled.iter().any(|row| {
        !row.fire_drift
            && row
                .last_success
                .is_some_and(|success| Some(success) > newest_drift)
    });
    if counterexample {
        return None;
    }
    Some(format!(
        "ALL {} enabled automations fire-drift together — the scheduler itself may \
         be wedged (a restart re-arms it; denkhaus/work#1143)",
        drifting.len()
    ))
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

    use super::{StatusRow, drift_flags, last_success, wedge_warning};

    fn row(id: &str, enabled: bool, flags: &[&str]) -> StatusRow {
        StatusRow {
            id: id.to_string(),
            schedule: "*/15 * * * *".to_string(),
            enabled,
            last_success: None,
            flags: flags.iter().map(|flag| (*flag).to_string()).collect(),
            fire_drift: flags.iter().any(|flag| (*flag).starts_with("fire drift")),
        }
    }

    fn row_at(id: &str, enabled: bool, flags: &[&str], last_success: Option<&str>) -> StatusRow {
        let mut base = row(id, enabled, flags);
        base.last_success = last_success.map(|at| {
            chrono::DateTime::parse_from_rfc3339(at)
                .expect("test datetime should parse")
                .with_timezone(&Utc)
        });
        base
    }

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
        assert!(
            flags[0].contains("no fire attempted"),
            "a drift with no run inside the window is a never-attempted quiet: {flags:?}"
        );
        assert!(flags[1].contains("breaker count 2"));
        assert!(flags[2].contains("last_error: scheduler failed"));
    }

    /// denkhaus/work#1411: the drift message distinguishes a fire that was
    /// attempted and failed from one that was never attempted.
    #[test]
    fn drift_flags_distinguish_attempted_and_failed_from_never_attempted() {
        let now = Utc.with_ymd_and_hms(2026, 9, 20, 12, 0, 0).unwrap();
        let automation = automation_json_triggers(None);
        // Newest run inside the 2-day drift window failed; the last success
        // predates the window — the schedule DID attempt and failed.
        let runs = vec![
            run_json(
                "2026-09-20T11:00:00Z",
                &serde_json::json!({ "kind": "failed", "reason": "workflow_error" }),
            ),
            run_json(
                "2026-09-10T03:00:00Z",
                &serde_json::json!({ "kind": "succeeded", "reason": "completed" }),
            ),
        ];

        let flags = drift_flags(&automation, &runs, now);

        let drift = flags
            .iter()
            .find(|flag| flag.starts_with("fire drift"))
            .expect("the attempted-and-failed quiet is still drift: {flags:?}");
        assert!(
            drift.contains("fire attempted and failed (last run failed at 2026-09-20T11:00:00"),
            "an in-window non-succeeded run is an attempted fire: {flags:?}"
        );
    }

    /// denkhaus/work#1411: a closed provider window is an explicit hold
    /// state, distinct from drift — fires were never attempted, and the
    /// flag must not carry the `fire drift` prefix (held lines never count
    /// toward the scheduler-wedge signature).
    #[test]
    fn closed_provider_window_is_a_hold_state_not_drift() {
        let now = Utc.with_ymd_and_hms(2026, 11, 17, 16, 25, 0).unwrap();
        let automation = serde_json::from_value::<Automation>(serde_json::json!({
            "id": "adjudicator",
            "revision": "1111111111111111111111111111111111111111111111111111111111111111",
            "name": "Adjudicator",
            "description": null,
            "environment_id": "toolchain",
            "last_error": null,
            "target": { "kind": "git", "repo": "fabro-sh/fabro", "branch": "main" },
            "workflow": "adjudicate",
            "triggers": [
                { "id": "manual", "type": "api", "enabled": true },
                {
                    "id": "schedule",
                    "type": "schedule",
                    "enabled": true,
                    "expression": "*/15 * * * *",
                    "provider_window": {
                        "providers": [
                            {
                                "provider": "zai",
                                "window": "closed",
                                "last_probe_at": "2026-11-17T16:20:00Z",
                                "next_probe_at": "2026-11-17T16:30:00Z"
                            },
                            {
                                "provider": "openai",
                                "window": "open",
                                "last_probe_at": "2026-11-17T16:20:00Z",
                                "next_probe_at": "2026-11-17T16:30:00Z"
                            }
                        ]
                    }
                }
            ]
        }))
        .unwrap();
        // Last success long before the hold: the schedule is quiet, but the
        // closed window — not the scheduler — explains the quiet.
        let runs = vec![run_json(
            "2026-11-17T13:00:00Z",
            &serde_json::json!({ "kind": "succeeded", "reason": "completed" }),
        )];

        let flags = drift_flags(&automation, &runs, now);

        assert_eq!(flags.len(), 1);
        assert!(
            flags[0].starts_with("provider window closed: schedule */15 * * * * fires held"),
            "the hold names the schedule and the state: {flags:?}"
        );
        assert!(
            flags[0].contains("by zai"),
            "only closed providers are named: {flags:?}"
        );
        assert!(
            flags[0].contains("never attempted"),
            "the hold says fires were never attempted: {flags:?}"
        );
        assert!(
            flags[0].contains("next probe 2026-11-17T16:30:00"),
            "the hold carries the next probe time: {flags:?}"
        );
        assert!(
            !flags[0].starts_with("fire drift"),
            "a hold is not drift: {flags:?}"
        );
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

    #[test]
    fn wedge_warning_fires_only_when_every_enabled_row_drifts() {
        // denkhaus/work#1143: the fork.30 wedge read as four independent
        // gear drifts; four schedules missing their ticks at once is the
        // scheduler's signature, and the view must say so.
        let all_drift = vec![
            row("adjudicator-fabro", true, &["fire drift: schedule */15"]),
            row("develop-fabro", true, &["fire drift: schedule 20,50"]),
            row("merger-fabro", true, &["fire drift: schedule 10,40"]),
            row("watchdog-fabro", true, &["fire drift: schedule 0"]),
        ];
        let warning = wedge_warning(&all_drift).expect("all-drift is the wedge signature");
        assert!(
            warning.to_lowercase().contains("scheduler"),
            "the warning names the scheduler: {warning}"
        );

        let one_healthy = vec![
            row_at(
                "adjudicator-fabro",
                true,
                &["fire drift: schedule */15"],
                Some("2026-11-06T23:15:00Z"),
            ),
            row_at("develop-fabro", true, &[], Some("2026-11-06T23:45:00Z")),
            row_at(
                "merger-fabro",
                true,
                &["fire drift: schedule 10,40"],
                Some("2026-11-06T23:10:00Z"),
            ),
        ];
        assert!(
            wedge_warning(&one_healthy).is_none(),
            "a flagless row that fired after the newest drift is a counterexample — no wedge"
        );

        let lone_drift = vec![row("nightly", true, &["fire drift: schedule 0 3"])];
        assert!(
            wedge_warning(&lone_drift).is_none(),
            "a single drifting schedule is its own problem, not the wedge"
        );

        let disabled_drift = vec![
            row("a", false, &["fire drift: schedule */15"]),
            row("b", true, &[]),
        ];
        assert!(
            wedge_warning(&disabled_drift).is_none(),
            "disabled schedules never count toward the signature"
        );

        // A flagless row that fired AFTER the newest drifting fire is a
        // counterexample: the scheduler served it post-silence — no wedge.
        let post_silence_fire = vec![
            row_at(
                "hourly",
                true,
                &["fire drift: schedule 0 * * * *"],
                Some("2026-11-06T23:00:00Z"),
            ),
            row_at(
                "quarterly",
                true,
                &["fire drift: schedule */15 * * * *"],
                Some("2026-11-06T23:15:00Z"),
            ),
            row_at("fresh", true, &[], Some("2026-11-06T23:45:00Z")),
        ];
        assert!(
            wedge_warning(&post_silence_fire).is_none(),
            "a flagless row that fired after the drift began disproves the wedge"
        );

        // A flagless row with an OLDER last success is simply not due yet
        // (a long cadence) — it neither confirms nor denies: the wedge
        // alarm still fires.
        let long_cadence_neutral = vec![
            row_at(
                "hourly",
                true,
                &["fire drift: schedule 0 * * * *"],
                Some("2026-11-06T23:00:00Z"),
            ),
            row_at(
                "quarterly",
                true,
                &["fire drift: schedule */15 * * * *"],
                Some("2026-11-06T23:15:00Z"),
            ),
            row_at("daily", true, &[], Some("2026-11-06T03:00:00Z")),
        ];
        assert!(
            wedge_warning(&long_cadence_neutral).is_some(),
            "a not-yet-due row does not suppress the wedge signature"
        );
    }
}
