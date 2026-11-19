//! Fork-only pins for the env-build holder reaper (work#1480): the
//! name→run attribution, the terminal/absent decision, and the reap
//! core over synthetic holder statuses driven into a scripted provider
//! — no daemon needed. The VENDOR-seam presence pin (the reaper must
//! survive an upstream merge that rewrites `fork_run_retention.rs`)
//! is this whole file's existence.

use std::collections::HashMap;
use std::sync::Arc;

use fabro_types::{RunId, RunStatus};
use sandbox_driver::{SandboxId, SandboxState, SandboxStatus};
use sandbox_driver_testing::{ScriptedProvider, ScriptedSandbox};

use super::fork_env_build_reaper::{
    EnvBuildReapStats, holder_owner_run, holder_reapable, reap_env_build_holders,
};

/// A run id shaped like the production ULIDs (the container name
/// carries the run's store key verbatim).
fn run_id(low: char) -> RunId {
    format!("01HY{}{low}", "0".repeat(21))
        .parse()
        .expect("valid run id")
}

fn holder_name(run: &RunId, lease: &str) -> String {
    format!("petri-{run}-l{lease}-env-build")
}

/// A running holder status for `run`, named exactly as the build names
/// its holders on the daemon.
fn holder_status(id: &str, run: &RunId) -> SandboxStatus {
    let mut status = SandboxStatus::new(
        SandboxId::try_new(id).expect("valid sandbox id"),
        SandboxState::Running,
    );
    status.name = Some(holder_name(run, "42"));
    status
        .labels
        .insert("petri.role".to_owned(), "env-build".to_owned());
    status
}

fn stats(
    scanned: usize,
    reaped: usize,
    live_skipped: usize,
    unattributed: usize,
    lookup_failed: usize,
    reap_failed: usize,
) -> EnvBuildReapStats {
    EnvBuildReapStats {
        scanned,
        reaped,
        live_skipped,
        unattributed,
        lookup_failed,
        reap_failed,
    }
}

#[test]
fn holder_owner_run_parses_the_build_name_shape() {
    let run = run_id('0');
    assert_eq!(holder_owner_run(&holder_name(&run, "7")), Some(run));
    // The lease segment is whatever follows the last `-l`.
    assert_eq!(
        holder_owner_run(&format!("petri-{run}-l1048576-env-build")),
        Some(run)
    );
    // Not a holder: a run workspace shares the prefix but lacks the
    // suffix — attribution is the safety filter, never just the label.
    assert_eq!(holder_owner_run(&format!("petri-{run}-l7")), None);
    assert_eq!(holder_owner_run("some-operator-container"), None);
    // A name whose run segment is not a store-key run id (the
    // no-coordinator hex ids) is unattributed — never reaped.
    assert_eq!(
        holder_owner_run("petri-0123456789abcdef-l7-env-build"),
        None
    );
}

#[test]
fn holder_reapable_is_terminal_or_absent_only() {
    assert!(holder_reapable(None), "an absent run row is reapable");
    assert!(holder_reapable(Some(RunStatus::Dead)));
    assert!(holder_reapable(Some(RunStatus::Succeeded {
        reason: fabro_types::SuccessReason::Completed,
    })));
    assert!(holder_reapable(Some(RunStatus::Failed {
        reason: fabro_types::FailureReason::WorkflowError,
    })));
    // Everything alive or waiting is protected.
    assert!(!holder_reapable(Some(RunStatus::Running)));
    assert!(!holder_reapable(Some(RunStatus::Starting)));
    assert!(!holder_reapable(Some(RunStatus::Blocked {
        blocked_reason: fabro_types::BlockedReason::HumanInputRequired,
    })));
    assert!(!holder_reapable(Some(RunStatus::HumanNeeded)));
    assert!(!holder_reapable(Some(RunStatus::Paused {
        prior_block: None,
    })));
    assert!(!holder_reapable(Some(RunStatus::Removing)));
}

/// The acceptance pin: a seeded orphaned holder (its run record
/// terminal, the holder running) is reaped within the pass, while a
/// live run's holder and an unattributable name are never touched.
#[tokio::test]
async fn orphaned_holders_are_reaped_and_live_ones_untouched() {
    let terminal_run = run_id('0');
    let live_run = run_id('1');
    let absent_run = run_id('2');

    let orphan = Arc::new(ScriptedSandbox::with_id_and_working_dir(
        "orphan",
        "/workspace",
    ));
    let orphan_of_absent = Arc::new(ScriptedSandbox::with_id_and_working_dir(
        "absent",
        "/workspace",
    ));
    let live = Arc::new(ScriptedSandbox::with_id_and_working_dir(
        "live",
        "/workspace",
    ));
    let provider: Arc<dyn sandbox_driver::SandboxProvider> = {
        let provider = ScriptedProvider::new("docker");
        provider.register(Arc::clone(&orphan));
        provider.register(Arc::clone(&orphan_of_absent));
        provider.register(Arc::clone(&live));
        Arc::new(provider)
    };

    let mut statuses = vec![
        holder_status("orphan", &terminal_run),
        holder_status("absent", &absent_run),
        holder_status("live", &live_run),
    ];
    // A name-less holder (a provider that cannot report names): never
    // touched.
    statuses.push(SandboxStatus::new(
        SandboxId::try_new("nameless").expect("valid sandbox id"),
        SandboxState::Running,
    ));

    let mut lifecycles: HashMap<RunId, RunStatus> = HashMap::new();
    lifecycles.insert(terminal_run, RunStatus::Dead);
    lifecycles.insert(live_run, RunStatus::Running);

    let pass = reap_env_build_holders(
        &statuses,
        |run_id| {
            let lifecycles = lifecycles.clone();
            async move { Ok(lifecycles.get(&run_id).copied()) }
        },
        |id| {
            let provider = Arc::clone(&provider);
            async move { super::fork_env_build_reaper::reap_provider_holder(&*provider, &id).await }
        },
    )
    .await;

    assert_eq!(
        pass,
        stats(4, 2, 1, 1, 0, 0),
        "terminal + absent reaped; live and nameless untouched"
    );
    assert_eq!(orphan.stop_count(), 1, "the orphaned holder was stopped");
    assert_eq!(orphan.delete_count(), 1, "the orphaned holder was deleted");
    assert_eq!(orphan.current_state(), SandboxState::Deleted);
    assert_eq!(
        orphan_of_absent.delete_count(),
        1,
        "the absent run's holder was deleted"
    );
    assert_eq!(
        live.stop_count(),
        0,
        "the live run's holder was never touched"
    );
    assert_eq!(live.delete_count(), 0);
    assert_eq!(live.current_state(), SandboxState::Running);
}

/// The fail-closed lookup: a run row that cannot be READ is not proven
/// terminal — the holder stays, and the failure is counted for the
/// next tick.
#[tokio::test]
async fn a_failing_run_lookup_never_reaps() {
    let run = run_id('0');
    let holder = Arc::new(ScriptedSandbox::with_id_and_working_dir(
        "holder",
        "/workspace",
    ));
    let provider: Arc<dyn sandbox_driver::SandboxProvider> = {
        let provider = ScriptedProvider::new("docker");
        provider.register(Arc::clone(&holder));
        Arc::new(provider)
    };
    let statuses = vec![holder_status("holder", &run)];

    let pass = reap_env_build_holders(
        &statuses,
        |_| async { anyhow::bail!("the summaries store is sealed") },
        |id| {
            let provider = Arc::clone(&provider);
            async move { super::fork_env_build_reaper::reap_provider_holder(&*provider, &id).await }
        },
    )
    .await;

    assert_eq!(pass, stats(1, 0, 0, 0, 1, 0));
    assert_eq!(holder.stop_count(), 0);
    assert_eq!(holder.delete_count(), 0);
}

/// A holder that vanished between list and attach is the expected
/// idempotency case: counted as reaped, no error.
#[tokio::test]
async fn an_already_gone_holder_is_idempotent() {
    let run = run_id('0');
    let provider: Arc<dyn sandbox_driver::SandboxProvider> =
        Arc::new(ScriptedProvider::new("docker"));
    let statuses = vec![holder_status("vanished", &run)];

    let pass = reap_env_build_holders(
        &statuses,
        |_| async { Ok(None) },
        |id| {
            let provider = Arc::clone(&provider);
            async move { super::fork_env_build_reaper::reap_provider_holder(&*provider, &id).await }
        },
    )
    .await;

    assert_eq!(pass, stats(1, 1, 0, 0, 0, 0));
}
