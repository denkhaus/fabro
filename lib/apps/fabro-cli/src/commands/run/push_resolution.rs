//! `fabro run push-resolution <run-id> <commit>` (ADR-0047,
//! denkhaus/work#1471): the CLI twin of the `fabro_run_push_resolution`
//! agent tool — one endpoint, one authorization, the tool's validation
//! verbatim. The resolve pass lands a conflict resolution on the target
//! run's own branch through this door; the server transfers the objects
//! from the calling merger run's checkout itself and pushes with its
//! own credentials, refusing any branch but `fabro/run/<id>` and every
//! non-fast-forward push.

use std::sync::Arc;

use anyhow::Result;
use fabro_tool::FabroToolBackend;
use fabro_tool::fabro_client::ClientBackend;
// Types by name (AGENTS.md import style); the FUNCTIONS stay
// module-qualified: run_push_resolution::run_push_resolution,
// run_push_resolution::run_push_resolution_text.
use fabro_tool::run_push_resolution;

use crate::args::RunPushResolutionArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

pub(super) async fn push_resolution_command(
    args: &RunPushResolutionArgs,
    base_ctx: &CommandContext,
) -> Result<()> {
    let validated = run_push_resolution::ValidatedRunPushResolution::try_from(
        run_push_resolution::FabroRunPushResolutionParams {
            run:    args.run_id.clone(),
            commit: args.commit.clone(),
        },
    )?;

    let ctx = base_ctx.with_target(&args.server)?;
    let client = ctx.server().await?;
    let backend: Arc<dyn FabroToolBackend> = Arc::new(ClientBackend::new(client));
    let result = run_push_resolution::run_push_resolution(backend, validated).await?;

    let printer = ctx.printer();
    if ctx.json_output() {
        print_json_pretty(&result)?;
    } else {
        fabro_util::printout!(
            printer,
            "{}",
            run_push_resolution::run_push_resolution_text(&result)
        );
    }
    Ok(())
}
