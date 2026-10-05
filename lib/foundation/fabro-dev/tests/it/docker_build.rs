use super::{fabro_dev, output_text};

#[test]
fn help_lists_docker_build_flags() {
    let output = fabro_dev()
        .args(["docker-build", "--help"])
        .assert()
        .success()
        .get_output()
        .clone();
    let stdout = output_text(&output.stdout);

    for flag in ["--arch", "--tag", "--compile-only", "--dry-run", "--git-sha"] {
        assert!(
            stdout.contains(flag),
            "docker-build help should list {flag}:\n{stdout}"
        );
    }
}

#[test]
fn invalid_arch_fails_with_clap_error() {
    let output = fabro_dev()
        .args(["docker-build", "--arch", "invalid"])
        .assert()
        .failure()
        .code(2)
        .get_output()
        .clone();
    let stderr = output_text(&output.stderr);

    assert!(
        stderr.contains("invalid value 'invalid'"),
        "invalid arch should be rejected by clap:\n{stderr}"
    );
}

#[test]
fn dry_run_prints_equivalent_build_commands() {
    let output = fabro_dev()
        .args([
            "docker-build",
            "--arch",
            "amd64",
            "--tag",
            "fabro:smoke",
            "--dry-run",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let stdout = output_text(&output.stdout);

    assert!(
        stdout.contains("cargo --locked dev spa refresh"),
        "dry-run should print SPA refresh command:\n{stdout}"
    );
    assert!(
        stdout.contains("docker run --rm --platform linux/amd64"),
        "dry-run should print builder docker run:\n{stdout}"
    );
    assert!(
        stdout.contains(
            "cargo zigbuild --locked --release -p fabro-cli --target x86_64-unknown-linux-musl"
        ),
        "dry-run should print cargo-zigbuild target:\n{stdout}"
    );
    assert!(
        stdout.contains("docker build --platform linux/amd64 -t fabro:smoke ."),
        "dry-run should print image build command:\n{stdout}"
    );
}

#[test]
fn dry_run_injects_git_sha_into_the_builder_container() {
    let output = fabro_dev()
        .args(["docker-build", "--arch", "amd64", "--dry-run"])
        .assert()
        .success()
        .get_output()
        .clone();
    let stdout = output_text(&output.stdout);

    // `to_shell_line` shell-quotes the env arg, e.g. -e 'FABRO_GIT_SHA=<sha>'.
    let marker = "FABRO_GIT_SHA=";
    let Some(index) = stdout.find(marker) else {
        panic!("dry-run should inject FABRO_GIT_SHA into the builder:\n{stdout}");
    };
    let sha = &stdout[index + marker.len()..];
    let sha: String = sha
        .chars()
        .take_while(|c| c.is_ascii_hexdigit() && !c.is_whitespace())
        .collect();
    assert_eq!(
        sha.len(),
        fabro_build_support::SHORT_SHA_LEN,
        "injected sha must be SHORT_SHA_LEN lowercase hex chars (fabro-6ffb):\n{stdout}"
    );
}

#[test]
fn an_explicit_git_sha_reaches_the_builder_environment() {
    // fabro-49af: in a GitButler workspace `git rev-parse HEAD` is the
    // never-pushed workspace commit, so the release script passes the
    // pushable line tip; the embedded sha and the release tag must agree.
    let output = fabro_dev()
        .args([
            "docker-build",
            "--arch",
            "amd64",
            "--dry-run",
            "--git-sha",
            "0123456789ab",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let stdout = output_text(&output.stdout);

    assert!(
        stdout.contains("FABRO_GIT_SHA=0123456789ab"),
        "the explicit --git-sha should be the injected build sha:\n{stdout}"
    );
}

#[test]
fn a_git_sha_that_is_not_12_hex_is_refused() {
    let output = fabro_dev()
        .args(["docker-build", "--dry-run", "--git-sha", "NOT-A-SHA"])
        .assert()
        .failure()
        .get_output()
        .clone();
    let stderr = output_text(&output.stderr);

    assert!(
        stderr.contains("--git-sha must be a 12-character lowercase hex git sha"),
        "an invalid --git-sha should be refused with the contract:\n{stderr}"
    );
}

#[test]
fn dry_run_compile_only_skips_image_build() {
    let output = fabro_dev()
        .args([
            "docker-build",
            "--arch",
            "arm64",
            "--compile-only",
            "--dry-run",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let stdout = output_text(&output.stdout);

    assert!(
        stdout.contains("tmp/docker-context/arm64/fabro"),
        "dry-run compile-only should print staged binary path:\n{stdout}"
    );
    assert!(
        !stdout.contains("docker build --platform"),
        "dry-run compile-only should not print image build:\n{stdout}"
    );
}
