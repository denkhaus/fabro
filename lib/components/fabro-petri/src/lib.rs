//! Fabro's adapters over Petri, the workflow engine Fabro runs its workflows
//! on.
//!
//! This crate is the one place Fabro touches Petri. Every other Fabro crate
//! reaches the engine through the types and functions exported here and never
//! depends on a Petri package itself. That keeps the engine's API surface in
//! one crate, so a Petri pin move is a change to this crate alone.
//!
//! What lives here, as the integration plan lands it:
//!
//! - [`SqliteRunStore`]: Petri's run store over Fabro's SQLite database, so a
//!   run's records are its source of truth in Fabro's tables;
//! - [`runtime`]: the Petri runtime Fabro assembles, at create time and at
//!   execution;
//! - [`check`]: Petri compiles a workflow version's bundle at create time, and
//!   its diagnostics come back in a shape Fabro maps onto its own;
//! - [`admission`]: the admitted graphs in Fabro's blob store, named on the run
//!   spec;
//! - [`engine`]: a run executed by Petri, started or resumed, in the run's
//!   worker process over the HTTP store (or in the server process under its
//!   test override), with the outcome read from its record;
//! - [`interviewer`]: the interviewer of a run nobody is watching;
//! - [`HttpRunStore`]: the same store as a run's worker process reaches it,
//!   over the server's API with the worker's token and its launch id as the
//!   lease owner;
//! - [`projection`] and [`projector`]: the view of a Petri run, folded from its
//!   records and Fabro's platform records, and the pass that writes it after
//!   each committed record;
//! - the platform adapters still to come: hooks, interviews over Fabro's API,
//!   secrets, output storage, the run tools.
//!
//! The Petri packages are pinned by revision in the workspace `Cargo.toml`
//! under `petri_*` keys.

pub mod admission;
pub mod check;
pub mod engine;
pub mod http_store;
pub mod interviewer;
pub mod petri;
pub mod projection;
pub mod projector;
pub mod run_store;
pub mod runtime;
#[cfg(feature = "test-support")]
pub mod test_support;

pub use http_store::HttpRunStore;
pub use run_store::SqliteRunStore;
