//! `GET /runs/{id}/snapshot` — the read door over a terminal run's snapshot
//! repository: a `git bundle` of all refs (the checkpoints chain plus the run
//! branch), so a salvage never needs an ssh detour onto the host that owns the
//! repository.
//!
//! The bundle is materialized and its exit status checked BEFORE the response
//! headers go out (work#1430): a `git bundle create` failure answers 500 with
//! the bundle's own stderr instead of a truncated 200. The complete bundle's
//! SHA256 and byte count then ride the response as integrity proof headers,
//! so a transfer cut mid-flight fails the consumer's verification loudly
//! instead of passing as a good export.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::stream::StreamExt;
use sha2::{Digest, Sha256};
use tokio::fs;
use tokio::io::AsyncReadExt as _;
use tokio::process::Command as TokioCommand;
use tokio_util::io::ReaderStream;
use tracing::{error, warn};

use super::super::AppState;
use crate::error::ApiError;
use crate::principal_middleware::RequireRunScoped;
use crate::server::run_publish::snapshots_dir;

/// Refuse a pathological snapshot repository up front instead of streaming an
/// unbounded bundle. The on-disk size of the bare repository bounds the bundle
/// tightly: a bundle is a repack of the same objects.
const SNAPSHOT_REPOSITORY_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// The integrity proof headers (work#1430): the SHA256 and exact byte count
/// the server computed over the COMPLETE bundle before streaming. A consumer
/// that receives fewer bytes, or bytes that hash differently, knows the
/// transfer was truncated mid-stream and must not trust the bundle.
const BUNDLE_SHA256_HEADER: header::HeaderName =
    header::HeaderName::from_static("x-bundle-sha256");
const BUNDLE_BYTES_HEADER: header::HeaderName = header::HeaderName::from_static("x-bundle-bytes");

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

    // Materialize the bundle into a staging file first: the response only
    // starts once `git bundle create` has succeeded, so a git failure answers
    // 500 with the bundle's own stderr rather than a truncated 200.
    let staging = match tempfile::Builder::new()
        .prefix("fabro-snapshot-")
        .suffix(".bundle")
        .tempfile()
    {
        Ok(staging) => staging,
        Err(err) => {
            return ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create staging file for the bundle: {err}"),
            )
            .into_response();
        }
    };

    // git writes through a duplicated handle onto the staging file; the
    // original handle's offset shares that position, so every later read
    // reopens the path instead.
    let bundle_output = match staging.as_file().try_clone() {
        Ok(handle) => handle,
        Err(err) => {
            return ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to prepare the bundle staging file: {err}"),
            )
            .into_response();
        }
    };

    let mut command = TokioCommand::new("git");
    command
        .arg("--git-dir")
        .arg(&repository)
        .args(["bundle", "create", "-", "--all"])
        .stdin(Stdio::null())
        .stdout(Stdio::from(bundle_output))
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            warn!(run_id = %id, "git bundle create could not be started: {err}");
            return ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to start git bundle: {err}"),
            )
            .into_response();
        }
    };
    let mut stderr_bytes = Vec::new();
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.read_to_end(&mut stderr_bytes).await;
    }
    match child.wait().await {
        Ok(status) if status.success() => {}
        Ok(status) => {
            let detail = String::from_utf8_lossy(&stderr_bytes).trim();
            warn!(
                run_id = %id,
                "git bundle create failed (exit {}): {detail}",
                status.code().unwrap_or(-1),
            );
            return ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("git bundle create failed: {detail}"),
            )
            .into_response();
        }
        Err(err) => {
            warn!(run_id = %id, "git bundle create could not be reaped: {err}");
            return ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("git bundle create could not be reaped: {err}"),
            )
            .into_response();
        }
    }

    // Hash and size the COMPLETE bundle in one pass: these become the
    // integrity proof the consumer verifies its received bytes against.
    let (bundle_sha256, bundle_bytes) = match hash_staged_bundle(&staging).await {
        Ok(proof) => proof,
        Err(err) => {
            warn!(run_id = %id, "the staged bundle could not be hashed: {err}");
            return ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to hash the staged bundle: {err}"),
            )
            .into_response();
        }
    };

    let reader = match staging.reopen() {
        Ok(reader) => tokio::fs::File::from_std(reader),
        Err(err) => {
            warn!(run_id = %id, "the staged bundle could not be reopened: {err}");
            return ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to reopen the staged bundle: {err}"),
            )
            .into_response();
        }
    };

    // A mid-body failure cannot change the already-sent response: log loudly
    // with the run and the partial size, and let the framing (content-length)
    // plus the integrity headers catch it on the consumer side. HTTP trailers
    // are not available on this transport; the header hash covers the case.
    let sent = Arc::new(AtomicU64::new(0));
    let stream = ReaderStream::new(reader).map(move |chunk| match chunk {
        Ok(bytes) => {
            sent.fetch_add(bytes.len() as u64, Ordering::Relaxed);
            Ok(bytes)
        }
        Err(err) => {
            error!(
                run_id = %id,
                sent = sent.load(Ordering::Relaxed),
                error = %err,
                "snapshot bundle body failed mid-stream; the client must refuse it \
                 against x-bundle-sha256/x-bundle-bytes"
            );
            Err(err)
        }
    });
    // The staged bundle's last handle rides the body: the staging file is
    // deleted exactly when the response body is finished (or abandoned).
    let staging_cleanup = futures_util::stream::unfold(Some(staging), |staging| async move {
        drop(staging);
        None::<(Result<Bytes, std::io::Error>, Option<tempfile::NamedTempFile>)>
    });

    let mut response = Body::from_stream(stream.chain(staging_cleanup)).into_response();
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
    response.headers_mut().insert(header::CONTENT_LENGTH, HeaderValue::from(bundle_bytes));
    response.headers_mut().insert(BUNDLE_BYTES_HEADER, HeaderValue::from(bundle_bytes));
    response.headers_mut().insert(
        BUNDLE_SHA256_HEADER,
        HeaderValue::from_str(&bundle_sha256).expect("hex sha256 is a valid header value"),
    );
    response
}

/// SHA256 (hex) and byte count of the staged bundle, read in bounded chunks:
/// the bundle never sits whole in memory.
async fn hash_staged_bundle(staging: &tempfile::NamedTempFile) -> std::io::Result<(String, u64)> {
    let reader = staging.reopen()?;
    let mut reader = tokio::fs::File::from_std(reader);
    let mut digest = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = vec![0_u8; 256 * 1024];
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        total += read as u64;
    }
    Ok((hex::encode(digest.finalize()), total))
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
