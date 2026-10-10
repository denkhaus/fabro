//! Context tokens through Fabro's own engine (fabro-e71b): `{{ context.NAME }}`
//! survives Fabro's lowering verbatim (scripts keep non-`inputs`/`vars`
//! tokens; the frontend masks prompt tokens behind PUA sentinels), and the
//! stage-dispatch pass resolves them against the run context — strictly, so
//! an unresolved token fails the run naming the token and the visible keys.
//!
//! This file is a fork-only presence pin (the petri fork's
//! `fork_context_tokens` carries the feature; upstream has none of it): a merge
//! that drops the feature reds here instead of regressing quietly.

use fabro_petri::check;
use fabro_petri::checkpoint::RunGitSettings;
use fabro_petri::engine::RunStatus;

use support::{EngineHarness, host_plugin_ready, request};

mod fork_support;
mod support;

/// A `{{ context.NAME }}` token in a script survives Fabro's lowering
/// verbatim: the admitted graph carries the token, not a rendering.
#[test]
fn a_script_token_survives_fabro_lowering_verbatim() {
    let workflow = fork_support::workflow(
        "  seed [shape=parallelogram, output_schema=\"routing\", script=\"echo \
         '{\\\"context_updates\\\": {\\\"seed_id\\\": \\\"fabro-e71b\\\"}}'\"]\n  \
         check [shape=parallelogram, script=\"echo {{ context.seed_id }}\"]\n",
    );
    let admitted = check::check(&request(&workflow)).expect("the bundle is admitted");
    let config = admitted
        .graph
        .nodes
        .iter()
        .find(|node| node.name == "check")
        .expect("the check node")
        .step
        .config
        .clone();
    assert_eq!(
        config.get("script").and_then(|v| v.as_str()),
        Some("echo {{ context.seed_id }}"),
        "the token rides the admitted graph verbatim: {config}"
    );
}

/// A seeding stage plus a script that proves the resolved value: the run
/// stays green only when `{{ context.seed_id }}` became `fabro-e71b`.
#[tokio::test]
async fn a_script_token_resolves_at_dispatch_through_the_fabro_engine() {
    if !host_plugin_ready() {
        return;
    }
    let harness = EngineHarness::new();
    let workflow = fork_support::workflow(
        "  seed [shape=parallelogram, output_schema=\"routing\", script=\"echo \
         '{\\\"context_updates\\\": {\\\"seed_id\\\": \\\"fabro-e71b\\\"}}'\"]\n  \
         check [shape=parallelogram, script=\"echo seed={{ context.seed_id }} | grep -qx seed=fabro-e71b\"]\n",
    );
    let outcome = harness
        .run(&workflow, RunGitSettings::default(), None)
        .await;
    assert_eq!(outcome.status, RunStatus::Success, "{outcome:?}");
}

/// Strict resolution through Fabro's engine: an unresolved token fails the
/// run naming the token and the visible keys.
#[tokio::test]
async fn an_unresolved_token_fails_the_run_naming_the_token() {
    if !host_plugin_ready() {
        return;
    }
    let harness = EngineHarness::new();
    let workflow = fork_support::workflow(
        "  seed [shape=parallelogram, output_schema=\"routing\", script=\"echo \
         '{\\\"context_updates\\\": {\\\"seed_id\\\": \\\"fabro-e71b\\\"}}'\"]\n  \
         check [shape=parallelogram, script=\"echo {{ context.absent_key }}\", on_failure=\"exit\"]\n",
    );
    let outcome = harness
        .run(&workflow, RunGitSettings::default(), None)
        .await;
    assert_eq!(outcome.status, RunStatus::Failed, "{outcome:?}");
    let failure = outcome.failure.expect("the failure message");
    assert!(
        failure.contains("`{{ context.absent_key }}`"),
        "names the token: {failure}"
    );
    assert!(
        failure.contains("visible context keys") && failure.contains("seed_id"),
        "names the visible keys: {failure}"
    );
}

/// This file's fork-specific extras: the graph shape its scenarios run on.
mod fork_support {
    pub(super) fn workflow(stages: &str) -> String {
        format!(
            "digraph Tokens {{\n  graph [goal=\"Resolve context tokens\", default_max_retries=0]\n  \
             start [shape=Mdiamond]\n  exit [shape=Msquare]\n{stages}\n  start -> seed -> check -> exit\n}}\n"
        )
    }
}
