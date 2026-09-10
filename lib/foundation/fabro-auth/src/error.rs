//! Why a provider's credentials could not be resolved.

use fabro_types::settings::ResolveError as InterpResolveError;
use fabro_vault::SecretType;
use lithos_llm::catalog::ProviderId;

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("{0} is not configured")]
    NotConfigured(ProviderId),
    #[error("{provider} header interpolation failed: {source}")]
    Interpolation {
        provider: ProviderId,
        #[source]
        source:   InterpResolveError,
    },
    #[error("{provider} vault credential '{name}' is not valid Oauth JSON: {source}")]
    VaultDecodeFailed {
        provider: ProviderId,
        name:     String,
        #[source]
        source:   serde_json::Error,
    },
    #[error("{provider} vault credential '{name}' has schema {actual:?}, expected Token or Oauth")]
    VaultSchemaMismatch {
        provider: ProviderId,
        name:     String,
        actual:   SecretType,
    },
    #[error("{provider} requires re-authentication: {source}")]
    RefreshFailed {
        provider: ProviderId,
        #[source]
        source:   anyhow::Error,
    },
    #[error("{0} requires re-authentication: missing refresh token")]
    RefreshTokenMissing(ProviderId),
    #[error("{provider} resolved a secret its `{scheme}` auth scheme cannot use")]
    SchemeMismatch {
        provider: ProviderId,
        scheme:   String,
    },
}

impl ResolveError {
    #[must_use]
    pub fn provider(&self) -> &ProviderId {
        match self {
            Self::NotConfigured(provider)
            | Self::RefreshTokenMissing(provider)
            | Self::Interpolation { provider, .. }
            | Self::VaultDecodeFailed { provider, .. }
            | Self::VaultSchemaMismatch { provider, .. }
            | Self::RefreshFailed { provider, .. }
            | Self::SchemeMismatch { provider, .. } => provider,
        }
    }
}

#[must_use]
pub fn auth_issue_message(provider: &ProviderId, err: &ResolveError) -> String {
    match err {
        ResolveError::NotConfigured(_) => format!("{provider} is not configured"),
        ResolveError::Interpolation { source, .. } => {
            format!("{provider} header interpolation failed: {source}")
        }
        ResolveError::VaultDecodeFailed { name, source, .. } => {
            format!("{provider} vault credential '{name}' is not valid OAuth JSON: {source}")
        }
        ResolveError::VaultSchemaMismatch { name, actual, .. } => format!(
            "{provider} vault credential '{name}' has schema {actual:?}, expected Token or Oauth"
        ),
        ResolveError::RefreshFailed { source, .. } => {
            format!("{provider} requires re-authentication: {source}")
        }
        ResolveError::RefreshTokenMissing(_) => {
            format!("{provider} requires re-authentication: refresh token missing")
        }
        ResolveError::SchemeMismatch { scheme, .. } => {
            format!("{provider} resolved a secret its `{scheme}` auth scheme cannot use")
        }
    }
}
