//! Command-layer binding seam for `fabro seeds`.
//!
//! Two bindings are possible, decision pending with the user:
//! - library API from denkhaus/seeds (command layer lifted upstream, keeps sd
//!   parity single-source),
//! - fabro-side implementation over the pinned format core.
//!
//! Until the decision lands, every subcommand refuses with an explicit
//! error so the skeleton cannot silently no-op.

use anyhow::{Result, bail};

use crate::args::SeedsCommand;

// The dispatch contract stays async: the fork-A binding (seeds library
// command API) performs store I/O and will await. The skeleton refuses
// synchronously, so the lint is silenced with that reason.
#[allow(
    clippy::unused_async,
    reason = "dispatch contract; the wired binding performs store I/O"
)]
pub(crate) async fn run(command: SeedsCommand) -> Result<()> {
    let name = command_name(&command);
    bail!(
        "`fabro seeds {name}` is not wired yet: the seeds command-layer \
         binding is pending (fabro-088b); the tracker format core is \
         already pinned in the workspace"
    )
}

fn command_name(command: &SeedsCommand) -> &'static str {
    match command {
        SeedsCommand::Create(_) => "create",
        SeedsCommand::Show(_) => "show",
        SeedsCommand::List(_) => "list",
        SeedsCommand::Ready(_) => "ready",
        SeedsCommand::Update(_) => "update",
        SeedsCommand::Close(_) => "close",
        SeedsCommand::Dep(_) => "dep",
        SeedsCommand::Prime(_) => "prime",
        SeedsCommand::Search(_) => "search",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::SeedsRawArgs;

    fn raw_args() -> SeedsRawArgs {
        SeedsRawArgs { args: Vec::new() }
    }

    #[tokio::test]
    async fn unwired_subcommands_refuse_with_pending_binding_error() {
        let error = run(SeedsCommand::List(raw_args()))
            .await
            .expect_err("skeleton backend must refuse");
        let message = error.to_string();
        assert!(
            message.contains("fabro-088b"),
            "error names the tracking seed: {message}"
        );
        assert!(
            message.contains("seeds list"),
            "error names the refused subcommand: {message}"
        );
    }
}
