use super::*;

#[tokio::test]
async fn router_redirects_web_page_requests_to_canonical_host() {
    let app = canonical_host_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/login")
                .header(header::HOST, "localhost:32276")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let response = checked_response!(response, StatusCode::PERMANENT_REDIRECT).await;
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        "http://127.0.0.1:32276/login"
    );
}

#[tokio::test]
async fn router_does_not_redirect_api_requests_to_canonical_host() {
    let app = canonical_host_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(api("/openapi.json"))
                .header(header::HOST, "localhost:32276")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_status!(response, StatusCode::OK).await;
}

#[test]
fn replace_settings_rejects_invalid_canonical_origin_and_keeps_previous_settings() {
    for invalid in [
        "",
        "/relative/path",
        "ftp://fabro.example.com",
        "http://0.0.0.0:32276",
    ] {
        // No FABRO_WEB_URL override: web.url is plain config now, so the
        // invalid value is rejected from the settings literal and the kept
        // previous settings stay valid.
        let state = test_app_state_with_env_lookup(
            canonical_origin_settings("http://valid.example.com"),
            RunLayer::default(),
            5,
            |_| None,
        );

        let err = state
            .replace_runtime_settings(resolved_runtime_settings_from_toml(&format!(
                r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[server.web]
url = "{invalid}"
"#,
            )))
            .expect_err("invalid canonical origin should be rejected");
        assert!(
            err.to_string()
                .contains("server.web.url is required and must be an absolute http(s) URL"),
            "unexpected error for {invalid}: {err}"
        );
        assert_eq!(
            state.canonical_origin().unwrap(),
            "http://valid.example.com".to_string()
        );
    }
}

#[test]
fn canonical_origin_prefers_fabro_web_url_env_override() {
    // FABRO_WEB_URL is the native control-plane override; it wins over the
    // plain `server.web.url` settings literal.
    let state = test_app_state_with_env_lookup(
        canonical_origin_settings("http://settings.example.com"),
        RunLayer::default(),
        5,
        |name| (name == "FABRO_WEB_URL").then(|| "http://env.example.com".to_string()),
    );

    assert_eq!(state.canonical_origin().unwrap(), "http://env.example.com");
}

#[test]
fn canonical_origin_uses_settings_literal_without_env_override() {
    // Without FABRO_WEB_URL set, the plain `server.web.url` literal is used.
    let state = test_app_state_with_env_lookup(
        canonical_origin_settings("http://settings.example.com"),
        RunLayer::default(),
        5,
        |_| None,
    );

    assert_eq!(
        state.canonical_origin().unwrap(),
        "http://settings.example.com"
    );
}

#[test]
fn replace_settings_updates_layer_and_typed_server_settings() {
    let state = test_app_state_with_options(
        server_settings_from_toml(
            r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[server.web]
url = "http://old.example.com"

[server.storage]
root = "/srv/old"
"#,
        ),
        manifest_run_defaults_from_toml(
            r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[server.web]
url = "http://old.example.com"

[server.storage]
root = "/srv/old"
"#,
        ),
        5,
    );

    let updated = r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[server.web]
url = "http://new.example.com"

[run.execution]
mode = "dry_run"

[server.storage]
root = "/srv/new"
"#;

    state
        .replace_runtime_settings(resolved_runtime_settings_from_toml(updated))
        .expect("valid settings should replace current state");

    assert_eq!(state.canonical_origin().unwrap(), "http://new.example.com");
    assert_eq!(state.server_settings().server.storage.root, "/srv/new");
    assert_eq!(
        state
            .manifest_run_settings()
            .expect("manifest run settings should resolve")
            .execution
            .mode,
        RunMode::DryRun
    );
    let manifest_run_defaults = state.manifest_run_defaults();
    assert_eq!(
        manifest_run_defaults
            .execution
            .as_ref()
            .and_then(|execution| execution.mode),
        Some(RunMode::DryRun)
    );
}

#[test]
fn replace_settings_caches_invalid_manifest_run_settings_tolerantly() {
    let state = test_app_state_with_options(
        server_settings_from_toml(
            r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[server.web]
url = "http://old.example.com"
"#,
        ),
        manifest_run_defaults_from_toml(
            r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[server.web]
url = "http://old.example.com"
"#,
        ),
        5,
    );

    let updated = r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[server.web]
url = "http://new.example.com"

[run.environment]
id = "missing"
"#;

    state
        .replace_runtime_settings(resolved_runtime_settings_from_toml(updated))
        .expect("invalid run defaults should not block replace");

    assert_eq!(state.canonical_origin().unwrap(), "http://new.example.com");
    assert!(
        state.manifest_run_settings().is_err(),
        "manifest run settings should stay tolerant for invalid defaults"
    );
}

#[tokio::test]
async fn demo_list_runs_returns_run_list_items() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(state);
    let req = Request::builder()
        .method("GET")
        .uri(api("/runs"))
        .header("X-Fabro-Demo", "1")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let data = body["data"].as_array().expect("data should be array");
    assert!(!data.is_empty(), "demo should return runs");
    let first = &data[0];
    assert!(first["id"].is_string());
    assert!(first["goal"].is_string());
    assert!(first["repository"].is_object());
    assert!(first["title"].is_string());
    assert!(run_json_status(first).is_object());
    assert!(first["workflow"]["slug"].is_string() || first["workflow"]["slug"].is_null());
    assert!(first["labels"].is_object());
    assert!(first["timestamps"]["created_at"].is_string());
}

#[tokio::test]
async fn demo_get_run_returns_run_summary_shape() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(state);
    let run_id = RunId::with_timestamp(
        "2026-03-06T14:30:00Z"
            .parse()
            .expect("demo timestamp should parse"),
        1,
    );
    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{run_id}")))
        .header("X-Fabro-Demo", "1")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    // Should have Run fields, not RunStatusResponse fields
    assert!(body["id"].is_string(), "should have id field");
    assert!(body["goal"].is_string(), "should have goal field");
    assert!(
        body["workflow"]["slug"].is_string(),
        "should have workflow.slug field"
    );
    assert!(body["lifecycle"]["queue_position"].is_null());
}

#[tokio::test]
async fn demo_get_run_returns_404_for_unknown_run() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(state);
    let req = Request::builder()
        .method("GET")
        .uri(api("/runs/nonexistent-run-id"))
        .header("X-Fabro-Demo", "1")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn demo_workflows_return_list_detail_and_runs() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(state);

    let list_req = Request::builder()
        .method("GET")
        .uri(api("/workflows"))
        .header("X-Fabro-Demo", "1")
        .body(Body::empty())
        .unwrap();
    let list_response = app.clone().oneshot(list_req).await.unwrap();
    let list_body = response_json!(list_response, StatusCode::OK).await;
    let workflows = list_body["data"]
        .as_array()
        .expect("workflow list data should be an array");
    assert!(!workflows.is_empty(), "demo should return workflows");
    let first = &workflows[0];
    assert!(first["name"].is_string());
    assert!(first["slug"].is_string());
    assert!(first["filename"].is_string());
    assert!(first["last_run"].is_object() || first["last_run"].is_null());
    assert!(first["schedule"].is_object() || first["schedule"].is_null());

    let detail_req = Request::builder()
        .method("GET")
        .uri(api("/workflows/implement"))
        .header("X-Fabro-Demo", "1")
        .body(Body::empty())
        .unwrap();
    let detail_response = app.clone().oneshot(detail_req).await.unwrap();
    let detail_body = response_json!(detail_response, StatusCode::OK).await;
    assert_eq!(detail_body["slug"], "implement");
    assert!(detail_body["settings"].is_object());
    assert!(
        detail_body["graph"]
            .as_str()
            .is_some_and(|graph| graph.contains("digraph"))
    );

    let runs_req = Request::builder()
        .method("GET")
        .uri(api("/workflows/implement/runs"))
        .header("X-Fabro-Demo", "1")
        .body(Body::empty())
        .unwrap();
    let runs_response = app.oneshot(runs_req).await.unwrap();
    let runs_body = response_json!(runs_response, StatusCode::OK).await;
    let runs = runs_body["data"]
        .as_array()
        .expect("workflow runs data should be an array");
    assert!(
        runs.iter()
            .all(|run| run["workflow"]["slug"].as_str() == Some("implement")),
        "workflow run list should be scoped to the requested workflow"
    );
}
