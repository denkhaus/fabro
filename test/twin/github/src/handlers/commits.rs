use axum::Json;
use axum::extract::{Path, State};
use axum::http::header::{ACCEPT, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::auth::{
    BearerTokenError, InstallationTokenAccessError, authorize_installation_token,
    ensure_repo_permission,
};
use crate::server::SharedState;
use crate::state::{AppState, PermissionLevel, TokenPermission};

const SHA_MEDIA_TYPE: &str = "application/vnd.github.sha";

/// GET /repos/{owner}/{repo}/commits/{ref}
pub async fn get_commit(
    State(state): State<SharedState>,
    Path((owner, repo, selector)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Response {
    let state = state.read().await;
    if !accepts(&headers, SHA_MEDIA_TYPE) {
        return message(StatusCode::NOT_ACCEPTABLE, "Not Acceptable");
    }
    if let Err(error) = authorize(&headers, &state, &repo) {
        return authorization_error(error);
    }

    let Some(repository) = state
        .repositories
        .iter()
        .find(|repository| repository.owner == owner && repository.name == repo)
    else {
        return message(StatusCode::NOT_FOUND, "Not Found");
    };

    let sha = repository.refs.get(&selector).cloned().or_else(|| {
        is_exact_commit_sha(&selector)
            .then(|| selector.clone())
            .filter(|sha| {
                repository.refs.values().any(|known| known == sha)
                    || repository.files.keys().any(|(known, _)| known == sha)
            })
    });
    let Some(sha) = sha else {
        return message(StatusCode::NOT_FOUND, "Not Found");
    };

    (StatusCode::OK, [(CONTENT_TYPE, SHA_MEDIA_TYPE)], sha).into_response()
}

#[derive(Clone, Copy)]
enum AuthorizationError {
    MissingCredentials,
    InvalidCredentials,
    RepoNotAccessible,
    PermissionDenied,
}

fn authorize(headers: &HeaderMap, state: &AppState, repo: &str) -> Result<(), AuthorizationError> {
    let token = authorize_installation_token(headers, state).map_err(|error| match error {
        BearerTokenError::Missing => AuthorizationError::MissingCredentials,
        BearerTokenError::Invalid => AuthorizationError::InvalidCredentials,
    })?;
    ensure_repo_permission(
        &token,
        repo,
        TokenPermission::Contents,
        PermissionLevel::Read,
    )
    .map_err(|error| match error {
        InstallationTokenAccessError::RepoNotAccessible => AuthorizationError::RepoNotAccessible,
        InstallationTokenAccessError::PermissionDenied => AuthorizationError::PermissionDenied,
    })
}

fn authorization_error(error: AuthorizationError) -> Response {
    match error {
        AuthorizationError::MissingCredentials => message(StatusCode::UNAUTHORIZED, "Unauthorized"),
        AuthorizationError::InvalidCredentials => {
            message(StatusCode::UNAUTHORIZED, "Bad credentials")
        }
        AuthorizationError::RepoNotAccessible => message(StatusCode::NOT_FOUND, "Not Found"),
        AuthorizationError::PermissionDenied => message(
            StatusCode::FORBIDDEN,
            "Resource not accessible by integration",
        ),
    }
}

fn accepts(headers: &HeaderMap, media_type: &str) -> bool {
    headers.get_all(ACCEPT).iter().any(|value| {
        value.to_str().is_ok_and(|value| {
            value
                .split(',')
                .any(|candidate| candidate.trim() == media_type)
        })
    })
}

fn is_exact_commit_sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn message(status: StatusCode, body: &'static str) -> Response {
    (status, Json(serde_json::json!({ "message": body }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::TestServer;
    use crate::state::AppState;
    use crate::test_support::test_http_client;

    const HEAD_SHA: &str = "1111111111111111111111111111111111111111";
    const TAG_SHA: &str = "2222222222222222222222222222222222222222";

    fn state_with_repository() -> (AppState, String) {
        let mut state = AppState::new();
        state.add_repository("owner", "repo", vec!["main".to_string()], true);
        state.set_repository_ref("owner", "repo", "heads/main", HEAD_SHA);
        state.set_repository_ref("owner", "repo", "tags/v1.0.0", TAG_SHA);
        let token = state.generate_access_token(
            "app",
            1,
            vec!["repo".to_string()],
            serde_json::json!({ "contents": "read" }),
        );
        (state, token)
    }

    #[tokio::test]
    async fn resolves_branch_tag_and_exact_sha_as_raw_bodies() {
        let (state, token) = state_with_repository();
        let server = TestServer::start(state).await;
        let client = test_http_client();

        for (selector, expected) in [
            ("heads%2Fmain", HEAD_SHA),
            ("tags%2Fv1.0.0", TAG_SHA),
            (HEAD_SHA, HEAD_SHA),
        ] {
            let response = client
                .get(format!(
                    "{}/repos/owner/repo/commits/{selector}",
                    server.url()
                ))
                .bearer_auth(&token)
                .header(ACCEPT, SHA_MEDIA_TYPE)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers().get(CONTENT_TYPE).unwrap(),
                SHA_MEDIA_TYPE
            );
            assert_eq!(response.text().await.unwrap(), expected);
        }

        let response = client
            .get(format!(
                "{}/repos/owner/repo/commits/heads%2Fmissing",
                server.url()
            ))
            .bearer_auth(&token)
            .header(ACCEPT, SHA_MEDIA_TYPE)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        server.shutdown().await;
    }

    #[tokio::test]
    async fn enforces_auth_repository_permission_and_media_type() {
        let (mut state, token) = state_with_repository();
        let other_repo_token = state.generate_access_token(
            "app",
            1,
            vec!["other".to_string()],
            serde_json::json!({ "contents": "read" }),
        );
        let denied_token =
            state.generate_access_token("app", 1, vec!["repo".to_string()], serde_json::json!({}));
        let server = TestServer::start(state).await;
        let client = test_http_client();
        let url = format!("{}/repos/owner/repo/commits/heads%2Fmain", server.url());

        let missing = client
            .get(&url)
            .header(ACCEPT, SHA_MEDIA_TYPE)
            .send()
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

        let invalid = client
            .get(&url)
            .bearer_auth("invalid")
            .header(ACCEPT, SHA_MEDIA_TYPE)
            .send()
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::UNAUTHORIZED);

        let hidden = client
            .get(&url)
            .bearer_auth(other_repo_token)
            .header(ACCEPT, SHA_MEDIA_TYPE)
            .send()
            .await
            .unwrap();
        assert_eq!(hidden.status(), StatusCode::NOT_FOUND);

        let denied = client
            .get(&url)
            .bearer_auth(denied_token)
            .header(ACCEPT, SHA_MEDIA_TYPE)
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);

        let unacceptable = client.get(&url).bearer_auth(token).send().await.unwrap();
        assert_eq!(unacceptable.status(), StatusCode::NOT_ACCEPTABLE);

        server.shutdown().await;
    }
}
