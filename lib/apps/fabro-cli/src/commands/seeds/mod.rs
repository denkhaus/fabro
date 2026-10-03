//! `fabro seeds`: sd-parity surface over the native seeds tracker
//! (fabro-088b, fork A). The seeds crate's `commands` API is compiled in
//! and owns every parity semantic; this namespace only maps arguments
//! and prints outcomes.

mod backend;

use crate::args::SeedsNamespace;
use crate::command_context::CommandContext;

pub(crate) fn dispatch(ns: SeedsNamespace, base_ctx: &CommandContext) {
    backend::dispatch(ns, base_ctx);
}
