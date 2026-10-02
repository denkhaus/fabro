//! Fork (denkhaus) presence pins for `url.<replacement>.insteadOf` rewrite
//! support in run-target derivation (seed fabro-f394).
//!
//! These tests prove the fork feature survives an upstream merge: an origin
//! stored under an SSH host alias must still derive a canonical GitHub run
//! target, and a canonical `[run.scm]` repository must still match an
//! alias-origin checkout. Upstream `fabro-manifest` compares origins
//! literally and fails both.

#![expect(
    clippy::disallowed_methods,
    reason = "fork presence-pin tests shell out to git synchronously, mirroring the crate's inline git test helpers"
)]
#![expect(
    clippy::unwrap_used,
    reason = "test fixtures may panic on setup failures"
)]

use std::path::Path;
use std::process::Command;

use fabro_manifest::{ExactCommitStatus, fork_insteadof, observe_git_run_target};

const ALIAS_PREFIX: &str = "git@denkhaus.github.com:denkhaus/";
const CANONICAL_PREFIX: &str = "https://github.com/denkhaus/";
const ALIAS_ORIGIN: &str = "git@denkhaus.github.com:denkhaus/fabro.git";

/// Alias-origin checkout (no `[run.scm]`) still derives the canonical
/// GitHub run target, and the best-effort push rides the `insteadOf`
/// transport rewrite to the (local stand-in) remote.
#[test]
fn fork_alias_origin_without_configured_scm_derives_canonical_run_target() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let bare_prefix = init_bare_origin(temp.path());
    init_git_repo(&workspace, "feature", ALIAS_ORIGIN);
    // Canonical -> alias transport rewrite (the denkhaus host shape) plus a
    // test-only alias -> local bare rewrite so the push has somewhere to go.
    run_git(&workspace, &[
        "config",
        &format!("url.{ALIAS_PREFIX}.insteadOf"),
        CANONICAL_PREFIX,
    ]);
    run_git(&workspace, &[
        "config",
        &format!("url.{bare_prefix}.insteadOf"),
        ALIAS_PREFIX,
    ]);

    let observation = observe_git_run_target(&workspace, None)
        .expect("an attached alias-origin checkout is observable");
    let target = observation
        .run_target
        .as_ref()
        .expect("the alias origin denotes the canonical GitHub repository");
    assert_eq!(target.repo, "denkhaus/fabro");
    assert_eq!(target.branch, "feature");
    assert_eq!(
        observation.legacy_git_context.origin_url,
        "https://github.com/denkhaus/fabro",
    );
    assert_eq!(observation.exact_commit, ExactCommitStatus::Available);
    assert_eq!(target.sha, observation.legacy_git_context.sha);
}

/// A canonical `[run.scm]` repository matches an alias-origin checkout:
/// the configured-vs-origin equality check accepts `insteadOf` rewrite
/// candidates instead of reporting `ConfiguredOriginMismatch`.
#[test]
fn fork_configured_scm_matches_alias_origin_checkout() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let bare_prefix = init_bare_origin(temp.path());
    init_git_repo(&workspace, "feature", ALIAS_ORIGIN);
    run_git(&workspace, &[
        "config",
        &format!("url.{ALIAS_PREFIX}.insteadOf"),
        CANONICAL_PREFIX,
    ]);
    run_git(&workspace, &[
        "config",
        &format!("url.{bare_prefix}.insteadOf"),
        ALIAS_PREFIX,
    ]);

    let observation = observe_git_run_target(&workspace, Some("https://github.com/denkhaus/fabro"))
        .expect("an attached alias-origin checkout is observable");
    assert!(
        observation.run_target.is_some(),
        "the configured repository IS this checkout's origin under insteadOf"
    );
    assert_ne!(
        observation.exact_commit,
        ExactCommitStatus::ConfiguredOriginMismatch,
        "the rewrite-aware equality check must accept the alias origin"
    );
}

/// The pure helpers stay reachable and shaped for the denkhaus alias: this
/// is the pin for the fork module's public surface itself.
#[test]
fn fork_insteadof_helpers_pin_the_denkhaus_alias_shape() {
    let rewrites = vec![(ALIAS_PREFIX.to_string(), CANONICAL_PREFIX.to_string())];
    assert_eq!(
        fork_insteadof::canonical_github_origin(ALIAS_ORIGIN, &rewrites),
        "https://github.com/denkhaus/fabro",
    );
    assert!(fork_insteadof::origins_denote_same_repository(
        "https://denkhaus.github.com/denkhaus/fabro.git",
        "https://github.com/denkhaus/fabro",
        &rewrites,
    ),);
    let target =
        fork_insteadof::github_run_target_under_rewrites(ALIAS_ORIGIN, "feature", &rewrites)
            .expect("the alias origin denotes a GitHub repository");
    assert_eq!(target.repo, "denkhaus/fabro");
}

/// `git config --get-regexp` output parses into `(replacement, matcher)`
/// pairs — the seam the host wrapper depends on.
#[test]
fn fork_host_rewrites_parse_from_a_real_repo() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    run_git(&workspace, &["init", "--quiet"]);
    run_git(&workspace, &[
        "config",
        &format!("url.{ALIAS_PREFIX}.insteadOf"),
        CANONICAL_PREFIX,
    ]);

    let rewrites = fork_insteadof::host_insteadof_rewrites(&workspace);
    assert!(rewrites.contains(&(ALIAS_PREFIX.to_string(), CANONICAL_PREFIX.to_string(),)));
}

fn init_git_repo(path: &Path, branch: &str, origin_url: &str) {
    run_git(path, &[
        "-c",
        &format!("init.defaultBranch={branch}"),
        "init",
        "--quiet",
    ]);
    run_git(path, &[
        "-c",
        "user.name=test",
        "-c",
        "user.email=test@example.com",
        "commit",
        "--allow-empty",
        "--quiet",
        "-m",
        "init",
    ]);
    run_git(path, &["remote", "add", "origin", origin_url]);
}

/// The `file://` transport stand-in: a bare repo whose insteadOf
/// replacement is a URL prefix (trailing slash), matching the denkhaus
/// alias shape.
fn init_bare_origin(parent: &Path) -> String {
    let dir = parent.join("origin");
    std::fs::create_dir_all(&dir).unwrap();
    // The alias origin is `<alias-prefix>fabro.git`, so the transport
    // rewrite lands the push on `<prefix>fabro.git`.
    let bare = dir.join("fabro.git");
    std::fs::create_dir_all(&bare).unwrap();
    run_git(&bare, &["init", "--bare", "--quiet"]);
    format!("file://{}/", dir.display())
}

fn run_git(path: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn git {args:?}: {e}"));
    assert!(
        output.status.success(),
        "git {args:?} failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
