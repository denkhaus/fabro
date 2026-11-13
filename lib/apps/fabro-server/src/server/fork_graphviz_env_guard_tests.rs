//! Fork-only graphviz environment guard (work#1293): the three fabro-server
//! graph-render tests (`server::tests::get_graph_returns_svg`,
//! `render_graph_from_manifest_returns_svg`,
//! `render_graph_from_manifest_accepts_fabro_dotted_attributes`) red
//! deterministically in fresh toolchain sandboxes whose render subprocess
//! binary is stale or lacks its native prerequisites (the work#946 /
//! work#1167 / work#1191 class). The defect is environmental, not a
//! regression, so the tests SKIP when a real render probe fails instead of
//! forcing every run to re-prove pre-existence.

use std::path::Path;

use tokio::sync::OnceCell;

use super::handler::graph::render_dot_subprocess;

const GRAPHVIZ_ENV_PROBE_DOT: &str = "digraph probe { start -> exit }";

static GRAPHVIZ_RENDER_ENV_PROBE: OnceCell<bool> = OnceCell::const_new();

/// Whether this environment can render DOT to SVG through the real
/// production render path. Probed once per test binary; the three
/// graph-render tests skip when this returns false.
pub(in crate::server) async fn graphviz_render_env_available() -> bool {
    *GRAPHVIZ_RENDER_ENV_PROBE
        .get_or_init(|| async { graphviz_render_env_available_with(None).await })
        .await
}

/// The uncached probe behind [`graphviz_render_env_available`], exposed with
/// an exe override so the guard's detection logic is testable against
/// scripted render binaries (the pinned tests below).
pub(in crate::server) async fn graphviz_render_env_available_with(
    exe_override: Option<&Path>,
) -> bool {
    render_dot_subprocess(GRAPHVIZ_ENV_PROBE_DOT, exe_override)
        .await
        .is_ok()
}

/// The skip line the guarded tests print when the environment probe fails.
pub(in crate::server) const GRAPHVIZ_ENV_SKIP_NOTE: &str = "SKIP (work#1293 environment guard): the graphviz render subprocess cannot \
     produce SVG in this environment — environmental, not a regression";

#[cfg(unix)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    use super::*;

    #[expect(
        clippy::disallowed_methods,
        reason = "test fixture stages a scripted render binary with sync fs writes"
    )]
    fn write_render_executable(script: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir should exist");
        let path = dir.path().join("fake-render");
        std::fs::write(&path, script).expect("script should be written");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("script should be executable");
        (dir, path)
    }

    /// Pin (work#1293): a render binary that emits SVG makes the guard
    /// report the environment available — the guard must not skip the
    /// graph-render tests on a healthy environment.
    #[tokio::test]
    async fn graphviz_env_guard_reports_available_for_healthy_render_binary() {
        let (_dir, script_path) = write_render_executable(
            "#!/bin/sh\ncat >/dev/null\nprintf '<svg xmlns=\"http://www.w3.org/2000/svg\"/>'\n",
        );

        assert!(
            graphviz_render_env_available_with(Some(&script_path)).await,
            "a render binary producing SVG must count as available"
        );
    }

    /// Pin (work#1293): the stale-binary failure shape from work#1191 — a
    /// render subprocess that exits 0 with EMPTY stdout — must make the
    /// guard report the environment unavailable, so the graph-render tests
    /// skip instead of red.
    #[tokio::test]
    async fn graphviz_env_guard_reports_unavailable_for_silent_render_binary() {
        let (_dir, script_path) = write_render_executable("#!/bin/sh\ncat >/dev/null\nexit 0\n");

        assert!(
            !graphviz_render_env_available_with(Some(&script_path)).await,
            "a silent render binary must count as unavailable"
        );
    }

    /// Pin (work#1293): a render subprocess that cannot even be spawned
    /// (missing native prerequisites) must count as unavailable.
    #[tokio::test]
    async fn graphviz_env_guard_reports_unavailable_for_missing_render_binary() {
        let dir = tempfile::tempdir().expect("temp dir should exist");
        let missing = dir.path().join("does-not-exist");

        assert!(
            !graphviz_render_env_available_with(Some(&missing)).await,
            "an unspawnable render binary must count as unavailable"
        );
    }
}
