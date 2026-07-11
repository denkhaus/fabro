//! Hook configuration runtime settings.

pub use fabro_types::settings::run::{
    HookEvent, RuntimeHookDefinition, RuntimeHookType, RuntimeHttpHook, TlsMode,
};

/// Top-level hook configuration: the boundary-resolved hooks for one run.
///
/// Every interpolatable hook field is resolved to a plain string at the run
/// boundary before it reaches this crate (see
/// `fabro_types::settings::run::HookDefinition::resolve_env`), so the runner
/// and executor never resolve tokens themselves.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HookSettings {
    pub hooks: Vec<RuntimeHookDefinition>,
}
