use anyhow::{Result, anyhow};
use fabro_api::types::AutomationTrigger;

use super::{ReplaceOutcome, automation_to_replace_request, replace_with_retry, schedule_triggers};
use crate::args::AutomationsPauseArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

/// Pause (`enabled = false`) or unpause (`enabled = true`) every schedule
/// trigger of an automation. The replace round-trips the full trigger list,
/// so unrelated triggers (e.g. the manual API trigger) are preserved.
pub(super) async fn pause_command(
    args: &AutomationsPauseArgs,
    ctx: &CommandContext,
    enabled: bool,
) -> Result<()> {
    let client = ctx.server().await?;
    let printer = ctx.printer();
    let verb = if enabled { "Unpaused" } else { "Paused" };

    let outcome = replace_with_retry(&client, &args.id, |current| {
        if schedule_triggers(current).next().is_none() {
            return Err(anyhow!(
                "automation {} has no schedule trigger to {}",
                current.id,
                if enabled { "unpause" } else { "pause" }
            ));
        }
        let mut request = automation_to_replace_request(current)?;
        let mut changed = false;
        for trigger in &mut request.triggers {
            if let AutomationTrigger::Schedule(schedule) = trigger {
                if schedule.enabled != enabled {
                    schedule.enabled = enabled;
                    changed = true;
                }
            }
        }
        if !changed {
            return Ok(None);
        }
        Ok(Some(request))
    })
    .await?;

    match outcome {
        ReplaceOutcome::Unchanged(current) => {
            fabro_util::printerr!(
                printer,
                "Automation {} schedule already {}.",
                current.id,
                if enabled { "enabled" } else { "paused" }
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
                "{} automation {} ({} -> {}).",
                verb,
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
