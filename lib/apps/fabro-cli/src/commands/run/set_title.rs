//! `fabro run set-title <TITLE> [--run <RUN>]` (fabro-9711/ADR-0030,
//! denkhaus/work#1159): the CLI twin of the `fabro_run_set_title` agent
//! tool — one endpoint, one authorization, the tool's validation
//! verbatim. Deterministic script stages set their run's title through
//! this door; agents use the tool.
//!
//! A script stage cannot pass its identity as a flag without noise: the
//! ENGINE binds `FABRO_RUN_ID` into every command script (stage
//! self-awareness). The command defaults from that environment; an
//! explicit `--run` overrides, and a caller with neither source is
//! taught the contract instead of guessing.

use std::sync::Arc;

use anyhow::{Result, bail};
use fabro_tool::fabro_client::ClientBackend;
// Free functions through the crate's re-export surface (the run_title
// module itself is private): these four ARE the tool's public contract.
use fabro_tool::{
    FabroRunSetTitleParams, FabroToolBackend, ValidatedRunSetTitle, set_run_title,
    set_run_title_text,
};

use crate::args::RunSetTitleArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

/// The run key the engine binds for a command script.
const RUN_ID_ENV: &str = "FABRO_RUN_ID";

pub(super) async fn set_title_command(
    args: &RunSetTitleArgs,
    base_ctx: &CommandContext,
) -> Result<()> {
    let selector = resolve_selector(args.run.clone(), crate::process_env_var(RUN_ID_ENV))?;
    let validated = ValidatedRunSetTitle::try_from(FabroRunSetTitleParams {
        run:   Some(selector),
        title: args.title.clone(),
    })
    .map_err(|err| anyhow::anyhow!(err.to_string()))?;

    let ctx = base_ctx.with_target(&args.server)?;
    let client = ctx.server().await?;
    let backend: Arc<dyn FabroToolBackend> = Arc::new(ClientBackend::new(client));
    let result = set_run_title(backend, validated).await?;

    let printer = ctx.printer();
    if ctx.json_output() {
        print_json_pretty(&result)?;
    } else {
        fabro_util::printout!(printer, "{}", set_run_title_text(&result));
    }
    Ok(())
}

/// The target run selector: `--run` where given, else the engine-bound
/// `FABRO_RUN_ID`; neither is a refusal that teaches both sources.
fn resolve_selector(flag_run: Option<String>, env_run: Option<String>) -> Result<String> {
    let trim = |value: String| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    };
    if let Some(selector) = flag_run.and_then(trim) {
        return Ok(selector);
    }
    let Some(selector) = env_run.and_then(trim) else {
        bail!(
            "the run is required — pass --run <run-id|workflow>, or run inside a workflow \
             stage: the engine binds {RUN_ID_ENV} for every command script"
        );
    };
    Ok(selector)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explicit_selector_wins_and_trims() {
        assert_eq!(
            resolve_selector(Some("  01M55  ".to_string()), None).expect("the flag resolves"),
            "01M55"
        );
    }

    #[test]
    fn the_engine_bound_run_is_the_default() {
        assert_eq!(
            resolve_selector(None, Some(" 01M55ENV ".to_string())).expect("the env resolves"),
            "01M55ENV"
        );
    }

    #[test]
    fn a_blank_flag_falls_through_to_the_environment() {
        assert_eq!(
            resolve_selector(Some("   ".to_string()), Some("01M55ENV".to_string()))
                .expect("the env resolves"),
            "01M55ENV"
        );
    }

    #[test]
    fn neither_source_is_a_refusal_that_teaches_both() {
        let error =
            resolve_selector(Some("   ".to_string()), None).expect_err("no source must refuse");
        assert!(error.to_string().contains("FABRO_RUN_ID"), "{error}");
        assert!(error.to_string().contains("--run"), "{error}");
    }
}
