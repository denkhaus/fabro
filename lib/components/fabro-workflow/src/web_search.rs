//! Built-in `web_search` backends for workflow agents.
//!
//! Agents always call the same tool. Brave is preferred when its credential
//! is present; otherwise Venice is used when its credential is present. With
//! neither, the agent gets no search tool.

use std::fmt::Write as _;
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use pebble_coding_agent::extensions::{
    SearchError, SearchErrorKind, SearchProvider, SearchRequest, SearchResult,
};

const BRAVE_SEARCH_URL: &str = "https://api.search.brave.com/res/v1/web/search";
const VENICE_SEARCH_URL: &str = "https://api.venice.ai/api/v1/augment/search";
const VENICE_QUERY_MAX_CHARS: usize = 400;
const VENICE_REQUEST_TIMEOUT: Duration = Duration::from_mins(1);
const MAX_RESULTS: u32 = 20;

/// Credentials for the built-in search backends, read from the vault.
#[derive(Clone, Default)]
pub struct SearchSecrets {
    pub brave_search_api_key: Option<String>,
    pub venice_api_key:       Option<String>,
}

impl std::fmt::Debug for SearchSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchSecrets")
            .field(
                "brave_search_configured",
                &self.brave_search_api_key.is_some(),
            )
            .field("venice_configured", &self.venice_api_key.is_some())
            .finish()
    }
}

/// One of the search services a workflow agent can call.
#[derive(Clone, Debug)]
pub enum SearchBackend {
    Brave {
        api_key:    String,
        search_url: String,
    },
    Venice {
        api_key:    String,
        search_url: String,
    },
}

impl SearchBackend {
    #[must_use]
    pub fn from_secrets(secrets: &SearchSecrets) -> Option<Self> {
        match (
            secrets.brave_search_api_key.as_ref(),
            secrets.venice_api_key.as_ref(),
        ) {
            (Some(api_key), _) => Some(Self::brave(api_key.clone())),
            (None, Some(api_key)) => Some(Self::venice(api_key.clone())),
            (None, None) => None,
        }
    }

    #[must_use]
    pub fn brave(api_key: String) -> Self {
        Self::Brave {
            api_key,
            search_url: BRAVE_SEARCH_URL.to_string(),
        }
    }

    #[must_use]
    pub fn venice(api_key: String) -> Self {
        Self::Venice {
            api_key,
            search_url: VENICE_SEARCH_URL.to_string(),
        }
    }

    #[cfg(test)]
    fn with_search_url(mut self, url: &str) -> Self {
        match &mut self {
            Self::Brave { search_url, .. } | Self::Venice { search_url, .. } => {
                *search_url = url.to_string();
            }
        }
        self
    }
}

#[async_trait]
impl SearchProvider for SearchBackend {
    async fn search(&self, request: SearchRequest) -> Result<Vec<SearchResult>, SearchError> {
        let max_results = request.max_results.clamp(1, MAX_RESULTS);
        match self {
            Self::Brave {
                api_key,
                search_url,
            } => search_brave(api_key, search_url, &request.query, max_results).await,
            Self::Venice {
                api_key,
                search_url,
            } => {
                if request.query.chars().count() > VENICE_QUERY_MAX_CHARS {
                    return Err(SearchError::new(
                        SearchErrorKind::InvalidRequest,
                        format!(
                            "query exceeds Venice Search maximum of {VENICE_QUERY_MAX_CHARS} \
                             characters"
                        ),
                    ));
                }
                search_venice(api_key, search_url, &request.query, max_results).await
            }
        }
    }
}

fn search_http_client() -> fabro_http::HttpClient {
    static CLIENT: OnceLock<fabro_http::HttpClient> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            #[cfg(test)]
            {
                fabro_http::test_http_client().expect("Search HTTP client should build")
            }
            #[cfg(not(test))]
            {
                fabro_http::http_client().expect("Search HTTP client should build")
            }
        })
        .clone()
}

fn request_failed(error: impl std::fmt::Display) -> SearchError {
    SearchError::new(
        SearchErrorKind::Execution,
        format!("HTTP request failed: {error}"),
    )
}

fn parse_failed(error: impl std::fmt::Display) -> SearchError {
    SearchError::new(
        SearchErrorKind::Execution,
        format!("Failed to parse response: {error}"),
    )
}

async fn search_brave(
    api_key: &str,
    search_url: &str,
    query: &str,
    max_results: u32,
) -> Result<Vec<SearchResult>, SearchError> {
    let resp = search_http_client()
        .get(search_url)
        .header("X-Subscription-Token", api_key)
        .header("Accept", "application/json")
        .query(&[("q", query), ("count", &max_results.to_string())])
        .send()
        .await
        .map_err(request_failed)?;

    if !resp.status().is_success() {
        return Err(SearchError::new(
            SearchErrorKind::Execution,
            format!("Brave Search API returned status {}", resp.status()),
        ));
    }

    let body: serde_json::Value = resp.json().await.map_err(parse_failed)?;
    Ok(brave_results(&body))
}

async fn search_venice(
    api_key: &str,
    search_url: &str,
    query: &str,
    max_results: u32,
) -> Result<Vec<SearchResult>, SearchError> {
    let resp = search_http_client()
        .post(search_url)
        .timeout(VENICE_REQUEST_TIMEOUT)
        .bearer_auth(api_key)
        .header("Accept", "application/json")
        .json(&serde_json::json!({
            "query": query,
            "limit": max_results,
            "search_provider": "brave",
        }))
        .send()
        .await
        .map_err(request_failed)?;

    let status = resp.status();
    if !status.is_success() {
        return Err(SearchError::new(
            SearchErrorKind::Execution,
            venice_status_error(status.as_u16(), &resp),
        ));
    }

    let body: serde_json::Value = resp.json().await.map_err(parse_failed)?;
    Ok(venice_results(&body))
}

fn venice_status_error(status: u16, resp: &fabro_http::Response) -> String {
    let mut message = format!("Venice Search API returned status {status}");
    if status == 402 {
        if let Some(balance) = header_str(resp, "x-venice-balance-usd") {
            let _ = write!(message, " (balance USD {balance})");
        } else if let Some(balance) = header_str(resp, "x-venice-balance-diem") {
            let _ = write!(message, " (balance DIEM {balance})");
        }
    }
    message
}

fn header_str(resp: &fabro_http::Response, name: &str) -> Option<String> {
    resp.headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn brave_results(body: &serde_json::Value) -> Vec<SearchResult> {
    body.get("web")
        .and_then(|web| web.get("results"))
        .and_then(serde_json::Value::as_array)
        .map(|results| {
            results
                .iter()
                .map(|result| {
                    SearchResult::new(
                        json_str(result, "title"),
                        json_str(result, "url"),
                        json_str(result, "description"),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn venice_results(body: &serde_json::Value) -> Vec<SearchResult> {
    body.get("results")
        .and_then(serde_json::Value::as_array)
        .map(|results| {
            results
                .iter()
                .map(|result| {
                    let hit = SearchResult::new(
                        json_str(result, "title"),
                        json_str(result, "url"),
                        json_str(result, "content"),
                    );
                    match optional_json_str(result, "date") {
                        Some(date) => hit.with_published_at(date),
                        None => hit,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn json_str(value: &serde_json::Value, key: &str) -> String {
    optional_json_str(value, key).unwrap_or_else(|| match key {
        "title" => "(no title)".to_string(),
        "url" => "(no url)".to_string(),
        _ => String::new(),
    })
}

fn optional_json_str(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use httpmock::Method::{GET, POST};
    use httpmock::MockServer;

    use super::*;

    fn secrets(brave: Option<&str>, venice: Option<&str>) -> SearchSecrets {
        SearchSecrets {
            brave_search_api_key: brave.map(str::to_string),
            venice_api_key:       venice.map(str::to_string),
        }
    }

    #[test]
    fn brave_is_preferred_and_venice_is_the_fallback() {
        assert!(matches!(
            SearchBackend::from_secrets(&secrets(Some("b"), Some("v"))),
            Some(SearchBackend::Brave { .. })
        ));
        assert!(matches!(
            SearchBackend::from_secrets(&secrets(None, Some("v"))),
            Some(SearchBackend::Venice { .. })
        ));
        assert!(SearchBackend::from_secrets(&secrets(None, None)).is_none());
    }

    #[tokio::test]
    async fn brave_results_are_returned_in_order() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/search")
                    .header("X-Subscription-Token", "brave-key")
                    .query_param("q", "fabro")
                    .query_param("count", "2");
                then.status(200).json_body(serde_json::json!({
                    "web": {"results": [
                        {"title": "One", "url": "https://one", "description": "first"},
                        {"title": "Two", "url": "https://two", "description": "second"}
                    ]}
                }));
            })
            .await;
        let backend = SearchBackend::brave("brave-key".to_string())
            .with_search_url(&format!("{}/search", server.base_url()));

        let results = backend
            .search(SearchRequest::new("fabro", 2))
            .await
            .unwrap();

        mock.assert_async().await;
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "One");
        assert_eq!(results[1].snippet, "second");
    }

    #[tokio::test]
    async fn venice_results_carry_dates_and_reject_long_queries() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/augment")
                    .header("Authorization", "Bearer venice-key");
                then.status(200).json_body(serde_json::json!({
                    "results": [
                        {"title": "One", "url": "https://one", "content": "first", "date": "2026-01-01"}
                    ]
                }));
            })
            .await;
        let backend = SearchBackend::venice("venice-key".to_string())
            .with_search_url(&format!("{}/augment", server.base_url()));

        let results = backend
            .search(SearchRequest::new("fabro", 5))
            .await
            .unwrap();
        mock.assert_async().await;
        assert_eq!(results[0].published_at.as_deref(), Some("2026-01-01"));

        let error = backend
            .search(SearchRequest::new(
                "x".repeat(VENICE_QUERY_MAX_CHARS + 1),
                5,
            ))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), SearchErrorKind::InvalidRequest);
    }
}
