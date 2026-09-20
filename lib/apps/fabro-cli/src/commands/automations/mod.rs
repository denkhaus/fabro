mod breaker;
mod fire;
mod list;
mod pause;
mod runs;
mod set_schedule;
mod show;
mod status;

use anyhow::{Context as _, Result};
use chrono::{DateTime, Utc};
use fabro_api::types;
use fabro_automation::{AutomationTrigger, ScheduleTrigger, parse_schedule_expression};
use fabro_client::api_failure_for;

use crate::args::{AutomationsBreakerCommand, AutomationsCommand, AutomationsNamespace};
use crate::command_context::CommandContext;
use crate::server_client::Client;

pub(crate) async fn dispatch(ns: AutomationsNamespace, base_ctx: &CommandContext) -> Result<()> {
    let ctx = base_ctx.with_target(&ns.target)?;
    match ns.command {
        AutomationsCommand::List(args) => list::list_command(&args, &ctx).await,
        AutomationsCommand::Show(args) => show::show_command(&args, &ctx).await,
        AutomationsCommand::Runs(args) => runs::runs_command(&args, &ctx).await,
        AutomationsCommand::SetSchedule(args) => {
            set_schedule::set_schedule_command(&args, &ctx).await
        }
        AutomationsCommand::Pause(args) => pause::pause_command(&args, &ctx, false).await,
        AutomationsCommand::Unpause(args) => pause::pause_command(&args, &ctx, true).await,
        AutomationsCommand::Breaker(ns) => match ns.command {
            AutomationsBreakerCommand::Reset(args) => breaker::reset_command(&args, &ctx).await,
        },
        AutomationsCommand::Fire(args) => fire::fire_command(&args, &ctx).await,
        AutomationsCommand::Status(args) => status::status_command(&args, &ctx).await,
    }
}

/// Outcome of an optimistic-concurrency replace attempt.
pub(super) enum ReplaceOutcome {
    /// The prepared request matched the stored automation; nothing was
    /// written.
    Unchanged(types::Automation),
    /// The automation was replaced; carries the revision observed before the
    /// write plus the updated definition with its new revision.
    Replaced {
        previous_revision: String,
        automation:        types::Automation,
    },
}

/// Converts a stored automation into a replace request body. Runtime status
/// (`last_error`) is not part of the replace contract, so it is dropped along
/// with the identity and revision fields.
pub(super) fn automation_to_replace_request(
    automation: &types::Automation,
) -> Result<types::ReplaceAutomationRequest> {
    let id = automation.id.to_string();
    let mut value =
        serde_json::to_value(automation).with_context(|| format!("serializing automation {id}"))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("automation {id} did not serialize to a JSON object"))?;
    object.remove("id");
    object.remove("revision");
    object.remove("last_error");
    serde_json::from_value(value)
        .with_context(|| format!("converting automation {id} to a replace request"))
}

pub(super) fn is_revision_conflict(err: &anyhow::Error) -> bool {
    api_failure_for(err).is_some_and(|failure| failure.status.as_u16() == 409)
}

/// One optimistic-concurrency replace attempt cycle: `prepare` inspects the
/// freshly-read automation and either reports no write needed (`Ok(None)`) or
/// produces the request body to persist.
pub(super) async fn replace_attempt(
    client: &Client,
    id: &str,
    prepare: &mut dyn FnMut(&types::Automation) -> Result<Option<types::ReplaceAutomationRequest>>,
) -> Result<ReplaceOutcome> {
    let current = client.retrieve_automation(id).await?;
    let expected_revision = current.revision.to_string();
    let Some(request) = prepare(&current)? else {
        return Ok(ReplaceOutcome::Unchanged(current));
    };
    match client
        .replace_automation(id, &expected_revision, request)
        .await
    {
        Ok(automation) => Ok(ReplaceOutcome::Replaced {
            previous_revision: expected_revision,
            automation,
        }),
        Err(err) if is_revision_conflict(&err) => Err(err.context(format!(
            "automation {id} revision mismatch: expected {expected_revision}, server is at a \
             newer revision"
        ))),
        Err(err) => Err(err.context(format!("updating automation {id}"))),
    }
}

/// GET-revision + PUT If-Match with a single retry on a revision race. The
/// caller's `prepare` runs against each freshly-read automation, so trigger
/// edits are re-derived from current state before the retry.
pub(super) async fn replace_with_retry(
    client: &Client,
    id: &str,
    mut prepare: impl FnMut(&types::Automation) -> Result<Option<types::ReplaceAutomationRequest>>,
) -> Result<ReplaceOutcome> {
    match replace_attempt(client, id, &mut prepare).await {
        Ok(outcome) => Ok(outcome),
        Err(first) if is_revision_conflict(&first) => {
            match replace_attempt(client, id, &mut prepare).await {
                Ok(outcome) => Ok(outcome),
                Err(second) => Err(second.context(format!(
                    "automation {id} replace retried once after a revision race and failed again"
                ))),
            }
        }
        Err(first) => Err(first),
    }
}

/// Renders a 64-hex revision as its 12-char prefix for tables and progress
/// lines.
pub(super) fn short_revision(revision: &str) -> &str {
    revision.get(..12).unwrap_or(revision)
}

/// Every schedule trigger of an automation, regardless of enabled state.
pub(super) fn schedule_triggers(
    automation: &types::Automation,
) -> impl Iterator<Item = &ScheduleTrigger> {
    automation
        .triggers
        .iter()
        .filter_map(|trigger| match trigger {
            AutomationTrigger::Schedule(schedule) => Some(schedule),
            AutomationTrigger::Api(_) => None,
        })
}

/// Next UTC fire time for a five-field cron expression after `after`.
/// Deterministic cron math only — no LLM, no server round-trip.
pub(super) fn next_fire_after(
    expression: &str,
    after: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>> {
    let cron = parse_schedule_expression(expression)
        .map_err(|err| anyhow::anyhow!("invalid cron expression {expression:?}: {err}"))?;
    Ok(cron.find_next_occurrence(&after, false).ok())
}

/// Nominal fire interval for a cron expression, derived from the gap between
/// the next two occurrences. Used by `automations status` to size the
/// fire-drift threshold (2x this interval).
pub(super) fn cron_interval(expression: &str, after: DateTime<Utc>) -> Result<Option<i64>> {
    let Some(first) = next_fire_after(expression, after)? else {
        return Ok(None);
    };
    let Some(second) = next_fire_after(expression, first + chrono::Duration::seconds(1))? else {
        return Ok(None);
    };
    Ok(Some((second - first).num_seconds()))
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::{cron_interval, next_fire_after};

    #[test]
    fn next_fire_after_computes_the_next_utc_occurrence() {
        let after = Utc.with_ymd_and_hms(2026, 9, 20, 12, 0, 30).unwrap();
        let next = next_fire_after("0 3 * * *", after)
            .unwrap()
            .expect("daily cron always has a next occurrence");
        assert_eq!(next.to_rfc3339(), "2026-09-21T03:00:00+00:00");
    }

    #[test]
    fn cron_interval_derives_the_schedule_period() {
        let after = Utc.with_ymd_and_hms(2026, 9, 20, 12, 0, 30).unwrap();
        assert_eq!(
            cron_interval("0 3 * * *", after).unwrap(),
            Some(24 * 60 * 60)
        );
        assert_eq!(cron_interval("*/30 * * * *", after).unwrap(), Some(30 * 60));
    }

    #[test]
    fn next_fire_after_rejects_invalid_expressions() {
        let after = Utc.with_ymd_and_hms(2026, 9, 20, 12, 0, 30).unwrap();
        let err = next_fire_after("not a cron", after).unwrap_err();
        assert!(err.to_string().contains("invalid cron expression"));
    }
}
