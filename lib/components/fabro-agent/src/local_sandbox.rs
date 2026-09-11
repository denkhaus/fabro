//! The host-backed sandbox fabro calls `local`, re-exported from
//! fabro-sandbox so agent consumers construct it without a second import.
pub use fabro_sandbox::local_sandbox;
