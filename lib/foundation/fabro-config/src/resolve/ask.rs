//! `[ask]` namespace resolution.

use fabro_types::settings::AskNamespace;

use crate::AskLayer;

/// Maps the sparse `[ask]` layer onto the resolved namespace. Every field
/// is a plain optional string: nothing to interpolate, default, or
/// cross-validate here. A `supervisor_model` that names no catalog model
/// is rejected where the setting is consumed, at runless session creation
/// (work#1690), because only there is the model catalog in scope.
pub fn resolve_ask(layer: &AskLayer) -> AskNamespace {
    AskNamespace {
        supervisor_model: layer.supervisor_model.clone(),
    }
}
