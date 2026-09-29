use std::path::Path;

fn main() {
    let metadata = fabro_build_support::collect_from(Path::new("."));

    for path in metadata.rerun_paths {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    // Image builds (`cargo dev docker-build`) compile without usable git
    // metadata inside the builder container; the build plan injects the
    // sha as a `FABRO_GIT_SHA` env var so release binaries never embed an
    // empty sha (fabro-6ffb).
    let short_sha = injected_git_sha().unwrap_or(metadata.short_sha);

    println!("cargo:rustc-env=FABRO_GIT_SHA={short_sha}");

    let build_date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    println!("cargo:rustc-env=FABRO_BUILD_DATE={build_date}");

    let profile = fabro_build_support::cargo_profile();
    println!("cargo:rustc-env=FABRO_BUILD_PROFILE={profile}");
}

#[expect(
    clippy::disallowed_methods,
    reason = "Build scripts read the FABRO_GIT_SHA image-build injection seam outside application runtime configuration."
)]
fn injected_git_sha() -> Option<String> {
    std::env::var("FABRO_GIT_SHA")
        .ok()
        .filter(|sha| !sha.is_empty())
}
