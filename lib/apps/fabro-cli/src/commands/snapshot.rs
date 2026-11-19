//! `fabro snapshot <run>` — export a run's snapshot repository as a `git
//! bundle` through the server's read door, so salvaging a terminal run's
//! checkpoint work needs no ssh detour onto the host that owns the store.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use fabro_types::RunId;
use futures::StreamExt;
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
    let bytes = write_bundle(client.as_ref(), &run_id, &output).await?;
    if ctx.json_output() {
        print_json_pretty(&serde_json::json!({
            "run_id": run_id,
            "output": absolute_or_current(&output),
            "bytes":   bytes,
        }))?;
    } else {
        fabro_util::printout!(
            printer,
            "Exported snapshot bundle for run {} to {} ({} bytes)",
            run_id,
            output.display(),
            bytes
        );
    }
    Ok(())
}

fn default_output(run_id: &RunId) -> PathBuf {
    PathBuf::from(format!("fabro-snapshot-{run_id}.bundle"))
}

/// Stream the bundle into a staging file beside the output, then move it into
/// place: a failed transfer never leaves a truncated bundle at the target
/// path, and the bytes never sit whole in memory.
async fn write_bundle(client: &Client, run_id: &RunId, output: &Path) -> Result<u64> {
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

    let mut stream = client.download_run_snapshot(run_id).await?;
    let mut file = fs::File::create(staging.path()).await?;
    let mut bytes = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("snapshot bundle stream failed")?;
        file.write_all(&chunk).await?;
        bytes += chunk.len() as u64;
    }
    file.flush().await?;
    drop(file);
    fs::rename(staging.path(), output).await.with_context(|| {
        format!(
            "failed to move staged bundle {} into {}",
            staging.path().display(),
            output.display()
        )
    })?;
    let _ = staging.keep();
    Ok(bytes)
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
}
