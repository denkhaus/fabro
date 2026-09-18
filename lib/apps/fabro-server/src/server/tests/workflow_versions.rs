use super::*;

#[tokio::test]
async fn validate_endpoint_returns_workflow_summary_without_preflight_checks() {
    let app = test_app_with();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api("/validate"))
                .header("content-type", "application/json")
                .body(manifest_body(MINIMAL_DOT))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;

    assert_eq!(body["ok"], true);
    assert_eq!(body["workflow"]["name"], "Test");
    assert_eq!(body["workflow"]["nodes"], 2);
    assert_eq!(body["workflow"]["edges"], 1);
    assert!(body.get("checks").is_none());
}

#[tokio::test]
async fn validate_endpoint_returns_template_source_coordinates() {
    let app = test_app_with();
    let dot = r#"digraph ValidatePlan {
        start [shape=Mdiamond, label="Start"]
        exit  [shape=Msquare, label="Exit"]
        test_imported_prompt [label="moo" prompt="@test.md"]
        start -> test_imported_prompt -> exit
    }"#;
    let manifest = serde_json::json!({
        "version": 1,
        "cwd": "/tmp",
        "target": {
            "path": "workflow.fabro",
        },
        "workflows": {
            "workflow.fabro": {
                "source": dot,
                "files": {
                    "test.md": {
                        "content": "{{ inputs.foo }}",
                        "ref": {
                            "type": "file_inline",
                            "original": "test.md",
                            "from": "workflow.fabro",
                        },
                    },
                },
            },
        },
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api("/validate"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&manifest).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let diagnostics = body["workflow"]["diagnostics"].as_array().unwrap();
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic["rule"] == "template_undefined_variable")
        .expect("expected template diagnostic");

    assert_eq!(diagnostic["source_path"], "test.md");
    assert_eq!(diagnostic["line"], 1);
    assert_eq!(diagnostic["column"], 4);
    assert!(
        diagnostic["node_id"]
            .as_str()
            .unwrap()
            .contains("test_imported_prompt")
    );
}

#[test]
fn validate_github_slug_accepts_real_names() {
    assert!(super::validate_github_slug("owner", "anthropic", 39).is_ok());
    assert!(super::validate_github_slug("repo", "claude-code", 100).is_ok());
    assert!(super::validate_github_slug("repo", "repo.name_1", 100).is_ok());
}

#[test]
fn validate_github_slug_rejects_path_traversal_and_separators() {
    for bad in ["", "..", "foo/bar", "foo%2Fbar", "foo\\bar", "foo?x", "a b"] {
        assert!(
            super::validate_github_slug("owner", bad, 39).is_err(),
            "expected rejection for {bad:?}"
        );
    }
}

#[test]
fn validate_github_slug_rejects_overlong() {
    let long = "a".repeat(40);
    assert!(super::validate_github_slug("owner", &long, 39).is_err());
}

#[tokio::test]
async fn workflow_version_registration_requires_user_or_run_tools_capability() {
    let (state, app) = jwt_auth_app();
    let run_id = RunId::new();
    let body = json!({
        "entrypoint": "workflow.fabro",
        "files": {"workflow.fabro": "digraph W {}"},
        "workflow_dependencies": {},
    });
    for (token, expected) in [
        (issue_test_user_jwt(), StatusCode::CREATED),
        (
            issue_test_run_tools_worker_token(&run_id),
            StatusCode::CREATED,
        ),
        (issue_test_worker_token(&run_id), StatusCode::FORBIDDEN),
    ] {
        let response = app
            .clone()
            .oneshot(json_bearer_request(
                Method::POST,
                "/workflow-versions",
                &token,
                &body,
            ))
            .await
            .unwrap();
        fabro_test::expect_axum_status(response, expected, "POST /workflow-versions actor matrix")
            .await;
    }
    for body in [serde_json::to_string(&body).unwrap(), "{".to_string()] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(api("/workflow-versions"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        fabro_test::expect_axum_status(
            response,
            StatusCode::UNAUTHORIZED,
            "anonymous POST /workflow-versions",
        )
        .await;
    }
    let response = app
        .oneshot(bearer_request(
            Method::GET,
            "/runs",
            &issue_test_user_jwt(),
            Body::empty(),
        ))
        .await
        .unwrap();
    let listed =
        fabro_test::expect_axum_json(response, StatusCode::OK, "GET /runs after registration")
            .await;
    assert_eq!(listed["data"], json!([]));
    assert!(
        state
            .stores
            .runs
            .load_run_projection(&run_id)
            .await
            .unwrap()
            .is_none()
    );
}
