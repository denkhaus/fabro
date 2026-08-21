use std::path::Path;

use anyhow::{Result, bail};
use fabro_github::{normalize_repo_origin_url, rewrite_candidates};
use fabro_sandbox::daytona::detect_repo_info;

pub(crate) fn ensure_matching_repo_origin(
    expected_origin_url: Option<&str>,
    action: &str,
) -> Result<()> {
    let Some(expected_origin_url) = expected_origin_url else {
        return Ok(());
    };

    let cwd = std::env::current_dir()?;
    let (origin_url, _) = detect_repo_info(&cwd).map_err(|_| {
        anyhow::anyhow!(
            "Current directory is not a git repository with an origin remote; refusing to {action} run from repository '{expected_origin_url}'"
        )
    })?;

    if !origin_matches_expected(&origin_url, expected_origin_url, &cwd) {
        let current_origin_url = normalize_repo_origin_url(&origin_url);
        bail!(
            "Current repository origin '{current_origin_url}' does not match run repository '{expected_origin_url}'; refusing to {action} this run from the wrong checkout"
        );
    }

    Ok(())
}

/// Whether the raw local `origin` URL denotes the same repository as
/// `expected`, honoring `url.<replacement>.insteadOf` config rewrites.
///
/// A checkout can store its origin in rewritten form (for example an SSH host
/// alias used for account separation) while the run spec stores the canonical
/// URL, or the other way around. Both rewrite directions are therefore
/// compared, after normalization, before the guard rejects the operation.
fn origin_matches_expected(raw_origin: &str, expected: &str, repo_path: &Path) -> bool {
    let expected_normalized = normalize_repo_origin_url(expected);
    rewrite_candidates(raw_origin, &insteadof_rewrites_for(repo_path))
        .iter()
        .any(|candidate| normalize_repo_origin_url(candidate) == expected_normalized)
}

/// `url.<replacement>.insteadOf` pairs visible from `repo_path`.
fn insteadof_rewrites_for(repo_path: &Path) -> Vec<(String, String)> {
    git2::Repository::discover(repo_path)
        .ok()
        .and_then(|repo| fabro_manifest::insteadof_rewrites(&repo))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::ensure_matching_repo_origin;

    #[test]
    fn missing_expected_origin_skips_guard() {
        ensure_matching_repo_origin(None, "fork").unwrap();
    }
}
