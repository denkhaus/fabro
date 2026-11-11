//! Fork presence pin (work#1253): the vendored sandbox-driver tree
//! carries the build-diagnostics enrichment — the docker plugin's
//! image-build path renders the bollard cause chain (unfolding the
//! stream detail bollard's own `Display` hides) and the daemon's build
//! output tail. These tests exist only on the fork side of the vendor
//! seam; a vendor sync that drops them dropped the feature — restore
//! it, never relax the test.

use std::path::PathBuf;

fn vendored(relative: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join("vendor/sandbox-driver")
        .join(relative);
    #[expect(
        clippy::disallowed_methods,
        reason = "a presence pin reads two small vendored files once at test start; \
                  async I/O buys nothing here"
    )]
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "vendored sandbox-driver is missing {relative} ({error}) — \
             a vendor sync dropped the fork feature (work#1253)"
        )
    });
    text
}

/// The diagnostics module exists with its three render helpers.
#[test]
fn the_diagnostics_module_survives_the_vendor_seam() {
    let module = vendored("crates/sandbox-driver-docker/src/build_diagnostics.rs");
    for marker in [
        "struct BuildOutputTail",
        "fn render_cause_chain",
        "fn render_api_failure",
        "fn with_output_tail",
    ] {
        assert!(
            module.contains(marker),
            "build_diagnostics.rs lost `{marker}` — the enrichment decayed"
        );
    }
}

/// The one-shot build loop stays wired to the diagnostics: both failure
/// arms carry the enriched message.
#[test]
fn the_one_shot_build_loop_carries_the_seam() {
    let one_shot = vendored("crates/sandbox-driver-docker/src/one_shot.rs");
    for marker in [
        "BuildOutputTail::new()",
        "render_api_failure(&error)",
        "with_output_tail(",
        "tail.record(line)",
    ] {
        assert!(
            one_shot.contains(marker),
            "one_shot.rs lost `{marker}` — the build path dropped the \
             enrichment seam"
        );
    }
}
