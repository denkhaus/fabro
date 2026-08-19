use fabro_http::header::{ACCEPT, CONTENT_TYPE, RETRY_AFTER, USER_AGENT};
use fabro_http::{HeaderMap, HttpClient, Response, StatusCode};
use fabro_redact::{DisplaySafeUrl, DisplaySafeUrlError};
use fabro_types::{GitHubRepositorySlug, repository};

use crate::GitHubContext;

const SHA_MEDIA_TYPE: &str = "application/vnd.github.sha";
const RAW_CONTENT_MEDIA_TYPE: &str = "application/vnd.github.raw+json";
const MAX_SHA_RESPONSE_BYTES: usize = 128;

/// Which repository read a failure came from. Carried on the errors that can
/// arise from either, so one status classification serves both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, strum::Display)]
#[strum(serialize_all = "lowercase")]
pub enum Operation {
    Revision,
    Content,
}

/// Failures while opening or using a repository-scoped GitHub reader.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RepositoryReadError {
    #[error("invalid GitHub API base URL ({reason})")]
    InvalidApiBaseUrl {
        reason: &'static str,
        #[source]
        source: Option<DisplaySafeUrlError>,
    },
    #[error("invalid GitHub ref selector")]
    InvalidRefSelector,
    #[error("invalid Git commit SHA")]
    InvalidCommitSha,
    #[error("invalid repository path ({reason})")]
    InvalidRepositoryPath { reason: &'static str },
    #[error("failed to resolve GitHub repository credentials")]
    CredentialResolution {
        #[source]
        source: anyhow::Error,
    },
    #[error("GitHub repository request failed")]
    RequestTransport {
        #[source]
        source: anyhow::Error,
    },
    #[error("GitHub authentication was rejected")]
    AuthenticationRejected,
    #[error("GitHub repository permission was denied")]
    PermissionDenied,
    #[error("GitHub rate limit reached (status {status})")]
    RateLimited { status: u16 },
    /// GitHub returned 404. GitHub may also use 404 to hide a private resource
    /// that these credentials cannot access, so this means "not observable"
    /// rather than "does not exist".
    #[error("GitHub {operation} was not observable")]
    NotFound { operation: Operation },
    #[error("GitHub repository content is not a file")]
    ContentNotFile,
    #[error("GitHub {operation} is unavailable (status {status})")]
    Unavailable {
        operation: Operation,
        status:    u16,
    },
    #[error("GitHub {operation} service is unavailable (status {status})")]
    UpstreamUnavailable {
        operation: Operation,
        status:    u16,
    },
    #[error("unexpected GitHub {operation} status {status}")]
    UnexpectedStatus {
        operation: Operation,
        status:    u16,
    },
    #[error("GitHub response exceeded the {max_bytes}-byte limit")]
    BodyTooLarge { max_bytes: usize },
    #[error("GitHub returned a malformed commit SHA")]
    MalformedCommitSha,
    #[error("GitHub repository content is not valid UTF-8")]
    InvalidUtf8 {
        #[source]
        source: std::str::Utf8Error,
    },
}

/// An authenticated read session scoped to one GitHub repository.
///
/// Opening resolves credentials once; every read reuses that token.
pub struct GitHubRepositoryReader {
    client:          HttpClient,
    /// `{api_base}/repos/{owner}/{repo}`, validated and built once at open.
    repository_base: DisplaySafeUrl,
    bearer_token:    String,
}

impl GitHubRepositoryReader {
    /// Opens an authenticated read session for one repository.
    pub async fn open(
        ctx: &GitHubContext<'_>,
        repository: &GitHubRepositorySlug,
    ) -> Result<Self, RepositoryReadError> {
        let client = ctx
            .http_client()
            .map_err(|source| RepositoryReadError::RequestTransport { source })?;
        let api_base = parse_api_base(ctx.base_url)?;
        let bearer_token = ctx
            .creds
            .resolve_bearer_token(
                &client,
                repository.owner(),
                repository.repo(),
                api_base.as_str().trim_end_matches('/'),
                serde_json::json!({ "contents": "read" }),
            )
            .await
            .map_err(|source| RepositoryReadError::CredentialResolution { source })?;

        Ok(Self {
            client,
            repository_base: repository_base(&api_base, repository),
            bearer_token,
        })
    }

    /// Resolves a validated ref selector to an exact lowercase commit SHA.
    pub async fn resolve_commit(&self, selector: &str) -> Result<String, RepositoryReadError> {
        if !repository::is_valid_github_ref_selector(selector) {
            return Err(RepositoryReadError::InvalidRefSelector);
        }

        let response = self
            .send(&self.commit_url(selector), SHA_MEDIA_TYPE)
            .await?;
        classify_status(response.status(), response.headers(), Operation::Revision)?;
        let bytes = collect_bounded(response, MAX_SHA_RESPONSE_BYTES).await?;
        parse_resolved_commit_sha(bytes)
    }

    /// Reads one UTF-8 file at an explicitly supplied exact commit SHA.
    pub async fn read_utf8_file_at(
        &self,
        commit_sha: &str,
        canonical_repo_path: &str,
        max_bytes: usize,
    ) -> Result<String, RepositoryReadError> {
        if !is_exact_commit_sha(commit_sha.as_bytes()) {
            return Err(RepositoryReadError::InvalidCommitSha);
        }
        validate_repository_path(canonical_repo_path)?;

        let url = self.content_url(commit_sha, canonical_repo_path);
        let response = self.send(&url, RAW_CONTENT_MEDIA_TYPE).await?;
        classify_status(response.status(), response.headers(), Operation::Content)?;
        if !has_raw_content_media_type(response.headers()) {
            return Err(RepositoryReadError::ContentNotFile);
        }
        let bytes = collect_bounded(response, max_bytes).await?;
        String::from_utf8(bytes).map_err(|source| RepositoryReadError::InvalidUtf8 {
            source: source.utf8_error(),
        })
    }

    /// `{repository_base}/commits/{selector}`, with the selector's slashes
    /// escaped so a `heads/a/b` ref stays one path segment.
    fn commit_url(&self, selector: &str) -> DisplaySafeUrl {
        let mut url = self.repository_base.clone();
        url.path_segments_mut()
            .expect("repository base is a hierarchical URL")
            .push("commits")
            .push(selector);
        url
    }

    /// `{repository_base}/contents/{path}?ref={commit_sha}`, with each path
    /// component escaped individually so separators survive as separators.
    fn content_url(&self, commit_sha: &str, path: &str) -> DisplaySafeUrl {
        let mut url = self.repository_base.clone();
        {
            let mut segments = url
                .path_segments_mut()
                .expect("repository base is a hierarchical URL");
            segments.push("contents");
            for component in path.split('/') {
                segments.push(component);
            }
        }
        url.query_pairs_mut().append_pair("ref", commit_sha);
        url
    }

    /// Issues one authenticated GET asking for exactly `media_type`.
    async fn send(
        &self,
        url: &DisplaySafeUrl,
        media_type: &str,
    ) -> Result<Response, RepositoryReadError> {
        self.client
            .get(url.raw_string())
            .bearer_auth(&self.bearer_token)
            .header(USER_AGENT, "fabro")
            .header(ACCEPT, media_type)
            .send()
            .await
            .map_err(|source| RepositoryReadError::RequestTransport {
                source: anyhow::Error::new(source.without_url()),
            })
    }
}

fn parse_api_base(base_url: &str) -> Result<DisplaySafeUrl, RepositoryReadError> {
    let mut url = DisplaySafeUrl::parse(base_url).map_err(|source| {
        RepositoryReadError::InvalidApiBaseUrl {
            reason: "parse",
            source: Some(source),
        }
    })?;
    if url.cannot_be_a_base() {
        return Err(invalid_base("cannot be a base"));
    }
    if url.host_str().is_none() {
        return Err(invalid_base("host"));
    }
    if !matches!(url.scheme(), "http" | "https") {
        return Err(invalid_base("scheme"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid_base("credentials"));
    }
    if url.query().is_some() {
        return Err(invalid_base("query"));
    }
    if url.fragment().is_some() {
        return Err(invalid_base("fragment"));
    }

    url.path_segments_mut()
        .map_err(|()| invalid_base("cannot be a base"))?
        .pop_if_empty();
    Ok(url)
}

const fn invalid_base(reason: &'static str) -> RepositoryReadError {
    RepositoryReadError::InvalidApiBaseUrl {
        reason,
        source: None,
    }
}

/// `{api_base}/repos/{owner}/{repo}`, the prefix every read extends.
///
/// Infallible: `parse_api_base` has already rejected cannot-be-a-base URLs.
fn repository_base(api_base: &DisplaySafeUrl, repository: &GitHubRepositorySlug) -> DisplaySafeUrl {
    let mut url = api_base.clone();
    url.path_segments_mut()
        .expect("api base is a hierarchical URL")
        .pop_if_empty()
        .push("repos")
        .push(repository.owner())
        .push(repository.repo());
    url
}

fn is_exact_commit_sha(bytes: &[u8]) -> bool {
    bytes.len() == 40 && bytes.iter().all(u8::is_ascii_hexdigit)
}

fn parse_resolved_commit_sha(mut bytes: Vec<u8>) -> Result<String, RepositoryReadError> {
    if !is_exact_commit_sha(&bytes) {
        return Err(RepositoryReadError::MalformedCommitSha);
    }
    bytes.make_ascii_lowercase();
    String::from_utf8(bytes).map_err(|_| RepositoryReadError::MalformedCommitSha)
}

fn validate_repository_path(path: &str) -> Result<(), RepositoryReadError> {
    if path.is_empty() {
        return Err(invalid_path("empty"));
    }
    if path.len() > 4096 {
        return Err(invalid_path("too long"));
    }
    if path.split('/').count() > 256 {
        return Err(invalid_path("too many components"));
    }
    let bytes = path.as_bytes();
    let windows_drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if path.starts_with(['/', '~']) || windows_drive {
        return Err(invalid_path("absolute"));
    }
    if path.contains('\\') {
        return Err(invalid_path("backslash"));
    }
    if path.chars().any(char::is_control) {
        return Err(invalid_path("control character"));
    }
    if path.ends_with('/') || path.contains("//") {
        return Err(invalid_path("empty component"));
    }
    if path
        .split('/')
        .any(|component| matches!(component, "." | ".."))
    {
        return Err(invalid_path("dot segment"));
    }
    Ok(())
}

const fn invalid_path(reason: &'static str) -> RepositoryReadError {
    RepositoryReadError::InvalidRepositoryPath { reason }
}

fn classify_status(
    status: StatusCode,
    headers: &HeaderMap,
    operation: Operation,
) -> Result<(), RepositoryReadError> {
    if status == StatusCode::OK {
        return Ok(());
    }

    let status_code = status.as_u16();
    Err(match status {
        StatusCode::UNAUTHORIZED => RepositoryReadError::AuthenticationRejected,
        StatusCode::FORBIDDEN if is_rate_limited(headers) => RepositoryReadError::RateLimited {
            status: status_code,
        },
        StatusCode::FORBIDDEN => RepositoryReadError::PermissionDenied,
        StatusCode::TOO_MANY_REQUESTS => RepositoryReadError::RateLimited {
            status: status_code,
        },
        StatusCode::NOT_FOUND => RepositoryReadError::NotFound { operation },
        StatusCode::CONFLICT | StatusCode::UNPROCESSABLE_ENTITY => {
            RepositoryReadError::Unavailable {
                operation,
                status: status_code,
            }
        }
        status if status.is_server_error() => RepositoryReadError::UpstreamUnavailable {
            operation,
            status: status_code,
        },
        _ => RepositoryReadError::UnexpectedStatus {
            operation,
            status: status_code,
        },
    })
}

fn is_rate_limited(headers: &HeaderMap) -> bool {
    headers
        .get("x-ratelimit-remaining")
        .is_some_and(|value| value.as_bytes() == b"0")
        || headers.contains_key(RETRY_AFTER)
}

fn has_raw_content_media_type(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case(RAW_CONTENT_MEDIA_TYPE))
}

fn checked_body_len(
    current: usize,
    chunk: usize,
    max_bytes: usize,
) -> Result<usize, RepositoryReadError> {
    current
        .checked_add(chunk)
        .filter(|length| *length <= max_bytes)
        .ok_or(RepositoryReadError::BodyTooLarge { max_bytes })
}

async fn collect_bounded(
    mut response: Response,
    max_bytes: usize,
) -> Result<Vec<u8>, RepositoryReadError> {
    // A declared length over the cap fails before any body is read; otherwise it
    // sizes the buffer exactly. Chunked responses start empty and grow under the
    // same bound, enforced per chunk below.
    let capacity = match response.content_length() {
        Some(length) => usize::try_from(length)
            .ok()
            .filter(|length| *length <= max_bytes)
            .ok_or(RepositoryReadError::BodyTooLarge { max_bytes })?,
        None => 0,
    };

    let mut bytes = Vec::with_capacity(capacity);
    while let Some(chunk) =
        response
            .chunk()
            .await
            .map_err(|source| RepositoryReadError::RequestTransport {
                source: anyhow::Error::new(source.without_url()),
            })?
    {
        checked_body_len(bytes.len(), chunk.len(), max_bytes)?;
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use super::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    fn repository() -> GitHubRepositorySlug {
        GitHubRepositorySlug::try_new("owner/repo").unwrap()
    }

    #[test]
    fn api_base_preserves_prefix_and_normalizes_trailing_slash() {
        let base = parse_api_base("https://ghe.example/api/v3/").unwrap();
        assert_eq!(base.as_str(), "https://ghe.example/api/v3");
        assert_eq!(
            base.as_str().trim_end_matches('/'),
            "https://ghe.example/api/v3"
        );

        let root = parse_api_base("https://api.github.com/").unwrap();
        assert_eq!(root.as_str(), "https://api.github.com/");
        assert_eq!(
            root.as_str().trim_end_matches('/'),
            "https://api.github.com"
        );
    }

    #[test]
    fn api_base_rejects_unsafe_shapes_without_echoing_input() {
        let cases = [
            ("ftp://example.test/api", "scheme"),
            ("mailto:test@example.test", "cannot be a base"),
            ("file:///api", "host"),
            ("https://user:password@example.test/api", "credentials"),
            ("https://example.test/api?token=sentinel", "query"),
            ("https://example.test/api#sentinel", "fragment"),
        ];
        for (input, reason) in cases {
            let error = parse_api_base(input).unwrap_err();
            assert!(matches!(
                error,
                RepositoryReadError::InvalidApiBaseUrl {
                    reason: actual,
                    source: None,
                } if actual == reason
            ));
            assert!(!format!("{error}").contains(input));
            assert!(!format!("{error:?}").contains(input));
            assert!(error.source().is_none());
        }
    }

    #[test]
    fn syntactically_invalid_api_base_preserves_parse_source() {
        let error = parse_api_base("not an absolute URL").unwrap_err();
        assert!(matches!(error, RepositoryReadError::InvalidApiBaseUrl {
            reason: "parse",
            source: Some(_),
        }));
        assert!(error.source().is_some());
    }

    /// Builds the URL a read would request without issuing it.
    fn reader_at(base_url: &str) -> GitHubRepositoryReader {
        let api_base = parse_api_base(base_url).unwrap();
        GitHubRepositoryReader {
            client:          fabro_http::test_http_client().unwrap(),
            repository_base: repository_base(&api_base, &repository()),
            bearer_token:    "sentinel-token".to_string(),
        }
    }

    #[test]
    fn url_builders_encode_selectors_and_paths_once() {
        let reader = reader_at("https://ghe.example/api/v3");
        let commit = reader.commit_url("heads/fabro/run/123");
        assert_eq!(
            commit.as_str(),
            "https://ghe.example/api/v3/repos/owner/repo/commits/heads%2Ffabro%2Frun%2F123"
        );

        let content = reader.content_url(SHA, "dir/a b/%2F/#?é.toml");
        assert_eq!(
            content.as_str(),
            "https://ghe.example/api/v3/repos/owner/repo/contents/dir/a%20b/%252F/%23%3F%C3%A9.toml?ref=0123456789abcdef0123456789abcdef01234567"
        );
    }

    #[tokio::test]
    async fn method_inputs_are_rejected_before_network_access() {
        let reader = reader_at("http://127.0.0.1:1");

        for invalid in ["", " main", "heads//main", "tags/v1.lock"] {
            assert!(matches!(
                reader.resolve_commit(invalid).await,
                Err(RepositoryReadError::InvalidRefSelector)
            ));
        }
        assert!(matches!(
            reader.read_utf8_file_at("not-a-sha", "../bad", 1).await,
            Err(RepositoryReadError::InvalidCommitSha)
        ));
    }

    #[test]
    fn canonical_ref_selectors_reach_url_construction() {
        let reader = reader_at("https://api.github.com");
        for selector in ["main", "heads/fabro/run/123", "tags/v1.0.0", SHA] {
            assert!(repository::is_valid_github_ref_selector(selector));
            assert!(
                reader
                    .commit_url(selector)
                    .as_str()
                    .starts_with("https://api.github.com/repos/owner/repo/commits/")
            );
        }
    }

    #[test]
    fn repository_path_validation_enforces_boundaries() {
        assert!(validate_repository_path("a b/%/é.toml").is_ok());
        assert!(validate_repository_path(&"a".repeat(4096)).is_ok());
        assert!(validate_repository_path(&vec!["a"; 256].join("/")).is_ok());

        let too_many = vec!["a"; 257].join("/");
        let cases = [
            ("", "empty"),
            ("/a", "absolute"),
            ("~a", "absolute"),
            ("C:/a", "absolute"),
            ("a\\b", "backslash"),
            ("a\nb", "control character"),
            ("a/", "empty component"),
            ("a//b", "empty component"),
            ("a/./b", "dot segment"),
            ("a/../b", "dot segment"),
            (&too_many, "too many components"),
        ];
        for (path, reason) in cases {
            assert!(matches!(
                validate_repository_path(path),
                Err(RepositoryReadError::InvalidRepositoryPath { reason: actual })
                    if actual == reason
            ));
        }
        assert!(matches!(
            validate_repository_path(&"a".repeat(4097)),
            Err(RepositoryReadError::InvalidRepositoryPath { reason: "too long" })
        ));
    }

    #[test]
    fn commit_sha_validation_is_exact() {
        assert!(is_exact_commit_sha(SHA.as_bytes()));
        assert!(is_exact_commit_sha(SHA.to_ascii_uppercase().as_bytes()));
        for invalid in [
            &SHA[..39],
            "0123456789abcdef0123456789abcdef012345678",
            "g123456789abcdef0123456789abcdef01234567",
            " 123456789abcdef0123456789abcdef01234567",
        ] {
            assert!(!is_exact_commit_sha(invalid.as_bytes()));
        }
    }

    #[test]
    fn resolved_commit_sha_is_exact_and_normalized() {
        assert_eq!(
            parse_resolved_commit_sha(SHA.as_bytes().to_vec()).unwrap(),
            SHA
        );
        assert_eq!(
            parse_resolved_commit_sha(SHA.to_ascii_uppercase().into_bytes()).unwrap(),
            SHA
        );
        let mut newline = SHA.as_bytes().to_vec();
        newline.push(b'\n');
        for invalid in [
            SHA.as_bytes()[..39].to_vec(),
            b"0123456789abcdef0123456789abcdef012345678".to_vec(),
            b"g123456789abcdef0123456789abcdef01234567".to_vec(),
            b" 123456789abcdef0123456789abcdef01234567".to_vec(),
            format!("\"{SHA}\"").into_bytes(),
            newline,
            format!("{SHA}suffix").into_bytes(),
        ] {
            assert!(matches!(
                parse_resolved_commit_sha(invalid),
                Err(RepositoryReadError::MalformedCommitSha)
            ));
        }
    }

    #[test]
    fn status_classification_keeps_operations_and_rate_limits_distinct() {
        let empty = HeaderMap::new();
        assert!(matches!(
            classify_status(StatusCode::UNAUTHORIZED, &empty, Operation::Revision),
            Err(RepositoryReadError::AuthenticationRejected)
        ));
        assert!(matches!(
            classify_status(StatusCode::NOT_FOUND, &empty, Operation::Revision),
            Err(RepositoryReadError::NotFound {
                operation: Operation::Revision,
            })
        ));
        assert!(matches!(
            classify_status(StatusCode::NOT_FOUND, &empty, Operation::Content),
            Err(RepositoryReadError::NotFound {
                operation: Operation::Content,
            })
        ));
        assert!(matches!(
            classify_status(StatusCode::FORBIDDEN, &empty, Operation::Content),
            Err(RepositoryReadError::PermissionDenied)
        ));

        let mut remaining = HeaderMap::new();
        remaining.insert("x-ratelimit-remaining", "0".parse().unwrap());
        assert!(matches!(
            classify_status(StatusCode::FORBIDDEN, &remaining, Operation::Content),
            Err(RepositoryReadError::RateLimited { status: 403 })
        ));

        let mut retry_after = HeaderMap::new();
        retry_after.insert(RETRY_AFTER, "60".parse().unwrap());
        assert!(matches!(
            classify_status(StatusCode::FORBIDDEN, &retry_after, Operation::Content),
            Err(RepositoryReadError::RateLimited { status: 403 })
        ));
        assert!(matches!(
            classify_status(StatusCode::TOO_MANY_REQUESTS, &empty, Operation::Content),
            Err(RepositoryReadError::RateLimited { status: 429 })
        ));
        assert!(matches!(
            classify_status(StatusCode::CONFLICT, &empty, Operation::Revision),
            Err(RepositoryReadError::Unavailable {
                operation: Operation::Revision,
                status:    409,
            })
        ));
        assert!(matches!(
            classify_status(StatusCode::UNPROCESSABLE_ENTITY, &empty, Operation::Content),
            Err(RepositoryReadError::Unavailable {
                operation: Operation::Content,
                status:    422,
            })
        ));
        assert!(matches!(
            classify_status(StatusCode::BAD_GATEWAY, &empty, Operation::Content),
            Err(RepositoryReadError::UpstreamUnavailable {
                operation: Operation::Content,
                status:    502,
            })
        ));
        assert!(matches!(
            classify_status(StatusCode::IM_A_TEAPOT, &empty, Operation::Revision),
            Err(RepositoryReadError::UnexpectedStatus {
                operation: Operation::Revision,
                status:    418,
            })
        ));
    }

    #[test]
    fn raw_content_media_type_ignores_case_and_parameters() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            "Application/Vnd.Github.Raw+Json; charset=utf-8"
                .parse()
                .unwrap(),
        );
        assert!(has_raw_content_media_type(&headers));
        headers.insert(CONTENT_TYPE, "application/json".parse().unwrap());
        assert!(!has_raw_content_media_type(&headers));
    }

    #[test]
    fn body_length_check_is_inclusive_and_overflow_safe() {
        assert_eq!(checked_body_len(2, 3, 5).unwrap(), 5);
        assert!(matches!(
            checked_body_len(2, 4, 5),
            Err(RepositoryReadError::BodyTooLarge { max_bytes: 5 })
        ));
        assert!(matches!(
            checked_body_len(usize::MAX, 1, usize::MAX),
            Err(RepositoryReadError::BodyTooLarge {
                max_bytes: usize::MAX,
            })
        ));
    }

    #[test]
    fn invalid_utf8_source_does_not_own_repository_bytes() {
        let sentinel = b"private-file-contents";
        let mut bytes = sentinel.to_vec();
        bytes.push(0xff);
        let source = String::from_utf8(bytes).unwrap_err().utf8_error();
        let error = RepositoryReadError::InvalidUtf8 { source };
        assert!(error.source().is_some());
        let rendered = format!("{error:?}");
        assert!(!rendered.contains("private-file-contents"));
        assert!(!format!("{:?}", error.source()).contains("private-file-contents"));
    }
}
