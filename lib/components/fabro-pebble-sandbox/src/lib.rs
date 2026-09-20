//! A sandbox-driver handle as the [`Environment`] pebble's coding agent runs
//! in.
//!
//! Petri creates and owns every run sandbox through the sandbox driver;
//! Fabro attaches to one for Ask Fabro, and `fabro exec` creates a host
//! sandbox of its own. Pebble's tools speak the `Environment` contract; the
//! driver speaks facets. This crate is the mapping between the two, and
//! Fabro's policy on the way through: [`PebbleSandbox`] resolves paths the
//! way Fabro resolves them and runs commands under [`SandboxExec`]'s exec
//! policy; [`SandboxPortRoutes`] answers pebble's port routing with the
//! driver's preview URLs; [`SecretRedactor`] is Fabro's secret scanner on
//! the text pebble hands the model; [`display_for_log`] renders a driver
//! failure with its redacted output tail.
//!
//! [`Environment`]: pebble_coding_agent::environment::Environment

mod environment;
mod exec;
mod log;
mod path;
mod ports;
mod redact;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use environment::PebbleSandbox;
pub use exec::{
    DEFAULT_EXEC_OUTPUT_TAIL_BYTES, DEFAULT_RETAINED_OUTPUT_BYTES, DEFAULT_STOP_GRACE,
    ExecResultExt, SandboxExec, command_termination, program_exit_code, redacted_output_tail,
};
pub use log::{default_redacted_output_tail, display_for_log};
pub use path::{join_sandbox_path, resolve_path};
pub use ports::{SandboxPortRoutes, port_routes};
pub use redact::SecretRedactor;
