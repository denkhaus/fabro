//! Petri's store vocabulary, re-exported for the Fabro crates that hold a
//! Petri run handle or answer for one (the server's worker endpoints) without
//! depending on the Petri packages themselves. Only this crate names them in
//! its `Cargo.toml`.

pub use petri_store::{
    Access, Digest, ExecutionId, LogId, OwnerId, Record, RunKey, RunLogs, RunStore, StoreError,
};
