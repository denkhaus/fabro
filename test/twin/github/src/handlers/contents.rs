use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::handlers::support::{accepts, authorize_repo_access, is_exact_commit_sha, message};
use crate::server::SharedState;
use crate::state::{PermissionLevel, TokenPermission};

const RAW_CONTENT_MEDIA_TYPE: &str = "application/vnd.github.raw+json";

#[derive(Deserialize)]
pub struct ContentQuery {
    #[serde(rename = "ref")]
    revision: String,
}

/// GET /repos/{owner}/{repo}/contents/{path}?ref={sha}
pub async fn get_content(
    State(state): State<SharedState>,
    Path((owner, repo, path)): Path<(String, String, String)>,
    Query(query): Query<ContentQuery>,
    headers: HeaderMap,
) -> Response {
    let state = state.read().await;
    if !accepts(&headers, RAW_CONTENT_MEDIA_TYPE) {
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
    if !is_exact_commit_sha(&query.revision) {
        return message(StatusCode::NOT_FOUND, "Not Found");
    }

    let Some(repository) = state.find_repository(&owner, &repo) else {
        return message(StatusCode::NOT_FOUND, "Not Found");
    };

    if let Some(contents) = repository.file_at(&query.revision, &path) {
        return (
            StatusCode::OK,
            [(CONTENT_TYPE, RAW_CONTENT_MEDIA_TYPE)],
            Body::from(contents.clone()),
        )
            .into_response();
    }

    // GitHub answers a directory path with a JSON listing rather than raw
    // bytes, which is what lets the reader reject non-file targets.
    if repository.has_directory_at(&query.revision, &format!("{path}/")) {
        return (StatusCode::OK, Json(serde_json::json!([]))).into_response();
    }

    message(StatusCode::NOT_FOUND, "Not Found")
}

#[cfg(test)]
mod tests {
    use axum::http::header::ACCEPT;

    use super::*;
    use crate::server::TestServer;
    use crate::state::AppState;
    use crate::test_support::test_http_client;

    const SHA: &str = "1111111111111111111111111111111111111111";

    fn state_with_file() -> (AppState, String) {
        let mut state = AppState::new();
        state.add_repository("owner", "repo", vec!["main".to_string()], true);
        state.set_repository_ref("owner", "repo", "heads/main", SHA);
        state.add_repository_file("owner", "repo", SHA, "dir/file.bin", vec![0, 0xff, 1]);
        let token = state.generate_access_token(
            "app",
            1,
            vec!["repo".to_string()],
            serde_json::json!({ "contents": "read" }),
        );
        (state, token)
    }

    #[tokio::test]
    async fn serves_only_exact_sha_files_and_marks_directories_as_json() {
        let (state, token) = state_with_file();
        let server = TestServer::start(state).await;
        let client = test_http_client();

        let response = client
            .get(format!(
                "{}/repos/owner/repo/contents/dir/file.bin?ref={SHA}",
                server.url()
            ))
            .bearer_auth(&token)
            .header(ACCEPT, RAW_CONTENT_MEDIA_TYPE)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(CONTENT_TYPE).unwrap(),
            RAW_CONTENT_MEDIA_TYPE
        );
        assert_eq!(response.bytes().await.unwrap().as_ref(), &[0, 0xff, 1]);

        let directory = client
            .get(format!(
                "{}/repos/owner/repo/contents/dir?ref={SHA}",
                server.url()
            ))
            .bearer_auth(&token)
            .header(ACCEPT, RAW_CONTENT_MEDIA_TYPE)
            .send()
            .await
            .unwrap();
        assert_eq!(directory.status(), StatusCode::OK);
        assert_eq!(
            directory.headers().get(CONTENT_TYPE).unwrap(),
            "application/json"
        );

        for suffix in [
            "dir/file.bin?ref=heads%2Fmain",
            "dir/file.bin?ref=2222222222222222222222222222222222222222",
            &format!("missing.bin?ref={SHA}"),
        ] {
            let response = client
                .get(format!(
                    "{}/repos/owner/repo/contents/{suffix}",
                    server.url()
                ))
                .bearer_auth(&token)
                .header(ACCEPT, RAW_CONTENT_MEDIA_TYPE)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }

        let missing_ref = client
            .get(format!(
                "{}/repos/owner/repo/contents/dir/file.bin",
                server.url()
            ))
            .bearer_auth(&token)
            .header(ACCEPT, RAW_CONTENT_MEDIA_TYPE)
            .send()
            .await
            .unwrap();
        assert_eq!(missing_ref.status(), StatusCode::BAD_REQUEST);

        server.shutdown().await;
    }

    #[tokio::test]
    async fn enforces_auth_repository_permission_and_media_type() {
        let (mut state, token) = state_with_file();
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
        let url = format!(
            "{}/repos/owner/repo/contents/dir/file.bin?ref={SHA}",
            server.url()
        );

        let missing = client
            .get(&url)
            .header(ACCEPT, RAW_CONTENT_MEDIA_TYPE)
            .send()
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

        let invalid = client
            .get(&url)
            .bearer_auth("invalid")
            .header(ACCEPT, RAW_CONTENT_MEDIA_TYPE)
            .send()
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::UNAUTHORIZED);

        let hidden = client
            .get(&url)
            .bearer_auth(other_repo_token)
            .header(ACCEPT, RAW_CONTENT_MEDIA_TYPE)
            .send()
            .await
            .unwrap();
        assert_eq!(hidden.status(), StatusCode::NOT_FOUND);

        let denied = client
            .get(&url)
            .bearer_auth(denied_token)
            .header(ACCEPT, RAW_CONTENT_MEDIA_TYPE)
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);

        let unacceptable = client.get(&url).bearer_auth(token).send().await.unwrap();
        assert_eq!(unacceptable.status(), StatusCode::NOT_ACCEPTABLE);

        server.shutdown().await;
    }
}
