//! Response and authorization helpers shared by the twin's handlers.
//!
//! GitHub answers every rejection with the same `{"message": ...}` envelope, so
//! the mapping from an auth failure to a status lives here once rather than in
//! each endpoint.

use axum::Json;
use axum::http::header::ACCEPT;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::auth::{
    BearerTokenError, InstallationTokenAccessError, authorize_installation_token,
    ensure_repo_permission,
};
use crate::state::{AppState, PermissionLevel, TokenInfo, TokenPermission};

/// Render GitHub's `{"message": ...}` error envelope.
pub fn message(status: StatusCode, body: &'static str) -> Response {
    (status, Json(serde_json::json!({ "message": body }))).into_response()
}

pub fn bearer_token_error_response(error: BearerTokenError) -> Response {
    match error {
        BearerTokenError::Missing => message(StatusCode::UNAUTHORIZED, "Unauthorized"),
        BearerTokenError::Invalid => message(StatusCode::UNAUTHORIZED, "Bad credentials"),
    }
}

pub fn repo_permission_error_response(error: InstallationTokenAccessError) -> Response {
    match error {
        InstallationTokenAccessError::RepoNotAccessible => {
            message(StatusCode::NOT_FOUND, "Not Found")
        }
        InstallationTokenAccessError::PermissionDenied => message(
            StatusCode::FORBIDDEN,
            "Resource not accessible by integration",
        ),
    }
}

/// Authorize an installation token against one repository permission,
/// rendering GitHub's response shape for every rejection.
///
/// The rejection is boxed because an `axum` `Response` is far larger than the
/// token it displaces in the `Ok` path.
pub fn authorize_repo_access(
    headers: &HeaderMap,
    state: &AppState,
    repo: &str,
    permission: TokenPermission,
    level: PermissionLevel,
) -> Result<TokenInfo, Box<Response>> {
    let token = authorize_installation_token(headers, state)
        .map_err(|error| Box::new(bearer_token_error_response(error)))?;
    ensure_repo_permission(&token, repo, permission, level)
        .map_err(|error| Box::new(repo_permission_error_response(error)))?;
    Ok(token)
}

/// Whether the client explicitly listed `media_type` in its `Accept` header.
pub fn accepts(headers: &HeaderMap, media_type: &str) -> bool {
    headers.get_all(ACCEPT).iter().any(|value| {
        value.to_str().is_ok_and(|value| {
            value
                .split(',')
                .any(|candidate| candidate.trim() == media_type)
        })
    })
}

/// Whether `value` is an exact 40-character hex commit SHA.
pub fn is_exact_commit_sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
