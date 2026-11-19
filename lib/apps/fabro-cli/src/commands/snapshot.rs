//! `fabro snapshot <run>` — export a run's snapshot repository as a `git
//! bundle` through the server's read door, so salvaging a terminal run's
//! checkpoint work needs no ssh detour onto the host that owns the store.
//!
//! Integrity is established before the verb declares good: when the server
//! declares the bundle's SHA-256 (`x-bundle-sha256`) the received bytes must
//! hash to it, and otherwise `git bundle verify` checks the saved bundle
//! locally — a truncated or corrupt export fails named instead of handing a
//! broken bundle to the salvage flow.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use fabro_client::SnapshotDownload;
use fabro_types::RunId;
use futures::StreamExt;
use sha2::{Digest, Sha256};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::args::SnapshotArgs;
use crate::command_context::CommandContext;
use crate::shared::{absolute_or_current, print_json_pretty};

pub(crate) async fn run(args: &SnapshotArgs, base_ctx: &CommandContext) -> Result<()> {
    let ctx = base_ctx.with_target(&args.server)?;
    let printer = ctx.printer();
    let client = ctx.server().await?;
    let run_id = client.resolve_run(&args.run).await?.id;
    let output = args
        .output
        .clone()
        .unwrap_or_else(|| default_output(&run_id));
    let download = client.download_run_snapshot(&run_id).await?;
    let (bytes, verification) = write_bundle(download, &output).await?;
    let verification = match verification {
        // The server's header settled it during the transfer.
        verification @ Verification::ServerSha256 { .. } => verification,
        Verification::Unverified => verify_bundle_locally(&output).await?,
    };
    let fetched_ref = match args.r#ref.as_deref() {
        Some(expected) => Some(require_bundle_ref(&output, expected).await?),
        None => None,
    };
    if ctx.json_output() {
        let mut payload = serde_json::json!({
            "run_id": run_id,
            "output": absolute_or_current(&output),
            "bytes":  bytes,
            "integrity": verification.label(),
        });
        if let Some((name, sha)) = fetched_ref {
            payload["ref"] = serde_json::json!({ "name": name, "sha": sha });
        }
        print_json_pretty(&payload)?;
    } else {
        fabro_util::printout!(
            printer,
            "Exported snapshot bundle for run {} to {} ({} bytes)",
            run_id,
            output.display(),
            bytes
        );
        fabro_util::printout!(printer, "Integrity: {}", verification.description());
        if let Some((name, sha)) = fetched_ref {
            fabro_util::printout!(printer, "  {} at {}", name, sha);
        }
    }
    Ok(())
}

fn default_output(run_id: &RunId) -> PathBuf {
    PathBuf::from(format!("fabro-snapshot-{run_id}.bundle"))
}

/// How the saved bundle's integrity was established. The header path is the
/// server's declaration checked over the received bytes; the verify path is
/// `git bundle verify` against a local repository, the fallback for a server
/// that does not declare the digest yet.
enum Verification {
    /// The received bytes hashed to the server's `x-bundle-sha256` header.
    ServerSha256 {
        sha256: String,
        declared_bytes: Option<u64>,
    },
    /// `git bundle verify` passed locally.
    BundleVerify { after_origin_fetch: bool },
    /// Nothing checked the bundle yet: written by `write_bundle` when the
    /// server declared no digest, settled by `verify_bundle_locally`.
    Unverified,
}

impl Verification {
    fn label(&self) -> &'static str {
        match self {
            Self::ServerSha256 { .. } => "server-sha256",
            Self::BundleVerify {
                after_origin_fetch: false,
            } => "git-bundle-verify",
            Self::BundleVerify {
                after_origin_fetch: true,
            } => "git-bundle-verify-after-origin-fetch",
            Self::Unverified => "unverified",
        }
    }

    fn description(&self) -> String {
        match self {
            Self::ServerSha256 {
                sha256,
                declared_bytes: Some(declared_bytes),
            } => {
                format!("sha256 matches the server header ({sha256}, {declared_bytes} bytes)")
            }
            Self::ServerSha256 { sha256, .. } => {
                format!("sha256 matches the server header ({sha256})")
            }
            Self::BundleVerify {
                after_origin_fetch: false,
            } => "git bundle verify passed".to_string(),
            Self::BundleVerify {
                after_origin_fetch: true,
            } => "git bundle verify passed, after fetching origin".to_string(),
            Self::Unverified => "unverified".to_string(),
        }
    }
}

/// Stream the bundle into a staging file beside the output, then move it into
/// place: a failed transfer never leaves a truncated bundle at the target
/// path, and the bytes never sit whole in memory. The received bytes are
/// hashed on the way in, and when the server declared the digest the hash
/// must match before the file moves into place — a corrupt transfer names
/// itself and the staging file is dropped instead of saved.
async fn write_bundle(download: SnapshotDownload, output: &Path) -> Result<(u64, Verification)> {
    if output.exists() {
        bail!(
            "output path {} already exists; remove it first or pass --out",
            output.display()
        );
    }
    let parent = match output.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    fs::create_dir_all(&parent)
        .await
        .with_context(|| format!("failed to create {}", parent.display()))?;
    let staging = tempfile::Builder::new()
        .prefix(".fabro-snapshot-")
        .suffix(".bundle")
        .tempfile_in(&parent)
        .with_context(|| format!("failed to create staging file in {}", parent.display()))?;

    let mut hasher = Sha256::new();
    let mut file = fs::File::create(staging.path()).await?;
    let mut bytes = 0_u64;
    let mut stream = download.stream;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("snapshot bundle stream failed")?;
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        bytes += chunk.len() as u64;
    }
    file.flush().await?;
    drop(file);

    let received_sha = hex::encode(hasher.finalize());
    let verification = match download.sha256.as_deref() {
        Some(declared) if declared.eq_ignore_ascii_case(&received_sha) => {
            if let Some(declared_bytes) = download.declared_bytes {
                if declared_bytes != bytes {
                    bail!(
                        "snapshot bundle is corrupt: received {bytes} bytes but the server \
                         declared {declared_bytes} — the export was truncated"
                    );
                }
            }
            Verification::ServerSha256 {
                sha256: received_sha,
                declared_bytes: download.declared_bytes,
            }
        }
        Some(declared) => bail!(
            "snapshot bundle is corrupt: received {bytes} bytes hashing to sha256 {received_sha} \
             but the server declared {declared} — the export was truncated or altered in transit"
        ),
        None => Verification::Unverified,
    };
    fs::rename(staging.path(), output).await.with_context(|| {
        format!(
            "failed to move staged bundle {} into {}",
            staging.path().display(),
            output.display()
        )
    })?;
    let _ = staging.keep();
    Ok((bytes, verification))
}

/// Establish integrity locally with `git bundle verify`, which checks the
/// bundle's checksums and that every prerequisite commit it names exists in
/// the repository it runs in. The repository is discovered from the output
/// directory upward, then the working directory; with neither, a temporary
/// bare repository stands in (a self-contained bundle verifies there, while a
/// prerequisite-carrying bundle is told to run from inside a clone holding
/// its base). When the first verify fails, origin is fetched once and the
/// verify retried — the usual cause is a missing prerequisite base commit.
async fn verify_bundle_locally(output: &Path) -> Result<Verification> {
    let bundle = output
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", output.display()))?;
    let bundle = bundle
        .to_str()
        .with_context(|| format!("{} is not valid UTF-8", output.display()))?
        .to_owned();

    let discovered = output
        .parent()
        .and_then(discover_repository)
        .or_else(|| std::env::current_dir().ok().and_then(|dir| discover_repository(&dir)));
    match discovered {
        Some(repository) => {
            let mut after_origin_fetch = false;
            let verified = git_in(Some(repository.as_path()), &["bundle", "verify", &bundle])
                .await?;
            let verified = if !verified.status.success() {
                // The usual cause is a prerequisite base commit the local
                // repository has not heard of; fetching origin is the cheap
                // repair before declaring the bundle bad.
                let fetched =
                    git_in(Some(repository.as_path()), &["fetch", "origin", "--quiet"]).await?;
                after_origin_fetch = fetched.status.success();
                git_in(Some(repository.as_path()), &["bundle", "verify", &bundle]).await?
            } else {
                verified
            };
            if !verified.status.success() {
                bail!(
                    "snapshot bundle failed `git bundle verify`{}: {}",
                    if after_origin_fetch {
                        " (after fetching origin)"
                    } else {
                        ""
                    },
                    git_failure_text(&verified)
                );
            }
            Ok(Verification::BundleVerify { after_origin_fetch })
        }
        None => {
            let stand_in = tempfile::tempdir().context(
                "failed to create a temporary repository to verify the bundle in",
            )?;
            let init = git_in(
                None,
                &["init", "--quiet", "--bare", &stand_in.path().display().to_string()],
            )
            .await?;
            if !init.status.success() {
                bail!(
                    "could not create a temporary repository to verify the bundle: {}",
                    git_failure_text(&init)
                );
            }
            let verified = git_in(Some(stand_in.path()), &["bundle", "verify", &bundle]).await?;
            if !verified.status.success() {
                bail!(
                    "snapshot bundle failed `git bundle verify`: {} — if the bundle carries \
                     prerequisite commits, run `fabro snapshot` from inside a clone that holds \
                     the run's base commit",
                    git_failure_text(&verified)
                );
            }
            Ok(Verification::BundleVerify {
                after_origin_fetch: false,
            })
        }
    }
}

/// The repository a directory belongs to, walking upward (a `.git` file —
/// a worktree or submodule's gitlink — counts too), if any.
fn discover_repository(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        if dir.join(".git").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Run `git` in `repository` (or anywhere, for repository-free reads) and
/// collect its output.
async fn git_in(repository: Option<&Path>, args: &[&str]) -> Result<std::process::Output> {
    let mut command = Command::new("git");
    if let Some(repository) = repository {
        command.arg("-C").arg(repository);
    }
    command.args(args);
    command.output().await.context("failed to run git")
}

/// Git's own account of a failure, for an error message that names it.
fn git_failure_text(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim();
    if stderr.is_empty() {
        format!("git exited with {}", output.status)
    } else {
        stderr.to_string()
    }
}

/// `--ref`'s check: `git bundle list-heads` reads the refs a bundle carries
/// without a repository or a fetch, so the salvage fails named when the
/// bundle would not deliver the ref it was asked for.
async fn require_bundle_ref(output: &Path, expected: &str) -> Result<(String, String)> {
    let heads = git_in(None, &["bundle", "list-heads", &output.display().to_string()]).await?;
    if !heads.status.success() {
        bail!(
            "could not list the refs in {}: {}",
            output.display(),
            git_failure_text(&heads)
        );
    }
    let listing = String::from_utf8_lossy(&heads.stdout);
    for line in listing.lines() {
        let Some((sha, name)) = line.split_once(' ') else {
            continue;
        };
        let name = name.trim();
        if name == expected {
            return Ok((name.to_owned(), sha.to_owned()));
        }
    }
    bail!(
        "the bundle does not carry ref {expected}; it carries:\n{}",
        listing.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_output_names_the_run() {
        let run_id: RunId = "01ARZ3NDEKTSV4RRFFQ69G5FAV"
            .parse()
            .expect("the fixture run id parses");
        assert_eq!(
            default_output(&run_id),
            Path::new("fabro-snapshot-01ARZ3NDEKTSV4RRFFQ69G5FAV.bundle")
        );
    }

    #[test]
    fn verification_labels_name_the_strategy() {
        assert_eq!(
            Verification::ServerSha256 {
                sha256: "abc".to_owned(),
                declared_bytes: None,
            }
            .label(),
            "server-sha256"
        );
        assert_eq!(
            Verification::BundleVerify {
                after_origin_fetch: false,
            }
            .label(),
            "git-bundle-verify"
        );
        assert_eq!(
            Verification::BundleVerify {
                after_origin_fetch: true,
            }
            .label(),
            "git-bundle-verify-after-origin-fetch"
        );
    }
}
