//! `fabro run mark <run-id> <field>=<verdict>` (ADR-0039 stage D,
//! denkhaus/work#1107): the CLI twin of the `fabro_run_mark` agent tool
//! — one endpoint, one authorization, the tool's validation verbatim.
//! The fast-path script writes verdicts from script stages through this
//! door; agents use the tool. The target run is always explicit, and
//! the server's workflow allowlist (the caller's own run is the writer)
//! is the gate.

use std::sync::Arc;

use anyhow::{Result, bail};
use fabro_tool::FabroToolBackend;
use fabro_tool::fabro_client::ClientBackend;
// Types by name (the trait from the crate root, the verb's types from
// its module); the FUNCTIONS stay module-qualified (AGENTS.md import
// style): run_mark::mark_run, run_mark::mark_run_text.
use fabro_tool::run_mark;
use fabro_tool::run_mark::{
    FabroOrchestrationField, FabroOrchestrationVerdict, FabroRunMarkParams, ValidatedRunMark,
};

use crate::args::RunMarkArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

/// Parse `<field>=<verdict>`: exactly one `=`, a known field, a known
/// verdict — every refusal teaches the accepted shapes.
fn parse_field_verdict(raw: &str) -> Result<(FabroOrchestrationField, FabroOrchestrationVerdict)> {
    let Some((field, verdict)) = raw.split_once('=') else {
        bail!(
            "`{raw}` is not <field>=<verdict> — pass merge=enabled, merge=refused, \
             merge=parked, closable=enabled, closable=refused, or closable=parked"
        );
    };
    let field = match field.trim() {
        "merge" => FabroOrchestrationField::Merge,
        "closable" => FabroOrchestrationField::Closable,
        other => bail!(
            "`{other}` is not an orchestration field — the fields are merge (the \
             adjudicator's) and closable (the merger's)"
        ),
    };
    let verdict = match verdict.trim() {
        "enabled" => FabroOrchestrationVerdict::Enabled,
        "refused" => FabroOrchestrationVerdict::Refused,
        "parked" => FabroOrchestrationVerdict::Parked,
        other => bail!(
            "`{other}` is not a verdict — the verdicts are enabled, refused, and parked \
             (pending is the absence of a verdict, never a write)"
        ),
    };
    Ok((field, verdict))
}

pub(super) async fn mark_command(args: &RunMarkArgs, base_ctx: &CommandContext) -> Result<()> {
    let (field, verdict) = parse_field_verdict(&args.field_verdict)?;
    let validated = ValidatedRunMark::try_from(FabroRunMarkParams {
        run: args.run_id.clone(),
        field,
        verdict,
    })?;

    let ctx = base_ctx.with_target(&args.server)?;
    let client = ctx.server().await?;
    let backend: Arc<dyn FabroToolBackend> = Arc::new(ClientBackend::new(client));
    let result = run_mark::mark_run(backend, validated).await?;

    let printer = ctx.printer();
    if ctx.json_output() {
        print_json_pretty(&result)?;
    } else {
        fabro_util::printout!(printer, "{}", run_mark::mark_run_text(&result));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_verdict_pairs_parse_with_tolerant_whitespace() {
        let (field, verdict) = parse_field_verdict(" merge = enabled ").expect("the pair parses");
        assert!(matches!(field, FabroOrchestrationField::Merge));
        assert!(matches!(verdict, FabroOrchestrationVerdict::Enabled));
    }

    #[test]
    fn a_missing_separator_teaches_the_accepted_shapes() {
        let error = parse_field_verdict("merge enabled").expect_err("no = must fail");
        assert!(
            error.to_string().contains("merge=enabled"),
            "the refusal shows the accepted shapes: {error}"
        );
    }

    #[test]
    fn an_unknown_field_names_the_two_fields() {
        let error = parse_field_verdict("title=enabled").expect_err("an unknown field must fail");
        assert!(
            error.to_string().contains("closable"),
            "the refusal names both fields: {error}"
        );
    }

    #[test]
    fn pending_is_never_a_write() {
        let error = parse_field_verdict("merge=pending").expect_err("pending must fail");
        assert!(
            error.to_string().contains("never a write"),
            "the refusal teaches that pending is absence: {error}"
        );
    }
}
