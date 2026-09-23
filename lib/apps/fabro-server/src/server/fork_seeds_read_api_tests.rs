//! The seeds read API (fabro-3488, ADR-0023 step 5): list, detail, and
//! dependency graph over a `SeedsSource`, plus the unconfigured `503`.
//!
//! Fork-only presence pin: upstream has no seeds endpoints, so a merge
//! that drops the read API reds here instead of silently stripping it.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use seeds::Store;
use serde_json::Value;
use tower::ServiceExt;

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
