//! Post-upgrade safety net for stored environments (fabro-94f6).
//!
//! Choice: a warning list, not auto-strip. Stored rows can predate the
//! write-path provider validation, and silently rewriting an operator's
//! environment on boot hides what changed; a warning names the exact fields
//! before the first run fails at sandbox creation, and the repair is a
//! one-field PUT (or the `fabro env update` CLI once fabro-4f44 lands).
//! Start is still allowed.

use std::sync::Arc;

use fabro_sandbox::environment::unsupported_resource_fields;
use tracing::warn;

use crate::server::AppState;

/// Logs every stored environment whose resources name fields its provider
/// does not enforce. Runs against the current provider capability
/// knowledge on every server start; see [`unsupported_resource_fields`]
/// for which providers enforce which fields.
pub(crate) fn warn_on_incompatible_environments(state: &Arc<AppState>) {
    for environment in state.environment_store().list() {
        let fields = unsupported_resource_fields(
            &environment.settings.provider,
            &environment.settings.resources,
        );
        if !fields.is_empty() {
            warn!(
                environment_id = environment.id.as_str(),
                fields = fields.join("; "),
                "stored environment carries resource fields its provider does not enforce; \
                 runs using it will fail at sandbox creation — clear the fields via the \
                 environments API"
            );
        }
    }
}
