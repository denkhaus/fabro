use crate::sandbox;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CloneDecision {
    EmptyWorkspace {
        reason: EmptyWorkspaceReason,
    },
    GitHub {
        origin_url: String,
        branch:     Option<String>,
        commit_sha: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GitHubRepoLayout {
    pub(crate) owner:               String,
    pub(crate) repo:                String,
    pub(crate) repos_owner_path:    String,
    pub(crate) primary_repo_path:   String,
    pub(crate) primary_repo_link:   String,
    pub(crate) execution_directory: String,
}

pub(crate) fn github_repo_layout(
    origin_url: &str,
    workspace_root: &str,
    repos_root: &str,
) -> crate::Result<GitHubRepoLayout> {
    let origin_url = fabro_github::normalize_repo_origin_url(origin_url);
    let (owner, repo) = fabro_github::parse_github_owner_repo(&origin_url).map_err(|err| {
        crate::Error::message(format!(
            "Clone-based sandboxes currently support GitHub repository origins only: {err}"
        ))
    })?;
    validate_path_component("owner", &owner)?;
    validate_path_component("repository", &repo)?;
    let workspace_root = trim_root(workspace_root);
    let repos_root = trim_root(repos_root);
    let repos_owner_path = sandbox::join_sandbox_path(repos_root, &owner);
    let primary_repo_path = sandbox::join_sandbox_path(&repos_owner_path, &repo);
    let primary_repo_link = sandbox::join_sandbox_path(workspace_root, &repo);

    Ok(GitHubRepoLayout {
        owner,
        repo,
        repos_owner_path,
        primary_repo_path,
        execution_directory: primary_repo_link.clone(),
        primary_repo_link,
    })
}

fn validate_path_component(label: &str, component: &str) -> crate::Result<()> {
    let is_safe = !matches!(component, "." | "..")
        && component
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if !is_safe {
        return Err(crate::Error::message(format!(
            "GitHub {label} is not a safe repository path component"
        )));
    }
    Ok(())
}

pub(crate) fn repo_symlink_command(layout: &GitHubRepoLayout) -> String {
    format!(
        "ln -s {} {}",
        sandbox::shell_quote(&layout.primary_repo_path),
        sandbox::shell_quote(&layout.primary_repo_link),
    )
}

pub(crate) fn exact_repository_init_command(clone_url: &str, checkout_path: &str) -> String {
    format!(
        "git -c maintenance.auto=0 -c gc.auto=0 init -- {} && git -C {} remote add origin {}",
        sandbox::shell_quote(checkout_path),
        sandbox::shell_quote(checkout_path),
        sandbox::shell_quote(clone_url),
    )
}

pub(crate) fn exact_fetch_command(
    checkout_path: &str,
    fetch_source: &str,
    commit_sha: &str,
) -> String {
    format!(
        "git -C {} -c maintenance.auto=0 -c gc.auto=0 fetch --depth 1 --no-tags {} -- {}",
        sandbox::shell_quote(checkout_path),
        sandbox::shell_quote(fetch_source),
        sandbox::shell_quote(commit_sha),
    )
}

pub(crate) fn exact_checkout_command(checkout_path: &str) -> String {
    format!(
        "git -C {} -c advice.detachedHead=false checkout --detach FETCH_HEAD",
        sandbox::shell_quote(checkout_path),
    )
}

pub(crate) fn head_revision_command(checkout_path: &str) -> String {
    format!(
        "git -C {} rev-parse HEAD",
        sandbox::shell_quote(checkout_path),
    )
}

pub(crate) fn verify_exact_head(output: &str, expected_sha: &str) -> crate::Result<()> {
    let actual_sha = output.trim();
    let actual_sha = normalize_exact_commit_sha(actual_sha).map_err(|err| {
        crate::Error::context("Exact checkout produced an invalid HEAD commit ID", err)
    })?;
    if actual_sha != expected_sha {
        return Err(crate::Error::message(
            "Exact checkout HEAD did not match the requested commit",
        ));
    }
    Ok(())
}

fn trim_root(root: &str) -> &str {
    let trimmed = root.trim_end_matches('/');
    if trimmed.is_empty() { "/" } else { trimmed }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EmptyWorkspaceReason {
    SkipClone,
    MissingOrigin,
}

impl EmptyWorkspaceReason {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::SkipClone => "clone disabled; creating an empty workspace",
            Self::MissingOrigin => {
                "no clone source was present; creating an empty workspace without repository files"
            }
        }
    }
}

pub(crate) fn decide_clone(
    skip_clone: bool,
    clone_origin_url: Option<&str>,
    clone_branch: Option<&str>,
    clone_commit_sha: Option<&str>,
) -> crate::Result<CloneDecision> {
    let commit_sha = clone_commit_sha
        .map(normalize_exact_commit_sha)
        .transpose()?;

    if commit_sha.is_some() {
        if skip_clone {
            return Err(crate::Error::message(
                "Exact commit checkout requires cloning to be enabled",
            ));
        }
        if clone_origin_url.is_none_or(|url| url.trim().is_empty()) {
            return Err(crate::Error::message(
                "Exact commit checkout requires a repository origin",
            ));
        }
    }

    if skip_clone {
        return Ok(CloneDecision::EmptyWorkspace {
            reason: EmptyWorkspaceReason::SkipClone,
        });
    }

    let Some(origin_url) = clone_origin_url.filter(|url| !url.trim().is_empty()) else {
        return Ok(CloneDecision::EmptyWorkspace {
            reason: EmptyWorkspaceReason::MissingOrigin,
        });
    };

    let origin_url = fabro_github::normalize_repo_origin_url(origin_url);
    if let Err(err) = fabro_github::parse_github_owner_repo(&origin_url) {
        return Err(crate::Error::message(format!(
            "Clone-based sandboxes currently support GitHub repository origins only: {err}"
        )));
    }

    Ok(CloneDecision::GitHub {
        origin_url,
        branch: clone_branch
            .filter(|branch| !branch.trim().is_empty())
            .map(str::to_string),
        commit_sha,
    })
}

fn normalize_exact_commit_sha(commit_sha: &str) -> crate::Result<String> {
    if commit_sha.len() != 40 || !commit_sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(crate::Error::message(
            "Exact commit SHA must be exactly 40 ASCII hexadecimal characters",
        ));
    }
    Ok(commit_sha.to_ascii_lowercase())
}

pub(crate) fn clean_clone_origin_for_record(clone_origin_url: Option<&str>) -> Option<String> {
    clone_origin_url
        .filter(|url| !url.trim().is_empty())
        .map(fabro_github::normalize_repo_origin_url)
}

pub(crate) fn repo_cloned_for_record(
    skip_clone: bool,
    clone_origin_url: Option<&str>,
) -> Option<bool> {
    Some(matches!(
        decide_clone(skip_clone, clone_origin_url, None, None).ok()?,
        CloneDecision::GitHub { .. }
    ))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::{Command, Output};

    use super::*;

    fn isolated_command(command: &mut Command) -> Output {
        command
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_AUTHOR_NAME", "Fabro Test")
            .env("GIT_AUTHOR_EMAIL", "fabro-test@example.com")
            .env("GIT_COMMITTER_NAME", "Fabro Test")
            .env("GIT_COMMITTER_EMAIL", "fabro-test@example.com")
            .output()
            .expect("test command should start")
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "hermetic Git proof intentionally runs the local git executable synchronously"
    )]
    fn run_git(cwd: &Path, args: &[&str]) -> String {
        let output = isolated_command(Command::new("git").current_dir(cwd).args(args));
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("git output should be UTF-8")
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "hermetic command-builder proof intentionally runs local Bash synchronously"
    )]
    fn run_shell(cwd: &Path, command: &str) -> String {
        let output = isolated_command(Command::new("/bin/bash").current_dir(cwd).args([
            "--noprofile",
            "--norc",
            "-c",
            command,
        ]));
        assert!(
            output.status.success(),
            "command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("command output should be UTF-8")
    }

    #[test]
    fn skip_clone_overrides_present_origin() {
        assert_eq!(
            decide_clone(
                true,
                Some("https://gitlab.com/acme/widgets.git"),
                Some("main"),
                None,
            )
            .unwrap(),
            CloneDecision::EmptyWorkspace {
                reason: EmptyWorkspaceReason::SkipClone,
            }
        );
    }

    #[test]
    fn missing_origin_creates_empty_workspace() {
        assert_eq!(
            decide_clone(false, None, None, None).unwrap(),
            CloneDecision::EmptyWorkspace {
                reason: EmptyWorkspaceReason::MissingOrigin,
            }
        );
    }

    #[test]
    fn github_origin_is_normalized_with_branch() {
        assert_eq!(
            decide_clone(
                false,
                Some("git@github.com:acme/widgets.git"),
                Some("feature/work"),
                None,
            )
            .unwrap(),
            CloneDecision::GitHub {
                origin_url: "https://github.com/acme/widgets".to_string(),
                branch:     Some("feature/work".to_string()),
                commit_sha: None,
            }
        );
    }

    #[test]
    fn non_github_origin_fails_without_skip_clone() {
        let error = decide_clone(
            false,
            Some("https://gitlab.com/acme/widgets.git"),
            None,
            None,
        )
        .expect_err("non-GitHub origins should fail");
        assert!(error.to_string().contains("GitHub repository origins only"));
    }

    #[test]
    fn exact_commit_sha_is_validated_and_normalized() {
        let lowercase = "0123456789abcdef0123456789abcdef01234567";
        let uppercase = "ABCDEF0123456789ABCDEF0123456789ABCDEF01";

        assert_eq!(
            decide_clone(
                false,
                Some("https://github.com/acme/widgets"),
                Some("moving-branch"),
                Some(lowercase),
            )
            .unwrap(),
            CloneDecision::GitHub {
                origin_url: "https://github.com/acme/widgets".to_string(),
                branch:     Some("moving-branch".to_string()),
                commit_sha: Some(lowercase.to_string()),
            }
        );
        assert_eq!(
            decide_clone(
                false,
                Some("https://github.com/acme/widgets"),
                None,
                Some(uppercase),
            )
            .unwrap(),
            CloneDecision::GitHub {
                origin_url: "https://github.com/acme/widgets".to_string(),
                branch:     None,
                commit_sha: Some(uppercase.to_ascii_lowercase()),
            }
        );
    }

    #[test]
    fn exact_commit_sha_rejects_noncanonical_inputs() {
        for sha in [
            "",
            "0123456789abcdef0123456789abcdef0123456",
            "0123456789abcdef0123456789abcdef012345678",
            "0123456789abcdef0123456789abcdef0123456g",
            " 0123456789abcdef0123456789abcdef01234567",
            "0123456789abcdef0123456789abcdef01234567 ",
            "0123456789abcdef0123456789abcdef012345é",
        ] {
            let error = decide_clone(
                false,
                Some("https://github.com/acme/widgets"),
                None,
                Some(sha),
            )
            .expect_err("invalid exact commit SHA should fail");
            assert!(
                error.to_string().contains("40 ASCII hexadecimal"),
                "unexpected error for {sha:?}: {error}"
            );
        }
    }

    #[test]
    fn exact_checkout_requires_clone_and_origin() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        let skip_error = decide_clone(
            true,
            Some("https://github.com/acme/widgets"),
            None,
            Some(sha),
        )
        .expect_err("exact checkout with skip-clone should fail");
        assert!(skip_error.to_string().contains("requires cloning"));

        for origin in [None, Some(""), Some("   ")] {
            let error = decide_clone(false, origin, None, Some(sha))
                .expect_err("exact checkout without an origin should fail");
            assert!(error.to_string().contains("requires a repository origin"));
        }
    }

    #[test]
    fn exact_checkout_commands_quote_inputs_and_ignore_branch_metadata() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        let init = exact_repository_init_command(
            "https://token@example.com/acme/widgets.git?x=a b",
            "/repos/acme's widgets",
        );
        let fetch = exact_fetch_command(
            "/repos/acme's widgets",
            "https://token@example.com/acme/widgets.git?x=a b",
            sha,
        );
        let checkout = exact_checkout_command("/repos/acme's widgets");
        let verify = head_revision_command("/repos/acme's widgets");

        assert_eq!(
            init,
            "git -c maintenance.auto=0 -c gc.auto=0 init -- \"/repos/acme's widgets\" && git -C \"/repos/acme's widgets\" remote add origin 'https://token@example.com/acme/widgets.git?x=a b'"
        );
        assert_eq!(
            fetch,
            "git -C \"/repos/acme's widgets\" -c maintenance.auto=0 -c gc.auto=0 fetch --depth 1 --no-tags 'https://token@example.com/acme/widgets.git?x=a b' -- 0123456789abcdef0123456789abcdef01234567"
        );
        assert_eq!(
            checkout,
            "git -C \"/repos/acme's widgets\" -c advice.detachedHead=false checkout --detach FETCH_HEAD"
        );
        assert_eq!(verify, "git -C \"/repos/acme's widgets\" rev-parse HEAD");
        for command in [&init, &fetch, &checkout, &verify] {
            assert!(!command.contains("moving-branch"));
        }
    }

    #[test]
    fn exact_checkout_verification_rejects_invalid_or_mismatched_head() {
        let expected = "0123456789abcdef0123456789abcdef01234567";
        verify_exact_head("0123456789ABCDEF0123456789ABCDEF01234567\n", expected)
            .expect("uppercase command output should normalize");

        let invalid = verify_exact_head("fatal: not a revision", expected)
            .expect_err("non-SHA output should fail verification");
        assert!(invalid.to_string().contains("invalid HEAD commit ID"));
        assert!(!invalid.to_string().contains("fatal: not a revision"));

        let mismatched = verify_exact_head("1123456789abcdef0123456789abcdef01234567", expected)
            .expect_err("mismatched SHA should fail verification");
        assert!(mismatched.to_string().contains("did not match"));
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "hermetic Git proof uses isolated synchronous temp-repository I/O"
    )]
    fn exact_checkout_fetches_admitted_commit_after_branch_advances() {
        let temp = tempfile::tempdir().expect("tempdir");
        let remote = temp.path().join("remote.git");
        let source = temp.path().join("source");
        let checkout = temp.path().join("exact checkout");
        fs::create_dir(&source).expect("source directory");

        run_git(temp.path(), &[
            "init",
            "--bare",
            remote.to_str().expect("UTF-8 remote path"),
        ]);
        run_git(&source, &["init"]);
        fs::write(source.join("revision.txt"), "A\n").expect("write commit A");
        run_git(&source, &["add", "revision.txt"]);
        run_git(&source, &["commit", "-m", "commit A"]);
        run_git(&source, &["branch", "-M", "main"]);
        run_git(&source, &[
            "remote",
            "add",
            "origin",
            remote.to_str().expect("UTF-8 remote path"),
        ]);
        run_git(&source, &["push", "-u", "origin", "main"]);
        let admitted_sha = run_git(&source, &["rev-parse", "HEAD"]).trim().to_string();

        fs::write(source.join("revision.txt"), "B\n").expect("write commit B");
        run_git(&source, &["commit", "-am", "commit B"]);
        run_git(&source, &["push", "origin", "main"]);
        let advanced_sha = run_git(&source, &["rev-parse", "HEAD"]).trim().to_string();
        assert_ne!(admitted_sha, advanced_sha);

        let remote_path = remote.to_str().expect("UTF-8 remote path");
        let checkout_path = checkout.to_str().expect("UTF-8 checkout path");
        run_shell(
            temp.path(),
            &exact_repository_init_command(remote_path, checkout_path),
        );
        run_shell(
            temp.path(),
            &exact_fetch_command(checkout_path, remote_path, &admitted_sha),
        );
        run_shell(temp.path(), &exact_checkout_command(checkout_path));
        let checked_out_sha = run_shell(temp.path(), &head_revision_command(checkout_path));

        assert_eq!(checked_out_sha.trim(), admitted_sha);
        assert_eq!(
            fs::read_to_string(checkout.join("revision.txt")).expect("checked-out contents"),
            "A\n"
        );
        let symbolic_head = isolated_command(Command::new("git").args([
            "-C",
            checkout_path,
            "symbolic-ref",
            "-q",
            "HEAD",
        ]));
        assert!(!symbolic_head.status.success(), "HEAD should be detached");
        assert_eq!(
            run_git(temp.path(), &[
                "--git-dir",
                remote_path,
                "rev-parse",
                "refs/heads/main",
            ],)
            .trim(),
            advanced_sha
        );
    }

    #[test]
    fn github_layout_maps_ssh_origin_to_repos_checkout_and_workspace_link() {
        let layout = github_repo_layout(
            "git@github.com:brynary/rack-test.git",
            "/workspace",
            "/repos",
        )
        .unwrap();

        assert_eq!(layout.owner, "brynary");
        assert_eq!(layout.repo, "rack-test");
        assert_eq!(layout.repos_owner_path, "/repos/brynary");
        assert_eq!(layout.primary_repo_path, "/repos/brynary/rack-test");
        assert_eq!(layout.primary_repo_link, "/workspace/rack-test");
        assert_eq!(layout.execution_directory, "/workspace/rack-test");
    }

    #[test]
    fn github_layout_normalizes_https_origin_and_trims_roots() {
        let layout = github_repo_layout(
            "https://github.com/fabro-sh/fabro.git/",
            "/workspace/",
            "/repos/",
        )
        .unwrap();

        assert_eq!(layout.owner, "fabro-sh");
        assert_eq!(layout.repo, "fabro");
        assert_eq!(layout.repos_owner_path, "/repos/fabro-sh");
        assert_eq!(layout.primary_repo_path, "/repos/fabro-sh/fabro");
        assert_eq!(layout.primary_repo_link, "/workspace/fabro");
        assert_eq!(layout.execution_directory, "/workspace/fabro");
    }

    #[test]
    fn github_layout_rejects_path_traversal_components() {
        for origin in [
            "https://github.com/../widgets",
            "https://github.com/acme/..",
            "https://github.com/%2e%2e/widgets",
        ] {
            let error = github_repo_layout(origin, "/workspace", "/repos")
                .expect_err("unsafe path component should fail");
            assert!(
                error.to_string().contains("safe repository path component"),
                "got {error} for {origin}"
            );
        }
    }

    #[test]
    fn repo_symlink_command_quotes_both_paths() {
        let layout = github_repo_layout(
            "https://github.com/fabro-sh/fabro",
            "/work space",
            "/repo root",
        )
        .unwrap();

        assert_eq!(
            repo_symlink_command(&layout),
            "ln -s '/repo root/fabro-sh/fabro' '/work space/fabro'"
        );
    }

    #[test]
    fn record_origin_strips_credentials() {
        assert_eq!(
            clean_clone_origin_for_record(Some(
                "https://x-access-token:secret@github.com/acme/widgets.git"
            )),
            Some("https://github.com/acme/widgets".to_string())
        );
    }
}
