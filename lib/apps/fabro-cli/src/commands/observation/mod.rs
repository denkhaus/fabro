//! The observation surface's CLI door (ADR-0038/0039,
//! denkhaus/work#1107 stage C): `fabro observation record` mirrors the
//! `fabro_observation_record` agent tool for script stages — the
//! reporter and the adjudicator's fast path write facts from
//! deterministic nu scripts through this verb, over the run channel
//! with the run's worker principal.

mod record;

use anyhow::Result;

use crate::args::{ObservationCommand, ObservationNamespace};
use crate::command_context::CommandContext;

pub(crate) async fn dispatch(ns: ObservationNamespace, base_ctx: &CommandContext) -> Result<()> {
    let ctx = base_ctx.with_target(&ns.target)?;
    match ns.command {
        ObservationCommand::Record(args) => record::record_command(&args, &ctx).await,
    }
}
