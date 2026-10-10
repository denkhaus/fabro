//! Stage envelopes on Petri, end to end (fabro-aa5f, ADR-0009 rev):
//! the `x.fs_write`/`x.fs_hide`/`x.preamble_*` values ride the run's
//! `graph_source`, the create-time fork lints refuse invalid globs and
//! warn about inconsistent ones, and the checkpoint guard ends a run
//! whose staged files fall outside the node's `fs_write` scope before
//! anything is committed.
//!
//! This file is a fork-only presence pin: upstream has no
//! `fork_stage_envelope`, so a merge that drops the feature reds here
//! instead of regressing quietly.

use std::sync::Arc;

use fabro_petri::check::{self, CheckError};
use fabro_petri::checkpoint::RunGitSettings;
use fabro_petri::engine::RunStatus;
use fabro_petri::fork_stage_envelope::StageEnvelopes;

use support::{EngineHarness, host_plugin_ready, request};

mod fork_support;
mod support;

/// An invalid envelope glob refuses the workflow at check, under the fork
/// lint's stable code — Petri itself accepts `x.*` blindly.
/// The per-node run-tools allowlist parses off the graph source and
/// reaches the host-tools filter (fabro-96c6).
#[test]
fn fabro_tools_allowlist_parses_per_node() {
    let workflow = fork_support::workflow(
        "  work [shape=parallelogram, script=\"echo hi\", \
         x.fabro_tools=\"fabro_run_search,fabro_run_wait\"]\n  bare \
         [shape=parallelogram, script=\"echo no\"]\n",
    );
    let envelopes = StageEnvelopes::parse(&workflow);
    let work = envelopes.envelope("work").expect("work declares tools");
    assert_eq!(
        work.fabro_tools,
        Some(vec![
            "fabro_run_search".to_string(),
            "fabro_run_wait".to_string()
        ])
    );
    assert!(envelopes.envelope("bare").is_none());
}

#[test]
fn invalid_fs_globs_are_refused_at_check() {
    let workflow = fork_support::workflow(
        "  work [shape=parallelogram, script=\"echo hi\", \
         x.fs_write=\"../escape\"]\n",
    );
    let Err(CheckError::Rejected(diagnostics)) = check::check(&request(&workflow)) else {
        panic!("the invalid glob refuses the workflow");
    };
    let finding = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "fork.fs_globs_valid")
        .expect("the fork lint fired");
    assert!(finding.is_error(), "{finding:?}");
    assert!(finding.message.contains("work"), "{finding:?}");
}

/// `x.fs_write=""` is a read-only stage (fabro-ba96): the envelope holds
/// an empty write list — which is not the same as no envelope at all, the
/// `None` that means the node declares none — and Petri's check admits
/// the workflow. The Attractor frontend's attribute surface takes the
/// empty value; fabro's envelope reader is what gives it meaning.
#[test]
fn an_empty_fs_write_is_a_read_only_stage() {
    let workflow =
        fork_support::workflow("  work [shape=parallelogram, script=\"echo hi\", x.fs_write=\"\"]\n");
    let envelopes = StageEnvelopes::parse(&workflow);
    let work = envelopes
        .envelope("work")
        .expect("an empty value still declares the envelope");
    assert_eq!(
        work.fs_write,
        Some(Vec::new()),
        "an empty list admits no write at all"
    );
    let scope = envelopes
        .fs_scope("work")
        .expect("the node declares the envelope")
        .expect("the empty envelope compiles");
    assert!(
        scope.check_write("/workspace", "workflow.fabro").is_err(),
        "nothing is writable under an empty fs_write"
    );

    let admitted = check::check(&request(&workflow)).expect("the bundle is admitted");
    let refusals: Vec<&str> = admitted
        .warnings
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .filter(|code| code.starts_with("fork.fs_"))
        .collect();
    assert!(refusals.is_empty(), "no fs finding fires for {refusals:?}");
}

/// Consistency findings warn without refusing: an inline ceiling above the
/// budget, and a write entry under a hide root.
#[test]
fn consistency_findings_warn_without_refusing() {
    let workflow = fork_support::workflow(
        "  work [shape=parallelogram, script=\"echo hi\", \
         x.preamble_inline_max_kb=64, x.fs_hide=\"lib/**\", \
         x.fs_write=\"lib/**\"]\n",
    );
    // The graph-level budget: the default (48) is below the node's 64.
    let admitted = check::check(&request(&workflow)).expect("the bundle is admitted");
    let codes: Vec<&str> = admitted
        .warnings
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    assert!(
        codes.contains(&"fork.preamble_budget_consistency"),
        "{codes:?}"
    );
    assert!(codes.contains(&"fork.fs_scope_consistency"), "{codes:?}");
}

/// A stage that writes outside its `x.fs_write` scope ends the run before
/// the violating file is committed: the failure names the node and the
/// path, under the `fs_policy_violation` class.
#[tokio::test]
async fn a_write_outside_fs_write_ends_the_run_uncommitted() {
    if !host_plugin_ready() {
        return;
    }
    let harness = EngineHarness::new();
    let workflow = fork_support::workflow(
        "  work [shape=parallelogram, script=\"mkdir -p allowed && echo ok > allowed/a.txt \
         && echo escape > escape.txt\", x.fs_write=\"allowed/**\"]\n",
    );
    let git = RunGitSettings {
        host_workspaces: true,
        ..RunGitSettings::default()
    };
    let outcome = harness
        .run(&workflow, git, Some(Arc::new(StageEnvelopes::parse(&workflow))))
        .await;
    assert_eq!(outcome.status, RunStatus::Failed, "{outcome:?}");
    let failure = outcome.failure.expect("the run failed with a reason");
    assert!(
        failure.contains("stage envelope violation in `work`"),
        "{failure}"
    );
    assert!(failure.contains("escape.txt"), "{failure}");
}

/// A stage that stays inside its scope commits normally: the envelope
/// restricts, it does not block.
#[tokio::test]
async fn a_write_inside_fs_write_commits() {
    if !host_plugin_ready() {
        return;
    }
    let harness = EngineHarness::new();
    let workflow = fork_support::workflow(
        "  work [shape=parallelogram, script=\"mkdir -p allowed && echo ok > \
         allowed/a.txt\", x.fs_write=\"allowed/**\"]\n",
    );
    let git = RunGitSettings {
        host_workspaces: true,
        ..RunGitSettings::default()
    };
    let outcome = harness
        .run(&workflow, git, Some(Arc::new(StageEnvelopes::parse(&workflow))))
        .await;
    assert_eq!(outcome.status, RunStatus::Success, "{outcome:?}");
    assert!(!harness.records.records(&harness.run_id).is_empty());
}

/// This file's fork-specific extras: the graph shape its scenarios run on.
mod fork_support {
    pub(super) fn workflow(stages: &str) -> String {
        format!(
            "digraph Envelope {{\n  graph [goal=\"Check the envelope\", default_max_retries=0]\n  \
             start [shape=Mdiamond]\n  exit [shape=Msquare]\n{stages}\n  start -> work -> exit\n}}\n"
        )
    }
}
