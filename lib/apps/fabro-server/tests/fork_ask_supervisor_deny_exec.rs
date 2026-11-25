//! Fork presence pin (ADR-0043, work#1678): the runless Ask Fabro
//! supervisor runs its pebble agent in `DenyExecEnvironment` — a vendored
//! pebble piece (`vendor/pebble`, exported as denkhaus/pebble#381d9d0a7398).
//! The supervisor is tools-only: its platform tools go over the minted
//! `agent:supervisor` token, and its own exec/file surface must refuse.
//!
//! This test lives in a fork-only file at the VENDOR seam: a
//! `just vendor import pebble` from a wrong fork state would silently drop
//! the piece, and this pin going red is what names that. The compile itself
//! is half the pin — if the export disappears, the import below stops
//! resolving.

use pebble_coding_agent::environment::{DenyExecEnvironment, Environment};

#[tokio::test]
async fn the_runless_supervisor_environment_refuses_exec_and_files() {
    let env = DenyExecEnvironment::new("/ask-fabro-supervisor");

    // Prompt-assembly accessors stay honest display values.
    assert_eq!(env.working_directory(), "/ask-fabro-supervisor");
    assert_eq!(env.platform(), "unknown");

    // Exec is denied, not silently successful.
    let outcome = env
        .exec(pebble_coding_agent::environment::ExecRequest::new("true"))
        .await;
    let error = outcome.expect_err("exec must be denied in the supervisor environment");
    assert!(
        error.message().contains("tools-only"),
        "the denial must name the tools-only contract: {}",
        error.message()
    );

    // File reads are denied too.
    let read = env.read_file_bytes("/anything").await;
    assert!(read.is_err(), "file reads must be denied");
}
