//! One-command toolchain pin for the release runbook (fabro-4f44).
//!
//! Ordering rule (2026-09-16 release-pipeline decision): the toolchain
//! environment must stay in sync with the deployed server binary, so the pin
//! advances only after the matching server deploy. This command enforces
//! that fail-closed: before EVERY write attempt (including the revision-race
//! retry) it compares the tag's git sha12 against the deployed server's
//! build sha (`GET /api/v1/system/info`) and aborts on mismatch, which also
//! guarantees a NEWER pin is never overwritten with an older tag.

use std::process::Command as StdCommand;

use anyhow::{Context as _, Result, bail};
use fabro_api::types;
use tokio::task::spawn_blocking;

use super::{
    ReplaceOutcome, environment_to_replace_request, is_revision_conflict, replace_attempt,
    short_revision,
};
use crate::args::EnvPinToolchainArgs;
use crate::command_context::CommandContext;
use crate::server_client::Client;

pub(super) const TOOLCHAIN_ENV_ID: &str = "toolchain";
const TOOLCHAIN_IMAGE_REPO: &str = "ghcr.io/denkhaus/fabro-toolchain";
const PIN_ATTEMPTS: usize = 2;

#[expect(
    clippy::disallowed_methods,
    reason = "sync git helper mirrors scripts/run-images.nu tagging; async callers use spawn_blocking"
)]
fn git_head_sha12() -> Result<String> {
    let output = StdCommand::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .context("failed to run git rev-parse")?;
    if !output.status.success() {
        bail!("not a git repository — pass --tag <sha12> instead of --from-run-images");
    }
    let sha12 = String::from_utf8(output.stdout)
        .context("git rev-parse output was not valid UTF-8")?
        .trim()
        .to_string();
    validate_sha12(&sha12)?;
    Ok(sha12)
}

fn validate_sha12(tag: &str) -> Result<()> {
    let valid = tag.len() == 12
        && tag
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    anyhow::ensure!(
        valid,
        "toolchain tag must be a 12-character lowercase hex git sha, got {tag:?}"
    );
    Ok(())
}

fn toolchain_image(sha12: &str) -> String {
    format!("{TOOLCHAIN_IMAGE_REPO}:{sha12}")
}

/// Fail-closed parity gate: the tag's sha12 must match the deployed server's
/// build sha. A mismatch means the deploy for this tag has not landed (or a
/// newer server is already running), so pinning must not proceed.
async fn verify_deployed_server_parity(client: &Client, sha12: &str) -> Result<()> {
    let info = client.get_system_info().await?;
    let Some(server_sha) = info.git_sha else {
        bail!(
            "deployed server did not report a build sha; refusing to pin {} \
             (deploy the server first, then pin)",
            toolchain_image(sha12)
        );
    };
    anyhow::ensure!(
        server_sha.starts_with(sha12),
        "deployed-server/tag mismatch: server is built from {server_sha}, tag is {sha12} ({}); \
         pin only after deploying the matching server image",
        toolchain_image(sha12)
    );
    Ok(())
}

fn prepare_pin<'a>(
    image: &'a str,
    previous_image: &'a mut Option<String>,
) -> impl FnMut(&types::Environment) -> Result<Option<types::ReplaceEnvironmentRequest>> + 'a {
    move |current| {
        if current.settings.image.docker.as_deref() == Some(image) {
            return Ok(None);
        }
        let mut request = environment_to_replace_request(current)?;
        if let Some(docker) = &current.settings.image.docker {
            *previous_image = Some(docker.clone());
        }
        request.image.docker = Some(image.to_string());
        Ok(Some(request))
    }
}

pub(super) async fn pin_toolchain_command(
    args: &EnvPinToolchainArgs,
    ctx: &CommandContext,
) -> Result<()> {
    let sha12 = if let Some(tag) = args.tag.as_deref() {
        validate_sha12(tag)?;
        tag.to_string()
    } else {
        spawn_blocking(git_head_sha12)
            .await
            .context("git rev-parse task panicked")??
    };
    let image = toolchain_image(&sha12);

    let client = ctx.server().await?;
    let printer = ctx.printer();

    verify_deployed_server_parity(&client, &sha12).await?;

    for attempt in 0..PIN_ATTEMPTS {
        let mut previous_image = None;
        let outcome = replace_attempt(
            &client,
            TOOLCHAIN_ENV_ID,
            &mut prepare_pin(&image, &mut previous_image),
        )
        .await;
        match outcome {
            Ok(ReplaceOutcome::Unchanged(current)) => {
                fabro_util::printerr!(
                    printer,
                    "Environment {} already pinned to {} ({}).",
                    current.id,
                    image,
                    short_revision(current.revision.as_str())
                );
                return Ok(());
            }
            Ok(ReplaceOutcome::Replaced {
                previous_revision,
                environment: updated,
            }) => {
                fabro_util::printerr!(
                    printer,
                    "Pinned environment {} ({} -> {}, {} -> {}).",
                    updated.id,
                    previous_image.as_deref().unwrap_or("-"),
                    image,
                    short_revision(&previous_revision),
                    short_revision(updated.revision.as_str()),
                );
                return Ok(());
            }
            Err(err) => {
                if attempt + 1 == PIN_ATTEMPTS || !is_revision_conflict(&err) {
                    return Err(err.context(format!(
                        "pinning environment {TOOLCHAIN_ENV_ID} to {image} failed"
                    )));
                }
                // Revision race: re-verify parity against the fresh server
                // state before the single retry, so a deploy that landed in
                // between still blocks an outdated pin.
                verify_deployed_server_parity(&client, &sha12).await?;
            }
        }
    }
    unreachable!("pin loop returns from every path")
}

#[cfg(test)]
mod tests {
    use super::validate_sha12;

    #[test]
    fn sha12_validation_accepts_lowercase_hex_and_rejects_everything_else() {
        assert!(validate_sha12("0123456789ab").is_ok());
        assert!(
            validate_sha12("0123456789AB").is_err(),
            "uppercase rejected"
        );
        assert!(validate_sha12("0123456789g").is_err(), "non-hex rejected");
        assert!(
            validate_sha12("0123456789abc").is_err(),
            "13 chars rejected"
        );
        assert!(validate_sha12("").is_err(), "empty rejected");
    }
}
