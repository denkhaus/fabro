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
//! - the run store over Fabro's SQLite database, so Petri's records are the
//!   run's source of truth in Fabro's tables;
//! - the platform adapters: hooks, interviews, secrets, output storage, the run
//!   tools, the event projection.
//!
//! The Petri packages are pinned by revision in the workspace `Cargo.toml`
//! under `petri_*` keys.
