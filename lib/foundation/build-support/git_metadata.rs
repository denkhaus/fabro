use std::path::{Path, PathBuf};
use std::process::Command;

/// Length of the short git sha embedded as `FABRO_GIT_SHA` and compared by
/// the `fabro env pin-toolchain` parity gate. Matches `git rev-parse
/// --short=12` and the `scripts/run-images.nu` image tags, so a
/// correctly-built server always passes the parity check (fabro-6ffb).
pub const SHORT_SHA_LEN: usize = 12;

#[derive(Debug, Eq, PartialEq)]
pub struct BuildGitMetadata {
    pub rerun_paths: Vec<PathBuf>,
    pub short_sha:   String,
}

pub fn collect_from(package_dir: &Path) -> BuildGitMetadata {
    let mut rerun_paths = Vec::new();

    if let Some(head_path) = git_output(package_dir, ["rev-parse", "--git-path", "HEAD"]) {
        rerun_paths.push(PathBuf::from(head_path));
    }

    if let Some(head_ref) = git_output(package_dir, ["symbolic-ref", "-q", "HEAD"]) {
        if let Some(ref_path) = git_output(package_dir, ["rev-parse", "--git-path", &head_ref]) {
            rerun_paths.push(PathBuf::from(ref_path));
        }
    }

    let short_sha = git_output(package_dir, ["rev-list", "-1", "HEAD"])
        .map(|sha| {
            if sha.len() >= SHORT_SHA_LEN {
                sha[..SHORT_SHA_LEN].to_string()
            } else {
                sha
            }
        })
        .unwrap_or_default();

    BuildGitMetadata {
        rerun_paths,
        short_sha,
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "Build scripts read Cargo-provided PROFILE outside application runtime configuration."
)]
pub fn cargo_profile() -> String {
    std::env::var("PROFILE").unwrap_or_default()
}

/// The `FABRO_GIT_SHA` image-build injection: image builds (`cargo dev
/// docker-build`) compile without usable git metadata inside the builder
/// container, so the build plan injects the sha as this env var and the
/// build script never embeds an empty sha (fabro-6ffb).
#[expect(
    clippy::disallowed_methods,
    reason = "Build scripts read the FABRO_GIT_SHA image-build injection seam outside application runtime configuration."
)]
pub fn injected_git_sha() -> Option<String> {
    std::env::var("FABRO_GIT_SHA")
        .ok()
        .filter(|sha| !sha.is_empty())
}

#[expect(
    clippy::disallowed_methods,
    reason = "Build scripts run outside Tokio and need synchronous git probes for embedded build metadata."
)]
fn git_output<const N: usize>(package_dir: &Path, args: [&str; N]) -> Option<String> {
    Command::new("git")
        .current_dir(package_dir)
        .args(args)
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout)
                    .ok()
                    .map(|output| output.trim().to_string())
            } else {
                None
            }
        })
        .filter(|output| !output.is_empty())
}
