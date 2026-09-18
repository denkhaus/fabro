use super::*;

#[expect(
    clippy::print_stderr,
    reason = "skip notice for the cold-sandbox no-renderer-binary case"
)]
#[tokio::test]
async fn get_graph_returns_svg() {
    if !render_graph_subprocess_available() {
        eprintln!(
            "skipping: no `fabro` renderer binary resolvable from this target dir \
             (cold sandbox); build fabro-cli to exercise the real render path"
        );
        return;
    }
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    // Start a run
    let req = Request::builder()
        .method("POST")
        .uri(api("/runs"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&test_intent(&app, MINIMAL_DOT).await).unwrap(),
        ))
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    let body = body_json(response.into_body()).await;
    let run_id = body["id"].as_str().unwrap().parse::<RunId>().unwrap();

    // Request graph SVG
    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{run_id}/graph")))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();

    let response = checked_response!(response, StatusCode::OK).await;

    let content_type = response
        .headers()
        .get("content-type")
        .expect("content-type header should be present")
        .to_str()
        .unwrap();
    assert_eq!(content_type, "image/svg+xml");

    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let svg = String::from_utf8_lossy(&bytes);
    assert!(
        svg.contains("<?xml") || svg.contains("<svg"),
        "expected SVG content, got: {}",
        &svg[..svg.len().min(200)]
    );
}

#[tokio::test]
async fn get_graph_source_returns_dot() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let req = Request::builder()
        .method("POST")
        .uri(api("/runs"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&test_intent(&app, MINIMAL_DOT).await).unwrap(),
        ))
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    let body = body_json(response.into_body()).await;
    let run_id = body["id"].as_str().unwrap().parse::<RunId>().unwrap();

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{run_id}/graph/source")))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let response = checked_response!(response, StatusCode::OK).await;

    let content_type = response
        .headers()
        .get("content-type")
        .expect("content-type header should be present")
        .to_str()
        .unwrap();
    assert_eq!(content_type, "text/vnd.graphviz");

    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let dot = String::from_utf8(bytes.to_vec()).unwrap();
    assert_eq!(dot, MINIMAL_DOT);
}

#[expect(
    clippy::print_stderr,
    reason = "skip notice for the cold-sandbox no-renderer-binary case"
)]
#[tokio::test]
async fn render_graph_from_manifest_returns_svg() {
    if !render_graph_subprocess_available() {
        eprintln!(
            "skipping: no `fabro` renderer binary resolvable from this target dir \
             (cold sandbox); build fabro-cli to exercise the real render path"
        );
        return;
    }
    let app = test_app_with();

    let req = Request::builder()
        .method("POST")
        .uri(api("/graph/render"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "manifest": {
                    "version": 1,
                    "cwd": "/tmp",
                    "target": {
                        "path": "workflow.fabro",
                    },
                    "workflows": {
                        "workflow.fabro": {
                            "source": MINIMAL_DOT,
                            "files": {},
                        },
                    },
                },
                "format": "svg",
            }))
            .unwrap(),
        ))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();

    let response = checked_response!(response, StatusCode::OK).await;
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .expect("content-type header should be present")
            .to_str()
            .unwrap(),
        "image/svg+xml"
    );

    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let svg = String::from_utf8_lossy(&bytes);
    assert!(
        svg.contains("<?xml") || svg.contains("<svg"),
        "expected SVG content, got: {}",
        &svg[..svg.len().min(200)]
    );
}

#[expect(
    clippy::print_stderr,
    reason = "skip notice for the cold-sandbox no-renderer-binary case"
)]
#[tokio::test]
async fn render_graph_from_manifest_accepts_fabro_dotted_attributes() {
    if !render_graph_subprocess_available() {
        eprintln!(
            "skipping: no `fabro` renderer binary resolvable from this target dir \
             (cold sandbox); build fabro-cli to exercise the real render path"
        );
        return;
    }
    let app = test_app_with();
    let dot_source = r#"digraph X {
  start [shape=Mdiamond]
  exit [shape=Msquare]
  a [label="A", acp.command="codex"]
  start -> a -> exit
}"#;

    let req = Request::builder()
        .method("POST")
        .uri(api("/graph/render"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "manifest": {
                    "version": 1,
                    "cwd": "/tmp",
                    "target": {
                        "path": "workflow.fabro",
                    },
                    "workflows": {
                        "workflow.fabro": {
                            "source": dot_source,
                            "files": {},
                        },
                    },
                },
                "format": "svg",
            }))
            .unwrap(),
        ))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();

    let response = checked_response!(response, StatusCode::OK).await;
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .expect("content-type header should be present")
            .to_str()
            .unwrap(),
        "image/svg+xml"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn render_graph_bytes_returns_bad_request_for_render_error_protocol() {
    let (_dir, script_path) = write_test_executable(
        "#!/bin/sh\ncat >/dev/null\nprintf 'RENDER_ERROR:failed to parse DOT source'\nexit 0\n",
    );

    let response =
        render_graph_bytes_with_exe_override("not valid dot {{{", Some(&script_path)).await;

    assert_status!(response, StatusCode::BAD_REQUEST).await;
}

#[cfg(unix)]
#[tokio::test]
async fn render_dot_subprocess_returns_child_crashed_for_nonzero_exit() {
    let (_dir, script_path) = write_test_executable("#!/bin/sh\nexit 1\n");

    let result = render_dot_subprocess("digraph { a -> b }", Some(&script_path)).await;

    assert!(matches!(
        result,
        Err(RenderSubprocessError::ChildCrashed(_))
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn render_graph_bytes_returns_internal_server_error_for_child_crash() {
    let (_dir, script_path) = write_test_executable("#!/bin/sh\nexit 1\n");

    let response = render_graph_with_override("digraph { a -> b }", &script_path).await;

    assert_status!(response, StatusCode::INTERNAL_SERVER_ERROR).await;
}

#[cfg(unix)]
#[tokio::test]
async fn render_dot_subprocess_returns_protocol_violation_for_garbage_stdout() {
    let (_dir, script_path) =
        write_test_executable("#!/bin/sh\ncat >/dev/null\nprintf 'garbage'\nexit 0\n");

    let result = render_dot_subprocess("digraph { a -> b }", Some(&script_path)).await;

    assert!(matches!(
        result,
        Err(RenderSubprocessError::ProtocolViolation(_))
    ));
}

#[tokio::test]
async fn get_graph_not_found() {
    let app = test_app_with();
    let missing_run_id = fixtures::RUN_64;

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{missing_run_id}/graph")))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::NOT_FOUND).await;
}
