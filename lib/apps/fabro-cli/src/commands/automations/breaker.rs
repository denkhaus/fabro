use anyhow::{Result, anyhow};
use fabro_api::types::AutomationTrigger;

use super::{ReplaceOutcome, automation_to_replace_request, replace_with_retry, schedule_triggers};
use crate::args::AutomationsBreakerResetArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

/// Explicit breaker unlatch: re-enables the schedule trigger, which the
/// server documents as clearing the breaker facts and resetting the
/// counter. A clean breaker (no facts, or count 0 and not paused) is a
/// no-op success.
pub(super) async fn reset_command(
    args: &AutomationsBreakerResetArgs,
    ctx: &CommandContext,
) -> Result<()> {
    let client = ctx.server().await?;
    let printer = ctx.printer();

    let outcome = replace_with_retry(&client, &args.id, |current| {
        if schedule_triggers(current).next().is_none() {
            return Err(anyhow!(
                "automation {} has no schedule trigger, so it has no breaker",
                current.id
            ));
        }
        let tripped = schedule_triggers(current).any(|schedule| {
            schedule
                .breaker
                .as_ref()
                .is_some_and(|breaker| breaker.paused_at.is_some() || breaker.consecutive_count > 0)
        });
        if !tripped {
            return Ok(None);
        }
        let mut request = automation_to_replace_request(current)?;
        for trigger in &mut request.triggers {
            if let AutomationTrigger::Schedule(schedule) = trigger {
                if schedule.breaker.as_ref().is_some_and(|breaker| {
                    breaker.paused_at.is_some() || breaker.consecutive_count > 0
                }) {
                    // Re-enabling clears the breaker facts server-side;
                    // submitted breaker values are ignored on input.
                    schedule.enabled = true;
                }
            }
        }
        Ok(Some(request))
    })
    .await?;

    match outcome {
        ReplaceOutcome::Unchanged(current) => {
            fabro_util::printerr!(
                printer,
                "Breaker for automation {} is already clean; nothing to reset.",
                current.id
            );
            if ctx.json_output() {
                print_json_pretty(&current)?;
            }
        }
        ReplaceOutcome::Replaced {
            previous_revision,
            automation: updated,
        } => {
            fabro_util::printerr!(
                printer,
                "Reset breaker for automation {} ({} -> {}).",
                updated.id,
                super::short_revision(&previous_revision),
                super::short_revision(updated.revision.as_str()),
            );
            if ctx.json_output() {
                print_json_pretty(&updated)?;
            }
        }
    }
    Ok(())
}
