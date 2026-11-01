//! `fabro observation record` (ADR-0038/0039): one observation from a
//! workflow stage, through the same backend the agent tool uses — the
//! validation is the tool's (`ValidatedObservation`), the append is the
//! tool's (`observation::record_observation` over `ClientBackend`), so
//! the two doors cannot drift.
//!
//! A script stage cannot pass its identity as flags without noise, and
//! cannot know its Petri position on its own: the ENGINE binds
//! `FABRO_RUN_ID`, `FABRO_EXECUTION` and `FABRO_FIRING` into every
//! command script's environment (stage self-awareness). The command
//! defaults from that environment; explicit flags override, and a caller
//! with neither source is taught the contract instead of guessing.

use std::sync::Arc;

use anyhow::{Result, bail};
use fabro_store::platform_records::StagePosition;
use fabro_tool::fabro_client::ClientBackend;
// Free functions through the crate's re-export surface (the observation
// module itself is private): these four ARE the tool's public contract.
use fabro_tool::{
    FabroObservationRecordParams, FabroToolBackend, ValidatedObservation, record_observation,
    record_observation_text,
};
use fabro_types::RunId;

use crate::args::ObservationRecordArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

/// The run key the engine binds for a command script.
const RUN_ID_ENV: &str = "FABRO_RUN_ID";
/// The Petri execution the engine binds for a command script.
const EXECUTION_ENV: &str = "FABRO_EXECUTION";
/// The Petri firing the engine binds for a command script.
const FIRING_ENV: &str = "FABRO_FIRING";

/// The identity one record call needs, from its two sources: the flags
/// and the engine-bound environment. A flag, where given, wins over the
/// environment — and the stage position is all-or-nothing per source:
/// `--execution` and `--firing` travel together, a pair half flag and
/// half environment belongs to no stage.
struct IdentitySources {
    flag_run_id:   Option<String>,
    flag_position: Option<(u64, u64)>,
    env_run_id:    Option<String>,
    env_position:  Option<(u64, u64)>,
}

impl IdentitySources {
    /// The flags of one call over the environment the engine bound this
    /// process: a position flag pair joins into one `flag_position`, so
    /// a half-flagged position is refused here, before any transport.
    fn new(args: &ObservationRecordArgs) -> Result<Self> {
        let flag_position = match (args.execution, args.firing) {
            (Some(execution), Some(firing)) => Some((execution, firing)),
            (None, None) => None,
            (Some(_), None) | (None, Some(_)) => {
                bail!("a stage position is both halves — pass --execution and --firing together")
            }
        };
        let env_number = |name: &str| {
            crate::process_env_var(name).and_then(|value| value.trim().parse::<u64>().ok())
        };
        let env_position = match (env_number(EXECUTION_ENV), env_number(FIRING_ENV)) {
            (Some(execution), Some(firing)) => Some((execution, firing)),
            _ => None,
        };
        Ok(Self {
            flag_run_id: args.run_id.clone(),
            flag_position,
            env_run_id: crate::process_env_var(RUN_ID_ENV),
            env_position,
        })
    }

    fn run_id(&self) -> Result<RunId> {
        let raw = self
            .flag_run_id
            .clone()
            .or_else(|| self.env_run_id.clone())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let Some(raw) = raw else {
            bail!(
                "the run is required — pass --run-id <id>, or run inside a workflow stage: the \
                 engine binds {RUN_ID_ENV} for every command script"
            );
        };
        raw.parse()
            .map_err(|_| anyhow::anyhow!("`{raw}` is not a run id"))
    }

    fn position(&self) -> Result<StagePosition> {
        let Some((execution, firing)) = self.flag_position.or(self.env_position) else {
            bail!(
                "the stage position is required — pass --execution and --firing, or run inside \
                 a workflow stage: the engine binds {EXECUTION_ENV} and {FIRING_ENV} for every \
                 command script"
            );
        };
        Ok(StagePosition { execution, firing })
    }
}

pub(super) async fn record_command(
    args: &ObservationRecordArgs,
    ctx: &CommandContext,
) -> Result<()> {
    // The tool's validation, verbatim: trim, the blank refusal with its
    // empty-entry teaching, and the character cap naming limit and length.
    let validated = ValidatedObservation::try_from(FabroObservationRecordParams {
        category: args.category.into(),
        text:     args.text.clone(),
    })?;
    let sources = IdentitySources::new(args)?;
    let run_id = sources.run_id()?;
    let position = sources.position()?;

    let client = ctx.server().await?;
    let backend: Arc<dyn FabroToolBackend> = Arc::new(ClientBackend::new(client));
    let result = record_observation(backend, run_id, position, validated).await?;

    let printer = ctx.printer();
    if ctx.json_output() {
        print_json_pretty(&result)?;
    } else {
        fabro_util::printout!(printer, "{}", record_observation_text(&result));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources(
        flag_run_id: Option<&str>,
        flag_position: Option<(u64, u64)>,
        env_run_id: Option<&str>,
        env_position: Option<(u64, u64)>,
    ) -> IdentitySources {
        IdentitySources {
            flag_run_id: flag_run_id.map(str::to_string),
            flag_position,
            env_run_id: env_run_id.map(str::to_string),
            env_position,
        }
    }

    const RUN: &str = "01KRBZW4DW0000000000000002";

    /// The engine-bound environment alone carries a script stage: no
    /// flags, the identity resolves.
    #[test]
    fn the_environment_alone_resolves_the_identity() {
        let identity = sources(None, None, Some(RUN), Some((0, 3)));
        let run_id = identity.run_id().expect("the env run id resolves");
        assert_eq!(run_id.to_string(), RUN);
        assert_eq!(
            identity.position().expect("the env position resolves"),
            StagePosition {
                execution: 0,
                firing:    3,
            }
        );
    }

    /// Flags win over the environment: an operator override beats the
    /// stage the script runs in.
    #[test]
    fn flags_override_the_environment() {
        let identity = sources(
            Some(RUN),
            Some((9, 11)),
            Some("01KRBZW4DW0000000000000099"),
            Some((0, 3)),
        );
        let run_id = identity.run_id().expect("the flag run id wins");
        assert_eq!(run_id.to_string(), RUN);
        assert_eq!(
            identity.position().expect("the flag position wins"),
            StagePosition {
                execution: 9,
                firing:    11,
            }
        );
    }

    /// No run from either source: the refusal names the flag and the
    /// engine binding, so one read tells the caller both ways.
    #[test]
    fn a_missing_run_names_both_sources() {
        let error = sources(None, None, None, Some((0, 3)))
            .run_id()
            .expect_err("no run id is an error");
        let message = error.to_string();
        assert!(message.contains("--run-id"), "names the flag: {message}");
        assert!(
            message.contains("FABRO_RUN_ID"),
            "names the binding: {message}"
        );
    }

    /// No position from either source: the refusal names both halves'
    /// flags and the engine bindings.
    #[test]
    fn a_missing_position_names_both_halves() {
        let error = sources(None, None, Some(RUN), None)
            .position()
            .expect_err("no position is an error");
        let message = error.to_string();
        assert!(message.contains("--execution"), "names a half: {message}");
        assert!(
            message.contains("--firing"),
            "names the other half: {message}"
        );
    }

    /// A run id string that is not a run id is refused naming the value.
    #[test]
    fn a_malformed_run_id_is_refused() {
        let error = sources(Some("not-a-run"), None, None, None)
            .run_id()
            .expect_err("a malformed run id is an error");
        assert!(
            error.to_string().contains("not-a-run"),
            "names the value: {error}"
        );
    }
}
