use anyhow::{Result, anyhow};
use fabro_api::types::{Automation, AutomationTrigger};

use super::{ReplaceOutcome, automation_to_replace_request, replace_with_retry, schedule_triggers};
use crate::args::AutomationsSetScheduleArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

pub(super) async fn set_schedule_command(
    args: &AutomationsSetScheduleArgs,
    ctx: &CommandContext,
) -> Result<()> {
    let client = ctx.server().await?;
    let printer = ctx.printer();

    // Validate the expression client-side (deterministic cron math) so an
    // invalid schedule fails before any write attempt.
    super::next_fire_after(&args.cron, chrono::Utc::now())?;

    let outcome = replace_with_retry(&client, &args.id, |current| {
        let trigger_id = select_schedule_trigger_id(args.trigger.as_deref(), current)?;
        let mut request = automation_to_replace_request(current)?;
        let mut changed = false;
        let mut found = false;
        for trigger in &mut request.triggers {
            if let AutomationTrigger::Schedule(schedule) = trigger {
                if schedule.id.to_string() == trigger_id {
                    found = true;
                    if schedule.expression != args.cron {
                        schedule.expression.clone_from(&args.cron);
                        changed = true;
                    }
                }
            }
        }
        if !found {
            return Err(anyhow!(
                "automation {} has no schedule trigger {trigger_id}",
                current.id
            ));
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
                "Automation {} schedule already set to {:?}.",
                current.id,
                args.cron
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
                "Set automation {} schedule to {:?} ({} -> {}).",
                updated.id,
                args.cron,
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

/// Selects the schedule trigger an id-addressed edit applies to: the
/// `--trigger` id when given, the single schedule trigger otherwise.
fn select_schedule_trigger_id(requested: Option<&str>, current: &Automation) -> Result<String> {
    if let Some(id) = requested {
        return Ok(id.to_string());
    }
    let mut schedules = schedule_triggers(current);
    let first = schedules.next().map(|schedule| schedule.id.to_string());
    if schedules.next().is_some() {
        return Err(anyhow!(
            "automation {} has multiple schedule triggers; pass --trigger <id> to pick one",
            current.id
        ));
    }
    first.ok_or_else(|| anyhow!("automation {} has no schedule trigger to edit", current.id))
}
