//! Secret resolution for one catalog provider.
//!
//! A provider's `metadata.fabro.credentials` names where its secret lives.
//! [`CredentialResolver`] walks that list against the vault and the process
//! environment, refreshes an expired OAuth credential, and shapes the result
//! into the lithos [`Credentials`] the provider's declared auth scheme
//! expects.

use std::collections::BTreeMap;
use std::sync::Arc;

use fabro_static::EnvVars;
use fabro_types::catalog_policy::{self, ProviderPolicy};
use fabro_types::provider_ids;
use fabro_types::settings::{InterpString, ResolveCtx, ResolveError as InterpResolveError};
use fabro_vault::{SecretType, Vault};
use lithos_llm::catalog::{AuthScheme, Catalog, CatalogProvider, ProviderId};
use lithos_llm::credentials::{
    CredentialHeader, Credentials, HttpAuthentication, HttpCredentials, SecretValue,
};
use tokio::sync::RwLock as AsyncRwLock;
use tokio::task::spawn_blocking;

use crate::credential::OAuthCredential;
use crate::credential_ref::{CredentialRef, CredentialRefParseError};
use crate::refresh::refresh_oauth_credential;
use crate::vault_ext::{
    VaultLookupError, vault_get_oauth, vault_get_token, vault_set_oauth, vault_token_lookup,
};

pub type EnvLookup = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

const CHATGPT_ACCOUNT_ID_HEADER: &str = "ChatGPT-Account-Id";
const OPENAI_ORGANIZATION_HEADER: &str = "OpenAI-Organization";
const OPENAI_PROJECT_HEADER: &str = "OpenAI-Project";

/// A secret found for a provider, before it is shaped into [`Credentials`].
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum ResolvedSecret {
    ApiKey(String),
    OAuth {
        credential: Box<OAuthCredential>,
        vault_name: String,
    },
    /// No static secret: the adapter signs with the AWS default chain.
    AwsDefaultChain,
}

impl std::fmt::Debug for ResolvedSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ApiKey(_) => f.write_str("ApiKey(<redacted>)"),
            Self::OAuth { vault_name, .. } => f
                .debug_struct("OAuth")
                .field("vault_name", vault_name)
                .finish_non_exhaustive(),
            Self::AwsDefaultChain => f.write_str("AwsDefaultChain"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("{0} is not configured")]
    NotConfigured(ProviderId),
    #[error("{provider} declares an invalid credential reference `{reference}`: {source}")]
    InvalidCredentialRef {
        provider:  ProviderId,
        reference: String,
        #[source]
        source:    CredentialRefParseError,
    },
    #[error("{provider} header interpolation failed: {source}")]
    Interpolation {
        provider: ProviderId,
        #[source]
        source:   InterpResolveError,
    },
    #[error("{provider} vault credential '{name}' has schema {actual:?}, expected Token or Oauth")]
    VaultSchemaMismatch {
        provider: ProviderId,
        name:     String,
        actual:   SecretType,
    },
    #[error("{provider} vault credential '{name}' is not valid Oauth JSON: {source}")]
    VaultDecodeFailed {
        provider: ProviderId,
        name:     String,
        #[source]
        source:   serde_json::Error,
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
            | Self::InvalidCredentialRef { provider, .. }
            | Self::Interpolation { provider, .. }
            | Self::VaultSchemaMismatch { provider, .. }
            | Self::VaultDecodeFailed { provider, .. }
            | Self::RefreshFailed { provider, .. }
            | Self::SchemeMismatch { provider, .. } => provider,
        }
    }
}

#[must_use]
pub fn auth_issue_message(provider: &ProviderId, err: &ResolveError) -> String {
    match err {
        ResolveError::NotConfigured(_) => format!("{provider} is not configured"),
        ResolveError::InvalidCredentialRef {
            reference, source, ..
        } => format!("{provider} declares an invalid credential reference `{reference}`: {source}"),
        ResolveError::Interpolation { source, .. } => {
            format!("{provider} header interpolation failed: {source}")
        }
        ResolveError::VaultSchemaMismatch { name, actual, .. } => format!(
            "{provider} vault credential '{name}' has schema {actual:?}, expected Token or Oauth"
        ),
        ResolveError::VaultDecodeFailed { name, source, .. } => {
            format!("{provider} vault credential '{name}' is not valid OAuth JSON: {source}")
        }
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

/// The credential references a provider declares, in resolution order.
pub fn credential_refs(provider: &CatalogProvider) -> Result<Vec<CredentialRef>, ResolveError> {
    credential_refs_from_policy(provider.id(), &catalog_policy::provider_policy(provider))
}

fn credential_refs_from_policy(
    provider: &ProviderId,
    policy: &ProviderPolicy,
) -> Result<Vec<CredentialRef>, ResolveError> {
    policy
        .credentials
        .iter()
        .map(|reference| {
            reference
                .parse()
                .map_err(|source| ResolveError::InvalidCredentialRef {
                    provider: provider.clone(),
                    reference: reference.clone(),
                    source,
                })
        })
        .collect()
}

/// The vault entry an operator should create to configure `provider`, when
/// the provider reads a vault secret.
#[must_use]
pub fn expected_vault_secret_name(provider: &CatalogProvider) -> Option<String> {
    credential_refs(provider)
        .ok()?
        .into_iter()
        .find_map(|reference| match reference {
            CredentialRef::Vault(name) => Some(name),
            CredentialRef::Env(_) | CredentialRef::AwsSigv4 => None,
        })
}

/// The environment variables an operator can set to configure `provider`.
#[must_use]
pub fn env_var_names(provider: &CatalogProvider) -> Vec<String> {
    credential_refs(provider)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|reference| match reference {
            CredentialRef::Env(name) => Some(name),
            CredentialRef::Vault(_) | CredentialRef::AwsSigv4 => None,
        })
        .collect()
}

/// Whether the provider takes a single API key an operator can paste in.
#[must_use]
pub fn accepts_api_key(provider: &CatalogProvider) -> bool {
    matches!(
        provider.auth(),
        AuthScheme::Bearer { .. } | AuthScheme::Header { .. } | AuthScheme::BedrockBearer
    ) && credential_refs(provider).is_ok_and(|refs| {
        refs.iter()
            .any(|reference| matches!(reference, CredentialRef::Vault(_) | CredentialRef::Env(_)))
    })
}

fn auth_scheme_name(scheme: &AuthScheme) -> &'static str {
    match scheme {
        AuthScheme::None => "none",
        AuthScheme::Bearer { .. } => "bearer",
        AuthScheme::Header { .. } => "header",
        AuthScheme::Headers => "headers",
        AuthScheme::Aws { .. } => "aws",
        AuthScheme::BedrockBearer => "bedrock_bearer",
        _ => "unknown",
    }
}

/// Shapes a caller-supplied API key into the provider's credentials.
///
/// Used to validate a key before it is stored. Extra headers that need vault
/// secrets are resolved against `vault`.
pub fn credentials_for_api_key(
    provider: &CatalogProvider,
    key: String,
    vault: &Vault,
) -> Result<Credentials, ResolveError> {
    let extra_headers = resolved_extra_headers(vault, provider)?;
    shape_secret(provider, ResolvedSecret::ApiKey(key), extra_headers, None)
}

/// Resolves a provider's `extra_headers` interpolation against the vault.
///
/// Resolved header values may contain secrets; keep this path free of value
/// logging.
fn resolved_extra_headers(
    vault: &Vault,
    provider: &CatalogProvider,
) -> Result<Vec<CredentialHeader>, ResolveError> {
    let policy = catalog_policy::provider_policy(provider);
    let mut ctx =
        ResolveCtx::new().with_secrets(|secret_name| vault_token_lookup(vault, secret_name));
    resolve_extra_headers(provider.id(), &policy.extra_headers, &mut ctx)
}

pub(crate) fn resolve_extra_headers(
    provider: &ProviderId,
    headers: &BTreeMap<String, String>,
    ctx: &mut ResolveCtx<'_>,
) -> Result<Vec<CredentialHeader>, ResolveError> {
    headers
        .iter()
        .map(|(name, source)| {
            let value = InterpString::parse(source)
                .resolve_with(ctx)
                .map_err(|source| ResolveError::Interpolation {
                    provider: provider.clone(),
                    source,
                })?;
            Ok(CredentialHeader::new(name.clone(), SecretValue::new(value)))
        })
        .collect()
}

fn shape_secret(
    provider: &CatalogProvider,
    secret: ResolvedSecret,
    mut extra_headers: Vec<CredentialHeader>,
    env_lookup: Option<&EnvLookup>,
) -> Result<Credentials, ResolveError> {
    let scheme = provider.auth();
    let mismatch = || ResolveError::SchemeMismatch {
        provider: provider.id().clone(),
        scheme:   auth_scheme_name(scheme).to_string(),
    };
    if let Some(env_lookup) = env_lookup.filter(|_| provider.id().as_str() == provider_ids::OPENAI)
    {
        for (variable, header) in [
            (EnvVars::OPENAI_ORG_ID, OPENAI_ORGANIZATION_HEADER),
            (EnvVars::OPENAI_PROJECT_ID, OPENAI_PROJECT_HEADER),
        ] {
            if let Some(value) = env_lookup(variable) {
                extra_headers.push(CredentialHeader::new(header, SecretValue::new(value)));
            }
        }
    }
    match (scheme, secret) {
        (AuthScheme::Bearer { .. }, ResolvedSecret::ApiKey(key)) => Ok(http_credentials(
            HttpAuthentication::Bearer(SecretValue::new(key)),
            extra_headers,
        )),
        (AuthScheme::Bearer { .. }, ResolvedSecret::OAuth { credential, .. }) => {
            if let Some(account_id) = &credential.account_id {
                extra_headers.push(CredentialHeader::new(
                    CHATGPT_ACCOUNT_ID_HEADER,
                    SecretValue::new(account_id.clone()),
                ));
            }
            Ok(http_credentials(
                HttpAuthentication::Bearer(SecretValue::new(
                    credential.tokens.access_token.clone(),
                )),
                extra_headers,
            ))
        }
        (AuthScheme::Header { name }, ResolvedSecret::ApiKey(key)) => Ok(http_credentials(
            HttpAuthentication::Header(CredentialHeader::new(name.clone(), SecretValue::new(key))),
            extra_headers,
        )),
        (AuthScheme::BedrockBearer | AuthScheme::Aws { .. }, ResolvedSecret::ApiKey(key)) => {
            Ok(Credentials::BedrockBearer(SecretValue::new(key)))
        }
        (AuthScheme::Aws { region }, ResolvedSecret::AwsDefaultChain) => {
            Ok(Credentials::AwsDefaultChain {
                region: region.clone(),
            })
        }
        _ => Err(mismatch()),
    }
}

fn http_credentials(auth: HttpAuthentication, extra_headers: Vec<CredentialHeader>) -> Credentials {
    let mut credentials = HttpCredentials::new(auth);
    credentials.extra_headers = extra_headers;
    Credentials::Http(credentials)
}

#[derive(Clone)]
pub struct CredentialResolver {
    vault:      Arc<AsyncRwLock<Vault>>,
    env_lookup: EnvLookup,
}

impl CredentialResolver {
    #[must_use]
    #[expect(
        clippy::disallowed_methods,
        reason = "CredentialResolver owns the process-env fallback used after vault lookup."
    )]
    pub fn new(vault: Arc<AsyncRwLock<Vault>>) -> Self {
        Self::with_env_lookup(vault, Arc::new(|name| std::env::var(name).ok()))
    }

    #[must_use]
    pub fn with_env_lookup(vault: Arc<AsyncRwLock<Vault>>, env_lookup: EnvLookup) -> Self {
        Self { vault, env_lookup }
    }

    /// Resolves `provider`'s credentials for one request attempt.
    ///
    /// An expired OAuth credential is refreshed and the refreshed tokens are
    /// written back to the vault before the credentials are returned.
    pub async fn resolve(&self, provider: &CatalogProvider) -> Result<Credentials, ResolveError> {
        let provider_id = provider.id().clone();
        match provider.auth() {
            AuthScheme::None => {
                let vault = self.vault.read().await;
                let headers = resolved_extra_headers(&vault, provider)?;
                return Ok(Credentials::headers(headers));
            }
            AuthScheme::Headers => {
                let vault = self.vault.read().await;
                let headers = resolved_extra_headers(&vault, provider)?;
                if headers.is_empty() {
                    return Err(ResolveError::NotConfigured(provider_id));
                }
                return Ok(Credentials::headers(headers));
            }
            _ => {}
        }

        let (initial_secret, extra_headers) = {
            let vault = self.vault.read().await;
            (
                self.find_secret(&vault, provider)?,
                resolved_extra_headers(&vault, provider)?,
            )
        };

        let secret = match initial_secret {
            ResolvedSecret::OAuth {
                credential,
                vault_name,
            } if credential.needs_refresh() => {
                if credential.tokens.refresh_token.is_none() {
                    return Err(ResolveError::RefreshTokenMissing(provider_id));
                }
                let refreshed = refresh_oauth_credential(&credential)
                    .await
                    .map_err(|source| ResolveError::RefreshFailed {
                        provider: provider_id.clone(),
                        source,
                    })?;
                self.persist_oauth(&provider_id, &vault_name, &refreshed)
                    .await?;
                ResolvedSecret::OAuth {
                    credential: Box::new(refreshed),
                    vault_name,
                }
            }
            secret => secret,
        };

        shape_secret(provider, secret, extra_headers, Some(&self.env_lookup))
    }

    async fn persist_oauth(
        &self,
        provider: &ProviderId,
        vault_name: &str,
        refreshed: &OAuthCredential,
    ) -> Result<(), ResolveError> {
        let refreshed = refreshed.clone();
        let vault_name = vault_name.to_string();
        let vault = Arc::clone(&self.vault);
        spawn_blocking(move || {
            let mut vault = vault.blocking_write();
            vault_set_oauth(&mut vault, &vault_name, &refreshed)
                .map(|_| ())
                .map_err(anyhow::Error::from)
        })
        .await
        .map_err(|join_err| ResolveError::RefreshFailed {
            provider: provider.clone(),
            source:   anyhow::Error::from(join_err),
        })?
        .map_err(|source| ResolveError::RefreshFailed {
            provider: provider.clone(),
            source,
        })
    }

    /// Providers with credential material present, without refreshing
    /// anything. Disabled providers are skipped.
    #[must_use]
    pub fn configured_providers(&self, vault: &Vault, catalog: &Catalog) -> Vec<ProviderId> {
        catalog
            .providers()
            .filter(|provider| catalog_policy::provider_policy(provider).is_enabled())
            .filter(|provider| self.has_credential_material(vault, provider))
            .map(|provider| provider.id().clone())
            .collect()
    }

    fn has_credential_material(&self, vault: &Vault, provider: &CatalogProvider) -> bool {
        match provider.auth() {
            AuthScheme::None => resolved_extra_headers(vault, provider).is_ok(),
            AuthScheme::Headers => {
                resolved_extra_headers(vault, provider).is_ok_and(|headers| !headers.is_empty())
            }
            _ => self.find_secret(vault, provider).is_ok(),
        }
    }

    fn find_secret(
        &self,
        vault: &Vault,
        provider: &CatalogProvider,
    ) -> Result<ResolvedSecret, ResolveError> {
        for reference in credential_refs(provider)? {
            if let Some(secret) = self.secret_from_ref(vault, provider.id(), &reference)? {
                return Ok(secret);
            }
        }
        Err(ResolveError::NotConfigured(provider.id().clone()))
    }

    fn secret_from_ref(
        &self,
        vault: &Vault,
        provider: &ProviderId,
        reference: &CredentialRef,
    ) -> Result<Option<ResolvedSecret>, ResolveError> {
        match reference {
            CredentialRef::Vault(name) => match vault_get_token(vault, name) {
                Ok(Some(token)) => Ok(Some(ResolvedSecret::ApiKey(token))),
                Ok(None) => Ok(None),
                Err(VaultLookupError::SchemaMismatch {
                    actual: SecretType::Oauth,
                    ..
                }) => vault_get_oauth(vault, name)
                    .map(|credential| {
                        credential.map(|credential| ResolvedSecret::OAuth {
                            credential: Box::new(credential),
                            vault_name: name.clone(),
                        })
                    })
                    .map_err(|err| vault_lookup_error(provider, name, err)),
                Err(err) => Err(vault_lookup_error(provider, name, err)),
            },
            CredentialRef::Env(name) => Ok((self.env_lookup)(name).map(ResolvedSecret::ApiKey)),
            CredentialRef::AwsSigv4 => Ok(Some(ResolvedSecret::AwsDefaultChain)),
        }
    }
}

fn vault_lookup_error(provider: &ProviderId, name: &str, err: VaultLookupError) -> ResolveError {
    match err {
        VaultLookupError::SchemaMismatch { actual, .. } => ResolveError::VaultSchemaMismatch {
            provider: provider.clone(),
            name: name.to_string(),
            actual,
        },
        VaultLookupError::DecodeFailed { source, .. } => ResolveError::VaultDecodeFailed {
            provider: provider.clone(),
            name: name.to_string(),
            source,
        },
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use httpmock::Method::POST;
    use httpmock::MockServer;

    use super::*;
    use crate::credential::{OAuthConfig, OAuthCredential, OAuthTokens};
    use crate::test_support::test_catalog;
    use crate::vault_ext::{vault_get_oauth, vault_set_oauth, vault_set_token};

    fn oauth_credential(token_url: String, expires_at: chrono::DateTime<Utc>) -> OAuthCredential {
        OAuthCredential {
            tokens:     OAuthTokens {
                access_token: "expired-access".to_string(),
                refresh_token: Some("refresh-token".to_string()),
                expires_at,
            },
            config:     OAuthConfig {
                auth_url: "https://auth.openai.com".to_string(),
                token_url,
                client_id: "test-client".to_string(),
                scopes: vec!["openid".to_string()],
                redirect_uri: Some("https://auth.openai.com/deviceauth/callback".to_string()),
                use_pkce: true,
            },
            account_id: Some("acct_123".to_string()),
        }
    }

    fn test_resolver(vault: Vault, env_lookup: EnvLookup) -> CredentialResolver {
        CredentialResolver::with_env_lookup(Arc::new(AsyncRwLock::new(vault)), env_lookup)
    }

    fn empty_vault() -> Vault {
        Vault::from_entries(std::collections::HashMap::new())
    }

    fn bearer_secret(credentials: &Credentials) -> &str {
        match credentials {
            Credentials::Http(HttpCredentials {
                auth: HttpAuthentication::Bearer(secret),
                ..
            }) => secret.expose_secret(),
            other => panic!("expected bearer credentials, got {other:?}"),
        }
    }

    fn header_value<'a>(credentials: &'a Credentials, name: &str) -> Option<&'a str> {
        match credentials {
            Credentials::Http(http) => http
                .extra_headers
                .iter()
                .find(|header| header.name.eq_ignore_ascii_case(name))
                .map(|header| header.value.expose_secret()),
            _ => None,
        }
    }

    #[tokio::test]
    async fn env_listed_first_wins_over_vault() {
        let mut vault = empty_vault();
        vault_set_token(&mut vault, "OPENAI_API_KEY", "vault-key").unwrap();
        let resolver = test_resolver(
            vault,
            Arc::new(|name| (name == "OPENAI_API_KEY").then(|| "env-key".to_string())),
        );
        let catalog = test_catalog();
        let credentials = resolver
            .resolve(catalog.provider("openai").unwrap())
            .await
            .unwrap();
        assert_eq!(bearer_secret(&credentials), "env-key");
    }

    #[tokio::test]
    async fn moonshot_falls_back_to_kimi_env_key() {
        let resolver = test_resolver(
            empty_vault(),
            Arc::new(|name| (name == EnvVars::KIMI_API_KEY).then(|| "kimi-key".to_string())),
        );
        let catalog = test_catalog();
        let credentials = resolver
            .resolve(catalog.provider("moonshot").unwrap())
            .await
            .unwrap();
        assert_eq!(bearer_secret(&credentials), "kimi-key");
    }

    #[tokio::test]
    async fn anthropic_uses_its_header_scheme() {
        let mut vault = empty_vault();
        vault_set_token(&mut vault, "ANTHROPIC_API_KEY", "anthropic-key").unwrap();
        let resolver = test_resolver(vault, Arc::new(|_| None));
        let catalog = test_catalog();
        let credentials = resolver
            .resolve(catalog.provider("anthropic").unwrap())
            .await
            .unwrap();
        match credentials {
            Credentials::Http(HttpCredentials {
                auth: HttpAuthentication::Header(header),
                ..
            }) => {
                assert_eq!(header.name, "x-api-key");
                assert_eq!(header.value.expose_secret(), "anthropic-key");
            }
            other => panic!("expected header credentials, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn codex_oauth_becomes_a_bearer_with_account_header() {
        let mut vault = empty_vault();
        vault_set_oauth(
            &mut vault,
            crate::OPENAI_CODEX_VAULT_SECRET_NAME,
            &oauth_credential(
                "https://auth.openai.com/oauth/token".to_string(),
                Utc::now() + Duration::hours(1),
            ),
        )
        .unwrap();
        let resolver = test_resolver(vault, Arc::new(|_| None));
        let catalog = test_catalog();
        let credentials = resolver
            .resolve(catalog.provider("openai-codex").unwrap())
            .await
            .unwrap();
        assert_eq!(bearer_secret(&credentials), "expired-access");
        assert_eq!(
            header_value(&credentials, CHATGPT_ACCOUNT_ID_HEADER),
            Some("acct_123")
        );
    }

    #[tokio::test]
    async fn openai_api_key_attaches_org_and_project_from_env() {
        let resolver = test_resolver(
            empty_vault(),
            Arc::new(|name| match name {
                "OPENAI_API_KEY" => Some("key".to_string()),
                "OPENAI_ORG_ID" => Some("org".to_string()),
                "OPENAI_PROJECT_ID" => Some("proj".to_string()),
                _ => None,
            }),
        );
        let catalog = test_catalog();
        let credentials = resolver
            .resolve(catalog.provider("openai").unwrap())
            .await
            .unwrap();
        assert_eq!(
            header_value(&credentials, "OpenAI-Organization"),
            Some("org")
        );
        assert_eq!(header_value(&credentials, "OpenAI-Project"), Some("proj"));
    }

    #[tokio::test]
    async fn bedrock_falls_back_to_the_aws_default_chain() {
        let resolver = test_resolver(empty_vault(), Arc::new(|_| None));
        let catalog = test_catalog();
        let credentials = resolver
            .resolve(catalog.provider("bedrock").unwrap())
            .await
            .unwrap();
        assert!(matches!(credentials, Credentials::AwsDefaultChain { .. }));

        let resolver = test_resolver(
            empty_vault(),
            Arc::new(|name| (name == "BEDROCK_API_KEY").then(|| "bearer".to_string())),
        );
        let credentials = resolver
            .resolve(catalog.provider("bedrock").unwrap())
            .await
            .unwrap();
        assert!(matches!(credentials, Credentials::BedrockBearer(_)));
    }

    #[tokio::test]
    async fn missing_provider_material_is_not_configured() {
        let resolver = test_resolver(empty_vault(), Arc::new(|_| None));
        let catalog = test_catalog();
        let err = resolver
            .resolve(catalog.provider("anthropic").unwrap())
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            ResolveError::NotConfigured(provider) if provider.as_str() == "anthropic"
        ));
    }

    #[tokio::test]
    async fn modal_resolves_both_vault_proxy_headers_without_authorization() {
        let mut vault = empty_vault();
        vault_set_token(&mut vault, "MODAL_TOKEN_ID", "wk-test").unwrap();
        vault_set_token(&mut vault, "MODAL_TOKEN_SECRET", "ws-test").unwrap();
        let resolver = test_resolver(vault, Arc::new(|_| None));
        let catalog = test_catalog();
        let modal = catalog.provider("modal").unwrap();
        {
            let vault = resolver.vault.read().await;
            assert!(resolver.has_credential_material(&vault, modal));
        }
        let credentials = resolver.resolve(modal).await.unwrap();
        match &credentials {
            Credentials::Http(http) => assert!(matches!(http.auth, HttpAuthentication::None)),
            other => panic!("expected header-only credentials, got {other:?}"),
        }
        assert_eq!(header_value(&credentials, "Modal-Key"), Some("wk-test"));
        assert_eq!(header_value(&credentials, "Modal-Secret"), Some("ws-test"));
    }

    #[tokio::test]
    async fn modal_is_not_configured_with_only_one_vault_proxy_token() {
        let mut vault = empty_vault();
        vault_set_token(&mut vault, "MODAL_TOKEN_ID", "wk-test").unwrap();
        let resolver = test_resolver(vault, Arc::new(|_| None));
        let catalog = test_catalog();
        let modal = catalog.provider("modal").unwrap();
        let err = resolver.resolve(modal).await.unwrap_err();
        assert!(matches!(err, ResolveError::Interpolation { .. }), "{err}");
        assert!(!err.to_string().contains("wk-test"));
    }

    #[tokio::test]
    async fn configured_providers_reads_vault_and_env_without_refreshing() {
        let mut vault = empty_vault();
        vault_set_token(&mut vault, "OPENAI_API_KEY", "vault-key").unwrap();
        let resolver = test_resolver(
            vault,
            Arc::new(|name| (name == "ANTHROPIC_API_KEY").then(|| "env".to_string())),
        );
        let catalog = test_catalog();
        let vault = resolver.vault.read().await;
        let configured = resolver.configured_providers(&vault, &catalog);
        assert!(configured.contains(&ProviderId::new("openai")));
        assert!(configured.contains(&ProviderId::new("anthropic")));
        // Bedrock always resolves through the AWS chain but ships disabled.
        assert!(!configured.contains(&ProviderId::new("bedrock")));
    }

    #[tokio::test]
    async fn refreshes_expired_oauth_credentials_and_persists_them() {
        let server = MockServer::start_async().await;
        let refresh_mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/oauth/token")
                    .form_urlencoded_tuple("grant_type", "refresh_token")
                    .form_urlencoded_tuple("client_id", "test-client")
                    .form_urlencoded_tuple("refresh_token", "refresh-token");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(
                        serde_json::json!({
                            "access_token": "new-access",
                            "refresh_token": "new-refresh",
                            "expires_in": 3600
                        })
                        .to_string(),
                    );
            })
            .await;

        let mut vault = empty_vault();
        vault_set_oauth(
            &mut vault,
            crate::OPENAI_CODEX_VAULT_SECRET_NAME,
            &oauth_credential(
                server.url("/oauth/token"),
                Utc::now() - Duration::minutes(1),
            ),
        )
        .unwrap();
        let vault = Arc::new(AsyncRwLock::new(vault));
        let resolver = CredentialResolver::with_env_lookup(Arc::clone(&vault), Arc::new(|_| None));
        let catalog = test_catalog();

        let credentials = resolver
            .resolve(catalog.provider("openai-codex").unwrap())
            .await
            .unwrap();
        assert_eq!(bearer_secret(&credentials), "new-access");

        let stored = {
            let vault = vault.read().await;
            vault_get_oauth(&vault, crate::OPENAI_CODEX_VAULT_SECRET_NAME)
                .unwrap()
                .unwrap()
        };
        assert_eq!(stored.tokens.access_token, "new-access");
        assert_eq!(stored.tokens.refresh_token.as_deref(), Some("new-refresh"));
        assert_eq!(stored.account_id.as_deref(), Some("acct_123"));
        refresh_mock.assert_async().await;
    }

    #[tokio::test]
    async fn expired_oauth_without_refresh_token_requires_reauthentication() {
        let mut vault = empty_vault();
        let mut credential = oauth_credential(
            "https://auth.openai.com/oauth/token".to_string(),
            Utc::now() - Duration::minutes(1),
        );
        credential.tokens.refresh_token = None;
        vault_set_oauth(
            &mut vault,
            crate::OPENAI_CODEX_VAULT_SECRET_NAME,
            &credential,
        )
        .unwrap();
        let resolver = test_resolver(vault, Arc::new(|_| None));
        let catalog = test_catalog();
        let err = resolver
            .resolve(catalog.provider("openai-codex").unwrap())
            .await
            .unwrap_err();
        assert!(matches!(err, ResolveError::RefreshTokenMissing(_)));
        assert_eq!(
            auth_issue_message(&ProviderId::new("openai-codex"), &err),
            "openai-codex requires re-authentication: refresh token missing"
        );
    }

    #[test]
    fn api_key_credentials_follow_the_provider_scheme() {
        let catalog = test_catalog();
        let vault = empty_vault();
        let openai = credentials_for_api_key(
            catalog.provider("openai").unwrap(),
            "sk-test".to_string(),
            &vault,
        )
        .unwrap();
        assert_eq!(bearer_secret(&openai), "sk-test");
        let modal = credentials_for_api_key(
            catalog.provider("modal").unwrap(),
            "sk-test".to_string(),
            &vault,
        );
        assert!(modal.is_err(), "modal has no single-key scheme");
        assert!(!accepts_api_key(catalog.provider("modal").unwrap()));
        assert!(accepts_api_key(catalog.provider("openai").unwrap()));
        assert!(!accepts_api_key(catalog.provider("ollama").unwrap()));
    }

    #[test]
    fn resolved_secret_debug_redacts_material() {
        let debug = format!("{:?}", ResolvedSecret::ApiKey("sk-test".to_string()));
        assert!(!debug.contains("sk-test"));
    }
}
