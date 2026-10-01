//! The seeds read API (fabro-3488, ADR-0023 step 5): list, detail, and
//! dependency graph over a `SeedsSource`, plus the unconfigured `503`,
//! and the production `GitRepoCache` mirror (fork decision B) with its
//! served-ref invalidation contract.
//!
//! Fork-only presence pin: upstream has no seeds endpoints, so a merge
//! that drops the read API reds here instead of silently stripping it.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use fabro_types::settings::server::SeedsMirrorSettings;
use seeds::Store;
use serde_json::Value;
use tower::ServiceExt;

use crate::server::fork_seeds_git_source::GitSeedsSource;
use crate::server::seeds_source::{
    DisabledSeedsSource, SeedsSnapshot, SeedsSource, SeedsSourceError,
};
use crate::test_support::{TestAppStateBuilder, build_test_router};

/// A source serving one fixed snapshot from a `.seeds` directory.
struct FixtureSeedsSource {
    snapshot: SeedsSnapshot,
}

#[async_trait::async_trait]
impl SeedsSource for FixtureSeedsSource {
    async fn snapshot(&self) -> Result<SeedsSnapshot, SeedsSourceError> {
        Ok(SeedsSnapshot {
            store:  Arc::clone(&self.snapshot.store),
            commit: self.snapshot.commit.clone(),
        })
    }
}

/// A source whose configured checkout cannot be read.
struct FailingSeedsSource;

#[async_trait::async_trait]
impl SeedsSource for FailingSeedsSource {
    async fn snapshot(&self) -> Result<SeedsSnapshot, SeedsSourceError> {
        Err(SeedsSourceError::Unavailable(
            "fixture: checkout gone".into(),
        ))
    }
}

/// Write a small `.seeds` store: two linked seeds plus one edge that
/// points at a seed that does not exist (the graph must drop it).
#[expect(
    clippy::disallowed_methods,
    reason = "test fixture writes a small .seeds store with sync std::fs::write"
)]
fn fixture_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join(".seeds");
    std::fs::create_dir(&root).expect("seeds dir");
    std::fs::write(
        root.join("config.yaml"),
        "project: \"fabro\"\nversion: \"1\"\n",
    )
    .expect("config");
    std::fs::write(
        root.join("issues.jsonl"),
        concat!(
            "{\"id\":\"fabro-0001\",\"title\":\"First\",\"status\":\"open\",\"type\":\"task\",\"priority\":2,",
            "\"assignee\":\"fabro\",\"labels\":[\"handoff\"],\"blockedBy\":[\"fabro-0002\",\"fabro-9999\"],",
            "\"createdAt\":\"2026-09-23T08:00:00Z\",\"updatedAt\":\"2026-09-23T09:00:00Z\"}\n",
            "{\"id\":\"fabro-0002\",\"title\":\"Second\",\"status\":\"closed\",\"type\":\"bug\",\"blockedBy\":[]}\n",
        ),
    )
    .expect("issues");
    dir
}

fn router_with(source: Arc<dyn SeedsSource>) -> axum::Router {
    let state = TestAppStateBuilder::new().seeds_source(source).build();
    build_test_router(state)
}

async fn get_json(router: &axum::Router, uri: &str) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(uri)
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router answers");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body reads");
    let json = serde_json::from_slice(&bytes).expect("body is JSON");
    (status, json)
}

#[tokio::test]
async fn seeds_list_serves_summaries_with_filters_and_paging() {
    let dir = fixture_dir();
    let store = Arc::new(Store::open(dir.path().join(".seeds")).expect("store opens"));
    let router = router_with(Arc::new(FixtureSeedsSource {
        snapshot: SeedsSnapshot {
            store,
            commit: Some("abc123".into()),
        },
    }));

    let (status, json) = get_json(&router, "/api/v1/seeds").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"].as_array().expect("data array").len(), 2);
    assert_eq!(json["meta"]["total"], 2);
    let first = &json["data"][0];
    assert_eq!(first["id"], "fabro-0001");
    assert_eq!(first["status"], "open");
    assert_eq!(first["type"], "task");
    assert_eq!(first["priority"], 2);
    assert_eq!(first["assignee"], "fabro");
    assert_eq!(first["labels"][0], "handoff");
    assert_eq!(first["blockedBy"][0], "fabro-0002");
    assert_eq!(first["createdAt"], "2026-09-23T08:00:00Z");

    // Filters compose: closed bugs leave exactly the second seed.
    let (status, json) = get_json(&router, "/api/v1/seeds?status=closed&type=bug").await;
    assert_eq!(status, StatusCode::OK);
    let data = json["data"].as_array().expect("data array");
    assert_eq!(data.len(), 1);
    assert_eq!(data[0]["id"], "fabro-0002");

    // Assignee `none` matches unassigned seeds only.
    let (status, json) = get_json(&router, "/api/v1/seeds?assignee=none").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"].as_array().expect("data array").len(), 1);

    // Offsets page without counting dropped items as a page.
    let (_, json) = get_json(&router, "/api/v1/seeds?page[limit]=1&page[offset]=1").await;
    assert_eq!(json["data"].as_array().expect("data array").len(), 1);
    assert_eq!(json["meta"]["has_more"], false);
}

#[tokio::test]
async fn seeds_detail_serves_raw_fields_and_reverse_blocks() {
    let dir = fixture_dir();
    let store = Arc::new(Store::open(dir.path().join(".seeds")).expect("store opens"));
    let router = router_with(Arc::new(FixtureSeedsSource {
        snapshot: SeedsSnapshot {
            store,
            commit: Some("abc123".into()),
        },
    }));

    let (status, json) = get_json(&router, "/api/v1/seeds/fabro-0002").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["id"], "fabro-0002");
    // The reverse index names the blocked seed; the raw fields preserve
    // the record verbatim, unknown keys included.
    assert_eq!(json["blocks"][0], "fabro-0001");
    assert_eq!(json["fields"]["title"], "Second");
    assert_eq!(json["fields"]["status"], "closed");

    let (status, body) = get_json(&router, "/api/v1/seeds/fabro-4242").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        body["errors"][0]["detail"]
            .as_str()
            .expect("detail")
            .contains("fabro-4242")
    );
}

#[tokio::test]
async fn seeds_graph_covers_all_seeds_and_drops_unknown_edges() {
    let dir = fixture_dir();
    let store = Arc::new(Store::open(dir.path().join(".seeds")).expect("store opens"));
    let router = router_with(Arc::new(FixtureSeedsSource {
        snapshot: SeedsSnapshot {
            store,
            commit: Some("abc123".into()),
        },
    }));

    let (status, json) = get_json(&router, "/api/v1/seeds/graph").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["nodes"].as_array().expect("nodes").len(), 2);
    let edges = json["edges"].as_array().expect("edges");
    // fabro-9999 does not exist: only the real edge survives.
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0]["from"], "fabro-0001");
    assert_eq!(edges[0]["to"], "fabro-0002");
}

#[tokio::test]
async fn an_unconfigured_seeds_source_serves_the_documented_503() {
    let router = router_with(Arc::new(DisabledSeedsSource));
    for uri in [
        "/api/v1/seeds",
        "/api/v1/seeds/graph",
        "/api/v1/seeds/fabro-0001",
    ] {
        let (status, json) = get_json(&router, uri).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "URI: {uri}");
        assert_eq!(json["errors"][0]["code"], "seeds_source_unconfigured");
    }
}

#[tokio::test]
async fn a_failing_seeds_source_serves_the_documented_503() {
    let router = router_with(Arc::new(FailingSeedsSource));
    let (status, json) = get_json(&router, "/api/v1/seeds").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(json["errors"][0]["code"], "seeds_source_unavailable");
    assert!(
        json["errors"][0]["detail"]
            .as_str()
            .expect("detail")
            .contains("fixture: checkout gone")
    );
}

#[tokio::test]
async fn the_fixture_snapshot_commit_travels_with_the_source() {
    // The snapshot's commit provenance is part of the source contract the
    // production mirror (fabro-3488 fork decision) will carry; the fixture
    // pins that it survives the handoff untouched.
    let dir = fixture_dir();
    let store = Arc::new(Store::open(dir.path().join(".seeds")).expect("store opens"));
    let source = FixtureSeedsSource {
        snapshot: SeedsSnapshot {
            store,
            commit: Some("abc123".into()),
        },
    };
    let snapshot = source.snapshot().await.expect("snapshot");
    assert_eq!(snapshot.commit.as_deref(), Some("abc123"));
}

// ── Production mirror (GitRepoCache, fork decision B) ──────────────────

#[expect(
    clippy::disallowed_methods,
    reason = "test fixture drives a local git upstream with sync std::process::Command"
)]
fn git(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git spawns");
    assert!(status.success(), "git {:?} in {}", args, dir.display());
}

#[expect(
    clippy::disallowed_methods,
    reason = "test fixture writes .seeds files with sync std::fs::write"
)]
fn write_seeds(work: &Path, issue_id: &str) {
    let root = work.join(".seeds");
    std::fs::create_dir_all(&root).expect("seeds dir");
    std::fs::write(
        root.join("config.yaml"),
        "project: \"fabro\"\nversion: \"1\"\n",
    )
    .expect("config");
    std::fs::write(
        root.join("issues.jsonl"),
        format!(
            "{{\"id\":\"{issue_id}\",\"title\":\"Seed {issue_id}\",\"status\":\"open\",\
             \"type\":\"task\",\"blockedBy\":[]}}\n"
        ),
    )
    .expect("issues");
}

/// Seed a local bare upstream whose branches each carry a one-seed
/// `.seeds` store; returns the origin path and each branch's tip SHA in
/// order.
#[expect(
    clippy::disallowed_methods,
    reason = "test fixture drives a local git upstream with sync std::process::Command"
)]
fn mirror_upstream(temp: &Path, branches: &[(&str, &str)]) -> (std::path::PathBuf, Vec<String>) {
    let upstream = temp.join("upstream.git");
    git(temp, &[
        "init",
        "--bare",
        "--initial-branch=main",
        &upstream.display().to_string(),
    ]);
    let work = temp.join("work");
    git(temp, &[
        "init",
        "--initial-branch=main",
        &work.display().to_string(),
    ]);
    for (key, value) in [
        ("user.email", "test@fabro.sh"),
        ("user.name", "Fabro Test"),
        ("commit.gpgsign", "false"),
    ] {
        git(&work, &["config", key, value]);
    }

    let mut tips = Vec::new();
    for (branch, issue_id) in branches {
        git(&work, &["checkout", "-b", branch]);
        write_seeds(&work, issue_id);
        git(&work, &["add", "."]);
        git(&work, &["commit", "-m", &format!("seeds {issue_id}")]);
        let refspec = format!("HEAD:refs/heads/{branch}");
        git(&work, &["push", &upstream.display().to_string(), &refspec]);
        let output = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&work)
            .output()
            .expect("rev-parse spawns");
        assert!(output.status.success());
        tips.push(String::from_utf8(output.stdout).unwrap().trim().to_string());
    }
    (upstream, tips)
}

fn mirror_settings(origin: &Path, branch: &str) -> SeedsMirrorSettings {
    SeedsMirrorSettings {
        origin:    origin.display().to_string(),
        branch:    branch.to_string(),
        cache_dir: None,
    }
}

async fn snapshot_ids(source: &GitSeedsSource) -> Vec<String> {
    source
        .snapshot()
        .await
        .expect("mirror snapshot")
        .store
        .issues
        .iter()
        .map(|record| record.id().to_string())
        .collect()
}

#[tokio::test]
async fn the_configured_mirror_serves_seeds_through_the_router() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (upstream, tips) = mirror_upstream(temp.path(), &[("line-a", "fabro-0001")]);
    let source = Arc::new(GitSeedsSource::new(
        &mirror_settings(&upstream, "line-a"),
        temp.path().join("cache"),
    ));

    // The snapshot carries the branch tip as its commit provenance.
    let snapshot = source.snapshot().await.expect("snapshot");
    assert_eq!(snapshot.commit.as_deref(), Some(tips[0].as_str()));
    assert_eq!(snapshot.store.issues.len(), 1);

    let router = router_with(source);
    let (status, json) = get_json(&router, "/api/v1/seeds").await;
    assert_eq!(status, StatusCode::OK);
    let data = json["data"].as_array().expect("data array");
    assert_eq!(data.len(), 1);
    assert_eq!(data[0]["id"], "fabro-0001");
}

#[tokio::test]
async fn a_cached_snapshot_never_survives_a_branch_switch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (upstream, tips) = mirror_upstream(temp.path(), &[
        ("line-a", "fabro-0001"),
        ("line-b", "fabro-0002"),
    ]);
    // One shared mirror cache, as a re-wired server reuses the same root.
    let cache_root = temp.path().join("cache");

    let source_a = GitSeedsSource::new(&mirror_settings(&upstream, "line-a"), cache_root.clone());
    assert_eq!(snapshot_ids(&source_a).await, ["fabro-0001"]);

    // The served ref flips to line-b over the SAME cache: the snapshot
    // from line-a must never survive the move.
    let source_b = GitSeedsSource::new(&mirror_settings(&upstream, "line-b"), cache_root.clone());
    let snapshot_b = source_b.snapshot().await.expect("snapshot b");
    assert_eq!(snapshot_b.commit.as_deref(), Some(tips[1].as_str()));
    assert_eq!(snapshot_b.store.issues.len(), 1);
    assert_eq!(ids(&snapshot_b), ["fabro-0002"]);

    // And back: a flip to line-a again re-reads line-a's seeds.
    let source_a2 = GitSeedsSource::new(&mirror_settings(&upstream, "line-a"), cache_root);
    assert_eq!(snapshot_ids(&source_a2).await, ["fabro-0001"]);
}

#[tokio::test]
async fn an_upstream_commit_move_invalidates_the_cached_snapshot() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (upstream, tips) = mirror_upstream(temp.path(), &[("line-b", "fabro-0002")]);
    let work = temp.path().join("work");
    let source = GitSeedsSource::new(
        &mirror_settings(&upstream, "line-b"),
        temp.path().join("cache"),
    )
    .with_refresh_window(Duration::ZERO);
    assert_eq!(snapshot_ids(&source).await, ["fabro-0002"]);

    // The branch tip moves upstream (a new seed lands on line-b).
    git(&work, &["checkout", "line-b"]);
    write_seeds(&work, "fabro-0003");
    git(&work, &["add", "."]);
    git(&work, &["commit", "-m", "seeds fabro-0003"]);
    git(&work, &[
        "push",
        &upstream.display().to_string(),
        "HEAD:refs/heads/line-b",
    ]);

    let snapshot = source.snapshot().await.expect("snapshot after move");
    assert_eq!(ids(&snapshot), ["fabro-0003"]);
    assert_ne!(snapshot.commit.as_deref(), tips.first().map(String::as_str));
}

#[tokio::test]
async fn a_failing_configured_mirror_serves_the_documented_503() {
    let temp = tempfile::tempdir().expect("tempdir");
    let origin = temp.path().join("missing-upstream.git");
    let source = Arc::new(GitSeedsSource::new(
        &mirror_settings(&origin, "line-a"),
        temp.path().join("cache"),
    ));
    let Err(error) = source.snapshot().await else {
        panic!("missing origin should fail")
    };
    assert!(matches!(error, SeedsSourceError::Unavailable(_)), "{error}");

    let router = router_with(source);
    let (status, json) = get_json(&router, "/api/v1/seeds").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(json["errors"][0]["code"], "seeds_source_unavailable");
}

fn ids(snapshot: &SeedsSnapshot) -> Vec<String> {
    snapshot
        .store
        .issues
        .iter()
        .map(|record| record.id().to_string())
        .collect()
}
