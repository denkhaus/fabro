use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use tokio::fs;
use tracing::{debug, info};

use crate::args::{CpArgs, ServerTargetArgs};
use crate::command_context::CommandContext;
use crate::server_client::Client;
use crate::shared::{print_json_pretty, split_run_path};

#[derive(Debug)]
enum CopyDirection {
    Download {
        run_prefix:  String,
        remote_path: String,
        local_path:  PathBuf,
    },
    Upload {
        local_path:  PathBuf,
        run_prefix:  String,
        remote_path: String,
    },
}

pub(crate) async fn cp_command(args: CpArgs, base_ctx: &CommandContext) -> Result<()> {
    let direction = parse_direction(&args.src, &args.dst)?;

    match direction {
        CopyDirection::Download {
            run_prefix,
            remote_path,
            local_path,
        } => {
            let (client, run_id) =
                resolve_client_and_run_id(base_ctx, &args.server, &run_prefix).await?;

            let file_count = if args.recursive {
                Some(download_recursive(&client, &run_id, &remote_path, &local_path).await?)
            } else {
                debug!(path = %remote_path, "Downloading file from sandbox");
                write_sandbox_file(&client, &run_id, &remote_path, &local_path).await?;
                None
            };

            if base_ctx.json_output() {
                let mut value = serde_json::json!({
                    "direction": "download",
                    "recursive": args.recursive,
                    "remote_path": remote_path,
                    "local_path": local_path,
                });
                if let Some(count) = file_count {
                    value["file_count"] = count.into();
                }
                print_json_pretty(&value)?;
            }

            info!(direction = "download", path = %remote_path, "Copy complete");
        }
        CopyDirection::Upload {
            local_path,
            run_prefix,
            remote_path,
        } => {
            let (client, run_id) =
                resolve_client_and_run_id(base_ctx, &args.server, &run_prefix).await?;

            let file_count = if args.recursive {
                Some(upload_recursive(&client, &run_id, &local_path, &remote_path).await?)
            } else {
                debug!(path = %remote_path, "Uploading file to sandbox");
                upload_sandbox_file(&client, &run_id, &local_path, &remote_path).await?;
                None
            };

            if base_ctx.json_output() {
                let mut value = serde_json::json!({
                    "direction": "upload",
                    "recursive": args.recursive,
                    "remote_path": remote_path,
                    "local_path": local_path,
                });
                if let Some(count) = file_count {
                    value["file_count"] = count.into();
                }
                print_json_pretty(&value)?;
            }
            info!(direction = "upload", path = %remote_path, "Copy complete");
        }
    }

    Ok(())
}

fn parse_direction(src: &str, dst: &str) -> Result<CopyDirection> {
    let src_parts = split_run_path(src);
    let dst_parts = split_run_path(dst);

    match (src_parts, dst_parts) {
        (Some((run_prefix, remote_path)), None) => Ok(CopyDirection::Download {
            run_prefix:  run_prefix.to_string(),
            remote_path: remote_path.to_string(),
            local_path:  PathBuf::from(dst),
        }),
        (None, Some((run_prefix, remote_path))) => Ok(CopyDirection::Upload {
            local_path:  PathBuf::from(src),
            run_prefix:  run_prefix.to_string(),
            remote_path: remote_path.to_string(),
        }),
        (Some(_), Some(_)) => {
            bail!("Cannot copy between two sandboxes; one argument must be a local path")
        }
        (None, None) => bail!("One argument must contain a run-id prefix (e.g. <run-id>:<path>)"),
    }
}

async fn resolve_client_and_run_id(
    base_ctx: &CommandContext,
    server: &ServerTargetArgs,
    run_prefix: &str,
) -> Result<(Client, fabro_types::RunId)> {
    let ctx = base_ctx.with_target(server)?;
    let client = ctx.server().await?;
    let run_id = client.resolve_run(run_prefix).await?.id;
    Ok((client.clone_for_reuse(), run_id))
}

async fn write_sandbox_file(
    client: &Client,
    run_id: &fabro_types::RunId,
    remote_path: &str,
    local_path: &Path,
) -> Result<()> {
    if let Some(parent) = local_path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }
    let bytes = client.get_sandbox_file(run_id, remote_path).await?;
    fs::write(local_path, bytes)
        .await
        .with_context(|| format!("Failed to write {}", local_path.display()))?;
    Ok(())
}

async fn upload_sandbox_file(
    client: &Client,
    run_id: &fabro_types::RunId,
    local_path: &Path,
    remote_path: &str,
) -> Result<()> {
    let bytes = fs::read(local_path)
        .await
        .with_context(|| format!("Failed to read {}", local_path.display()))?;
    client.put_sandbox_file(run_id, remote_path, bytes).await
}

async fn download_recursive(
    client: &Client,
    run_id: &fabro_types::RunId,
    remote_path: &str,
    local_path: &Path,
) -> Result<usize> {
    let archive = client.get_sandbox_archive(run_id, remote_path).await?;
    info!(bytes = archive.len(), path = %remote_path, "Fetched sandbox archive");

    // `unpack_in` canonicalizes its destination, so the target directory
    // must exist before the first entry unpacks.
    fs::create_dir_all(local_path)
        .await
        .with_context(|| format!("Failed to create {}", local_path.display()))?;
    let file_count = untar_archive(&archive, local_path)?;
    debug!(count = file_count, "Recursive download complete");
    Ok(file_count)
}

/// Unpacks a sandbox archive under `local_path` and returns the number of
/// regular files it contained (work#1416). The archive's entries are
/// relative to the copied directory, so unpacking under `local_path`
/// preserves the per-file layout of the old per-file download loop.
/// `unpack_in` ignores the leading `./` a tar of `.` produces and refuses
/// `..` traversal.
fn untar_archive(archive: &[u8], local_path: &Path) -> Result<usize> {
    let mut tar_archive = tar::Archive::new(archive);
    let mut file_count = 0usize;
    for entry in tar_archive
        .entries()
        .context("Failed to read the sandbox archive")?
    {
        let mut entry = entry.context("Failed to read a sandbox archive entry")?;
        let name = entry
            .path()
            .context("Failed to read a sandbox archive entry path")?
            .into_owned();
        if entry.header().entry_type().is_file() {
            file_count += 1;
        }
        entry.unpack_in(local_path).with_context(|| {
            format!(
                "Failed to unpack '{}' into {}",
                name.display(),
                local_path.display()
            )
        })?;
    }
    Ok(file_count)
}

async fn upload_recursive(
    client: &Client,
    run_id: &fabro_types::RunId,
    local_path: &Path,
    remote_path: &str,
) -> Result<usize> {
    let mut file_count = 0usize;
    let mut stack = vec![(local_path.to_path_buf(), remote_path.to_string())];

    while let Some((dir_path, dir_remote)) = stack.pop() {
        let mut entries = fs::read_dir(&dir_path)
            .await
            .with_context(|| format!("Failed to read directory {}", dir_path.display()))?;

        while let Some(entry) = entries.next_entry().await? {
            let entry_path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();
            let remote_file = format!("{dir_remote}/{file_name}");

            if entry.file_type().await?.is_dir() {
                stack.push((entry_path, remote_file));
            } else {
                debug!(path = %remote_file, "Uploading file to sandbox");
                upload_sandbox_file(client, run_id, &entry_path, &remote_file).await?;
                file_count += 1;
            }
        }
    }
    debug!(count = file_count, "Recursive upload complete");
    Ok(file_count)
}

#[cfg(test)]
mod tests {
    use tokio::fs;

    use super::*;

    #[test]
    fn parse_direction_download() {
        let direction = parse_direction("abc123:/some/file.txt", "./local.txt").unwrap();
        match direction {
            CopyDirection::Download {
                run_prefix,
                remote_path,
                local_path,
            } => {
                assert_eq!(run_prefix, "abc123");
                assert_eq!(remote_path, "/some/file.txt");
                assert_eq!(local_path, PathBuf::from("./local.txt"));
            }
            CopyDirection::Upload { .. } => panic!("expected download"),
        }
    }

    #[test]
    fn parse_direction_upload() {
        let direction = parse_direction("./local.txt", "abc123:/remote.txt").unwrap();
        match direction {
            CopyDirection::Upload {
                local_path,
                run_prefix,
                remote_path,
            } => {
                assert_eq!(local_path, PathBuf::from("./local.txt"));
                assert_eq!(run_prefix, "abc123");
                assert_eq!(remote_path, "/remote.txt");
            }
            CopyDirection::Download { .. } => panic!("expected upload"),
        }
    }

    #[test]
    fn parse_direction_rejects_sandbox_to_sandbox_copy() {
        let err = parse_direction("abc123:/in.txt", "def456:/out.txt").unwrap_err();
        assert!(
            err.to_string()
                .contains("Cannot copy between two sandboxes")
        );
    }

    /// A small in-memory tar with `./`-prefixed entries, the shape the
    /// server's in-sandbox `tar -C <dir> .` produces.
    fn sandbox_tar(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, content) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, name, content.as_bytes())
                .unwrap();
        }
        builder.into_inner().unwrap()
    }

    #[tokio::test]
    async fn untar_archive_preserves_layout_and_counts_regular_files() {
        let workspace = tempfile::tempdir().unwrap();
        let archive = sandbox_tar(&[
            ("./src/main.rs", "fn main() {}"),
            ("./README.md", "hello"),
            ("./src/nested/deep.rs", "pub fn f() {}"),
        ]);

        let count = untar_archive(&archive, workspace.path()).unwrap();

        assert_eq!(count, 3);
        assert_eq!(
            fs::read_to_string(workspace.path().join("src/main.rs"))
                .await
                .unwrap(),
            "fn main() {}"
        );
        assert_eq!(
            fs::read_to_string(workspace.path().join("README.md"))
                .await
                .unwrap(),
            "hello"
        );
        assert_eq!(
            fs::read_to_string(workspace.path().join("src/nested/deep.rs"))
                .await
                .unwrap(),
            "pub fn f() {}"
        );
    }

    #[tokio::test]
    async fn untar_archive_refuses_traversal_entries() {
        let workspace = tempfile::tempdir().unwrap();
        let archive = raw_traversal_tar("../escape.txt", "outside");

        let count = untar_archive(&archive, workspace.path()).unwrap();

        assert_eq!(count, 1, "the file is counted, never unpacked outside");
        assert!(
            !fs::try_exists(workspace.path().join("../escape.txt"))
                .await
                .unwrap(),
            "a traversal entry must not escape the unpack root"
        );
    }

    /// A one-entry GNU tar whose member name is a traversal path. Built
    /// from raw header bytes because `tar::Builder` refuses `..` member
    /// names — the unpack side must refuse them on its own.
    fn raw_traversal_tar(name: &str, content: &str) -> Vec<u8> {
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[100..108].copy_from_slice(b"0000644\0");
        header[108..116].copy_from_slice(b"0000000\0");
        header[116..124].copy_from_slice(b"0000000\0");
        let size = format!("{:011o}\0", content.len());
        header[124..136].copy_from_slice(size.as_bytes());
        header[136..148].copy_from_slice(b"00000000000\0");
        header[148..156].copy_from_slice(b"        ");
        header[156] = b'0';
        header[257..262].copy_from_slice(b"ustar");
        header[263..265].copy_from_slice(b"00");
        let checksum: u32 = header.iter().map(|&byte| u32::from(byte)).sum();
        header[148..156].copy_from_slice(format!("{checksum:06o}\0 ").as_bytes());

        let mut archive = header.to_vec();
        archive.extend_from_slice(content.as_bytes());
        archive.extend(std::iter::repeat_n(0, 512 - content.len()));
        archive.extend(std::iter::repeat_n(0, 1024));
        archive
    }
}
