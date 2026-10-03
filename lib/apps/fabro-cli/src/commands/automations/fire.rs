use anyhow::Result;

use crate::args::AutomationsFireArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

/// The sanctioned manual trigger: creates a run through the automation's
/// enabled API trigger, making the actor visible and auditable.
pub(super) async fn fire_command(args: &AutomationsFireArgs, ctx: &CommandContext) -> Result<()> {
    let client = ctx.server().await?;
    let printer = ctx.printer();
    let run = client.create_automation_run(&args.id).await?;

    fabro_util::printerr!(
        printer,
        "Fired automation {} -> run {} ({}).",
        args.id,
        run.id,
        run.lifecycle.status
    );
    if ctx.json_output() {
        print_json_pretty(&run)?;
    }
    Ok(())
}
