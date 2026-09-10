mod api_key_source;
mod context;
mod credential;
mod credential_source;
mod error;
mod extra_headers_source;
mod refresh;
mod secrets;
mod sql_vault_source;
mod strategy;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
mod vault_ext;
mod vault_source;

pub mod strategies;

pub use api_key_source::ApiKeyCredentialSource;
pub use context::{AuthContextRequest, AuthContextResponse};
pub use credential::{OAuthConfig, OAuthCredential, OAuthTokens};
pub use credential_source::{CredentialSource, ResolvedCredentials, lithos_credentials};
pub use error::{ResolveError, auth_issue_message};
pub use extra_headers_source::ExtraHeadersCredentialSource;
pub use refresh::refresh_oauth_credential;
pub use secrets::{accepts_api_key, expected_secret_name, secret_names};
pub use sql_vault_source::SqlVaultCredentialSource;
pub use strategy::{
    AuthMethod, AuthStrategy, CODEX_AUTH_URL, CODEX_CLIENT_ID, CODEX_TOKEN_URL, LoginResult,
    codex_oauth_config, strategy_for,
};
pub use vault_ext::{
    VaultLookupError, vault_get_oauth, vault_get_token, vault_set_oauth, vault_set_token,
};
pub use vault_source::{EnvLookup, VaultCredentialSource};

/// The vault entry holding the Codex OAuth credential that serves the
/// `openai-codex` provider.
pub const OPENAI_CODEX_VAULT_SECRET_NAME: &str = "OPENAI_CODEX";
