use axum::extract::{Path, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::handlers::support::{accepts, authorize_repo_access, is_exact_commit_sha, message};
use crate::server::SharedState;
use crate::state::{PermissionLevel, TokenPermission};

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
    if let Err(response) = authorize_repo_access(
        &headers,
        &state,
        &repo,
        TokenPermission::Contents,
        PermissionLevel::Read,
    ) {
        return *response;
    }

    let Some(repository) = state.find_repository(&owner, &repo) else {
        return message(StatusCode::NOT_FOUND, "Not Found");
    };

    // A selector is either a known ref, or an exact SHA this repository can
    // already serve; an unknown SHA must not resolve to itself.
    let sha = repository.refs.get(&selector).cloned().or_else(|| {
        (is_exact_commit_sha(&selector) && repository.knows_commit(&selector))
            .then(|| selector.clone())
    });
    let Some(sha) = sha else {
        return message(StatusCode::NOT_FOUND, "Not Found");
    };

    (StatusCode::OK, [(CONTENT_TYPE, SHA_MEDIA_TYPE)], sha).into_response()
}

#[cfg(test)]
mod tests {
    use axum::http::header::ACCEPT;

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
