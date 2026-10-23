mod upsert;

use anyhow::Result;

use crate::args::{ProjectsCommand, ProjectsNamespace};
use crate::command_context::CommandContext;

pub(crate) async fn dispatch(ns: ProjectsNamespace, base_ctx: &CommandContext) -> Result<()> {
    let ctx = base_ctx.with_target(&ns.target)?;
    match ns.command {
        ProjectsCommand::Upsert(args) => upsert::upsert_command(&args, &ctx).await,
    }
}
