fn main() {
    // Build scripts run with the crate root as their working directory, so
    // relative paths are correct here. Without these directives, cargo does
    // not reliably invalidate the crate fingerprint when migration files are
    // added or removed under local-target and docker-volume caches, and
    // `sqlx::migrate!` keeps embedding a stale MIGRATOR.
    println!("cargo:rerun-if-changed=migrations");
    println!("cargo:rerun-if-changed=build.rs");
}
