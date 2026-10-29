#![allow(
    clippy::absolute_paths,
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "This crate intentionally re-exports reqwest types behind the approved facade."
)]

use std::path::Path;
use std::time::Duration;

use fabro_static::EnvVars;
pub use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
pub use reqwest::{
    Body, Method, RequestBuilder, Response, StatusCode, Url, header, multipart, tls,
};

pub type HttpClient = reqwest::Client;
pub type HttpError = reqwest::Error;
pub type BlockingHttpClient = reqwest::blocking::Client;
pub type BlockingRequestBuilder = reqwest::blocking::RequestBuilder;
pub type BlockingResponse = reqwest::blocking::Response;
pub type Proxy = reqwest::Proxy;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProxyPolicy {
    System,
    Disabled,
}

impl ProxyPolicy {
    fn parse(value: &str) -> Result<Self, HttpClientBuildError> {
        match value.to_ascii_lowercase().as_str() {
            "system" => Ok(Self::System),
            "disabled" => Ok(Self::Disabled),
            _ => Err(HttpClientBuildError::InvalidProxyPolicy(value.to_string())),
        }
    }

    fn resolve_with_env_value(
        explicit: Option<Self>,
        env_value: Option<&str>,
    ) -> Result<Self, HttpClientBuildError> {
        if let Some(policy) = explicit {
            return Ok(policy);
        }

        match env_value {
            Some(value) => Self::parse(value),
            None => Ok(Self::System),
        }
    }

    fn resolve(explicit: Option<Self>) -> Result<Self, HttpClientBuildError> {
        match std::env::var(EnvVars::FABRO_HTTP_PROXY_POLICY) {
            Ok(value) => Self::resolve_with_env_value(explicit, Some(&value)),
            Err(std::env::VarError::NotPresent) => Self::resolve_with_env_value(explicit, None),
            Err(std::env::VarError::NotUnicode(value)) => Err(
                HttpClientBuildError::InvalidProxyPolicy(value.to_string_lossy().into_owned()),
            ),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HttpClientBuildError {
    #[error("invalid {env} value `{0}`; expected `system` or `disabled`", env = EnvVars::FABRO_HTTP_PROXY_POLICY)]
    InvalidProxyPolicy(String),

    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
}

/// Generates a proxy-policy-aware builder that wraps a reqwest client builder.
macro_rules! define_builder {
    ($builder_name:ident, $inner_builder:ty, $inner_new:expr, $client_type:ty) => {
        #[derive(Default)]
        pub struct $builder_name {
            inner:        $inner_builder,
            proxy_policy: Option<ProxyPolicy>,
        }

        impl $builder_name {
            #[must_use]
            pub fn new() -> Self {
                Self {
                    inner:        $inner_new,
                    proxy_policy: None,
                }
            }

            #[must_use]
            pub fn proxy_policy(mut self, proxy_policy: ProxyPolicy) -> Self {
                self.proxy_policy = Some(proxy_policy);
                self
            }

            #[must_use]
            pub fn no_proxy(mut self) -> Self {
                self.inner = self.inner.no_proxy();
                self
            }

            #[must_use]
            pub fn proxy(mut self, proxy: Proxy) -> Self {
                self.inner = self.inner.proxy(proxy);
                self
            }

            #[must_use]
            pub fn user_agent(mut self, value: impl Into<String>) -> Self {
                self.inner = self.inner.user_agent(value.into());
                self
            }

            #[must_use]
            pub fn redirect(mut self, policy: reqwest::redirect::Policy) -> Self {
                self.inner = self.inner.redirect(policy);
                self
            }

            #[must_use]
            pub fn cookie_store(mut self, enabled: bool) -> Self {
                self.inner = self.inner.cookie_store(enabled);
                self
            }

            #[must_use]
            pub fn default_headers(mut self, headers: HeaderMap) -> Self {
                self.inner = self.inner.default_headers(headers);
                self
            }

            #[must_use]
            pub fn connect_timeout(mut self, timeout: Duration) -> Self {
                self.inner = self.inner.connect_timeout(timeout);
                self
            }

            #[must_use]
            pub fn timeout(mut self, timeout: Duration) -> Self {
                self.inner = self.inner.timeout(timeout);
                self
            }

            #[must_use]
            pub fn danger_accept_invalid_certs(mut self, accept_invalid_certs: bool) -> Self {
                self.inner = self.inner.danger_accept_invalid_certs(accept_invalid_certs);
                self
            }

            #[cfg(unix)]
            #[must_use]
            pub fn unix_socket<P>(mut self, path: P) -> Self
            where
                P: AsRef<Path>,
            {
                self.inner = self.inner.unix_socket(path.as_ref());
                self
            }

            pub fn build(self) -> Result<$client_type, HttpClientBuildError> {
                let proxy_policy = ProxyPolicy::resolve(self.proxy_policy)?;
                let inner = match proxy_policy {
                    ProxyPolicy::System => self.inner,
                    ProxyPolicy::Disabled => self.inner.no_proxy(),
                };
                inner.build().map_err(Into::into)
            }
        }
    };
}

define_builder!(
    HttpClientBuilder,
    reqwest::ClientBuilder,
    reqwest::Client::builder(),
    HttpClient
);

// `read_timeout` is only available on the async builder.
impl HttpClientBuilder {
    #[must_use]
    pub fn read_timeout(mut self, timeout: Duration) -> Self {
        self.inner = self.inner.read_timeout(timeout);
        self
    }
}

define_builder!(
    BlockingHttpClientBuilder,
    reqwest::blocking::ClientBuilder,
    reqwest::blocking::Client::builder(),
    BlockingHttpClient
);

pub fn http_client() -> Result<HttpClient, HttpClientBuildError> {
    HttpClientBuilder::new().build()
}

/// The run channel handshake (denkhaus/work#1043): the ONE attach site
/// for `x-fabro-run-channel-token`. Every Unix-socket client builder
/// runs through here, so no client can drift out of the channel's
/// handshake — a process carrying the owning run's
/// `FABRO_RUN_CHANNEL_TOKEN` gets the header; every other process is
/// untouched. A value that cannot form a header is dropped: the serve
/// then refuses loudly instead of this client panicking.
#[must_use]
pub fn with_run_channel_handshake(builder: HttpClientBuilder) -> HttpClientBuilder {
    let token = std::env::var(fabro_static::EnvVars::FABRO_RUN_CHANNEL_TOKEN).ok();
    apply_run_channel_handshake(builder, token.as_deref())
}

/// The pure seam of [`with_run_channel_handshake`]: the env read is the
/// only impure step, so the shape logic is unit-tested here — tests
/// never mutate the process env (server-secrets strategy).
fn apply_run_channel_handshake(
    builder: HttpClientBuilder,
    token: Option<&str>,
) -> HttpClientBuilder {
    let Some(header) = handshake_header(token) else {
        return builder;
    };
    let mut headers = HeaderMap::new();
    headers.insert(header.0, header.1);
    // reqwest merges default headers per key (verified against the
    // vendored source), so a bearer applied earlier survives.
    builder.default_headers(headers)
}

/// The header a handshake `token` produces: `None` for absent or
/// empty tokens and for values that cannot form a header value.
fn handshake_header(token: Option<&str>) -> Option<(HeaderName, HeaderValue)> {
    let token = token?.trim();
    if token.is_empty() {
        return None;
    }
    let name = fabro_static::RUN_CHANNEL_TOKEN_HEADER
        .parse::<HeaderName>()
        .ok()?;
    Some((name, HeaderValue::from_str(token).ok()?))
}

pub fn test_http_client() -> Result<HttpClient, HttpClientBuildError> {
    HttpClientBuilder::new()
        .proxy_policy(ProxyPolicy::Disabled)
        .build()
}

pub fn blocking_http_client() -> Result<BlockingHttpClient, HttpClientBuildError> {
    BlockingHttpClientBuilder::new().build()
}

pub fn blocking_test_http_client() -> Result<BlockingHttpClient, HttpClientBuildError> {
    BlockingHttpClientBuilder::new()
        .proxy_policy(ProxyPolicy::Disabled)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The handshake header's shapes (work#1043): a real token forms
    /// the named header; absent, empty, whitespace-only, and
    /// header-hostile values produce nothing (the serve refuses such a
    /// client loudly instead of this client panicking).
    #[test]
    fn handshake_header_shapes() {
        let header = handshake_header(Some("0123456789abcdef")).expect("a token forms a header");
        assert_eq!(header.0.as_str(), "x-fabro-run-channel-token");
        assert_eq!(header.1, "0123456789abcdef");

        assert_eq!(handshake_header(None), None, "absent token");
        assert_eq!(handshake_header(Some("")), None, "empty token");
        assert_eq!(handshake_header(Some("   ")), None, "whitespace token");
        assert_eq!(
            handshake_header(Some("bad\r\nvalue")),
            None,
            "a header-hostile value is dropped, never panics"
        );
    }

    #[test]
    fn proxy_policy_defaults_to_system() {
        assert_eq!(
            ProxyPolicy::resolve_with_env_value(None, None).unwrap(),
            ProxyPolicy::System
        );
    }

    #[test]
    fn proxy_policy_reads_disabled_from_env() {
        assert_eq!(
            ProxyPolicy::resolve_with_env_value(None, Some("disabled")).unwrap(),
            ProxyPolicy::Disabled
        );
    }

    #[test]
    fn proxy_policy_rejects_invalid_env_values() {
        let error = ProxyPolicy::resolve_with_env_value(None, Some("bogus")).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("expected `system` or `disabled`")
        );
    }

    #[test]
    fn explicit_proxy_policy_overrides_env() {
        assert_eq!(
            ProxyPolicy::resolve_with_env_value(Some(ProxyPolicy::Disabled), Some("system"))
                .unwrap(),
            ProxyPolicy::Disabled
        );
    }
}
