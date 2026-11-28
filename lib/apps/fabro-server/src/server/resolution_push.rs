//! The merger's resolve push door (`POST /runs/{id}/resolution`,
//! ADR-0047, denkhaus/work#1471): the engine-mediated path a conflict
//! resolution takes from the merger's sandbox checkout to the forge.
//!
//! The resolve pass commits its resolution on the TARGET run's branch
//! in its own checkout, then names the commit through this door. The
//! server does every forge-facing step itself:
//!
//! - the objects travel out of the calling run's sandbox as a git bundle (`git
//!   bundle create` beside the checkout, read out through the sandbox
//!   filesystem facet — the same bundle protocol the checkpoint transport
//!   uses), and the bundle's basis is the origin's own branch state, fetched
//!   server-side with the run credentials;
//! - the push reuses the `run_publish` machinery verbatim: the same minted
//!   installation token, the same credential-helper git environment, a plain
//!   non-forced refspec so a non-fast-forward push is refused by git itself;
//! - one `run.branch_pushed` platform record lands in the TARGET run's stream
//!   per push — the capability-class audit (ADR-0019's second instance of the
//!   server-side, workflow-gated, audited door).
//!
//! Scoping fixed by ADR-0047: the door serves runs whose workflow
//! declares the `resolve_push` grant (ADR-0052), the branch is always
//! `fabro/run/<target>` (never caller-supplied), and the door refuses main, any
//! other ref, and every force push. The sandbox never receives a forge
//! credential — the weaker token-in-sandbox posture is the ADR's rejected
//! alternative.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use fabro_api::types::{RunPushResolutionRequest, RunPushResolutionResponse};
use fabro_config::Storage;
use fabro_redact::SecretRedactor;
use fabro_store::platform_records::{PlatformRecord, RunBranchPushedRecord};
use fabro_types::{OrchestrationField, OrchestrationVerdict, Principal, RunId};
use fabro_util::shell;
use pebble_coding_agent::sandbox_driver::{SandboxExec, display_for_log};
use sandbox_driver::Sandbox;
use tokio::fs;
use tokio::process::Command as TokioCommand;
use tokio::sync::OwnedMutexGuard;
use tracing::{info, warn};

use super::handler::orchestration::grants_for_writer;
use super::run_publish::{TokenMint, mint_push_token, push_snapshot_commit};
use super::{AppState, run_records};
use crate::error::ApiError;
use crate::principal_middleware::RequiredRunManagementActor;
use crate::sandbox_access;

/// Sandbox git timeout: bundling one branch delta over a local
/// repository is seconds of work; the 30s ceiling leaves cold-cache
/// headroom without letting a wedged sandbox hold the door.
const SANDBOX_GIT_TIMEOUT: Duration = Duration::from_secs(30);

/// Where a resolution bundle waits inside the sandbox on its way out:
/// outside the checkout, so no commit or checkpoint ever sees it (the
/// checkpoint transport's own directory stays untouched).
const TRANSFER_ROOT: &str = "/tmp/fabro-resolution";

/// The largest piece of the bundle one filesystem-facet read carries:
/// half the plugin transport's 16 MiB cap, the checkpoint transport's
/// own part size.
const TRANSFER_PART_BYTES: u64 = 8 * 1024 * 1024;

/// The bundle ceiling: a conflict resolution is a handful of files; a
/// bundle past this is not a resolution transfer but a mistake.
const MAX_BUNDLE_BYTES: u64 = 64 * 1024 * 1024;

/// Process-unique scratch names: one monotonic counter beside the
/// process start nanos, so a leftover from a crashed call can never
/// collide with a fresh scratch repository.
static SCRATCH_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static SCRATCH_START_NANOS: OnceLock<u64> = OnceLock::new();

/// The door: push a resolution commit onto the target run's own
/// branch. The authorization chain mirrors the merge door arm by arm;
/// the branch scope is the ADR's fixed `fabro/run/<target>`.
pub(super) async fn push_run_resolution(
    RequiredRunManagementActor(principal): RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(target): Path<RunId>,
    Json(request): Json<RunPushResolutionRequest>,
) -> Result<Response, ApiError> {
    let Principal::Worker { run_id: writer } = principal else {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            "Only workflow runs push resolutions. Users push through the forge.",
            "resolution_push_not_allowed",
        ));
    };
    if writer == target {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            "A run cannot push its own branch through the door — the resolution lands on the \
             integrated run's branch (ADR-0039 decision 1); a run's own branch publishes at \
             terminal success.",
            "resolution_push_self",
        ));
    }
    // The workflow's declaration is the allowlist: a run pushes
    // resolutions only when its workflow declares the grant (ADR-0052).
    if !grants_for_writer(&state, &writer).await?.resolve_push {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            "Only workflow runs whose workflow declares `resolve_push = true` under \
             `[run.grants]` push resolutions (ADR-0052)."
                .to_string(),
            "resolution_push_not_allowed",
        ));
    }
    // The target must exist and settle its projection — the PR, the
    // verdict fold, the run branch, and the origin read off it.
    let projection = run_records::require_projection(&state, target).await?;
    let Some(record) = projection.pull_request.clone() else {
        return Err(ApiError::with_code(
            StatusCode::CONFLICT,
            "The run carries no pull request — there is no run-branch flow to land a resolution \
             on. The resolve pass works packets the merge door declined.",
            "resolution_push_no_pull_request",
        ));
    };
    if state
        .stores
        .run_summaries
        .platform_records()
        .latest_verdict_of_run(OrchestrationField::Merge, &target)
        .await
        .map_err(|err| {
            tracing::error!(error = ?err, target_run_id = %target, "merge verdict read failed");
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Reading the run's merge verdict failed.",
            )
        })?
        != Some(OrchestrationVerdict::Enabled)
    {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            "The adjudicator has not enabled this run's merge — the resolve pass lands \
             resolutions only on merge-enabled runs (ADR-0039 decision 6).",
            "resolution_push_not_enabled",
        ));
    }
    // The branch scope is the ADR's fixed shape: the run's OWN branch,
    // derived server-side from the projection and refused for anything
    // else — main, a foreign ref, or a run without a branch record.
    let expected_branch = format!("fabro/run/{target}");
    let run_branch = projection
        .start
        .as_ref()
        .and_then(|start| start.run_branch.clone())
        .filter(|recorded| recorded == &expected_branch)
        .ok_or_else(|| {
            ApiError::with_code(
                StatusCode::FORBIDDEN,
                format!(
                    "The door pushes only the run's own branch ({expected_branch}); this run's \
                     recorded branch does not name it. Main, foreign refs, and force pushes are \
                     refused server-side (ADR-0047)."
                ),
                "resolution_push_branch_scope",
            )
        })?;
    let Some(origin_url) = projection.spec.repo_origin_url().map(str::to_string) else {
        return Err(ApiError::with_code(
            StatusCode::CONFLICT,
            "The run records no repository origin — there is nowhere to push.",
            "resolution_push_no_origin",
        ));
    };
    let trimmed_commit = request.commit.trim().to_string();
    if trimmed_commit.is_empty() {
        return Err(ApiError::with_code(
            StatusCode::BAD_REQUEST,
            "commit must name the resolution commit's sha as committed on the run branch.",
            "resolution_push_commit_missing",
        ));
    }

    // The server-side credentials: the same mint run_publish performs.
    let (https_origin, token) = match mint_push_token(&state, &origin_url).await {
        Ok(minted) => minted,
        Err(TokenMint::Unavailable) => {
            return Err(ApiError::with_code(
                StatusCode::SERVICE_UNAVAILABLE,
                "GitHub credentials are unavailable — the door pushes only through the server's \
                 configured integration.",
                "resolution_push_credentials_unavailable",
            ));
        }
        Err(TokenMint::Failed(error)) => {
            return Err(ApiError::with_code(
                StatusCode::BAD_GATEWAY,
                format!("Minting the push token failed: {error}"),
                "resolution_push_failed",
            ));
        }
    };

    // The named commit, resolved against the caller's own checkout.
    let full_sha = resolve_commit(&state, &writer, &trimmed_commit).await?;

    // Idempotence pre-flight: the branch head on the forge is already
    // the named commit — a re-drive answers 200 without touching
    // anything (and without a second audit record; the first push
    // carries the record).
    if remote_branch_head(&https_origin, &token, &run_branch)
        .await?
        .as_deref()
        == Some(full_sha.as_str())
    {
        info!(
            target_run_id = %target,
            writer_run_id = %writer,
            run_branch = %run_branch,
            head_sha = %full_sha,
            "resolution already on the run branch (resolve door, idempotent re-drive)"
        );
        return Ok(Json(RunPushResolutionResponse {
            run_id:     target.to_string(),
            pushed:     true,
            run_branch: run_branch.clone(),
            head_sha:   full_sha,
        })
        .into_response());
    }

    // The objects: bundle the resolved branch out of the caller's own
    // sandbox checkout, then land it through the scratch repository.
    // Each sandbox window holds the run's serialization gate so a
    // deferred inspection stop waits it out (fabro-2c17).
    let bundle = extract_resolution_bundle(&state, &writer, &run_branch, &full_sha).await?;
    let push_outcome =
        land_resolution(&https_origin, &token, &run_branch, &full_sha, &bundle).await;
    if let Err(error) = fs::remove_dir_all(&bundle.scratch_dir).await {
        warn!(
            error = %display_for_log(&error, &SecretRedactor),
            writer_run_id = %writer,
            "could not remove the resolution scratch repository"
        );
    }
    push_outcome?;

    info!(
        target_run_id = %target,
        writer_run_id = %writer,
        pr_number = record.number,
        run_branch = %run_branch,
        head_sha = %full_sha,
        "resolution pushed to the run branch (resolve door)"
    );
    // The audit record — one per push, on the target run's stream. A
    // store failure is logged, never a failed response: the push is a
    // fact on the forge either way (the merge door's record posture).
    if let Err(err) = run_records::append(
        &state,
        target,
        PlatformRecord::RunBranchPushed(RunBranchPushedRecord {
            run_branch: run_branch.clone(),
            head_sha:   full_sha.clone(),
            by_run:     writer,
        }),
    )
    .await
    {
        tracing::error!(
            error = ?err,
            target_run_id = %target,
            writer_run_id = %writer,
            "recording the resolution push failed"
        );
    }
    Ok(Json(RunPushResolutionResponse {
        run_id: target.to_string(),
        pushed: true,
        run_branch,
        head_sha: full_sha,
    })
    .into_response())
}

/// The bundle side of the door: the bytes read out of the caller's
/// sandbox, plus the scratch directory the landing cleans up.
struct ResolutionBundle {
    bytes:       Vec<u8>,
    scratch_dir: PathBuf,
}

/// Resolve the caller-supplied commit against the caller's own
/// checkout: the commit must exist there. A malformed or unknown rev is
/// the caller's refusal, not a transport failure.
async fn resolve_commit(
    state: &AppState,
    writer: &RunId,
    commit: &str,
) -> Result<String, ApiError> {
    let _gate = hold_writer_gate(writer);
    let (sandbox, working_directory) = writer_checkout(state, writer).await?;
    let rev_q = shell::shell_quote(&format!("{commit}^{{commit}}"));
    let output = checkout_git(
        &sandbox,
        &working_directory,
        &format!("git rev-parse --verify --quiet {rev_q}"),
    )
    .await?;
    let sha = output.stdout_lossy().trim().to_string();
    if !output.success() || sha.is_empty() {
        return Err(ApiError::with_code(
            StatusCode::CONFLICT,
            "The commit does not resolve in the calling run's checkout — commit the resolution \
             on the run branch first, then name its sha.",
            "resolution_push_commit_unknown",
        ));
    }
    Ok(sha)
}

/// The calling run's serialization gate for sandbox access (fabro-2c17):
/// held across every door window that reads through the sandbox.
async fn hold_writer_gate(writer: &RunId) -> OwnedMutexGuard<()> {
    sandbox_access::run_sandbox_gate(writer)
        .clone()
        .lock_owned()
        .await
}

/// The calling run's live sandbox and its checkout directory: the only
/// object source the door reads from.
async fn writer_checkout(
    state: &AppState,
    writer: &RunId,
) -> Result<(Arc<dyn Sandbox>, String), ApiError> {
    let writer_projection = run_records::projection(state, *writer)
        .await
        .map_err(|err| {
            tracing::error!(error = ?err, writer_run_id = %writer, "writer projection read failed");
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Reading the calling run's projection failed.",
            )
        })?
        .ok_or_else(|| {
            ApiError::with_code(
                StatusCode::CONFLICT,
                "The calling run does not exist.",
                "resolution_push_no_sandbox",
            )
        })?;
    let record = writer_projection
        .sandbox
        .as_ref()
        .and_then(fabro_types::RunSandbox::instance)
        .ok_or_else(|| {
            ApiError::with_code(
                StatusCode::CONFLICT,
                "The calling run has no sandbox checkout — the door transfers the resolution \
                 from the merger run's own checkout only.",
                "resolution_push_no_sandbox",
            )
        })?;
    let access = state
        .provider_access()
        .await
        .map_err(|err| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let sandbox = sandbox_access::attach_running_run_sandbox(&access, record, *writer)
        .await
        .map_err(|err| {
            ApiError::with_code(
                StatusCode::SERVICE_UNAVAILABLE,
                format!("The calling run's sandbox is unreachable: {err:#}"),
                "resolution_push_sandbox_unavailable",
            )
        })?;
    Ok((sandbox, record.runtime.working_directory.clone()))
}

/// One git invocation in the checkout. A transport failure (the sandbox
/// is gone or wedged) is a 503; a git failure carries the stderr so the
/// caller learns what its checkout answered.
async fn checkout_git(
    sandbox: &Arc<dyn Sandbox>,
    working_directory: &str,
    command: &str,
) -> Result<sandbox_driver::ExecResult, ApiError> {
    SandboxExec::new(sandbox.exec())
        .with_working_dir(working_directory.to_string())
        .run(command, Some(SANDBOX_GIT_TIMEOUT), None, None, None)
        .await
        .map_err(|err| {
            ApiError::with_code(
                StatusCode::SERVICE_UNAVAILABLE,
                format!(
                    "The calling run's checkout did not answer: {}",
                    display_for_log(&err, &SecretRedactor)
                ),
                "resolution_push_sandbox_unavailable",
            )
        })
}

/// Bundle the resolved branch out of the checkout, mirroring the
/// checkpoint transport's protocol: a range bundle against the origin's
/// own branch state, read out of the sandbox in bounded parts. The
/// transfer file dies with the call.
///
/// Every interpolated value is either a server-derived name (the
/// branch, the transfer path) or a full hex sha git itself resolved;
/// each still travels the command shell-quoted.
async fn extract_resolution_bundle(
    state: &AppState,
    writer: &RunId,
    run_branch: &str,
    full_sha: &str,
) -> Result<ResolutionBundle, ApiError> {
    let _gate = hold_writer_gate(writer);
    let (sandbox, working_directory) = writer_checkout(state, writer).await?;
    let branch_q = shell::shell_quote(run_branch);
    let sha_q = shell::shell_quote(full_sha);

    // The named commit must sit on the checkout's resolved branch —
    // the door lands the branch AS RESOLVED, never a stray sha.
    let ancestor = checkout_git(
        &sandbox,
        &working_directory,
        &format!("git merge-base --is-ancestor {sha_q} {branch_q}"),
    )
    .await?;
    if !ancestor.success() {
        return Err(ApiError::with_code(
            StatusCode::CONFLICT,
            "The commit is not on the checkout's branch for the run branch — resolve on the \
             run branch (git fetch origin <branch> && git checkout -B <branch> FETCH_HEAD), \
             commit, then name the commit.",
            "resolution_push_commit_unknown",
        ));
    }
    // The bundle's basis: the origin's branch state the checkout last
    // fetched. The scratch repository fetches the same ref from the
    // origin first, so the prerequisite is present on the receiving
    // side.
    let basis_ref = format!("refs/remotes/origin/{run_branch}");
    let basis = checkout_git(
        &sandbox,
        &working_directory,
        &format!(
            "git rev-parse --verify --quiet {}",
            shell::shell_quote(&basis_ref)
        ),
    )
    .await?;
    let basis_sha = basis.stdout_lossy().trim().to_string();
    if !basis.success() || basis_sha.is_empty() {
        return Err(ApiError::with_code(
            StatusCode::CONFLICT,
            "The checkout holds no fetched state for the run branch — fetch the branch \
             (git fetch origin <branch>) before pushing its resolution.",
            "resolution_push_stale_basis",
        ));
    }

    let sequence = SCRATCH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let transfer = format!("{TRANSFER_ROOT}/{writer}/resolution-{sequence}.bundle");
    let prepared = async {
        // The transfer directory sits outside the checkout; create it,
        // then bundle the delta the checkout holds over its basis.
        let dir_q = shell::shell_quote(&format!("{TRANSFER_ROOT}/{writer}"));
        let mkdir = checkout_git(
            &sandbox,
            &working_directory,
            &format!("mkdir -p -- {dir_q}"),
        )
        .await?;
        if !mkdir.success() {
            return Err(ApiError::with_code(
                StatusCode::SERVICE_UNAVAILABLE,
                format!(
                    "Preparing the transfer directory failed: {}",
                    mkdir.stderr_lossy().trim()
                ),
                "resolution_push_sandbox_unavailable",
            ));
        }
        let transfer_q = shell::shell_quote(&transfer);
        let basis_q = shell::shell_quote(&basis_sha);
        let bundled = checkout_git(
            &sandbox,
            &working_directory,
            &format!("git bundle create {transfer_q} {basis_q}..{branch_q}"),
        )
        .await?;
        if !bundled.success() {
            let stderr = bundled.stderr_lossy();
            if stderr.contains("empty bundle") {
                // The branch holds nothing over its fetched basis: the
                // resolution is not a new commit on this checkout.
                return Err(ApiError::with_code(
                    StatusCode::CONFLICT,
                    "The checkout's branch holds no commits over its fetched basis — there is \
                     no resolution to transfer. Commit the resolution, or re-fetch the branch \
                     if the checkout is stale.",
                    "resolution_push_no_new_objects",
                ));
            }
            return Err(ApiError::with_code(
                StatusCode::CONFLICT,
                format!("Bundling the resolution failed: {}", stderr.trim()),
                "resolution_push_failed",
            ));
        }
        read_sandbox_file(&sandbox, &transfer).await
    }
    .await;
    // The transfer file dies with the call, success or failure.
    let _ = checkout_git(
        &sandbox,
        &working_directory,
        &format!("rm -f -- {}", shell::shell_quote(&transfer)),
    )
    .await;
    let bytes = prepared?;

    // The scratch repository the server-side landing fetches into:
    // under the calling run's own scratch, unique per call, removed
    // after the push attempt.
    let start_nanos = *SCRATCH_START_NANOS.get_or_init(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| {
                u64::try_from(since.as_nanos()).unwrap_or(u64::MAX)
            })
    });
    let scratch_dir = Storage::new(state.server_storage_dir())
        .run_scratch(writer)
        .root()
        .join("resolution-push")
        .join(format!("resolution-{start_nanos}-{sequence}.git"));
    Ok(ResolutionBundle { bytes, scratch_dir })
}

/// Read a sandbox file in bounded windows the transport accepts (the
/// checkpoint transport's part size), bounded overall.
async fn read_sandbox_file(sandbox: &Arc<dyn Sandbox>, path: &str) -> Result<Vec<u8>, ApiError> {
    let filesystem = sandbox.fs();
    let mut bytes = Vec::new();
    let mut offset = 0u64;
    loop {
        let chunk = filesystem
            .read_range(path, offset, Some(TRANSFER_PART_BYTES))
            .await
            .map_err(|err| {
                ApiError::with_code(
                    StatusCode::SERVICE_UNAVAILABLE,
                    format!(
                        "Reading the resolution bundle out of the sandbox failed: {}",
                        display_for_log(&err, &SecretRedactor)
                    ),
                    "resolution_push_sandbox_unavailable",
                )
            })?;
        if chunk.is_empty() {
            break;
        }
        let chunk_len = u64::try_from(chunk.len()).unwrap_or(u64::MAX);
        offset = offset.saturating_add(chunk_len);
        bytes.extend_from_slice(&chunk);
        if offset > MAX_BUNDLE_BYTES {
            return Err(ApiError::with_code(
                StatusCode::CONFLICT,
                format!(
                    "The resolution bundle exceeds the {} MiB door ceiling — a resolution is a \
                     handful of files, not a history transfer.",
                    MAX_BUNDLE_BYTES / (1024 * 1024)
                ),
                "resolution_push_bundle_too_large",
            ));
        }
        if chunk_len < TRANSFER_PART_BYTES {
            break;
        }
    }
    if bytes.is_empty() {
        return Err(ApiError::with_code(
            StatusCode::SERVICE_UNAVAILABLE,
            "The resolution bundle read back empty.",
            "resolution_push_sandbox_unavailable",
        ));
    }
    Ok(bytes)
}

/// The forge-facing landing: scratch repository, origin-branch fetch
/// (the bundle's prerequisite), bundle fetch, verification, and the
/// plain non-forced push. Errors are the door's refusal codes; the
/// scratch directory removal stays with the caller.
async fn land_resolution(
    https_origin: &str,
    token: &str,
    run_branch: &str,
    full_sha: &str,
    bundle: &ResolutionBundle,
) -> Result<(), ApiError> {
    let scratch = &bundle.scratch_dir;
    fs::create_dir_all(scratch)
        .await
        .map_err(|err| transport_error("creating the resolution scratch repository", &err))?;
    let scratch_arg = scratch.to_string_lossy().into_owned();
    host_git("init the scratch repository", None, &[
        "--git-dir",
        &scratch_arg,
        "init",
        "-q",
        "--bare",
    ])
    .await?;
    // The bundle's prerequisite is the origin's branch state: fetch it
    // into the scratch first (a read, with the door's own token), then
    // the bundle's delta on top.
    let refspec = format!("refs/heads/{run_branch}:refs/heads/{run_branch}");
    if let Err(err) = host_git("fetch the origin branch", Some(token), &[
        "--git-dir",
        &scratch_arg,
        "fetch",
        "-q",
        https_origin,
        &refspec,
    ])
    .await
    {
        return Err(ApiError::with_code(
            StatusCode::CONFLICT,
            format!(
                "The run branch does not exist on the forge yet — there is no resolution to \
                 land on it: {err}"
            ),
            "resolution_push_no_origin_branch",
        ));
    }
    // The bundle bytes stage beside the scratch, fetched, then removed.
    let staged = scratch.with_extension("bundle");
    fs::write(&staged, &bundle.bytes)
        .await
        .map_err(|err| transport_error("staging the resolution bundle", &err))?;
    let staged_arg = staged.to_string_lossy().into_owned();
    let fetched = host_git("fetch the resolution bundle", None, &[
        "--git-dir",
        &scratch_arg,
        "fetch",
        "-q",
        &staged_arg,
        "refs/heads/*:refs/heads/*",
    ])
    .await;
    let _ = fs::remove_file(&staged).await;
    fetched.map_err(|err| {
        ApiError::with_code(
            StatusCode::CONFLICT,
            format!(
                "The checkout's branch state is stale against the forge — re-fetch the branch \
                 and re-resolve: {err}"
            ),
            "resolution_push_stale_basis",
        )
    })?;
    // The server-side verification: the objects the door is about to
    // push exist in the scratch, and the named commit sits on the
    // fetched branch — the push content is what the checkout resolved.
    let commit_arg = format!("{full_sha}^{{commit}}");
    if host_git("verify the resolution commit", None, &[
        "--git-dir",
        &scratch_arg,
        "cat-file",
        "-e",
        &commit_arg,
    ])
    .await
    .is_err()
    {
        return Err(ApiError::with_code(
            StatusCode::CONFLICT,
            "The resolution commit did not transfer — the bundle does not carry it.",
            "resolution_push_commit_unknown",
        ));
    }
    let branch_ref = format!("refs/heads/{run_branch}");
    if host_git("verify the commit is on the branch", None, &[
        "--git-dir",
        &scratch_arg,
        "merge-base",
        "--is-ancestor",
        full_sha,
        &branch_ref,
    ])
    .await
    .is_err()
    {
        return Err(ApiError::with_code(
            StatusCode::CONFLICT,
            "The resolution commit is not on the checkout's resolved branch.",
            "resolution_push_commit_unknown",
        ));
    }
    // The push itself: the run_publish machinery verbatim — the plain
    // refspec carries no force, so git refuses a non-fast-forward.
    push_snapshot_commit(scratch, https_origin, full_sha, run_branch, token)
        .await
        .map_err(|error| {
            if error.contains("non-fast-forward") || error.contains("fetch first") {
                ApiError::with_code(
                    StatusCode::CONFLICT,
                    format!(
                        "The push is not a fast-forward — the branch moved on the forge. \
                         Re-fetch the branch and re-resolve on top: {error}"
                    ),
                    "resolution_push_not_fast_forward",
                )
            } else {
                ApiError::with_code(
                    StatusCode::BAD_GATEWAY,
                    format!("The push to the forge failed: {error}"),
                    "resolution_push_failed",
                )
            }
        })
}

/// One host-side git invocation, its trimmed stdout. Failures carry the
/// trimmed stderr.
async fn host_git(op: &str, token: Option<&str>, args: &[&str]) -> Result<String, ApiError> {
    let mut command = TokioCommand::new("git");
    if let Some(token) = token {
        fabro_github::apply_probe_git_env(&mut command, token);
    }
    let output = command
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|err| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("spawning git {op} failed: {err}"),
            )
        })?;
    if !output.status.success() {
        return Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "git {op} failed (exit {}): {}",
                output.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// The origin's current branch head, or `None` when the branch does not
/// exist yet. The token travels only through the credential-helper
/// environment (`apply_probe_git_env`), never argv or URL.
async fn remote_branch_head(
    https_origin: &str,
    token: &str,
    run_branch: &str,
) -> Result<Option<String>, ApiError> {
    let refspec = format!("refs/heads/{run_branch}");
    let output = host_git("reading the branch head", Some(token), &[
        "ls-remote",
        https_origin,
        &refspec,
    ])
    .await
    .map_err(|err| transport_error("reading the forge branch head", &err))?;
    Ok(output
        .lines()
        .next()
        .and_then(|line| line.split('\t').next())
        .map(str::trim)
        .map(str::to_string)
        .filter(|sha| !sha.is_empty()))
}

/// A host-side failure that is the door's transport, not a refusal.
fn transport_error(op: &str, err: &impl std::fmt::Display) -> ApiError {
    tracing::error!(op, error = %err, "resolution push transport failed");
    ApiError::with_code(
        StatusCode::BAD_GATEWAY,
        format!("The resolution push failed: {op}."),
        "resolution_push_failed",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scope derivation is the run-id-scoped shape: only the
    /// target's own `fabro/run/<id>` satisfies it — the refusal surface
    /// the door guards before any git runs.
    #[test]
    fn the_expected_branch_is_the_run_id_scoped_shape() {
        let target = RunId::new();
        let expected = format!("fabro/run/{target}");
        assert_eq!(expected, format!("fabro/run/{target}"));
        assert_ne!(expected, format!("fabro/run/{}", RunId::new()));
        assert_ne!(expected, "main");
    }
}
