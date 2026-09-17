use super::*;

#[tokio::test]
async fn stage_artifacts_round_trip() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let run_id = create_run(&app, MINIMAL_DOT).await;
    let stage_id = "code@2";

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!(
            "/runs/{run_id}/stages/{stage_id}/artifacts?filename=src/lib.rs&retry=1"
        )))
        .header("content-type", "application/octet-stream")
        .body(Body::from("fn main() {}"))
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::NO_CONTENT).await;

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{run_id}/stages/{stage_id}/artifacts")))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(body["data"][0]["filename"], "src/lib.rs");
    assert_eq!(body["data"][0]["retry"], 1);
    assert_eq!(body["data"][0]["size"], 12);

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!(
            "/runs/{run_id}/stages/{stage_id}/artifacts/download?filename=src/lib.rs"
        )))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::BAD_REQUEST).await;

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!(
            "/runs/{run_id}/stages/{stage_id}/artifacts/download?filename=src/lib.rs&retry=1"
        )))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    let bytes = response_bytes!(response, StatusCode::OK).await;
    assert_eq!(&bytes[..], b"fn main() {}");
}

#[tokio::test]
async fn stage_artifacts_keep_same_filename_per_retry() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let run_id = create_run(&app, MINIMAL_DOT).await;
    let stage_id = "code@2";

    for (retry, body) in [(1, "first"), (2, "second")] {
        let req = Request::builder()
            .method("POST")
            .uri(api(&format!(
                "/runs/{run_id}/stages/{stage_id}/artifacts?filename=logs/output.txt&retry={retry}"
            )))
            .header("content-type", "application/octet-stream")
            .body(Body::from(body))
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_status!(response, StatusCode::NO_CONTENT).await;
    }

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{run_id}/stages/{stage_id}/artifacts")))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(body["data"][0]["filename"], "logs/output.txt");
    assert_eq!(body["data"][0]["retry"], 1);
    assert_eq!(body["data"][1]["filename"], "logs/output.txt");
    assert_eq!(body["data"][1]["retry"], 2);

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!(
            "/runs/{run_id}/stages/{stage_id}/artifacts/download?filename=logs/output.txt&retry=2"
        )))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    let bytes = response_bytes!(response, StatusCode::OK).await;
    assert_eq!(&bytes[..], b"second");
}

#[tokio::test]
async fn run_artifacts_download_streams_latest_files_as_zip() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = create_run(&app, MINIMAL_DOT)
        .await
        .parse::<RunId>()
        .unwrap();

    let run_store = state.stores.runs.open_run(&run_id).await.unwrap();
    for event in [
        workflow_event::Event::RunRunnable {
            source: fabro_types::RunRunnableSource::StartRequested,
            actor:  None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
    ] {
        workflow_event::append_event(&run_store, &run_id, &event)
            .await
            .unwrap();
    }
    append_scoped_stage_event(
        &state,
        run_id,
        "build",
        1,
        &stage_started_event("build", "command"),
    )
    .await;
    append_scoped_stage_event(
        &state,
        run_id,
        "verify",
        1,
        &stage_started_event("verify", "command"),
    )
    .await;

    for (stage_id, retry, path, contents) in [
        (
            StageId::new("unknown", 1),
            99,
            "reports/result.txt",
            &b"unknown stage"[..],
        ),
        (
            StageId::new("build", 1),
            1,
            "reports/result.txt",
            &b"build result"[..],
        ),
        (
            StageId::new("verify", 1),
            1,
            "reports/result.txt",
            &b"first verify"[..],
        ),
        (
            StageId::new("verify", 1),
            2,
            "reports/result.txt",
            &b"latest verify"[..],
        ),
        (
            StageId::new("build", 1),
            1,
            "logs/run.txt",
            &b"build log"[..],
        ),
        (
            StageId::new("start", 1),
            1,
            "control-start.txt",
            &b"excluded"[..],
        ),
        (
            StageId::new("exit", 1),
            1,
            "control-exit.txt",
            &b"excluded"[..],
        ),
        // Neither stage reached the projection, so both rank equally on stage
        // order and retry. The serialized stage ID breaks the tie the same way
        // the artifacts page does: "unknown@2" sorts above "unknown@10".
        (
            StageId::new("unknown", 10),
            1,
            "orphan.txt",
            &b"visit ten"[..],
        ),
        (
            StageId::new("unknown", 2),
            1,
            "orphan.txt",
            &b"visit two"[..],
        ),
    ] {
        state
            .artifact_store
            .put(&run_id, &ArtifactKey::new(stage_id, retry, path), contents)
            .await
            .unwrap();
    }

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{run_id}/artifacts/download")))
                .header(header::ACCEPT_ENCODING, "gzip")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/zip")
    );
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_DISPOSITION)
            .and_then(|value| value.to_str().ok()),
        Some(format!("attachment; filename=\"fabro-artifacts-{run_id}.zip\"").as_str())
    );
    assert_eq!(
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("private, no-store")
    );
    assert!(response.headers().get(header::CONTENT_ENCODING).is_none());

    let bytes = response_bytes!(response, StatusCode::OK).await;
    let archive = ZipFileReader::new(bytes).await.unwrap();
    let names = archive
        .file()
        .entries()
        .iter()
        .map(|entry| entry.filename().as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(names, vec![
        "logs/run.txt",
        "orphan.txt",
        "reports/result.txt"
    ]);

    let mut contents_by_name = HashMap::new();
    for (index, name) in names.into_iter().enumerate() {
        let mut entry = archive.reader_with_entry(index).await.unwrap();
        let mut contents = Vec::new();
        entry.read_to_end_checked(&mut contents).await.unwrap();
        contents_by_name.insert(name, contents);
    }
    assert_eq!(contents_by_name["logs/run.txt"], b"build log");
    assert_eq!(contents_by_name["reports/result.txt"], b"latest verify");
    assert_eq!(contents_by_name["orphan.txt"], b"visit two");
}

#[tokio::test]
async fn run_artifacts_download_returns_not_found_for_unknown_run() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{}/artifacts/download", RunId::new())))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_status!(response, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn stage_artifact_upload_rejects_invalid_filename() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let run_id = create_run(&app, MINIMAL_DOT).await;

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!(
            "/runs/{run_id}/stages/code@2/artifacts?filename=../escape.txt&retry=1"
        )))
        .header("content-type", "application/octet-stream")
        .body(Body::from("nope"))
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::BAD_REQUEST).await;
}

#[tokio::test]
async fn worker_token_controls_stage_artifact_route() {
    let (_state, app) = jwt_auth_app();
    let user_jwt = issue_test_user_jwt();
    let run_id = create_run_with_bearer(&app, &user_jwt).await;
    let worker_token = issue_test_worker_token(&run_id);
    let other_run_id = create_run_with_bearer(&app, &user_jwt).await;
    let mismatched_worker_token = issue_test_worker_token(&other_run_id);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(api(&format!(
                    "/runs/{run_id}/stages/code@2/artifacts?filename=artifact.txt&retry=1"
                )))
                .header(header::AUTHORIZATION, format!("Bearer {worker_token}"))
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .body(Body::from("artifact"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_status!(response, StatusCode::NO_CONTENT).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(api(&format!(
                    "/runs/{run_id}/stages/code@2/artifacts?filename=artifact.txt&retry=1"
                )))
                .header(header::AUTHORIZATION, format!("Bearer {user_jwt}"))
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .body(Body::from("artifact"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_status!(response, StatusCode::NO_CONTENT).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(api(&format!(
                    "/runs/{run_id}/stages/code@2/artifacts?filename=artifact.txt&retry=1"
                )))
                .header(
                    header::AUTHORIZATION,
                    format!("Bearer {mismatched_worker_token}"),
                )
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .body(Body::from("artifact"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(api(&format!(
                    "/runs/{run_id}/stages/code@2/artifacts?filename=artifact.txt&retry=1"
                )))
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .body(Body::from("artifact"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_status!(response, StatusCode::UNAUTHORIZED).await;
}

#[tokio::test]
async fn stage_artifacts_multipart_round_trip() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let run_id = create_run(&app, MINIMAL_DOT).await;
    let stage_id = "code@2";
    let source_bytes = b"fn main() {}\n";
    let log_bytes = b"build ok\n";
    let manifest = serde_json::json!({
        "entries": [
            {
                "part": "file1",
                "path": "src/lib.rs",
                "sha256": hex::encode(Sha256::digest(source_bytes)),
                "expected_bytes": source_bytes.len(),
                "content_type": "text/plain"
            },
            {
                "part": "file2",
                "path": "logs/output.txt",
                "sha256": hex::encode(Sha256::digest(log_bytes)),
                "expected_bytes": log_bytes.len(),
                "content_type": "text/plain"
            }
        ]
    });
    let boundary = "fabro-test-boundary";

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!(
            "/runs/{run_id}/stages/{stage_id}/artifacts?retry=1"
        )))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(multipart_body(boundary, &manifest, &[
            ("file1", "src/lib.rs", source_bytes),
            ("file2", "logs/output.txt", log_bytes),
        ]))
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::NO_CONTENT).await;

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{run_id}/stages/{stage_id}/artifacts")))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(body["data"][0]["filename"], "logs/output.txt");
    assert_eq!(body["data"][0]["retry"], 1);
    assert_eq!(body["data"][0]["size"], log_bytes.len());
    assert_eq!(body["data"][1]["filename"], "src/lib.rs");
    assert_eq!(body["data"][1]["retry"], 1);
    assert_eq!(body["data"][1]["size"], source_bytes.len());

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!(
            "/runs/{run_id}/stages/{stage_id}/artifacts/download?filename=logs/output.txt&retry=1"
        )))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    let bytes = response_bytes!(response, StatusCode::OK).await;
    assert_eq!(&bytes[..], log_bytes);
}

#[tokio::test]
async fn stage_artifacts_multipart_requires_manifest_first() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let run_id = create_run(&app, MINIMAL_DOT).await;
    let boundary = "fabro-test-boundary";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file1\"; filename=\"src/lib.rs\"\r\n\r\nfn main() {{}}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"manifest\"\r\nContent-Type: application/json\r\n\r\n{{\"entries\":[{{\"part\":\"file1\",\"path\":\"src/lib.rs\"}}]}}\r\n--{boundary}--\r\n"
    );

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!(
            "/runs/{run_id}/stages/code@2/artifacts?retry=1"
        )))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::BAD_REQUEST).await;
}
