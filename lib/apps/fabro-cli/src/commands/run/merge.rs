//! `fabro run merge <run-id>` (ADR-0039 stage E, denkhaus/work#1107):
//! the CLI twin of the `fabro_run_merge` agent tool — one endpoint, one
//! authorization, the tool's validation verbatim. The merger's fast-path
//! script merges the clean mass through this door from script stages;
//! the agent pass uses the tool. The target run is always explicit, and
//! the server's workflow allowlist (the caller's own run is the writer)
//! is the gate.

use std::sync::Arc;

use anyhow::Result;
use fabro_tool::FabroToolBackend;
use fabro_tool::fabro_client::ClientBackend;
// Types by name (AGENTS.md import style); the FUNCTIONS stay
// module-qualified: run_merge::run_merge, run_merge::run_merge_text.
use fabro_tool::run_merge;

use crate::args::RunMergeArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

pub(super) async fn merge_command(args: &RunMergeArgs, base_ctx: &CommandContext) -> Result<()> {
    let validated = run_merge::ValidatedRunMerge::try_from(run_merge::FabroRunMergeParams {
        run: args.run_id.clone(),
    })?;

    let ctx = base_ctx.with_target(&args.server)?;
    let client = ctx.server().await?;
    let backend: Arc<dyn FabroToolBackend> = Arc::new(ClientBackend::new(client));
    let result = run_merge::run_merge(backend, validated).await?;

    let printer = ctx.printer();
    if ctx.json_output() {
        print_json_pretty(&result)?;
    } else {
        fabro_util::printout!(printer, "{}", run_merge::run_merge_text(&result));
    }
    Ok(())
}
