//! Test-only credential sources and catalogs.
//!
//! Feature-gated so they never link into production builds. Production code
//! resolves credentials through [`VaultCredentialSource`] over a real vault;
//! these helpers exist so tests can supply a source without one.

use std::collections::HashMap;
use std::sync::Arc;

use fabro_vault::Vault;
use lithos_llm::catalog::Catalog;
use tokio::sync::RwLock as AsyncRwLock;

use crate::credential_source::CredentialSource;
use crate::vault_source::VaultCredentialSource;

/// Fabro's policy layer, checked in under `fabro-llm`. Tests in this crate
/// need the built-in catalog with `metadata.fabro.credentials` attached.
pub const FABRO_POLICY_TOML: &str =
    include_str!("../../../components/fabro-llm/catalog/fabro-policy.toml");

/// The lithos built-in catalog with Fabro's policy layer applied.
#[must_use]
pub fn test_catalog() -> Catalog {
    Catalog::builder()
        .with_builtin()
        .toml_layer("fabro-policy.toml", FABRO_POLICY_TOML)
        .expect("fabro policy layer should parse")
        .build()
        .expect("built-in catalog with fabro policy should build")
}

/// A detached in-memory vault holding no secrets.
#[must_use]
pub fn empty_vault() -> Arc<AsyncRwLock<Vault>> {
    Arc::new(AsyncRwLock::new(Vault::from_entries(HashMap::new())))
}

/// A vault-backed source whose credentials come only from `env_lookup`.
///
/// Tests that inject fake provider keys use this instead of reading the real
/// process environment, which would make them order-dependent.
#[must_use]
pub fn env_credential_source<F>(env_lookup: F) -> Arc<dyn CredentialSource>
where
    F: Fn(&str) -> Option<String> + Send + Sync + 'static,
{
    Arc::new(VaultCredentialSource::with_env_lookup(
        empty_vault(),
        env_lookup,
    ))
}

/// A vault-backed source over an empty vault with no process-env fallback.
#[must_use]
pub fn vault_only_credential_source() -> Arc<dyn CredentialSource> {
    Arc::new(VaultCredentialSource::vault_only(empty_vault()))
}
