//! `fabro seeds`: sd-parity surface over the native seeds tracker.
//!
//! denkhaus/seeds crate (pinned git dependency, ADR-0023): the format
//! core is compiled in; subcommand arguments pass through verbatim in
//! this skeleton phase and `backend::run` refuses until the
//! command-layer binding is decided and wired.

mod backend;

use anyhow::Result;

use crate::args::SeedsNamespace;
use crate::command_context::CommandContext;

#[allow(
    clippy::unused_async,
    reason = "namespace dispatch contract shared with wired commands"
)]
pub(crate) async fn dispatch(ns: SeedsNamespace, _base_ctx: &CommandContext) -> Result<()> {
    backend::run(ns.command).await
}
