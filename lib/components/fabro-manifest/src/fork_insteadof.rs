//! Fork (denkhaus): `url.<replacement>.insteadOf` rewrite awareness for
//! run-target derivation (seed fabro-f394).
//!
//! Upstream `fabro-manifest` compares and parses origin URLs literally: an
//! origin stored under an SSH HOST ALIAS (`git@denkhaus.github.com:denkhaus/
//! fabro.git`) cannot be represented as a canonical GitHub run target, and a
//! configured `[run.scm]` origin never matches it, so alias-origin checkouts
//! cannot start runs. This module threads the host's git `insteadOf` config
//! into those comparisons through [`fabro_github::rewrite_candidates`], the
//! helper built for exactly this shape.
//!
//! Fork placement: this behavior lives in its own fork-owned file so an
//! upstream merge cannot silently absorb it; the upstream-side touch points
//! in `lib.rs` are three lines (module declaration, origin fallback,
//! publish equality). Presence pins live in `tests/fork_insteadof_tests.rs`.

use std::path::Path;

use fabro_types::GitRunTarget;

use crate::github_run_target;

/// The `url.<replacement>.insteadOf` rewrites visible to `repo_path`, as
/// `(replacement, matcher)` pairs — the shape
/// [`fabro_github::rewrite_candidates`] consumes.
///
/// Runs `git config --get-regexp` in `repo_path` so repository-local
/// rewrites participate alongside host-level ones. Git lowercases config
/// keys, so the `url.`/`.insteadof` affixes match regardless of how the
/// user wrote them. Any failure (no git binary, no config) yields no
/// rewrites: comparisons then behave exactly like upstream.
#[must_use]
pub fn host_insteadof_rewrites(repo_path: &Path) -> Vec<(String, String)> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["config", "--get-regexp", r"url\..*\.insteadof"])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_insteadof_regexp(&String::from_utf8_lossy(&output.stdout))
}

fn parse_insteadof_regexp(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let (key, matcher) = line.split_once(' ')?;
            let replacement = key.strip_prefix("url.")?.strip_suffix(".insteadof")?;
            if replacement.is_empty() || matcher.is_empty() {
                return None;
            }
            Some((replacement.to_string(), matcher.trim().to_string()))
        })
        .collect()
}

/// The normalized GitHub origin `raw_origin` denotes under `rewrites`,
/// when any rewrite candidate is one.
///
/// Tries every rewrite candidate (raw URL first, then git's forward and
/// inverse rewrites) and returns the first normalized candidate
/// [`fabro_github::parse_github_owner_repo`] accepts. `None` when no
/// candidate is a GitHub HTTPS URL.
///
/// Callers should try this against BOTH origin views a checkout offers:
/// libgit2 reports `remote.url()` with `insteadOf` rewrites already
/// applied (a transport URL like `file://…`), while the raw
/// `remote.origin.url` config bytes keep the pre-rewrite form. Each view
/// recovers the canonical GitHub origin under the other's rewrites.
#[must_use]
pub fn try_canonical_github_origin(
    raw_origin: &str,
    rewrites: &[(String, String)],
) -> Option<String> {
    fabro_github::rewrite_candidates(raw_origin, rewrites)
        .iter()
        .map(|candidate| fabro_github::normalize_repo_origin_url(candidate))
        .find(|normalized| fabro_github::parse_github_owner_repo(normalized).is_ok())
}

/// The normalized GitHub origin `raw_origin` denotes under `rewrites`.
///
/// Delegates to [`try_canonical_github_origin`]; when no candidate is a
/// GitHub HTTPS URL, the raw origin is normalized unchanged, preserving
/// upstream behavior for non-GitHub checkouts.
#[must_use]
pub fn canonical_github_origin(raw_origin: &str, rewrites: &[(String, String)]) -> String {
    try_canonical_github_origin(raw_origin, rewrites)
        .unwrap_or_else(|| fabro_github::normalize_repo_origin_url(raw_origin))
}

/// A GitHub run target for `origin_url` under `rewrites`, trying every
/// rewrite candidate before giving up.
#[must_use]
pub fn github_run_target_under_rewrites(
    origin_url: &str,
    branch: &str,
    rewrites: &[(String, String)],
) -> Option<GitRunTarget> {
    fabro_github::rewrite_candidates(origin_url, rewrites)
        .iter()
        .find_map(|candidate| github_run_target(candidate, branch))
}

/// Whether `a` and `b` denote the same repository once each side's
/// `insteadOf` rewrite candidates are normalized — the comparison
/// `publish_manifest_branch_best_effort` and the `[run.scm]`
/// configured-vs-origin equality check need so an alias-origin checkout
/// still matches its canonical configured repository.
#[must_use]
pub fn origins_denote_same_repository(a: &str, b: &str, rewrites: &[(String, String)]) -> bool {
    let candidates = |url: &str| {
        fabro_github::rewrite_candidates(url, rewrites)
            .into_iter()
            .map(|candidate| fabro_github::normalize_repo_origin_url(&candidate))
            .collect::<Vec<_>>()
    };
    let left = candidates(a);
    let right = candidates(b);
    left.iter().any(|l| right.contains(l))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_insteadof_regexp_extracts_replacement_matcher_pairs() {
        let rewrites = parse_insteadof_regexp(
            "url.git@alias.example:owner/.insteadof https://github.com/owner/\n\
             url.https://example.com/.insteadof git@example.com:owner/\n",
        );
        assert_eq!(rewrites, vec![
            (
                "git@alias.example:owner/".to_string(),
                "https://github.com/owner/".to_string(),
            ),
            (
                "https://example.com/".to_string(),
                "git@example.com:owner/".to_string(),
            ),
        ],);
    }

    #[test]
    fn parse_insteadof_regexp_skips_malformed_lines() {
        assert!(parse_insteadof_regexp("").is_empty());
        assert!(parse_insteadof_regexp("remote.origin.url git@alias:owner/repo\n").is_empty());
        assert!(parse_insteadof_regexp("url..insteadof \n").is_empty());
    }

    #[test]
    fn canonical_github_origin_recovers_the_canonical_form_from_an_alias() {
        let rewrites = vec![(
            "git@denkhaus.github.com:denkhaus/".to_string(),
            "https://github.com/denkhaus/".to_string(),
        )];
        assert_eq!(
            canonical_github_origin("git@denkhaus.github.com:denkhaus/fabro.git", &rewrites),
            "https://github.com/denkhaus/fabro",
        );
    }

    #[test]
    fn canonical_github_origin_passes_canonical_origins_through() {
        assert_eq!(
            canonical_github_origin("https://github.com/acme/widgets.git", &[]),
            "https://github.com/acme/widgets",
        );
    }

    #[test]
    fn canonical_github_origin_normalizes_non_github_origins_unchanged_in_shape() {
        assert_eq!(
            canonical_github_origin("git@denkhaus.github.com:denkhaus/fabro.git", &[]),
            "https://denkhaus.github.com/denkhaus/fabro",
        );
    }

    #[test]
    fn origins_denote_same_repository_matches_alias_against_canonical() {
        let rewrites = vec![(
            "git@denkhaus.github.com:denkhaus/".to_string(),
            "https://github.com/denkhaus/".to_string(),
        )];
        assert!(origins_denote_same_repository(
            "https://denkhaus.github.com/denkhaus/fabro.git",
            "https://github.com/denkhaus/fabro",
            &rewrites,
        ));
        assert!(!origins_denote_same_repository(
            "https://denkhaus.github.com/denkhaus/fabro.git",
            "https://github.com/denkhaus/fabro",
            &[],
        ));
        assert!(!origins_denote_same_repository(
            "https://github.com/denkhaus/fabro",
            "https://github.com/other/repo",
            &rewrites,
        ));
    }

    #[test]
    fn github_run_target_under_rewrites_resolves_an_alias_origin() {
        let rewrites = vec![(
            "git@denkhaus.github.com:denkhaus/".to_string(),
            "https://github.com/denkhaus/".to_string(),
        )];
        let target = github_run_target_under_rewrites(
            "git@denkhaus.github.com:denkhaus/fabro.git",
            "feature",
            &rewrites,
        )
        .expect("the alias origin denotes a GitHub repository");
        assert_eq!(target.repo, "denkhaus/fabro");
        assert_eq!(target.branch, "feature");
    }
}
