//! HTTP-level integration tests for `GET /api/v1/runs/{id}/snapshot` — the
//! read door over a terminal run's snapshot repository.

#![expect(
    clippy::disallowed_methods,
    reason = "integration tests stage git fixtures with sync std::process and std::fs"
)]

use std::path::{Path, PathBuf};
use std::process::Command;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use fabro_config::Storage;
use fabro_types::RunId;
use tempfile::tempdir;
use tower::ServiceExt;

use crate::helpers::{
    MINIMAL_DOT, TestAppSettings, api, minimal_intent_json_with_dry_run, response_json,
    response_status, test_app_state_with_options, test_settings,
};

fn temp_storage_settings() -> (tempfile::TempDir, TestAppSettings, PathBuf) {
    let temp = tempdir().expect("tempdir should create");
    let mut settings = test_settings();
    let storage_dir = temp.path().join("storage");
    settings.server_settings.server.storage.root = storage_dir.to_string_lossy().into_owned();
    (temp, settings, storage_dir)
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

async fn create_run(app: &axum::Router, workspace: &Path) -> String {
    let intent = minimal_intent_json_with_dry_run(app, MINIMAL_DOT, workspace).await;
    let request = Request::builder()
        .method("POST")
        .uri(api("/runs"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&intent).expect("intent fixture should serialize"),
        ))
        .expect("create run request should build");
    let response = app.clone().oneshot(request).await.unwrap();
    let body = response_json(response, StatusCode::CREATED, "POST /api/v1/runs").await;
    body["id"]
        .as_str()
        .expect("run creation returns an id")
        .to_owned()
}

/// Plant a bare snapshot repository with one commit under the run's scratch,
/// returning `(run scratch root, head sha)`. The commit rides a branch named
/// after the workspace's run branch so the fetch proves refs travel whole.
fn plant_snapshot_repository(
    storage_dir: &Path,
    run_id: &str,
    name: &str,
) -> (PathBuf, String, tempfile::TempDir) {
    let source = tempdir().expect("source tempdir creates");
    let source_path = source.path().to_path_buf();
    git(&source_path, &["init", "-q", "-b", "fabro/run/test"]);
    git(&source_path, &["config", "user.email", "test@fabro.sh"]);
    git(&source_path, &["config", "user.name", "Snapshot Test"]);
    std::fs::write(source_path.join("checkpoint.txt"), "salvaged work\n")
        .expect("fixture file writes");
    git(&source_path, &["add", "."]);
    git(&source_path, &["commit", "-q", "-m", "checkpoint work"]);
    let head = git(&source_path, &["rev-parse", "HEAD"]);

    let run_id: RunId = run_id.parse().expect("run id parses");
    let snapshots = Storage::new(storage_dir)
        .run_scratch(&run_id)
        .root()
        .join("petri")
        .join("snapshots");
    std::fs::create_dir_all(&snapshots).expect("snapshots directory creates");
    let bare = snapshots.join(name);
    let output = Command::new("git")
        .args(["clone", "-q", "--bare"])
        .arg(&source_path)
        .arg(&bare)
        .output()
        .expect("git clone runs");
    assert!(
        output.status.success(),
        "git clone --bare failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    (snapshots, head, source)
}

fn snapshot_url(run_id: &str) -> String {
    api(&format!("/runs/{run_id}/snapshot"))
}

async fn get_snapshot(app: &axum::Router, run_id: &str) -> axum::http::Response<Body> {
    let request = Request::builder()
        .method("GET")
        .uri(snapshot_url(run_id))
        .body(Body::empty())
        .expect("snapshot request should build");
    app.clone().oneshot(request).await.unwrap()
}

#[tokio::test]
async fn invalid_run_id_returns_400() {
    let app = fabro_server::test_support::build_test_router(crate::helpers::test_app_state());
    let response = get_snapshot(&app, "not-a-ulid").await;
    response_status(
        response,
        StatusCode::BAD_REQUEST,
        "GET /api/v1/runs/not-a-ulid/snapshot",
    )
    .await;
}

#[tokio::test]
async fn unknown_run_returns_404() {
    let app = fabro_server::test_support::build_test_router(crate::helpers::test_app_state());
    let response = get_snapshot(&app, "01ARZ3NDEKTSV4RRFFQ69G5FAV").await;
    response_status(
        response,
        StatusCode::NOT_FOUND,
        "GET /api/v1/runs/01ARZ3NDEKTSV4RRFFQ69G5FAV/snapshot",
    )
    .await;
}

#[tokio::test]
async fn run_without_a_snapshot_repository_returns_404() {
    let workspace = tempdir().unwrap();
    let (_temp, settings, _storage_dir) = temp_storage_settings();
    let app =
        fabro_server::test_support::build_test_router(test_app_state_with_options(settings, 5));
    let run_id = create_run(&app, workspace.path()).await;

    let response = get_snapshot(&app, &run_id).await;
    let body = response_json(
        response,
        StatusCode::NOT_FOUND,
        format!("GET /api/v1/runs/{run_id}/snapshot"),
    )
    .await;
    assert!(
        body["errors"][0]["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("Run snapshot not available"),
        "expected the no-snapshot 404, got {body}"
    );
}

#[tokio::test]
async fn snapshot_streams_a_bundle_the_salvage_fetch_recovers() {
    let workspace = tempdir().unwrap();
    let (temp, settings, storage_dir) = temp_storage_settings();
    let app =
        fabro_server::test_support::build_test_router(test_app_state_with_options(settings, 5));
    let run_id = create_run(&app, workspace.path()).await;
    let (_snapshots, head, _source) =
        plant_snapshot_repository(&storage_dir, &run_id, "invocation-0-scope-0.git");

    let response = get_snapshot(&app, &run_id).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "GET /api/v1/runs/{run_id}/snapshot"
    );
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/octet-stream")
    );
    let body = to_bytes(response.into_body(), 64 * 1024 * 1024)
        .await
        .expect("bundle body reads");

    // The salvage flow: write the bundle, fetch every ref out of it.
    let bundle = temp.path().join("salvage.bundle");
    std::fs::write(&bundle, &body).expect("bundle writes");
    let fetch_root = tempdir().expect("fetch tempdir creates");
    let fetch_repo = fetch_root.path().join("repo");
    std::fs::create_dir_all(&fetch_repo).expect("fetch repo dir creates");
    git(&fetch_repo, &["init", "-q"]);
    git(&fetch_repo, &[
        "fetch",
        "-q",
        bundle.to_string_lossy().as_ref(),
        "refs/*:refs/remotes/run/*",
    ]);
    let recovered = git(&fetch_repo, &[
        "rev-parse",
        "refs/remotes/run/heads/fabro/run/test",
    ]);
    assert_eq!(recovered, head, "the salvaged commit round-trips");
}

#[tokio::test]
async fn multiple_snapshot_repositories_are_refused_with_a_named_conflict() {
    let workspace = tempdir().unwrap();
    let (_temp, settings, storage_dir) = temp_storage_settings();
    let app =
        fabro_server::test_support::build_test_router(test_app_state_with_options(settings, 5));
    let run_id = create_run(&app, workspace.path()).await;
    plant_snapshot_repository(&storage_dir, &run_id, "invocation-0-scope-0.git");
    plant_snapshot_repository(&storage_dir, &run_id, "invocation-1-scope-0.git");

    let response = get_snapshot(&app, &run_id).await;
    let body = response_json(
        response,
        StatusCode::CONFLICT,
        format!("GET /api/v1/runs/{run_id}/snapshot"),
    )
    .await;
    assert!(
        body["errors"][0]["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("multiple snapshot repositories"),
        "expected the multi-scope conflict, got {body}"
    );
}
