//! The ADR-0035 acceptance (denkhaus/work#989): the compiled `fabro`
//! binary — the process shape a sandbox runs — reaches the engine ONLY
//! through the run's capability channel socket. The child gets
//! `FABRO_SERVER=<socket path>` and the run's HANDSHAKE token
//! (`FABRO_RUN_CHANNEL_TOKEN`, work#1043 — exactly what the mount
//! gives a sandbox-side stage script) and NOTHING else: no bearer, no
//! Authorization — the server stamps those itself.
//!
//! The server half runs in this test process (test-support AppState,
//! production `spawn_for_run`), the CLI half in the child — two
//! processes, one Unix socket, worker principal stamped server-side.

#![expect(
    clippy::disallowed_methods,
    reason = "This real CLI channel IT intentionally drives the compiled fabro binary as a child process."
)]
#![expect(
    clippy::disallowed_types,
    reason = "This real CLI channel IT intentionally writes the child's stdin through blocking std::io."
)]

use std::io::Write as _;
use std::process::{Command, Output, Stdio};
use std::sync::Arc;

use async_trait::async_trait;
use fabro_server::server::AppState;
use fabro_server::test_support::{
    TestAppStateBuilder, build_test_router, create_test_run_with_target, spawn_for_run_for_test,
};
use fabro_types::RunId;
use fabro_work::{TrackerProvider, WorkItem, WorkItemDraft, WorkItemFilter, WorkProject};
use serde_json::json;
use tower::ServiceExt;

/// The scripted tracker behind the channel's work routes: one open
/// item, create mirrors the draft, close always lands, and every list
/// call's filter is recorded (the label/page seam, denkhaus/work#1001).
struct ChannelCliFakeTracker {
    filters: std::sync::Mutex<Vec<WorkItemFilter>>,
}

fn fixture_item(key: &str, title: &str) -> WorkItem {
    WorkItem {
        item_key:   key.to_string(),
        title:      title.to_string(),
        body:       format!("body of {key}"),
        state:      "open".to_string(),
        labels:     vec!["project:fabro".to_string()],
        priority:   2,
        url:        format!("https://github.com/{key}"),
        updated_at: None,
    }
}

impl ChannelCliFakeTracker {
    fn recorded_filters(&self) -> Vec<WorkItemFilter> {
        self.filters.lock().expect("filter recorder lock").clone()
    }
}

#[async_trait]
impl TrackerProvider for ChannelCliFakeTracker {
    async fn list(
        &self,
        _project: &WorkProject,
        filter: WorkItemFilter,
    ) -> anyhow::Result<Vec<WorkItem>> {
        self.filters
            .lock()
            .expect("filter recorder lock")
            .push(filter);
        Ok(vec![fixture_item("denkhaus/work#1", "channel fixture")])
    }

    async fn get(
        &self,
        _project: &WorkProject,
        item_key: &str,
    ) -> anyhow::Result<Option<WorkItem>> {
        Ok(Some(fixture_item(item_key, "shown through the channel")))
    }

    async fn create(
        &self,
        _project: &WorkProject,
        draft: WorkItemDraft,
    ) -> anyhow::Result<WorkItem> {
        Ok(WorkItem {
            item_key: "denkhaus/work#2".to_string(),
            title: draft.title,
            ..fixture_item("denkhaus/work#2", "unused")
        })
    }

    async fn update(
        &self,
        _project: &WorkProject,
        _item_key: &str,
        _title: Option<&str>,
        _body: Option<&str>,
    ) -> anyhow::Result<WorkItem> {
        anyhow::bail!("update is outside this IT's scope")
    }

    async fn close(
        &self,
        _project: &WorkProject,
        _item_key: &str,
        _reason: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn comment(
        &self,
        _project: &WorkProject,
        _item_key: &str,
        _body: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn project_claim(
        &self,
        _project: &WorkProject,
        _item_key: &str,
        _run_id: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn clear_claim(
        &self,
        _project: &WorkProject,
        _item_key: &str,
        _run_id: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Upsert a work project through the user surface (the worker principal
/// must not seed this itself — the channel IT pins that elsewhere).
async fn seed_project(state: &Arc<AppState>, id: &str, target_repo: &str) {
    let app = build_test_router(Arc::clone(state));
    let request = axum::http::Request::builder()
        .method(axum::http::Method::PUT)
        .uri(format!("/api/v1/projects/{id}"))
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(
            serde_json::to_vec(&json!({
                "displayName": id,
                "trackerProvider": "github",
                "trackerRepo": "denkhaus/work",
                "targetRepo": target_repo,
                "projectLabel": format!("project:{id}"),
                "laneLabels": {"loop": "lane:loop"},
                "claimLabelPrefix": "fabro:run:",
                "claimCommentEnabled": false
            }))
            .expect("project fixture serializes"),
        ))
        .expect("project request builds");
    let response = app.oneshot(request).await.expect("project upsert");
    fabro_test::assert_axum_status(
        response,
        axum::http::StatusCode::OK,
        "PUT /api/v1/projects/fabro (user seed)",
    )
    .await;
}

/// Grant the run's own claim on the fixture item through the user
/// surface — `work close` requires a live claim held by the run
/// (denkhaus/work#991), so the close verb can prove its in-scope path.
async fn grant_own_claim(state: &Arc<AppState>, run_id: &RunId) {
    let app = build_test_router(Arc::clone(state));
    let request = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri("/api/v1/projects/fabro/claims")
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(
            serde_json::to_vec(&json!({
                "itemKey": "denkhaus/work#1",
                "runId": run_id.to_string(),
                "lane": "loop"
            }))
            .expect("claim fixture serializes"),
        ))
        .expect("claim request builds");
    let response = app.oneshot(request).await.expect("claim grant");
    fabro_test::assert_axum_status(
        response,
        axum::http::StatusCode::OK,
        "POST /api/v1/projects/fabro/claims (own run)",
    )
    .await;
}

/// Grant a claim held by a FOREIGN run through the user surface — the
/// CLI's second refusal path closes against it and must be refused
/// (close requires a claim held by THIS run, denkhaus/work#991).
async fn seed_foreign_claim(state: &Arc<AppState>, foreign_run: &RunId) {
    let app = build_test_router(Arc::clone(state));
    let request = axum::http::Request::builder()
        .method(axum::http::Method::POST)
        .uri("/api/v1/projects/fabro/claims")
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(
            serde_json::to_vec(&json!({
                "itemKey": "denkhaus/work#7",
                "runId": foreign_run.to_string(),
                "lane": "loop"
            }))
            .expect("foreign claim fixture serializes"),
        ))
        .expect("foreign claim request builds");
    let response = app.oneshot(request).await.expect("foreign claim grant");
    fabro_test::assert_axum_status(
        response,
        axum::http::StatusCode::OK,
        "POST /api/v1/projects/fabro/claims (foreign run seed)",
    )
    .await;
}

/// Run the compiled CLI against the channel socket — the sandbox's
/// environment: the socket path, the run's handshake token (work#1043:
/// the serve refuses without it), the upgrade-check suppression, and
/// nothing that could authenticate a request by itself.
fn cli_over_channel(
    socket: &std::path::Path,
    token: &str,
    args: &[&str],
    stdin_body: Option<&str>,
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fabro"));
    command
        .args(args)
        .env("FABRO_SERVER", socket)
        .env(fabro_static::EnvVars::FABRO_RUN_CHANNEL_TOKEN, token)
        .env("FABRO_NO_UPGRADE_CHECK", "true")
        .env_remove("FABRO_RUN_CHANNEL_SOCK")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if stdin_body.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command.spawn().expect("the CLI child should start");
    if let Some(body) = stdin_body {
        let mut stdin = child.stdin.take().expect("piped stdin");
        stdin
            .write_all(body.as_bytes())
            .expect("the body should reach the CLI");
    }
    child.wait_with_output().expect("the CLI should finish")
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The acceptance run: list, show, create, close through the channel,
/// and a beyond-scope request refused — all with a child process that
/// holds no credential.
///
/// multi_thread: the channel's serve task lives in THIS runtime while
/// the CLI child blocks a worker with its synchronous wait.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_cli_reaches_the_engine_only_through_the_run_channel() {
    let tracker = Arc::new(ChannelCliFakeTracker {
        filters: std::sync::Mutex::new(Vec::new()),
    });
    let state = TestAppStateBuilder::new()
        .work_tracker(Arc::clone(&tracker) as Arc<dyn TrackerProvider>)
        .build();
    seed_project(&state, "fabro", "denkhaus/fabro").await;
    seed_project(&state, "other", "denkhaus/unrelated").await;
    let run_id = create_test_run_with_target(&state, "denkhaus/fabro", "denkhaus").await;
    let (socket, token) = spawn_for_run_for_test(&state, run_id)
        .await
        .expect("the run's channel spawns");
    grant_own_claim(&state, &run_id).await;

    // list: the fixture item renders, exit 0, no token anywhere
    let list = cli_over_channel(
        &socket,
        &token,
        &["work", "list", "--project", "fabro"],
        None,
    );
    assert!(
        list.status.success(),
        "work list through the channel: {}",
        stderr_of(&list)
    );
    assert!(
        stdout_of(&list).contains("denkhaus/work#1"),
        "the scripted item is served to the CLI: {}",
        stdout_of(&list)
    );

    // the label/page seam (denkhaus/work#1001): --label and --page travel
    // the channel and reach the tracker's filter — a census can slice by
    // any label and walk past the first 100 items.
    let labeled = cli_over_channel(
        &socket,
        &token,
        &[
            "work",
            "list",
            "--project",
            "fabro",
            "--label",
            "arch",
            "--page",
            "2",
        ],
        None,
    );
    assert!(
        labeled.status.success(),
        "work list --label --page through the channel: {}",
        stderr_of(&labeled)
    );
    let filters = tracker.recorded_filters();
    let seam = filters
        .iter()
        .find(|filter| filter.label.as_deref() == Some("arch"))
        .expect("the label filter reached the tracker");
    assert_eq!(seam.page, 2, "the page flag reached the tracker");

    // show: the bare number resolves against the project and renders
    let show = cli_over_channel(
        &socket,
        &token,
        &["work", "show", "denkhaus/work#1", "--project", "fabro"],
        None,
    );
    assert!(
        show.status.success(),
        "work show through the channel: {}",
        stderr_of(&show)
    );
    assert!(
        stdout_of(&show).contains("shown through the channel"),
        "the item body reaches the CLI: {}",
        stdout_of(&show)
    );

    // create: the draft travels over the socket and the new key returns
    let create = cli_over_channel(
        &socket,
        &token,
        &[
            "work",
            "create",
            "--project",
            "fabro",
            "--title",
            "filed via channel",
            "--body-file",
            "-",
        ],
        Some("created through the capability channel\n"),
    );
    assert!(
        create.status.success(),
        "work create through the channel: {}",
        stderr_of(&create)
    );
    assert!(
        stdout_of(&create).contains("denkhaus/work#2"),
        "the filed key returns to the CLI: {}",
        stdout_of(&create)
    );

    // close: the run's own claim, closed through the channel
    let close = cli_over_channel(
        &socket,
        &token,
        &[
            "work",
            "close",
            "denkhaus/work#1",
            "--project",
            "fabro",
            "--reason",
            "closed through the channel",
        ],
        None,
    );
    assert!(
        close.status.success(),
        "work close through the channel: {}",
        stderr_of(&close)
    );

    // beyond scope: another project than the run's target repository —
    // the worker principal is project-bound and is refused server-side.
    let refusal = cli_over_channel(
        &socket,
        &token,
        &["work", "list", "--project", "other"],
        None,
    );
    assert!(
        !refusal.status.success(),
        "a foreign project is refused for the channel's principal"
    );
    let refusal_text = format!("{}{}", stdout_of(&refusal), stderr_of(&refusal)).to_lowercase();
    assert!(
        refusal_text.contains("scope") || refusal_text.contains("403"),
        "the refusal names the boundary: {refusal_text}"
    );

    // beyond scope (second refusal path): closing an item claimed by
    // ANOTHER run — close/update require a live claim held by this run
    // (denkhaus/work#991), so the verb is refused before any tracker
    // call.
    seed_foreign_claim(&state, &RunId::new()).await;
    let close_refusal = cli_over_channel(
        &socket,
        &token,
        &[
            "work",
            "close",
            "denkhaus/work#7",
            "--project",
            "fabro",
            "--reason",
            "another run owns this claim",
        ],
        None,
    );
    assert!(
        !close_refusal.status.success(),
        "closing another run's claim is refused for the channel's principal"
    );
    let close_text =
        format!("{}{}", stdout_of(&close_refusal), stderr_of(&close_refusal)).to_lowercase();
    assert!(
        close_text.contains("belongs to run") || close_text.contains("claim"),
        "the refusal names the claim boundary: {close_text}"
    );
}
