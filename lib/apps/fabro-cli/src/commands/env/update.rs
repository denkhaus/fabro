use anyhow::Result;
use fabro_api::types;

use super::{ReplaceOutcome, environment_to_replace_request, replace_with_retry, short_revision};
use crate::args::EnvUpdateArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

fn apply_flags(args: &EnvUpdateArgs, request: &mut types::ReplaceEnvironmentRequest) {
    if let Some(image) = &args.image {
        request.image.docker = Some(image.clone());
    }
    if args.cpu.is_some() {
        request.resources.cpu = args.cpu;
    }
    if let Some(memory) = &args.memory {
        request.resources.memory = Some(memory.clone());
    }
    if let Some(disk) = &args.disk {
        request.resources.disk = Some(disk.clone());
    }
    if args.preserve {
        request.lifecycle.preserve = true;
    }
    if args.no_preserve {
        request.lifecycle.preserve = false;
    }
    if args.stop_on_terminal {
        request.lifecycle.stop_on_terminal = true;
    }
    if args.no_stop_on_terminal {
        request.lifecycle.stop_on_terminal = false;
    }
    if let Some(auto_stop) = &args.auto_stop {
        request.lifecycle.auto_stop = Some(auto_stop.clone());
    }
    if args.no_auto_stop {
        request.lifecycle.auto_stop = None;
    }
}

pub(super) async fn update_command(args: &EnvUpdateArgs, ctx: &CommandContext) -> Result<()> {
    let client = ctx.server().await?;
    let printer = ctx.printer();

    let outcome = replace_with_retry(&client, &args.id, |current| {
        let mut request = environment_to_replace_request(current)?;
        let before = serde_json::to_value(&request)?;
        apply_flags(args, &mut request);
        let after = serde_json::to_value(&request)?;
        if before == after {
            return Ok(None);
        }
        Ok(Some(request))
    })
    .await?;

    match outcome {
        ReplaceOutcome::Unchanged(current) => {
            fabro_util::printerr!(
                printer,
                "Environment {} unchanged ({}).",
                current.id,
                short_revision(current.revision.as_str())
            );
            if ctx.json_output() {
                print_json_pretty(&current)?;
            }
        }
        ReplaceOutcome::Replaced {
            previous_revision,
            environment: updated,
        } => {
            fabro_util::printerr!(
                printer,
                "Updated environment {} ({} -> {}).",
                updated.id,
                short_revision(&previous_revision),
                short_revision(updated.revision.as_str()),
            );
            if ctx.json_output() {
                print_json_pretty(&updated)?;
            }
        }
    }
    Ok(())
}
