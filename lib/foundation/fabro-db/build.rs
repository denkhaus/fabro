//! `sqlx::migrate!` embeds every file under `migrations/` at compile time,
//! but on stable Rust the macro cannot tell Cargo about the directory. Without
//! this hint a newly added migration file does not recompile the crate, so a
//! stale build silently ships without it.

fn main() {
    // Build scripts run with the crate root as their working directory, so
    // relative paths are correct here. Also rerun when build.rs itself or the
    // embedded migration set changes, so local-target and docker-volume
    // caches cannot ship a stale MIGRATOR.
    println!("cargo:rerun-if-changed=migrations");
    println!("cargo:rerun-if-changed=build.rs");
}
