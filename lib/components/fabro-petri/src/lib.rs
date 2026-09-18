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
//! - [`HttpRunStore`]: the same store as a run's worker process reaches it,
//!   over the server's API with the worker's token;
//! - the platform adapters: hooks, interviews, secrets, output storage, the run
//!   tools, the event projection.
//!
//! The Petri packages are pinned by revision in the workspace `Cargo.toml`
//! under `petri_*` keys.

pub mod http_store;
pub mod petri;
pub mod run_store;
#[cfg(feature = "test-support")]
pub mod test_support;

pub use http_store::HttpRunStore;
pub use run_store::SqliteRunStore;
