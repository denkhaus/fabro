mod create;
mod list;
mod show;

use anyhow::Result;

use crate::args::{WorkflowCommand, WorkflowNamespace};
use crate::command_context::CommandContext;

pub(crate) async fn dispatch(ns: WorkflowNamespace, base_ctx: &CommandContext) -> Result<()> {
    match ns.command {
        WorkflowCommand::List(args) => list::list_command(&args, base_ctx),
        WorkflowCommand::Create(args) => create::create_command(&args, base_ctx),
        WorkflowCommand::Show(args) => show::show_command(&args, base_ctx).await,
    }
}
