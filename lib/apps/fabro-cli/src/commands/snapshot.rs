//! `fabro snapshot <run>` — export a run's snapshot repository as a `git
//! bundle` through the server's read door, so salvaging a terminal run's
//! checkpoint work needs no ssh detour onto the host that owns the store.
//!
//! The door carries an integrity proof (work#1430): the server computes the
//! complete bundle's SHA256 and byte count before streaming, and this command
//! verifies the received bytes against that proof before the file is moved
//! into place — a transfer cut mid-flight fails loudly instead of leaving a
//! truncated bundle that looks like a good export.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use fabro_types::RunId;
use futures::StreamExt;
use sha2::{Digest, Sha256};
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::args::SnapshotArgs;
use crate::command_context::CommandContext;
use crate::server_client::Client;
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
    let export = write_bundle(client.as_ref(), &run_id, &output).await?;
    if ctx.json_output() {
        print_json_pretty(&serde_json::json!({
            "run_id": run_id,
            "output": absolute_or_current(&output),
            "bytes":   export.bytes,
            "sha256":  export.sha256,
        }))?;
    } else {
        fabro_util::printout!(
            printer,
            "Exported snapshot bundle for run {} to {} ({} bytes, sha256 {})",
            run_id,
            output.display(),
            export.bytes,
            export.sha256
        );
    }
    Ok(())
}

fn default_output(run_id: &RunId) -> PathBuf {
    PathBuf::from(format!("fabro-snapshot-{run_id}.bundle"))
}

/// What one export produced: the received size and the hash the CLI computed
/// over the bytes it actually received.
struct BundleExport {
    bytes:  u64,
    sha256: String,
}

/// Stream the bundle into a staging file beside the output, verify the
/// received bytes against the server's integrity proof, then move the file
/// into place (work#1430): a failed or truncated transfer never leaves a
/// bundle at the target path, and the bytes never sit whole in memory.
async fn write_bundle(client: &Client, run_id: &RunId, output: &Path) -> Result<BundleExport> {
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

    let mut download = client.download_run_snapshot(run_id).await?;
    let mut file = fs::File::create(staging.path()).await?;
    let mut hasher = Sha256::new();
    let mut bytes = 0_u64;
    while let Some(chunk) = download.stream.next().await {
        let chunk = chunk.context("snapshot bundle stream failed")?;
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        bytes += chunk.len() as u64;
    }
    file.flush().await?;
    drop(file);
    let sha256 = format!("{:x}", hasher.finalize());
    verify_bundle_integrity(
        run_id,
        bytes,
        &sha256,
        download.sha256.as_deref(),
        download.bytes,
    )?;
    fs::rename(staging.path(), output).await.with_context(|| {
        format!(
            "failed to move staged bundle {} into {}",
            staging.path().display(),
            output.display()
        )
    })?;
    let _ = staging.keep();
    if download.sha256.is_none() && download.bytes.is_none() {
        // An old server predates the proof (work#1430): the export cannot be
        // verified, so say so instead of passing silently.
        fabro_util::warn_user!(
            "server sent no bundle integrity proof (x-bundle-sha256/x-bundle-bytes); \
             the export could not be verified — check it with \
             `git bundle verify {}`",
            output.display()
        );
    }
    Ok(BundleExport { bytes, sha256 })
}

/// The consumer half of the door's integrity proof (work#1430): the received
/// byte count and hash must match what the server computed over the complete
/// bundle. A short transfer, or bytes that hash differently, mean the
/// transfer was cut mid-stream — refuse with a named truncation error, never
/// pass the bundle off as a good export.
fn verify_bundle_integrity(
    run_id: &RunId,
    received_bytes: u64,
    received_sha256: &str,
    expected_sha256: Option<&str>,
    expected_bytes: Option<u64>,
) -> Result<()> {
    if let Some(expected_bytes) = expected_bytes {
        if received_bytes != expected_bytes {
            bail!(
                "snapshot bundle for run {run_id} is truncated: received {received_bytes} of \
                 {expected_bytes} bytes — the transfer was cut mid-stream; the output file was \
                 not written, remove it if present and retry"
            );
        }
    }
    if let Some(expected_sha256) = expected_sha256 {
        if received_sha256 != expected_sha256 {
            bail!(
                "snapshot bundle for run {run_id} is truncated or corrupt: the received bytes \
                 hash to {received_sha256} but the server's complete bundle is \
                 {expected_sha256} — the transfer did not deliver the bundle whole; the output \
                 file was not written, remove it if present and retry"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_output_names_the_run() {
        let run_id: RunId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
        assert_eq!(
            default_output(&run_id),
            Path::new("fabro-snapshot-01ARZ3NDEKTSV4RRFFQ69G5FAV.bundle")
        );
    }

    #[test]
    fn a_transfer_matching_the_proof_passes() {
        let run_id: RunId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
        let sha256 = format!("{:x}", Sha256::digest(b"bundle bytes"));
        verify_bundle_integrity(&run_id, 12, &sha256, Some(sha256.as_str()), Some(12))
            .expect("a transfer matching the proof is accepted");
    }

    #[test]
    fn a_short_transfer_is_refused_as_truncated() {
        let run_id: RunId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
        let sha256 = format!("{:x}", Sha256::digest(b"half a bundle"));
        let err = verify_bundle_integrity(&run_id, 100, &sha256, Some(sha256.as_str()), Some(200))
            .expect_err("a short transfer must be refused");
        let message = err.to_string();
        assert!(
            message.contains("truncated"),
            "the refusal names truncation: {message}"
        );
        assert!(
            message.contains("100") && message.contains("200"),
            "the refusal names both sizes: {message}"
        );
        assert!(
            message.contains(&run_id.to_string()),
            "the refusal names the run: {message}"
        );
    }

    #[test]
    fn a_hash_mismatch_is_refused_as_truncated_even_at_full_length() {
        let run_id: RunId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
        let received = format!("{:x}", Sha256::digest(b"corrupted bytes"));
        let expected = format!("{:x}", Sha256::digest(b"bundle bytes"));
        let err = verify_bundle_integrity(&run_id, 12, &received, Some(expected.as_str()), Some(12))
            .expect_err("a hash mismatch must be refused");
        let message = err.to_string();
        assert!(
            message.contains("truncated") || message.contains("corrupt"),
            "the refusal names the damage: {message}"
        );
        assert!(
            message.contains(&received) && message.contains(&expected),
            "the refusal names both hashes: {message}"
        );
    }

    #[test]
    fn an_absent_proof_passes_unverified() {
        let run_id: RunId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
        verify_bundle_integrity(&run_id, 12, "abc", None, None)
            .expect("a server without the proof does not fail the export");
    }
}
