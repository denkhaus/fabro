//! Fork-only staleness-supervisor tests (extracted from `server/tests.rs`
//! per fabro-ab8e, user directive 2026-09-18): the update-branch / cap /
//! age-close lifecycle over run-linked pull requests is fork-added behavior
//! (`server/pull_request_supervisor.rs`). Keeping the tests in a fork-owned
//! file means an upstream merge can never silently drop them, and the
//! upstream-owned `tests.rs` monolith stays structurally upstream-identical.

use std::sync::Arc;

use chrono::{Duration as ChronoDuration, Utc};
use fabro_store::platform_records::{PlatformRecord, PullRequestLinkedRecord};
use fabro_types::RunId;
use httpmock::MockServer;
use serde_json::json;

use super::fork_staleness_supervisor::{self, StalePrState};
use super::tests::create_github_token_app_state;
use super::{run_records, *};

/// Fork-seeding (petri): a run whose platform records link PR and whose
/// stored projection carries the link — the supervisor's two read paths.
async fn create_run_with_linked_pull_request_record(
    state: &Arc<AppState>,
    run_id: RunId,
    pull_request: fabro_types::PullRequestLink,
) {
    run_records::append(
        state,
        run_id,
        PlatformRecord::PullRequestLinked(PullRequestLinkedRecord {
            owner:  pull_request.owner.clone(),
            repo:   pull_request.repo.clone(),
            number: pull_request.number,
        }),
    )
    .await
    .expect("seed pull request link record");
    // Let a projector pass the append may have woken finish, then pin the
    // view directly: the wire tests exercise the supervisor's READ paths
    // (candidates from platform records, link from the projection), not
    // the fold itself.
    state.petri_projector.settle(run_id).await;
    let projection = linked_pr_projection(run_id, pull_request);
    let projection_json = serde_json::to_string(&projection).expect("projection json");
    let pool = state.stores.run_summaries.pool();
    let mut tx = pool.begin().await.expect("seed tx");
    fabro_store::RunSummaryStore::write_petri_run_row_on_connection(&mut tx, &run_id, &projection)
        .await
        .expect("seed run row");
    sqlx::query(
        "INSERT INTO petri_projection (run_id, projection_json, fold_json, positions_json,          stream_seq, updated_at_ms) VALUES (?, ?, '{}', '{}', 0, 0)          ON CONFLICT(run_id) DO UPDATE SET projection_json = excluded.projection_json",
    )
    .bind(run_id.to_string())
    .bind(projection_json)
    .execute(&mut *tx)
    .await
    .expect("seed projection row");
    tx.commit().await.expect("seed commit");
}

fn linked_pr_projection(
    run_id: RunId,
    pull_request: fabro_types::PullRequestLink,
) -> fabro_types::RunProjection {
    use std::collections::HashMap;
    let mut projection = fabro_types::RunProjection::new(
        "stale pr".to_string(),
        fabro_types::RunSpec {
            run_id,
            settings: fabro_types::WorkflowSettings::default(),
            graph: fabro_types::RunGraph::new("test"),
            graph_source: None,
            workflow_slug: None,
            workflow_version_id: None,
            target: None,
            automation: None,
            source_directory: None,
            labels: HashMap::new(),
            provenance: fabro_types::test_support::test_run_provenance(),
            definition_blob: None,
            spec_blob: None,
            git: None,
            fork_source_ref: None,
            admission: fabro_types::PetriAdmission::default(),
        },
        chrono::Utc::now(),
    );
    projection.pull_request = Some(pull_request);
    projection
}

/// Fork wire app (petri): GitHub-token state against a mocked API.
fn pr_test_app(token: Option<&str>, github_api_base_url: Option<String>) -> (Arc<AppState>, RunId) {
    let state = create_github_token_app_state(token, github_api_base_url);
    (state, RunId::new())
}

// ---------------------------------------------------------------------------
// Staleness supervisor (fabro-94e8): update-branch on dirty run PRs, cap/age
// close with a durable pull_request.closed event.
// ---------------------------------------------------------------------------

/// Wire fixtures for one staleness pass over a run-linked PR #42.
fn dirty_run_pull_request_mock<'a>(
    github: &'a MockServer,
    pr_state: &str,
    mergeable: bool,
    mergeable_state: &str,
    created_at: &str,
) -> httpmock::Mock<'a> {
    github.mock(|when, then| {
        when.method("GET")
            .path("/repos/acme/widgets/pulls/42")
            .header("authorization", "Bearer ghu_test");
        then.status(200).json_body(json!({
            "number": 42,
            "title": "Ship it",
            "body": "",
            "state": pr_state,
            "draft": false,
            "merged": false,
            "mergeable": mergeable,
            "mergeable_state": mergeable_state,
            "additions": 1,
            "deletions": 0,
            "changed_files": 1,
            "html_url": "https://github.com/acme/widgets/pull/42",
            "user": { "login": "octocat" },
            "head": { "ref": "fabro/run/42" },
            "base": { "ref": "main" },
            "created_at": created_at,
            "updated_at": created_at
        }));
    })
}

fn update_branch_mock(github: &MockServer, status: u16) -> httpmock::Mock<'_> {
    github.mock(move |when, then| {
        when.method("PUT")
            .path("/repos/acme/widgets/pulls/42/update-branch")
            .header("authorization", "Bearer ghu_test");
        then.status(status)
            .json_body(json!({ "message": "Updating pull request branch." }));
    })
}

fn close_branch_mock(github: &MockServer) -> httpmock::Mock<'_> {
    github.mock(|when, then| {
        when.method("PATCH")
            .path("/repos/acme/widgets/pulls/42")
            .header("authorization", "Bearer ghu_test")
            .json_body(json!({ "state": "closed" }));
        then.status(200).json_body(json!({
            "number": 42,
            "state": "closed",
        }));
    })
}

/// Files changed on one side of a `compare/{base}...{head}` (fabro-895d).
fn compare_files_mock<'a>(
    github: &'a MockServer,
    basehead: &str,
    filenames: &[&str],
) -> httpmock::Mock<'a> {
    let files: Vec<_> = filenames
        .iter()
        .map(|filename| json!({ "filename": filename }))
        .collect();
    github.mock(move |when, then| {
        when.method("GET")
            .path(format!("/repos/acme/widgets/compare/{basehead}"))
            .header("authorization", "Bearer ghu_test");
        then.status(200).json_body(json!({
            "status": "diverged",
            "files": files,
        }));
    })
}

/// Files changed by the pull request (age-cap bookkeeping check, fabro-895d).
fn pr_files_mock<'a>(
    github: &'a MockServer,
    number: u64,
    filenames: &[&str],
) -> httpmock::Mock<'a> {
    let files: Vec<_> = filenames
        .iter()
        .map(|filename| json!({ "filename": filename }))
        .collect();
    github.mock(move |when, then| {
        when.method("GET")
            .path(format!("/repos/acme/widgets/pulls/{number}/files"))
            .header("authorization", "Bearer ghu_test");
        then.status(200).json_body(files);
    })
}

/// Raw file content served by the contents API (fabro-895d).
fn raw_contents_mock<'a>(
    github: &'a MockServer,
    path: &str,
    git_ref: &str,
    body: &str,
) -> httpmock::Mock<'a> {
    github.mock(move |when, then| {
        when.method("GET")
            .path(format!("/repos/acme/widgets/contents/{path}"))
            .query_param("ref", git_ref)
            .header("authorization", "Bearer ghu_test");
        then.status(200).body(body);
    })
}

/// `GET /repos/.../commits/{git_ref}` answering commit sha and tree sha.
fn commit_info_mock<'a>(
    github: &'a MockServer,
    git_ref: &str,
    sha: &str,
    tree: &str,
) -> httpmock::Mock<'a> {
    github.mock(move |when, then| {
        when.method("GET")
            .path(format!("/repos/acme/widgets/commits/{git_ref}"))
            .header("authorization", "Bearer ghu_test");
        then.status(200).json_body(json!({
            "sha": sha,
            "commit": { "tree": { "sha": tree } },
        }));
    })
}

/// `POST /repos/.../git/{suffix}` (blobs, trees, commits) answering a sha.
fn git_object_create_mock<'a>(
    github: &'a MockServer,
    suffix: &str,
    response_sha: &str,
) -> httpmock::Mock<'a> {
    github.mock(move |when, then| {
        when.method("POST")
            .path(format!("/repos/acme/widgets/{suffix}"))
            .header("authorization", "Bearer ghu_test");
        then.status(201).json_body(json!({ "sha": response_sha }));
    })
}

/// Fast-forward push of the run branch head (git refs API, fabro-895d).
fn push_ref_mock<'a>(github: &'a MockServer, branch: &str, status: u16) -> httpmock::Mock<'a> {
    github.mock(move |when, then| {
        when.method("PATCH")
            .path(format!("/repos/acme/widgets/git/refs/heads/{branch}"))
            .header("authorization", "Bearer ghu_test")
            .json_body(json!({ "sha": "m1", "force": false }));
        then.status(status)
            .json_body(json!({ "object": { "sha": "m1" } }));
    })
}

async fn staleness_test_run(state: &Arc<AppState>, run_id: RunId) {
    create_run_with_linked_pull_request_record(state, run_id, PullRequestLink {
        owner:  "acme".to_string(),
        repo:   "widgets".to_string(),
        number: 42,
    })
    .await;
}

/// Petri: closing records a `pull_request.unlinked` platform record (the
/// engine-era `PullRequestClosed` event has no petri equivalent).
async fn run_pull_request_unlinked_records(
    state: &AppState,
    run_id: &RunId,
) -> Vec<PullRequestLinkedRecord> {
    state
        .stores
        .run_summaries
        .platform_records()
        .read(run_id)
        .await
        .unwrap()
        .into_iter()
        .filter_map(|stored| match stored.record {
            PlatformRecord::PullRequestUnlinked(record) => Some(record),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn staleness_supervisor_updates_dirty_run_pull_request_branch() {
    let github = MockServer::start();
    let created_at = Utc::now().to_rfc3339();
    let _get_mock = dirty_run_pull_request_mock(&github, "open", false, "dirty", &created_at);
    let update_mock = update_branch_mock(&github, 202);
    let close_mock = close_branch_mock(&github);
    let (state, run_id) = pr_test_app(Some("ghu_test"), Some(github.base_url()));
    staleness_test_run(&state, run_id).await;

    let mut counters = StalePrState::default();
    fork_staleness_supervisor::process_stale_pull_requests(&state, &mut counters)
        .await
        .unwrap();

    update_mock.assert();
    assert_eq!(
        close_mock.calls(),
        0,
        "a successful update must not close the PR"
    );
    assert!(
        counters.update_failures.is_empty(),
        "a successful update resets the failure counter"
    );
    assert!(
        run_pull_request_unlinked_records(&state, &run_id)
            .await
            .is_empty(),
        "no close event may be emitted on a successful update"
    );
}

#[tokio::test]
async fn staleness_supervisor_keeps_pr_open_on_conflict_and_counts_the_failure() {
    let github = MockServer::start();
    let created_at = Utc::now().to_rfc3339();
    let _get_mock = dirty_run_pull_request_mock(&github, "open", false, "dirty", &created_at);
    let update_mock = update_branch_mock(&github, 422);
    let close_mock = close_branch_mock(&github);
    // Mixed conflict (fabro-895d): the conflict intersection includes a code
    // file outside the loop's tracker paths, so the supervisor must strike.
    let _head_compare = compare_files_mock(&github, "main...fabro/run/42", &[
        ".seeds/issues.jsonl",
        "lib/foo.rs",
    ]);
    let _base_compare = compare_files_mock(&github, "fabro/run/42...main", &["lib/foo.rs"]);
    let push_mock = push_ref_mock(&github, "fabro/run/42", 200);
    let (state, run_id) = pr_test_app(Some("ghu_test"), Some(github.base_url()));
    staleness_test_run(&state, run_id).await;

    let mut counters = StalePrState::default();
    fork_staleness_supervisor::process_stale_pull_requests(&state, &mut counters)
        .await
        .unwrap();

    update_mock.assert();
    assert_eq!(
        close_mock.calls(),
        0,
        "a 422 conflict must not close the PR"
    );
    assert_eq!(
        push_mock.calls(),
        0,
        "a mixed JSONL+code conflict must not be auto-resolved"
    );
    assert_eq!(
        counters.update_failures.get(&run_id),
        Some(&1),
        "the conflict counts as one failed attempt"
    );
    assert!(
        run_pull_request_unlinked_records(&state, &run_id)
            .await
            .is_empty(),
        "no close event may be emitted on conflict"
    );
    // The stored link stays: the wait endpoint keeps observing the open PR and
    // can only answer closed_unmerged/timeout, routing the conductor manual.
    let projection = state
        .stores
        .runs
        .load_run_projection(&run_id)
        .await
        .unwrap();
    assert!(
        projection
            .expect("run projection should load")
            .pull_request
            .is_some()
    );
}

#[tokio::test]
async fn staleness_supervisor_closes_pr_after_three_failed_updates() {
    let github = MockServer::start();
    let created_at = Utc::now().to_rfc3339();
    let _get_mock = dirty_run_pull_request_mock(&github, "open", false, "dirty", &created_at);
    let update_mock = update_branch_mock(&github, 422);
    let close_mock = close_branch_mock(&github);
    let (state, run_id) = pr_test_app(Some("ghu_test"), Some(github.base_url()));
    staleness_test_run(&state, run_id).await;

    let mut counters = StalePrState::default();
    counters.update_failures.insert(run_id, 3);
    fork_staleness_supervisor::process_stale_pull_requests(&state, &mut counters)
        .await
        .unwrap();

    close_mock.assert();
    assert_eq!(
        update_mock.calls(),
        0,
        "a capped PR is closed without another update attempt"
    );
    let closed = run_pull_request_unlinked_records(&state, &run_id).await;
    assert_eq!(
        closed,
        vec![PullRequestLinkedRecord {
            owner:  "acme".to_string(),
            repo:   "widgets".to_string(),
            number: 42,
        }],
        "the close must be recorded as a pull_request.unlinked platform record"
    );
    assert!(
        !counters.update_failures.contains_key(&run_id),
        "retiring the PR clears its counter"
    );
    // Petri: the close appends a pull_request.unlinked platform record;
    // the projector folds it asynchronously. Drive the fold and read the
    // settled view.
    state.petri_projector.settle(run_id).await;
    let unlinked = run_pull_request_unlinked_records(&state, &run_id).await;
    assert_eq!(
        unlinked.len(),
        1,
        "the close must be recorded as a pull_request.unlinked platform record"
    );
}

#[tokio::test]
async fn staleness_supervisor_closes_pr_older_than_24h() {
    let github = MockServer::start();
    let created_at = (Utc::now() - ChronoDuration::hours(25)).to_rfc3339();
    let _get_mock = dirty_run_pull_request_mock(&github, "open", false, "dirty", &created_at);
    // Over-age PRs carrying non-bookkeeping changes keep closing (fabro-895d).
    let _files_mock = pr_files_mock(&github, 42, &["lib/foo.rs"]);
    let update_mock = update_branch_mock(&github, 202);
    let close_mock = close_branch_mock(&github);
    let (state, run_id) = pr_test_app(Some("ghu_test"), Some(github.base_url()));
    staleness_test_run(&state, run_id).await;

    let mut counters = StalePrState::default();
    fork_staleness_supervisor::process_stale_pull_requests(&state, &mut counters)
        .await
        .unwrap();

    close_mock.assert();
    assert_eq!(
        update_mock.calls(),
        0,
        "an over-age PR is closed without an update attempt"
    );
    let closed = run_pull_request_unlinked_records(&state, &run_id).await;
    assert_eq!(
        closed,
        vec![PullRequestLinkedRecord {
            owner:  "acme".to_string(),
            repo:   "widgets".to_string(),
            number: 42,
        }],
        "the age close must be recorded as a pull_request.unlinked platform record"
    );
}

#[tokio::test]
async fn staleness_supervisor_skips_clean_and_blocked_run_pull_requests() {
    let github = MockServer::start();
    let created_at = Utc::now().to_rfc3339();
    let _clean_mock = dirty_run_pull_request_mock(&github, "open", true, "clean", &created_at);
    let update_mock = update_branch_mock(&github, 202);
    let close_mock = close_branch_mock(&github);
    let (state, run_id) = pr_test_app(Some("ghu_test"), Some(github.base_url()));
    staleness_test_run(&state, run_id).await;

    let mut counters = StalePrState::default();
    counters.update_failures.insert(run_id, 2);
    fork_staleness_supervisor::process_stale_pull_requests(&state, &mut counters)
        .await
        .unwrap();

    assert_eq!(
        update_mock.calls(),
        0,
        "a clean PR must not be update-branched"
    );
    assert_eq!(close_mock.calls(), 0, "a clean PR must not be closed");
    assert!(
        !counters.update_failures.contains_key(&run_id),
        "a clean PR resets its counter"
    );
}

#[tokio::test]
async fn staleness_supervisor_resolves_jsonl_only_conflict_without_strike() {
    let github = MockServer::start();
    let created_at = Utc::now().to_rfc3339();
    let _get_mock = dirty_run_pull_request_mock(&github, "open", false, "dirty", &created_at);
    let update_mock = update_branch_mock(&github, 422);
    let close_mock = close_branch_mock(&github);
    // Conflict intersection is the tracker JSONL only: the head also added a
    // fresh code file, but the base moved only `.seeds/issues.jsonl`.
    let _head_compare = compare_files_mock(&github, "main...fabro/run/42", &[
        ".seeds/issues.jsonl",
        "lib/new.rs",
    ]);
    let _base_compare =
        compare_files_mock(&github, "fabro/run/42...main", &[".seeds/issues.jsonl"]);
    let base_tracker = concat!(
        "{\"id\":\"fabro-1\",\"status\":\"closed\"}\n",
        "{\"id\":\"fabro-2\",\"status\":\"open\"}\n",
    );
    let head_tracker = concat!(
        // stale pre-fork copy that base has since closed
        "{\"id\":\"fabro-1\",\"status\":\"open\"}\n",
        // branch-side update of an open record
        "{\"id\":\"fabro-2\",\"status\":\"in_progress\"}\n",
        // branch-only record
        "{\"id\":\"fabro-9\",\"status\":\"open\"}\n",
    );
    let _base_contents = raw_contents_mock(&github, ".seeds/issues.jsonl", "main", base_tracker);
    let _head_contents =
        raw_contents_mock(&github, ".seeds/issues.jsonl", "fabro/run/42", head_tracker);
    // Diff-based publish (fabro-4ebd): the run's own non-tracker change is
    // carried onto the merge tree from the head side, by content.
    let _head_code_contents =
        raw_contents_mock(&github, "lib/new.rs", "fabro/run/42", "fn new() {}\n");
    let _head_commit = commit_info_mock(&github, "fabro/run/42", "h1", "th");
    let _base_commit = commit_info_mock(&github, "main", "b1", "tb");
    // The stored blob must carry the closed-wins union. The request body is
    // JSON-escaped, so the matchers use the escaped form: base-closed fabro-1
    // stays closed, the head's in_progress update wins, and the branch-only
    // fabro-9 record survives.
    let blob_mock = github.mock(|when, then| {
        when.method("POST")
            .path("/repos/acme/widgets/git/blobs")
            .header("authorization", "Bearer ghu_test")
            .body_includes("fabro-1\\\",\\\"status\\\":\\\"closed")
            .body_includes("fabro-2\\\",\\\"status\\\":\\\"in_progress")
            .body_includes("fabro-9");
        then.status(201).json_body(json!({ "sha": "blob1" }));
    });
    let _code_blob_mock = github.mock(|when, then| {
        when.method("POST")
            .path("/repos/acme/widgets/git/blobs")
            .header("authorization", "Bearer ghu_test")
            .body_includes("fn new() {}");
        then.status(201).json_body(json!({ "sha": "blob-code" }));
    });
    // The merge tree must grow from the CURRENT base tree ("tb"), not the
    // run's (possibly stale) head tree — the base's concurrent work survives
    // verbatim (fabro-4ebd (b)).
    let _tree_mock = github.mock(|when, then| {
        when.method("POST")
            .path("/repos/acme/widgets/git/trees")
            .header("authorization", "Bearer ghu_test")
            .body_includes("\"base_tree\":\"tb\"");
        then.status(201).json_body(json!({ "sha": "t2" }));
    });
    let _merge_commit_mock = git_object_create_mock(&github, "git/commits", "m1");
    let push_mock = push_ref_mock(&github, "fabro/run/42", 200);
    let (state, run_id) = pr_test_app(Some("ghu_test"), Some(github.base_url()));
    staleness_test_run(&state, run_id).await;

    let mut counters = StalePrState::default();
    fork_staleness_supervisor::process_stale_pull_requests(&state, &mut counters)
        .await
        .unwrap();

    update_mock.assert();
    blob_mock.assert();
    push_mock.assert();
    assert_eq!(
        close_mock.calls(),
        0,
        "a resolved JSONL conflict must not close the PR"
    );
    assert!(
        counters.update_failures.is_empty(),
        "a resolved JSONL conflict resets the strike counter"
    );
    assert!(
        counters.parked.is_empty(),
        "a resolved JSONL conflict must not park the PR"
    );
    assert!(
        run_pull_request_unlinked_records(&state, &run_id)
            .await
            .is_empty(),
        "no close event may be emitted on a resolved conflict"
    );
}

#[tokio::test]
async fn staleness_supervisor_parks_pr_when_jsonl_resolution_fails() {
    let github = MockServer::start();
    let created_at = Utc::now().to_rfc3339();
    let _get_mock = dirty_run_pull_request_mock(&github, "open", false, "dirty", &created_at);
    let update_mock = update_branch_mock(&github, 422);
    let close_mock = close_branch_mock(&github);
    let _head_compare =
        compare_files_mock(&github, "main...fabro/run/42", &[".seeds/issues.jsonl"]);
    let _base_compare =
        compare_files_mock(&github, "fabro/run/42...main", &[".seeds/issues.jsonl"]);
    let _base_contents = raw_contents_mock(
        &github,
        ".seeds/issues.jsonl",
        "main",
        "{\"id\":\"fabro-1\",\"status\":\"open\"}\n",
    );
    // Unparseable head-side tracker state: the union cannot be computed.
    let _head_contents = raw_contents_mock(
        &github,
        ".seeds/issues.jsonl",
        "fabro/run/42",
        "not jsonl at all\n",
    );
    let blob_mock = git_object_create_mock(&github, "git/blobs", "blob1");
    let push_mock = push_ref_mock(&github, "fabro/run/42", 200);
    let (state, run_id) = pr_test_app(Some("ghu_test"), Some(github.base_url()));
    staleness_test_run(&state, run_id).await;

    let mut counters = StalePrState::default();
    fork_staleness_supervisor::process_stale_pull_requests(&state, &mut counters)
        .await
        .unwrap();

    assert_eq!(blob_mock.calls(), 0, "no merge state may be written");
    assert_eq!(push_mock.calls(), 0, "no push may be attempted");
    assert_eq!(
        close_mock.calls(),
        0,
        "a failed JSONL resolution must park, not close"
    );
    assert!(
        counters.parked.contains_key(&run_id),
        "the PR must be parked after a failed resolution"
    );
    assert!(
        !counters.update_failures.contains_key(&run_id),
        "parking does not count a strike"
    );
    assert!(
        run_pull_request_unlinked_records(&state, &run_id)
            .await
            .is_empty(),
        "no close event may be emitted when parking"
    );

    // A parked PR receives no further update attempts on later passes.
    let update_calls = update_mock.calls();
    fork_staleness_supervisor::process_stale_pull_requests(&state, &mut counters)
        .await
        .unwrap();
    assert_eq!(
        update_mock.calls(),
        update_calls,
        "a parked PR is skipped entirely on the next pass"
    );
}

#[tokio::test]
async fn staleness_supervisor_parks_over_age_bookkeeping_only_pr() {
    let github = MockServer::start();
    let created_at = (Utc::now() - ChronoDuration::hours(25)).to_rfc3339();
    let _get_mock = dirty_run_pull_request_mock(&github, "open", false, "dirty", &created_at);
    // Over-age PR whose changed paths are all dev-loop bookkeeping
    // (tracker files plus stage journals) parks instead of closing
    // (fabro-895d).
    let _files_mock = pr_files_mock(&github, 42, &[
        ".seeds/issues.jsonl",
        ".mulch/mulch.config.yaml",
        ".fabro/journal/run.jsonl",
    ]);
    let update_mock = update_branch_mock(&github, 202);
    let close_mock = close_branch_mock(&github);
    let (state, run_id) = pr_test_app(Some("ghu_test"), Some(github.base_url()));
    staleness_test_run(&state, run_id).await;

    let mut counters = StalePrState::default();
    fork_staleness_supervisor::process_stale_pull_requests(&state, &mut counters)
        .await
        .unwrap();

    assert_eq!(
        close_mock.calls(),
        0,
        "an over-age bookkeeping-only PR must park, not close"
    );
    assert_eq!(
        update_mock.calls(),
        0,
        "an over-age PR is not update-branched"
    );
    assert!(
        counters.parked.contains_key(&run_id),
        "the over-age bookkeeping-only PR must be parked"
    );
    assert!(
        run_pull_request_unlinked_records(&state, &run_id)
            .await
            .is_empty(),
        "no close event may be emitted when parking"
    );
    let projection = state
        .stores
        .runs
        .load_run_projection(&run_id)
        .await
        .unwrap();
    assert!(
        projection
            .expect("run projection should load")
            .pull_request
            .is_some(),
        "the parked PR stays linked to the run"
    );
}
