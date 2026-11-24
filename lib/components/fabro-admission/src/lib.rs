//! Run admission: the domain cluster that turns a workflow + settings into
//! an admitted run.
//!
//! Owns the manifest checks ([`run_manifest`]), run settings compilation
//! ([`run_compiler`]), run intent resolution ([`run_intent`]), git-author
//! rebase prep ([`run_rebase`]), Petri admission preflight ([`petri_check`]),
//! and the validation entry point ([`manifest_validation`]). Extracted out of
//! the fabro-server app crate so clients (CLI, MCP, run workers) consume the
//! admission domain without the server (work#1672).
//!
//! The cluster never touches server state: callers hand it an
//! [`AdmissionContext`] (plain inputs + two callbacks the app wires) and,
//! for [`run_manifest::check_prepared_manifest`], a ready-made Petri
//! [`RuntimeSpec`](fabro_petri::runtime::RuntimeSpec).

mod context;

pub use context::{AdmissionContext, GithubCredentialsSource, ProviderHealthSource};

pub mod manifest_validation;
pub mod petri_check;
pub mod run_compiler;
pub mod run_intent;
pub mod run_manifest;
pub mod run_rebase;

#[cfg(test)]
mod test_support;
