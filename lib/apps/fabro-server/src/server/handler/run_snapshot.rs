//! `GET /runs/{id}/snapshot` — the read door over a terminal run's snapshot
//! repository: a `git bundle` of all refs (the checkpoints chain plus the run
//! branch), streamed straight from the store so a salvage never needs an ssh
//! detour onto the host that owns the repository.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use tokio::fs;
use tokio::io::AsyncReadExt as _;
use tokio::process::Command as TokioCommand;
use tokio_util::io::ReaderStream;
use tracing::warn;

use super::super::AppState;
use crate::error::ApiError;
use crate::principal_middleware::RequireRunScoped;
use crate::server::run_publish::snapshots_dir;

/// Refuse a pathological snapshot repository up front instead of streaming an
/// unbounded bundle. The on-disk size of the bare repository bounds the bundle
/// tightly: a bundle is a repack of the same objects.
const SNAPSHOT_REPOSITORY_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/runs/{id}/snapshot", get(get_run_snapshot))
}

async fn get_run_snapshot(
    RequireRunScoped(id): RequireRunScoped,
    State(state): State<Arc<AppState>>,
) -> Response {
    if let Err(err) = state.load_run_projection(&id).await {
        return err.into_response();
    }

    let repositories = match snapshot_repositories(&snapshots_dir(&state, &id)).await {
        Ok(repositories) => repositories,
        Err(response) => return response,
    };
    if repositories.len() > 1 {
        return ApiError::new(
            StatusCode::CONFLICT,
            "Run has multiple snapshot repositories; snapshot export supports single-workspace \
             runs.",
        )
        .into_response();
    }
    let Some(repository) = repositories.first().cloned() else {
        return ApiError::not_found("Run snapshot not available.").into_response();
    };

    match repository_size_bytes(&repository).await {
        Ok(size) if size > SNAPSHOT_REPOSITORY_MAX_BYTES => {
            return ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                format!(
                    "Run snapshot repository is {size} bytes, over the \
                     {SNAPSHOT_REPOSITORY_MAX_BYTES} byte export cap."
                ),
            )
            .into_response();
        }
        Ok(_) => {}
        Err(response) => return response,
    }

    let mut command = TokioCommand::new("git");
    command
        .arg("--git-dir")
        .arg(&repository)
        .args(["bundle", "create", "-", "--all"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            return ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to start git bundle: {err}"),
            )
            .into_response();
        }
    };
    let stdout = child.stdout.take().expect("stdout was configured as piped");
    let stderr = child.stderr.take().expect("stderr was configured as piped");
    let run_id = id;
    tokio::spawn(async move {
        let mut stderr_bytes = Vec::new();
        let mut stderr = stderr;
        let _ = stderr.read_to_end(&mut stderr_bytes).await;
        match child.wait().await {
            Ok(status) if !status.success() => {
                warn!(
                    %run_id,
                    "git bundle create failed (exit {}): {}",
                    status.code().unwrap_or(-1),
                    String::from_utf8_lossy(&stderr_bytes).trim()
                );
            }
            Ok(_) => {}
            Err(err) => warn!(%run_id, "git bundle create could not be reaped: {err}"),
        }
    });

    let body = Body::from_stream(ReaderStream::new(stdout));
    let mut response = body.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!(
            "attachment; filename=\"fabro-snapshot-{id}.bundle\""
        ))
        .expect("run IDs produce valid attachment filenames"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    response
}

/// The `*.git` bare snapshot repositories under a run's snapshots directory,
/// in deterministic order. An unreadable directory is a server error, not a
/// missing snapshot: the run exists but its store cannot be inspected.
async fn snapshot_repositories(dir: &Path) -> Result<BTreeSet<PathBuf>, Response> {
    let mut entries = match fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(BTreeSet::new());
        }
        Err(err) => {
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to read run snapshots directory: {err}"),
            )
            .into_response());
        }
    };
    let mut repositories = BTreeSet::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().is_some_and(|extension| extension == "git") && path.is_dir() {
            repositories.insert(path);
        }
    }
    Ok(repositories)
}

/// The on-disk size of the repository, files summed recursively (an explicit
/// stack: a recursive async fn would need boxing for no gain).
async fn repository_size_bytes(dir: &Path) -> Result<u64, Response> {
    fn size_error(err: &std::io::Error) -> Response {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to measure run snapshot repository: {err}"),
        )
        .into_response()
    }

    let mut total = 0;
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let mut entries = fs::read_dir(&dir).await.map_err(|err| size_error(&err))?;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let metadata = entry.metadata().await.map_err(|err| size_error(&err))?;
            if metadata.is_dir() {
                pending.push(entry.path());
            } else {
                total += metadata.len();
            }
        }
    }
    Ok(total)
}
