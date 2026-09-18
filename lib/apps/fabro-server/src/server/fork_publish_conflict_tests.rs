//! FORK-ONLY PRESENCE PIN (fabro-4ebd, PR #221; backfilled by fabro-ec00):
//! diff-based publish squash-revert protection. A publish from a possibly
//! stale run workspace must build its merge tree from the run's OWN diff,
//! never from the workspace tree — so commits that landed on the base
//! branch after the workspace snapshot are carried by the base tree and
//! can never be reverted by the publish (proven live 2026-09-17 when
//! ca7449c5b..36b2f9f54 survived a racing publish).
//!
//! The seam spans `server/pull_request_conflict.rs` (run-scoped tree
//! changes, JSONL union, git-data-API merge commit),
//! `server/pull_request_supervisor.rs` (resolution driver),
//! `fabro-workflow pipeline/pull_request.rs` (out-of-scope merge gate), and
//! `fabro-github` (`PullRequestFileStatus` + file-status listing). This
//! file exists only on our fork so a merge resolution can never silently
//! drop the protection's tests (the 00ffd60f6 incident class). Registry
//! row: `.agents/skills/merge-upstream/references/touchpoints.md`
//! ("Diff-based publish squash-revert protection").

#[cfg(test)]
mod tests {
    use fabro_github::PullRequestFileStatus;

    use super::super::pull_request_conflict::{
        CompareEntry, TreeChange, run_scoped_tree_changes, union_jsonl_closed_wins,
    };

    /// The publish merge tree is scoped to the run's own compare entries:
    /// a path that landed on base AFTER the workspace snapshot (absent
    /// from the run's diff) gets no tree change at all, so the base tree
    /// — which already contains it — survives the publish untouched. This
    /// is the exact stale-workspace squash-revert shape PR #221 fixed.
    #[test]
    fn stale_workspace_publish_never_reverts_concurrently_landed_base_commits() {
        // The run's own diff: one modified tracker file, one added source
        // file. `lib/base_landed_after_snapshot.rs` is NOT in it — it was
        // committed to the base branch after this workspace was created.
        let head_entries = vec![
            CompareEntry {
                filename:          ".seeds/issues.jsonl".to_string(),
                status:            "modified".to_string(),
                previous_filename: None,
            },
            CompareEntry {
                filename:          "lib/run_change.rs".to_string(),
                status:            "added".to_string(),
                previous_filename: None,
            },
        ];

        let changes = run_scoped_tree_changes(&head_entries);
        assert_eq!(changes, vec![
            TreeChange::Set(".seeds/issues.jsonl".to_string()),
            TreeChange::Set("lib/run_change.rs".to_string()),
        ]);
        // The concurrently landed base path has no entry: the base tree in
        // create_merge_commit keeps it, and no Remove ever targets it.
        assert!(!changes.iter().any(|change| match change {
            TreeChange::Set(path) | TreeChange::Remove(path) => {
                path == "lib/base_landed_after_snapshot.rs"
            }
        }));
    }

    /// The publish path's tracker union keeps records that landed on base
    /// after the workspace snapshot: base-only records survive the union
    /// and closed still wins — a stale open copy on the publish side can
    /// never resurrect a record the base already closed.
    #[test]
    fn publish_union_keeps_base_records_landed_after_the_workspace_snapshot() {
        let base = concat!(
            "{\"id\":\"fabro-1\",\"status\":\"closed\",\"updatedAt\":\"2026-09-17T10:00:00Z\"}\n",
            "{\"id\":\"fabro-2\",\"status\":\"open\"}\n",
        );
        // The stale workspace side predates fabro-2 landing on base.
        let head =
            "{\"id\":\"fabro-1\",\"status\":\"open\",\"updatedAt\":\"2026-09-17T09:00:00Z\"}\n";

        let merged = union_jsonl_closed_wins(base, head).expect("union should succeed");
        let lines: Vec<&str> = merged.lines().collect();
        assert_eq!(lines, vec![
            // base closed record survives the stale open copy
            "{\"id\":\"fabro-1\",\"status\":\"closed\",\"updatedAt\":\"2026-09-17T10:00:00Z\"}",
            // base-only record kept — never dropped by the publish
            "{\"id\":\"fabro-2\",\"status\":\"open\"}",
        ]);
    }

    /// Compile-level presence pin for the seam's file-status type
    /// (`fabro-github::PullRequestFileStatus`, PR #221): the run-scoped
    /// diff this file pins is derived from these statuses, so the type and
    /// its shape must survive every upstream merge.
    #[test]
    fn pull_request_file_status_shape_survives_merges() {
        let status = PullRequestFileStatus {
            filename:          "lib/run_change.rs".to_string(),
            status:            "modified".to_string(),
            previous_filename: Some("lib/renamed_from.rs".to_string()),
        };
        assert_eq!(status.filename, "lib/run_change.rs");
        assert_eq!(status.status, "modified");
        assert_eq!(
            status.previous_filename.as_deref(),
            Some("lib/renamed_from.rs")
        );
    }
}
