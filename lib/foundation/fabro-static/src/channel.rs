//! Names shared by both ends of the per-run capability channel
//! (ADR-0035, denkhaus/work#1043): the transport credential's header,
//! owned here so the server's handshake check and every client's
//! attach site cannot drift apart.

/// The request header carrying the run's channel HANDSHAKE token
/// (`FABRO_RUN_CHANNEL_TOKEN`'s wire spelling). Server side: required
/// on every request reaching the channel socket before the worker
/// bearer is stamped. Client side: attached automatically when the
/// process env carries the token — set only for the owning run.
pub const RUN_CHANNEL_TOKEN_HEADER: &str = "x-fabro-run-channel-token";
