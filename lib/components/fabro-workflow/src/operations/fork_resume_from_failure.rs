//! Fork-only resume-from-failure (fabro-7627, salvaged from run
//! 01M2J1BA2JC4S6SA10YEJHHMVP 2026-09-15). One-click Resume of a terminal
//! failed run: the engine — not the caller — selects the rewind target.
//!
//! Fork-file policy (fabro-986b / user directive 2026-09-13): this module
//! lives only on our fork; upstream merges cannot conflict it away. The
//! seam is the `pub use` in `operations/mod.rs`; presence is pinned by
//! `handler/llm/fork_seam_tests.rs` (resolve_failure_rewind_target
//! semantics + OpenAPI resume path) — a dropped feature reds the gate.

use fabro_store::Database;
use fabro_types::{CheckpointRecord, Principal, RunId, RunStatus};

use super::archive;
use super::fork::ResolvedForkTarget;
use super::rewind::{RewindInput, RewindOutcome, rewind};
use super::timeline::ForkTarget;
use crate::error::Error;

#[derive(Debug, Clone)]
pub struct ResumeFailureInput {
    pub run_id: RunId,
}

impl RewindOutcome {
    #[must_use]
    pub fn new_run_id(&self) -> RunId {
        match self {
            Self::Full { new_run_id, .. } | Self::Partial { new_run_id, .. } => *new_run_id,
        }
    }

    #[must_use]
    pub fn target(&self) -> &ResolvedForkTarget {
        match self {
            Self::Full { target, .. } | Self::Partial { target, .. } => target,
        }
    }
}

/// One-click Resume of a terminal failed run: the engine — not the caller —
/// selects the rewind target (the entry checkpoint of the last failed stage),
/// executes the rewind (fork + archive) path, and returns the replacement run
/// for scheduling. Unlike plain `start {resume:true}` replay, this drops the
/// failed stage's committed outcome/routing decision, so a soft-exit park
/// re-runs the failed node instead of re-parking.
pub async fn resume_from_failure(
    store: &Database,
    input: &ResumeFailureInput,
    actor: Option<Principal>,
) -> Result<RewindOutcome, Error> {
    let run_store = store
        .open_run(&input.run_id)
        .await
        .map_err(|err| Error::engine(err.to_string()))?;
    let state = run_store
        .state()
        .await
        .map_err(|err| Error::engine(err.to_string()))?;
    archive::ensure_not_archived(state.archived_at.is_some(), &input.run_id)?;
    if matches!(state.status, RunStatus::Succeeded { .. }) {
        return Err(Error::Precondition(
            "run already finished successfully — nothing to resume".to_string(),
        ));
    }
    if state.status.terminal_status().is_none() {
        return Err(Error::Precondition(format!(
            "run {} must be terminal (failed or dead) to resume; current status is {}",
            input.run_id, state.status
        )));
    }
    if state.checkpoints.is_empty() {
        return Err(Error::Precondition(
            "run has no checkpoints to resume from — use retry instead".to_string(),
        ));
    }
    let target = resolve_failure_rewind_target(&state.checkpoints)?;
    rewind(
        store,
        &RewindInput {
            run_id: input.run_id,
            target,
        },
        actor,
    )
    .await
}

/// Auto-select the rewind target that re-runs exactly the last failed stage.
///
/// Each checkpoint commits its `current_node`'s outcome and the routing
/// decision (`next_node_id`). When the failed stage committed its own
/// checkpoint (soft-exit park), the target is the PREVIOUS checkpoint — the
/// failed stage's entry checkpoint — dropping the committed failure while
/// keeping every earlier checkpoint. When it never did (crash before
/// checkpoint), its entry checkpoint IS the latest checkpoint, so the default
/// rewind target (`None`) is correct.
pub fn resolve_failure_rewind_target(
    checkpoints: &[CheckpointRecord],
) -> Result<Option<ForkTarget>, Error> {
    for (index, record) in checkpoints.iter().enumerate().rev() {
        let failed = record
            .checkpoint
            .node_outcomes
            .get(&record.checkpoint.current_node)
            .is_some_and(|outcome| outcome.status.is_failure());
        if !failed {
            continue;
        }
        let node = &record.checkpoint.current_node;
        let Some(entry_index) = index.checked_sub(1) else {
            return Err(Error::Precondition(format!(
                "last failed stage '{node}' committed the first checkpoint; \
                 no earlier committed work to keep — use retry instead"
            )));
        };
        return Ok(Some(ForkTarget::Ordinal(entry_index + 1)));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::time::Duration;

    use fabro_graphviz::graph::Graph;
    use fabro_store::{Database, RunProjectionReducer};
    use fabro_types::{
        Checkpoint, DirtyStatus, FailureCategory, FailureDetail, FailureReason, GitContext,
        GitRunTarget, Outcome, RunFailure, RunTarget, StageTiming, WorkflowSettings, fixtures,
        test_support,
    };
    use object_store::memory::InMemory;

    use super::*;
    use crate::event::{self, Event};

    fn test_store() -> Database {
        fabro_store::test_support::test_database(
            Arc::new(InMemory::new()),
            "",
            Duration::from_millis(1),
            None,
        )
    }

    fn record(
        current_node: &str,
        completed: &[&str],
        next_node_id: Option<&str>,
        outcome: Outcome<Option<fabro_types::ModelUsage>>,
    ) -> CheckpointRecord {
        let mut node_outcomes: std::collections::HashMap<
            String,
            Outcome<Option<fabro_types::ModelUsage>>,
        > = std::collections::HashMap::new();
        node_outcomes.insert(current_node.to_string(), outcome);
        CheckpointRecord {
            seq:        0,
            checkpoint: Checkpoint {
                timestamp: chrono::Utc::now(),
                current_node: current_node.to_string(),
                completed_nodes: completed.iter().map(ToString::to_string).collect(),
                node_retries: std::collections::HashMap::new(),
                context_values: std::collections::HashMap::new(),
                node_outcomes,
                next_node_id: next_node_id.map(ToString::to_string),
                git_commit_sha: Some(format!("sha-{current_node}")),
                loop_failure_signatures: std::collections::HashMap::new(),
                restart_failure_signatures: std::collections::HashMap::new(),
                node_visits: std::collections::HashMap::new(),
            },
            diff:       fabro_types::RunDiff::default(),
        }
    }

    #[test]
    fn soft_exit_park_targets_the_failed_stages_entry_checkpoint() {
        let checkpoints = vec![
            record("analyze", &["analyze"], Some("file"), Outcome::success()),
            record(
                "file",
                &["analyze", "file"],
                Some("exit"),
                Outcome::fail("429"),
            ),
            record(
                "exit",
                &["analyze", "file", "exit"],
                None,
                Outcome::success(),
            ),
        ];

        let target = resolve_failure_rewind_target(&checkpoints).unwrap();
        assert_eq!(target, Some(ForkTarget::Ordinal(1)));
    }

    #[test]
    fn crash_before_checkpoint_defaults_to_latest_checkpoint() {
        // The failed stage never committed its own checkpoint; the latest
        // checkpoint IS its entry checkpoint.
        let checkpoints = vec![record(
            "analyze",
            &["analyze"],
            Some("file"),
            Outcome::success(),
        )];

        let target = resolve_failure_rewind_target(&checkpoints).unwrap();
        assert_eq!(target, None);
    }

    #[test]
    fn first_checkpoint_failure_has_no_entry_to_rewind_to() {
        let checkpoints = vec![record(
            "file",
            &["file"],
            Some("exit"),
            Outcome::fail("429"),
        )];

        let err = resolve_failure_rewind_target(&checkpoints).unwrap_err();
        assert!(err.to_string().contains("use retry instead"));
    }

    async fn seed_failed_run(
        store: &Database,
        run_id: RunId,
        park_after_failure: bool,
        terminal: bool,
    ) {
        let source = store.create_run(&run_id).await.unwrap();
        let graph = Graph::new("resume-source");
        let settings = WorkflowSettings::default();
        event::append_event(&source, &run_id, &Event::RunCreated {
            run_id,
            title: None,
            settings: serde_json::to_value(&settings).unwrap(),
            graph: serde_json::to_value(&graph).unwrap(),
            workflow_source: Some("digraph resume_source {}".to_string()),
            labels: BTreeMap::new(),
            source_directory: Some("/client/source".to_string()),
            workflow_slug: Some("resume-source".to_string()),
            workflow_version_id: Some(test_support::test_workflow_version_id()),
            target: Some(RunTarget::Git(GitRunTarget {
                repo:   "example/repo".to_string(),
                branch: "main".to_string(),
                tag:    None,
                sha:    None,
            })),
            automation: None,
            provenance: test_support::test_run_provenance(),
            spec_blob: None,
            git: Some(GitContext {
                origin_url: "https://github.com/example/repo".to_string(),
                branch:     "main".to_string(),
                sha:        None,
                dirty:      DirtyStatus::Clean,
            }),
            fork_source_ref: None,
            retried_from: None,
            parent_id: None,
            web_url: None,
        })
        .await
        .unwrap();

        event::append_event(&source, &run_id, &Event::StageCompleted {
            node_id: "analyze".to_string(),
            name: "Analyze".to_string(),
            index: 1,
            timing: StageTiming::wall_only(10),
            status: "succeeded".to_string(),
            preferred_label: None,
            suggested_next_ids: Vec::new(),
            usage_by_model: Vec::new(),
            usage: None,
            failure: None,
            notes: None,
            files_touched: Vec::new(),
            context_updates: None,
            jump_to_node: None,
            context_values: None,
            node_visits: None,
            loop_failure_signatures: None,
            restart_failure_signatures: None,
            response: Some("analyze findings".to_string()),
            attempt: 1,
            max_attempts: 1,
        })
        .await
        .unwrap();

        // Checkpoint 1: analyze succeeded, routed to file — the failed
        // stage's entry checkpoint. This must survive the resume fork.
        append_checkpoint(
            &source,
            &run_id,
            "analyze",
            &["analyze"],
            Some("file"),
            Outcome::success(),
        )
        .await;

        if park_after_failure {
            // Soft-exit park: the failed stage's outcome AND routing decision
            // committed, then the exit node completed its own checkpoint.
            append_checkpoint(
                &source,
                &run_id,
                "file",
                &["analyze", "file"],
                Some("exit"),
                Outcome::fail("429 exhausted"),
            )
            .await;
            append_checkpoint(
                &source,
                &run_id,
                "exit",
                &["analyze", "file", "exit"],
                None,
                Outcome::success(),
            )
            .await;
        }

        if terminal {
            crate::test_support::mark_run_running(&source, &run_id).await;
            event::append_event(&source, &run_id, &Event::WorkflowRunFailed {
                failure:              RunFailure {
                    reason: FailureReason::TransientInfra,
                    detail: FailureDetail::new("429 exhausted", FailureCategory::TransientInfra),
                },
                timing:               fabro_types::RunTiming::wall_only(600_000),
                final_git_commit_sha: None,
                final_patch:          None,
                diff_summary:         None,
                usage:                None,
            })
            .await
            .unwrap();
        }
    }

    async fn append_checkpoint(
        source: &fabro_store::RunDatabase,
        run_id: &RunId,
        current_node: &str,
        completed: &[&str],
        next_node_id: Option<&str>,
        outcome: Outcome<Option<fabro_types::ModelUsage>>,
    ) {
        let mut node_outcomes: BTreeMap<String, Outcome<Option<fabro_types::ModelUsage>>> =
            BTreeMap::new();
        node_outcomes.insert(current_node.to_string(), outcome.clone());
        event::append_event(source, run_id, &Event::CheckpointCompleted {
            graph_visit: None,
            resumed_from_stage_id: None,
            node_id: current_node.to_string(),
            status: outcome.status.to_string(),
            current_node: current_node.to_string(),
            completed_nodes: completed.iter().map(ToString::to_string).collect(),
            node_retries: BTreeMap::new(),
            context_values: BTreeMap::new(),
            node_outcomes,
            next_node_id: next_node_id.map(ToString::to_string),
            git_commit_sha: Some(format!("sha-{current_node}")),
            loop_failure_signatures: BTreeMap::new(),
            restart_failure_signatures: BTreeMap::new(),
            node_visits: BTreeMap::new(),
            diff: None,
            diff_summary: None,
        })
        .await
        .unwrap();
    }

    async fn projection_of(store: &Database, run_id: RunId) -> fabro_store::RunProjection {
        let run_store = store.open_run(&run_id).await.unwrap();
        let events = run_store.list_events().await.unwrap();
        fabro_store::RunProjection::apply_events(&events).unwrap()
    }

    #[tokio::test]
    async fn resume_from_failure_re_runs_only_the_failed_node_after_soft_exit_park() {
        let store = test_store();
        let source_run_id = fixtures::RUN_1;
        seed_failed_run(&store, source_run_id, true, true).await;

        let outcome = resume_from_failure(
            &store,
            &ResumeFailureInput {
                run_id: source_run_id,
            },
            None,
        )
        .await
        .unwrap();

        // Engine auto-selected the failed stage's ENTRY checkpoint (@1,
        // analyze), dropping the file/exit checkpoints that committed the
        // failure and the park routing.
        assert_eq!(outcome.target().checkpoint_ordinal, 1);
        assert_eq!(outcome.target().node_id, "analyze");

        let forked = projection_of(&store, outcome.new_run_id()).await;
        assert_eq!(forked.checkpoints.len(), 1);
        let entry = &forked.checkpoints[0].checkpoint;
        assert_eq!(entry.current_node, "analyze");
        // The routing decision re-enters the failed node.
        assert_eq!(entry.next_node_id.as_deref(), Some("file"));
        // Prior committed stage outcome survives the fork.
        assert!(
            entry
                .node_outcomes
                .get("analyze")
                .is_some_and(|o| o.status.is_successful())
        );
        assert!(entry.completed_nodes.contains(&"analyze".to_string()));

        // Source run was archived and superseded.
        let source = projection_of(&store, source_run_id).await;
        assert!(source.archived_at.is_some());
        assert_eq!(source.superseded_by, Some(outcome.new_run_id()));
    }

    #[tokio::test]
    async fn resume_from_failure_defaults_to_latest_checkpoint_when_crash_preceded_checkpoint() {
        let store = test_store();
        let source_run_id = fixtures::RUN_1;
        seed_failed_run(&store, source_run_id, false, true).await;

        let outcome = resume_from_failure(
            &store,
            &ResumeFailureInput {
                run_id: source_run_id,
            },
            None,
        )
        .await
        .unwrap();

        // No failure checkpoint exists: the latest checkpoint (@1) is the
        // failed stage's entry checkpoint.
        assert_eq!(outcome.target().checkpoint_ordinal, 1);
        let forked = projection_of(&store, outcome.new_run_id()).await;
        assert_eq!(forked.checkpoints.len(), 1);
        assert_eq!(
            forked.checkpoints[0].checkpoint.next_node_id.as_deref(),
            Some("file")
        );
    }

    #[tokio::test]
    async fn resume_from_failure_rejects_archived_and_non_terminal_runs() {
        let store = test_store();
        let archived_id = fixtures::RUN_1;
        seed_failed_run(&store, archived_id, true, true).await;
        archive::archive(&store, &archived_id, None).await.unwrap();
        let err = resume_from_failure(
            &store,
            &ResumeFailureInput {
                run_id: archived_id,
            },
            None,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("archived"));

        let active_id = RunId::new();
        seed_failed_run(&store, active_id, true, false).await;
        let err = resume_from_failure(&store, &ResumeFailureInput { run_id: active_id }, None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("must be terminal"));
    }
}
