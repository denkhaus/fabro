//! Presence pin for the reshaped seeds read API (fabro-3488 → rework
//! 2026-10-19, user decision): the ONLY seeds read route is
//! `/runs/{id}/seeds`, served live from the run's own sandbox workspace.
//! The mirror-era routes (`/seeds`, `/seeds/graph`, `/seeds/{id}`) and
//! the `[server.seeds]` settings are GONE — a merge that resurrects the
//! server-side mirror machinery must red this pin, not pass silently.

#[cfg(test)]
mod tests {
    use crate::server::tests::test_router_paths;

    #[tokio::test]
    async fn seeds_reads_are_run_scoped_only() {
        let paths = test_router_paths().await;
        assert!(
            paths
                .lines()
                .any(|p| p.ends_with("/seeds") && p.contains("/runs/")),
            "the run-scoped seeds route must exist, got:\n{paths}"
        );
        assert!(
            !paths.lines().any(|p| p == "/api/v1/seeds"),
            "the mirror-era tracker-wide /seeds route must stay removed:\n{paths}"
        );
        assert!(
            !paths.lines().any(|p| p == "/api/v1/seeds/graph"),
            "the mirror-era graph route must stay removed:\n{paths}"
        );
    }
}
