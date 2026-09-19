//! Fork extension (denkhaus line): reclaiming managed sandboxes by native
//! id, for the server's sandbox GC (fabro-44d8).
//!
//! This lives in its own file — not as a method on
//! [`SandboxInventory`](crate::provider::SandboxInventory) — so upstream
//! changes to `provider.rs` cannot overwrite fork behavior: fork features
//! stay in fork-owned files, anchored in fabro-server's
//! `fork_seam_test.rs`.

use sandbox_driver::SandboxId;

use crate::provider::{InventoryEntry, SandboxInventory, SandboxLookupError, provider_error};

/// Deletes a managed sandbox by its native id, on every provider the
/// inventory covers that can list. Idempotent from the caller's side:
/// the driver's delete succeeds for an id it does not know, so `Ok(())`
/// means "gone or was never there", never "definitely existed". The
/// ownership scope each provider is connected through refuses ids fabro
/// did not create, so this can never remove a foreign sandbox sharing
/// the backend.
///
/// The reconciliation path garbage-collecting non-revisable run
/// sandboxes (the server's sandbox GC) deletes through here rather than
/// a backend-specific client, so it inherits the same ownership
/// narrowing as every other fabro operation.
pub async fn delete_managed_by_native_id(
    inventory: &SandboxInventory,
    id: &str,
) -> Result<(), SandboxLookupError> {
    let Ok(sandbox_id) = SandboxId::try_new(id) else {
        return Err(SandboxLookupError::NotFound { id: id.to_string() });
    };
    delete_on_entries(inventory.entries(), &sandbox_id, id).await
}

async fn delete_on_entries(
    entries: &[std::sync::Arc<InventoryEntry>],
    sandbox_id: &SandboxId,
    id: &str,
) -> Result<(), SandboxLookupError> {
    let mut contacted = false;
    let mut provider_errors = Vec::new();
    for entry in entries {
        let provider = match entry.provider().await {
            Ok(provider) => provider,
            Err(err) => {
                provider_errors.push(provider_error(entry.kind().clone(), &err));
                continue;
            }
        };
        let Some(provider) = provider else {
            continue;
        };
        contacted = true;
        if let Err(err) = provider.delete(sandbox_id, None).await {
            provider_errors.push(provider_error(entry.kind().clone(), &err));
        }
    }
    if contacted && provider_errors.is_empty() {
        return Ok(());
    }
    if !provider_errors.is_empty() {
        return Err(SandboxLookupError::ProviderUnavailable {
            id: id.to_string(),
            provider_errors,
        });
    }
    Err(SandboxLookupError::NotFound { id: id.to_string() })
}
