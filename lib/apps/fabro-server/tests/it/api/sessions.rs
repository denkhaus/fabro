use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use fabro_static::EnvVars;
use tower::ServiceExt;

use crate::helpers::{
    MINIMAL_DOT, api, minimal_intent_json, response_json, response_status, test_app_state,
};

async fn create_run(app: &axum::Router) -> (String, tempfile::TempDir) {
    let workspace = tempfile::tempdir().expect("run target workspace should be created");
    let request = Request::builder()
        .method("POST")
        .uri(api("/runs"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&minimal_intent_json(app, MINIMAL_DOT, workspace.path()).await)
                .expect("intent should serialize"),
        ))
        .expect("create-run request should build");
    let body = response_json(
        app.clone().oneshot(request).await.unwrap(),
        StatusCode::CREATED,
        "POST /api/v1/runs",
    )
    .await;
    let run_id = body["id"]
        .as_str()
        .expect("create-run response should include an id")
        .to_string();
    (run_id, workspace)
}

async fn create_session(app: &axum::Router, run_id: &str, title: &str) -> serde_json::Value {
    create_session_with_body(app, run_id, serde_json::json!({ "title": title })).await
}

async fn create_session_with_model(
    app: &axum::Router,
    run_id: &str,
    title: &str,
    model: &str,
) -> serde_json::Value {
    create_session_with_body(
        app,
        run_id,
        serde_json::json!({ "title": title, "model": model }),
    )
    .await
}

async fn create_session_with_body(
    app: &axum::Router,
    run_id: &str,
    body: serde_json::Value,
) -> serde_json::Value {
    response_json(
        create_session_response(app, run_id, body).await,
        StatusCode::CREATED,
        format!("POST /api/v1/runs/{run_id}/sessions"),
    )
    .await
}

async fn create_session_response(
    app: &axum::Router,
    run_id: &str,
    body: serde_json::Value,
) -> axum::response::Response {
    let request = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/sessions")))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&body).expect("session request should serialize"),
        ))
        .expect("create-session request should build");
    app.clone().oneshot(request).await.unwrap()
}

#[tokio::test]
async fn run_bound_session_is_created_with_a_session_event_and_resolves_by_flat_id() {
    let app = fabro_server::test_support::build_test_router(test_app_state());
    let (run_id, _run_id_workspace) = create_run(&app).await;

    let created = create_session(&app, &run_id, "Ask Fabro").await;
    let session_id = created["id"]
        .as_str()
        .expect("session response should include an id");
    assert_eq!(created["run_id"], run_id);
    assert_eq!(created["title"], "Ask Fabro");
    assert_session_metadata_only(&created);
    assert!(session_id.parse::<fabro_types::SessionId>().is_ok());

    let get_request = Request::builder()
        .method("GET")
        .uri(api(&format!("/sessions/{session_id}")))
        .body(Body::empty())
        .expect("get-session request should build");
    let fetched = response_json(
        app.clone().oneshot(get_request).await.unwrap(),
        StatusCode::OK,
        format!("GET /api/v1/sessions/{session_id}"),
    )
    .await;
    assert_eq!(fetched["id"], session_id);
    assert_eq!(fetched["run_id"], run_id);
    assert_session_metadata_only(&fetched);
    assert!(
        fetched.get("messages").is_none(),
        "the conversation is held by the server's session record, not the API"
    );
    assert!(fetched["active_turn"].is_null());

    let events_request = Request::builder()
        .method("GET")
        .uri(api(&format!("/sessions/{session_id}/events")))
        .body(Body::empty())
        .expect("session-events request should build");
    let events = response_json(
        app.clone().oneshot(events_request).await.unwrap(),
        StatusCode::OK,
        format!("GET /api/v1/sessions/{session_id}/events"),
    )
    .await;
    let session_events: Vec<_> = events["data"]
        .as_array()
        .expect("events response should include data")
        .iter()
        .filter(|event| event["session_id"] == session_id)
        .collect();
    assert_eq!(session_events.len(), 1);
    assert_eq!(fetched["last_seq"], session_events[0]["seq"]);
    assert_eq!(session_events[0]["event"], "run.session.created");
    assert!(
        session_events[0]["properties"].get("permissions").is_none(),
        "run session creation event should not expose permissions"
    );
}

#[tokio::test]
async fn sessions_are_listed_only_under_their_owning_run() {
    let app = fabro_server::test_support::build_test_router(test_app_state());
    let (first_run_id, _first_run_id_workspace) = create_run(&app).await;
    let (second_run_id, _second_run_id_workspace) = create_run(&app).await;
    let created = create_session(&app, &first_run_id, "First run chat").await;

    let first_request = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{first_run_id}/sessions")))
        .body(Body::empty())
        .expect("list sessions request should build");
    let first = response_json(
        app.clone().oneshot(first_request).await.unwrap(),
        StatusCode::OK,
        format!("GET /api/v1/runs/{first_run_id}/sessions"),
    )
    .await;
    assert_eq!(first["data"].as_array().unwrap().len(), 1);
    assert_eq!(first["data"][0]["id"], created["id"]);
    assert_session_metadata_only(&first["data"][0]);

    let second_request = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{second_run_id}/sessions")))
        .body(Body::empty())
        .expect("list sessions request should build");
    let second = response_json(
        app.clone().oneshot(second_request).await.unwrap(),
        StatusCode::OK,
        format!("GET /api/v1/runs/{second_run_id}/sessions"),
    )
    .await;
    assert!(second["data"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn supplied_session_model_alias_is_canonicalized() {
    let app = fabro_server::test_support::build_test_router(test_app_state());
    let (run_id, _run_id_workspace) = create_run(&app).await;

    let created = create_session_with_model(&app, &run_id, "Ask Fabro", "gpt54").await;
    assert_eq!(created["model"], "gpt-5.4");
    let session_id = created["id"].as_str().expect("session id");

    let events_request = Request::builder()
        .method("GET")
        .uri(api(&format!("/sessions/{session_id}/events")))
        .body(Body::empty())
        .expect("session-events request should build");
    let events = response_json(
        app.clone().oneshot(events_request).await.unwrap(),
        StatusCode::OK,
        format!("GET /api/v1/sessions/{session_id}/events"),
    )
    .await;

    let created_event = events["data"]
        .as_array()
        .expect("events response should include data")
        .iter()
        .find(|event| event["event"] == "run.session.created")
        .expect("session creation event should be recorded");
    assert_eq!(created_event["properties"]["model"], "gpt-5.4");
    assert_eq!(created_event["properties"]["provider"], "openai");
}

#[tokio::test]
async fn provider_qualified_session_model_is_canonicalized() {
    let app = fabro_server::test_support::build_test_router(test_app_state());
    let (run_id, _run_id_workspace) = create_run(&app).await;

    let created =
        create_session_with_model(&app, &run_id, "Ask Fabro", "openai/gpt-5.4-mini").await;

    assert_eq!(created["model"], "gpt-5.4-mini");
    assert_eq!(created["provider"], "openai");
}

#[tokio::test]
async fn unknown_session_models_preserve_passthrough_on_the_selected_provider() {
    let app = fabro_server::test_support::build_test_router(test_app_state());
    let (run_id, _run_id_workspace) = create_run(&app).await;

    for model in ["not-a-real-model", "openai/not-a-real-model"] {
        let created = create_session_with_model(&app, &run_id, "Ask Fabro", model).await;
        assert_eq!(created["model"], "not-a-real-model");
        assert_eq!(created["provider"], "openai");
    }
}

#[tokio::test]
async fn invalid_session_model_refs_are_rejected_at_creation() {
    let app = fabro_server::test_support::build_test_router(test_app_state());
    let (run_id, _run_id_workspace) = create_run(&app).await;

    for model in ["openai", "openai/", "anthropic/gpt-5.4"] {
        let response = create_session_response(
            &app,
            &run_id,
            serde_json::json!({ "title": "Ask Fabro", "model": model }),
        )
        .await;
        response_status(
            response,
            StatusCode::BAD_REQUEST,
            format!("POST /api/v1/runs/{run_id}/sessions with model {model}"),
        )
        .await;
    }
}

#[tokio::test]
async fn ambiguous_session_model_refs_are_rejected_at_creation() {
    let state = fabro_server::test_support::TestAppStateBuilder::new()
        .llm_overlay_toml("[providers.openai]\naliases = [\"gpt54\"]\n")
        .vault_entries([(EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
        .build();
    let app = fabro_server::test_support::build_test_router(state);
    let (run_id, _run_id_workspace) = create_run(&app).await;

    let response = create_session_response(
        &app,
        &run_id,
        serde_json::json!({ "title": "Ask Fabro", "model": "gpt54" }),
    )
    .await;
    response_status(
        response,
        StatusCode::BAD_REQUEST,
        format!("POST /api/v1/runs/{run_id}/sessions with ambiguous model gpt54"),
    )
    .await;
}

#[tokio::test]
async fn session_turn_fails_when_selected_model_provider_becomes_unconfigured() {
    let app = fabro_server::test_support::build_test_router(test_app_state());
    let (run_id, _run_id_workspace) = create_run(&app).await;
    let created = create_session_with_model(&app, &run_id, "Ask Fabro", "gpt54").await;
    let session_id = created["id"]
        .as_str()
        .expect("session response should include an id");
    delete_openai_credential(&app).await;

    let request = Request::builder()
        .method("POST")
        .uri(api(&format!("/sessions/{session_id}/turns")))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"input":"Which provider are you using?"}"#))
        .expect("submit-turn request should build");
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers().contains_key("x-fabro-turn-id"),
        "submit turn should return the generated turn id"
    );
    let events = session_sse_events(response).await;

    let failed = events
        .iter()
        .find(|event| event["event"] == "run.session.turn.failed")
        .expect("selected provider failure should be streamed");
    assert!(
        failed["properties"]["error"]
            .as_str()
            .expect("failure event should include an error")
            .contains("provider 'openai'"),
        "failure should be for the selected model provider: {failed:?}"
    );
    assert_eq!(failed["properties"]["code"], "model_unavailable");
    assert_eq!(failed["properties"]["retryable"], false);
}

#[tokio::test]
async fn session_metadata_patch_route_is_removed() {
    let app = fabro_server::test_support::build_test_router(test_app_state());
    let (run_id, _run_id_workspace) = create_run(&app).await;
    let created = create_session(&app, &run_id, "Ask Fabro").await;
    let session_id = created["id"]
        .as_str()
        .expect("session response should include an id");

    let request = Request::builder()
        .method("PATCH")
        .uri(api(&format!("/sessions/{session_id}")))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"title":"Renamed"}"#))
        .expect("patch-session request should build");
    response_status(
        app.clone().oneshot(request).await.unwrap(),
        StatusCode::NOT_FOUND,
        format!("PATCH /api/v1/sessions/{session_id}"),
    )
    .await;
}

#[tokio::test]
async fn unsupported_derived_turn_read_routes_are_removed() {
    let app = fabro_server::test_support::build_test_router(test_app_state());
    let (run_id, _run_id_workspace) = create_run(&app).await;
    let created = create_session(&app, &run_id, "Ask Fabro").await;
    let session_id = created["id"]
        .as_str()
        .expect("session response should include an id");
    let turn_id = fabro_types::TurnId::new();

    for path in [format!("/sessions/{session_id}/turns/{turn_id}")] {
        let request = Request::builder()
            .method("GET")
            .uri(api(&path))
            .body(Body::empty())
            .expect("removed session read request should build");
        response_status(
            app.clone().oneshot(request).await.unwrap(),
            StatusCode::NOT_FOUND,
            format!("GET /api/v1{path}"),
        )
        .await;
    }
}

#[tokio::test]
async fn session_events_are_listed_by_session_and_paginated_by_session_sequence() {
    let app = fabro_server::test_support::build_test_router(test_app_state());
    let (run_id, _run_id_workspace) = create_run(&app).await;
    let first = create_session(&app, &run_id, "First").await;
    let second = create_session(&app, &run_id, "Second").await;
    let first_id = first["id"].as_str().unwrap();
    let second_id = second["id"].as_str().unwrap();
    let get_first = Request::builder()
        .method("GET")
        .uri(api(&format!("/sessions/{first_id}")))
        .body(Body::empty())
        .expect("get-session request should build");
    let first_detail = response_json(
        app.clone().oneshot(get_first).await.unwrap(),
        StatusCode::OK,
        format!("GET /api/v1/sessions/{first_id}"),
    )
    .await;
    let after_first_created_seq = first_detail["last_seq"].as_u64().unwrap() + 1;
    delete_openai_credential(&app).await;

    let turn_id = fabro_types::TurnId::new();
    let submit = Request::builder()
        .method("POST")
        .uri(api(&format!("/sessions/{first_id}/turns")))
        .header("content-type", "application/json")
        .body(Body::from(format!(
            r#"{{"turn_id":"{turn_id}","input":"Which provider?"}}"#
        )))
        .expect("submit-turn request should build");
    let response = app.clone().oneshot(submit).await.unwrap();
    assert_eq!(
        response.headers().get("x-fabro-turn-id").unwrap(),
        turn_id.to_string().as_str()
    );
    let _ = session_sse_events(response).await;

    let request = Request::builder()
        .method("GET")
        .uri(api(&format!(
            "/sessions/{first_id}/events?since_seq={after_first_created_seq}&limit=1"
        )))
        .body(Body::empty())
        .expect("session events request should build");
    let page = response_json(
        app.clone().oneshot(request).await.unwrap(),
        StatusCode::OK,
        format!("GET /api/v1/sessions/{first_id}/events"),
    )
    .await;

    assert_eq!(page["data"].as_array().unwrap().len(), 1);
    assert_eq!(page["data"][0]["session_id"], first_id);
    assert_ne!(page["data"][0]["session_id"], second_id);
    assert_eq!(page["data"][0]["event"], "run.session.turn.started");
    assert_eq!(
        page["data"][0]["properties"]["turn_id"],
        turn_id.to_string()
    );
    assert_eq!(page["meta"]["has_more"], true);
}

#[tokio::test]
async fn inactive_turn_interrupt_returns_conflict() {
    let app = fabro_server::test_support::build_test_router(test_app_state());
    let (run_id, _run_id_workspace) = create_run(&app).await;
    let created = create_session(&app, &run_id, "Ask Fabro").await;
    let session_id = created["id"]
        .as_str()
        .expect("session response should include an id");
    let turn_id = fabro_types::TurnId::new();

    let request = Request::builder()
        .method("POST")
        .uri(api(&format!(
            "/sessions/{session_id}/turns/{turn_id}/interrupt"
        )))
        .body(Body::empty())
        .expect("interrupt request should build");
    response_status(
        app.clone().oneshot(request).await.unwrap(),
        StatusCode::CONFLICT,
        format!("POST /api/v1/sessions/{session_id}/turns/{turn_id}/interrupt"),
    )
    .await;
}

/// One page of the global session index door (ADR-0046, work#1452), read
/// with an optional query string.
async fn global_sessions_page(app: &axum::Router, query: &str) -> serde_json::Value {
    let request = Request::builder()
        .method("GET")
        .uri(api(&format!("/sessions{query}")))
        .body(Body::empty())
        .expect("global sessions request should build");
    response_json(
        app.clone().oneshot(request).await.unwrap(),
        StatusCode::OK,
        "GET /api/v1/sessions",
    )
    .await
}

/// The global session index door (ADR-0046, work#1452; bounded in SQL by
/// work#1469): sessions from every run share one page, the store hands out
/// at most `page[limit]` rows plus the has-more probe, and the summary
/// shape stays the metadata projection — last activity and capability,
/// never the event payloads.
#[tokio::test]
async fn the_global_sessions_index_lists_across_runs_and_paginates_in_sql() {
    let app = fabro_server::test_support::build_test_router(test_app_state());
    let (first_run, _first_workspace) = create_run(&app).await;
    let (second_run, _second_workspace) = create_run(&app).await;
    let first = create_session(&app, &first_run, "First run chat").await;
    let second = create_session(&app, &first_run, "Second run chat").await;
    let third = create_session(&app, &second_run, "Third run chat").await;
    let mut expected_ids: Vec<String> = [first, second, third]
        .into_iter()
        .map(|created| {
            created["id"]
                .as_str()
                .expect("session response should include an id")
                .to_string()
        })
        .collect();
    expected_ids.sort();

    // The unfiltered door lists every session across both runs.
    let all = global_sessions_page(&app, "").await;
    let rows = all["data"]
        .as_array()
        .expect("sessions response should include data");
    assert_eq!(rows.len(), 3);
    assert_eq!(all["meta"]["has_more"], false);
    let mut ids: Vec<String> = rows
        .iter()
        .map(|row| {
            row["id"]
                .as_str()
                .expect("session row should include an id")
                .to_string()
        })
        .collect();
    ids.sort();
    assert_eq!(ids, expected_ids);
    let anchors: Vec<&str> = rows
        .iter()
        .map(|row| {
            row["run_id"]
                .as_str()
                .expect("session row should include its run anchor")
        })
        .collect();
    assert!(anchors.contains(&first_run.as_str()), "the door spans runs");
    assert!(
        anchors.contains(&second_run.as_str()),
        "the door spans runs"
    );

    // The metadata projection shape: last activity and capability state,
    // not the event log.
    for row in rows {
        assert_eq!(row["status"], "idle");
        assert_eq!(row["capability"], "read_only");
        assert!(row["created_at"].is_string());
        assert!(row["updated_at"].is_string());
        assert!(
            row.get("last_seq").is_none(),
            "the summary carries no event-log position"
        );
        assert!(
            row.get("properties").is_none(),
            "the summary carries no event payloads"
        );
    }

    // The page bound is the store's SQL: two rows with one more behind.
    let page = global_sessions_page(&app, "?page[limit]=2").await;
    let page_rows = page["data"].as_array().unwrap();
    assert_eq!(page_rows.len(), 2);
    assert_eq!(page["meta"]["has_more"], true);

    // The offset continues where the page stopped; together the pages
    // cover the sessions exactly once.
    let tail = global_sessions_page(&app, "?page[limit]=2&page[offset]=2").await;
    let tail_rows = tail["data"].as_array().unwrap();
    assert_eq!(tail_rows.len(), 1);
    assert_eq!(tail["meta"]["has_more"], false);
    let mut covered: Vec<String> = page_rows
        .iter()
        .chain(tail_rows.iter())
        .map(|row| row["id"].as_str().unwrap().to_string())
        .collect();
    covered.sort();
    assert_eq!(covered, expected_ids);

    // Both orderings are served by the same bound.
    let by_creation = global_sessions_page(&app, "?order=created_desc").await;
    assert_eq!(by_creation["data"].as_array().unwrap().len(), 3);
    assert_eq!(by_creation["meta"]["has_more"], false);
}

/// Explicit session deletion (work#1535): DELETE removes the session's
/// events and its stored record — the row leaves the global index, the
/// detail and events doors 404 afterward, and sibling sessions stay
/// intact. The reset door keeps the old session readable; this door does
/// not. An unknown id answers 404.
#[tokio::test]
async fn delete_session_removes_the_row_and_closes_transcript_access() {
    let app = fabro_server::test_support::build_test_router(test_app_state());
    let (run_id, _run_id_workspace) = create_run(&app).await;
    let doomed = create_session(&app, &run_id, "Doomed chat").await;
    let survivor = create_session(&app, &run_id, "Surviving chat").await;
    let doomed_id = doomed["id"].as_str().unwrap().to_string();
    let survivor_id = survivor["id"].as_str().unwrap().to_string();

    let delete = Request::builder()
        .method("DELETE")
        .uri(api(&format!("/sessions/{doomed_id}")))
        .body(Body::empty())
        .expect("delete-session request should build");
    let response = app.clone().oneshot(delete).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(
        bytes.is_empty(),
        "a 204 delete carries no body, got {bytes:?}"
    );

    // The session itself is gone: detail and transcript access 404.
    for (method, path, label) in [
        ("GET", format!("/sessions/{doomed_id}"), "session detail"),
        (
            "GET",
            format!("/sessions/{doomed_id}/events"),
            "session events",
        ),
    ] {
        let request = Request::builder()
            .method(method)
            .uri(api(&path))
            .body(Body::empty())
            .expect("post-delete read request should build");
        response_status(
            app.clone().oneshot(request).await.unwrap(),
            StatusCode::NOT_FOUND,
            format!("post-delete {label} read"),
        )
        .await;
    }

    // The global index no longer lists the deleted session; the sibling
    // stays.
    let index = global_sessions_page(&app, "").await;
    let ids: Vec<&str> = index["data"]
        .as_array()
        .expect("sessions response should include data")
        .iter()
        .map(|row| {
            row["id"]
                .as_str()
                .expect("session row should include an id")
        })
        .collect();
    assert_eq!(ids, [survivor_id.as_str()]);

    // Deleting an unknown (well-formed) id answers 404, and a repeat
    // delete of the deleted session 404s too.
    for target in [fabro_types::SessionId::new().to_string(), doomed_id.clone()] {
        let request = Request::builder()
            .method("DELETE")
            .uri(api(&format!("/sessions/{target}")))
            .body(Body::empty())
            .expect("repeat delete request should build");
        response_status(
            app.clone().oneshot(request).await.unwrap(),
            StatusCode::NOT_FOUND,
            format!("DELETE /api/v1/sessions/{target} after deletion"),
        )
        .await;
    }

    // A malformed session id is a 400, not a 404.
    let request = Request::builder()
        .method("DELETE")
        .uri(api("/sessions/not-a-session-id"))
        .body(Body::empty())
        .expect("malformed delete request should build");
    response_status(
        app.clone().oneshot(request).await.unwrap(),
        StatusCode::BAD_REQUEST,
        "DELETE with a malformed session id",
    )
    .await;

    // The survivor is untouched.
    let get_survivor = Request::builder()
        .method("GET")
        .uri(api(&format!("/sessions/{survivor_id}")))
        .body(Body::empty())
        .expect("survivor request should build");
    let fetched = response_json(
        app.clone().oneshot(get_survivor).await.unwrap(),
        StatusCode::OK,
        format!("GET /api/v1/sessions/{survivor_id}"),
    )
    .await;
    assert_eq!(fetched["id"], survivor_id.as_str());
}

async fn session_sse_events(response: axum::response::Response) -> Vec<serde_json::Value> {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("session event stream body should be readable");
    let body = String::from_utf8(bytes.to_vec()).expect("session event stream should be UTF-8");
    body.lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(|data| serde_json::from_str(data).expect("session event data should be JSON"))
        .collect()
}

async fn delete_openai_credential(app: &axum::Router) {
    let request = Request::builder()
        .method("DELETE")
        .uri(api("/secrets"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "name": EnvVars::OPENAI_API_KEY,
            }))
            .expect("delete-secret request should serialize"),
        ))
        .expect("delete-secret request should build");
    response_status(
        app.clone().oneshot(request).await.unwrap(),
        StatusCode::NO_CONTENT,
        "DELETE /api/v1/secrets",
    )
    .await;
}

fn assert_session_metadata_only(value: &serde_json::Value) {
    let object = value
        .as_object()
        .expect("session response should be a JSON object");
    assert_eq!(value["provider"], "openai");
    for field in [
        "working_dir",
        "permissions",
        "deleted_at",
        "runtime_context",
    ] {
        assert!(
            !object.contains_key(field),
            "session metadata should not expose {field}"
        );
    }
}

/// The server-enforced capability flip (ADR-0046, work#1451): a worker
/// token cannot flip a session; an authenticated user can. The flip
/// persists on the session (its `run.session.capability.changed` event),
/// audits into the run stream (the `session.capability` platform record,
/// observable via run events), and the latest flip wins.
#[tokio::test]
async fn capability_flip_requires_a_user_persists_per_session_and_audits_into_the_run_stream() {
    let state = test_app_state();
    let app = fabro_server::test_support::build_test_router(state.clone());
    let (run_id, _run_id_workspace) = create_run(&app).await;
    let created = create_session(&app, &run_id, "Ask Fabro").await;
    let session_id = created["id"].as_str().unwrap().to_string();
    assert_eq!(
        created["capability"], "read_only",
        "a new session starts in the read-only posture"
    );

    // A worker token (the stage agent's fabro_ask caller) is not the
    // elevated principal the flip requires.
    let worker_token = state.test_issue_worker_token(&run_id.parse().unwrap());
    let worker_flip = Request::builder()
        .method("POST")
        .uri(api(&format!("/sessions/{session_id}/capability")))
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {worker_token}"))
        .body(Body::from(r#"{"capability":"full_access"}"#))
        .expect("worker flip request should build");
    response_status(
        app.clone().oneshot(worker_flip).await.unwrap(),
        StatusCode::UNAUTHORIZED,
        "worker-token capability flip",
    )
    .await;

    // The user flip succeeds and the response carries the new posture.
    let user_flip = Request::builder()
        .method("POST")
        .uri(api(&format!("/sessions/{session_id}/capability")))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"capability":"full_access"}"#))
        .expect("user flip request should build");
    let flipped = response_json(
        app.clone().oneshot(user_flip).await.unwrap(),
        StatusCode::OK,
        "user capability flip",
    )
    .await;
    assert_eq!(flipped["capability"], "full_access");

    // Per-session persistence: the session detail and the session's own
    // event stream both carry the change.
    let get_session = Request::builder()
        .method("GET")
        .uri(api(&format!("/sessions/{session_id}")))
        .body(Body::empty())
        .expect("get-session request should build");
    let detail = response_json(
        app.clone().oneshot(get_session).await.unwrap(),
        StatusCode::OK,
        format!("GET /api/v1/sessions/{session_id}"),
    )
    .await;
    assert_eq!(detail["capability"], "full_access");

    let session_events = Request::builder()
        .method("GET")
        .uri(api(&format!("/sessions/{session_id}/events")))
        .body(Body::empty())
        .expect("session-events request should build");
    let events = response_json(
        app.clone().oneshot(session_events).await.unwrap(),
        StatusCode::OK,
        format!("GET /api/v1/sessions/{session_id}/events"),
    )
    .await;
    let capability_events: Vec<_> = events["data"]
        .as_array()
        .expect("events response should include data")
        .iter()
        .filter(|event| event["event"] == "run.session.capability.changed")
        .collect();
    assert_eq!(capability_events.len(), 1);
    assert_eq!(
        capability_events[0]["properties"]["capability"], "full_access",
        "the session event names the posture it moved to"
    );

    // The run-stream audit: observable via run events.
    let run_events = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{run_id}/events")))
        .body(Body::empty())
        .expect("run-events request should build");
    let stream = response_json(
        app.clone().oneshot(run_events).await.unwrap(),
        StatusCode::OK,
        format!("GET /api/v1/runs/{run_id}/events"),
    )
    .await;
    let capability_records: Vec<_> = stream["data"]
        .as_array()
        .expect("run stream should include data")
        .iter()
        .filter(|item| {
            item["kind"] == "platform" && item["item"]["record"]["kind"] == "session.capability"
        })
        .collect();
    assert_eq!(
        capability_records.len(),
        1,
        "the flip audits into the run stream exactly once"
    );
    assert_eq!(
        capability_records[0]["item"]["record"]["session_id"],
        session_id
    );
    assert_eq!(
        capability_records[0]["item"]["record"]["capability"],
        "full_access"
    );

    // The latest flip wins: flipping back moves the session to read-only.
    let flip_back = Request::builder()
        .method("POST")
        .uri(api(&format!("/sessions/{session_id}/capability")))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"capability":"read_only"}"#))
        .expect("flip-back request should build");
    let flipped_back = response_json(
        app.clone().oneshot(flip_back).await.unwrap(),
        StatusCode::OK,
        "capability flip back to read-only",
    )
    .await;
    assert_eq!(flipped_back["capability"], "read_only");
}

/// ADR-0046 scope 3: at full access the ask session's worker token stops
/// being rejected on cross-run run-scoped routes — the supervisor
/// capability widens what was a run-pinned rejection — while a plain
/// run worker stays scoped to its own run.
#[tokio::test]
async fn supervisor_worker_scope_widens_run_scoped_reads_across_runs() {
    let state = test_app_state();
    let app = fabro_server::test_support::build_test_router(state.clone());
    let (first_run, _first_workspace) = create_run(&app).await;
    let (second_run, _second_workspace) = create_run(&app).await;
    let first: fabro_types::RunId = first_run.parse().unwrap();

    let logs_of_second = |token: String| {
        let app = app.clone();
        let second_run = second_run.clone();
        async move {
            let request = Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{second_run}/logs")))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .expect("run-logs request should build");
            app.oneshot(request).await.unwrap()
        }
    };

    // A plain run worker is pinned to its own run: the foreign run's
    // route refuses it.
    let plain = state.test_issue_worker_token(&first);
    let response = logs_of_second(plain).await;
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "a plain worker stays run-scoped"
    );

    // The supervisor capability (a full-access ask session's backend)
    // passes the run-scoped guard on the foreign run: the route answers
    // for the run itself (here: no log file), not with a scope refusal.
    let supervisor = state.test_issue_worker_token_supervisor(&first);
    let response = logs_of_second(supervisor).await;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "the supervisor scope passes the run-scoped guard cross-run"
    );
}

/// A run-scoped session, created while the test credentials still exist
/// (creation resolves the default model through an eligible provider).
async fn create_titled_session(app: &axum::Router, run_id: &str, title: &str) -> String {
    let created = create_session(app, run_id, title).await;
    created["id"]
        .as_str()
        .expect("session response should include an id")
        .to_string()
}

/// Submit one turn that fails fast (no LLM credential — the caller
/// deletes it first): the turn-start and user-message events are recorded
/// synchronously before the turn even spawns. The stream is drained so
/// the failed turn's events have landed before the doors are exercised.
async fn submit_failing_turn(app: &axum::Router, session_id: &str, text: &str) {
    let submit = Request::builder()
        .method("POST")
        .uri(api(&format!("/sessions/{session_id}/turns")))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({ "input": text }))
                .expect("submit-turn body should serialize"),
        ))
        .expect("submit-turn request should build");
    let response = app.clone().oneshot(submit).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let _ = session_sse_events(response).await;
}

async fn get_json(app: &axum::Router, path: &str, token: Option<&str>) -> axum::response::Response {
    let mut builder = Request::builder().method("GET").uri(api(path));
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let request = builder
        .body(Body::empty())
        .expect("GET request should build");
    app.clone().oneshot(request).await.unwrap()
}

/// The session archaeology search door (work#1679): a keyword finds
/// sessions by run anchor, title, and recorded message text; sessions
/// matching nothing stay off the page.
#[tokio::test]
async fn session_search_door_finds_by_title_anchor_and_text() {
    let app = fabro_server::test_support::build_test_router(test_app_state());
    let (first_run, _first_workspace) = create_run(&app).await;
    let (second_run, _second_workspace) = create_run(&app).await;

    let archaeology = create_titled_session(&app, &first_run, "Deploy archaeology").await;
    let _unrelated = create_titled_session(&app, &second_run, "Something else").await;
    delete_openai_credential(&app).await;
    submit_failing_turn(&app, &archaeology, "why did the nightly stall").await;
    submit_failing_turn(&app, &_unrelated, "entirely different topic").await;

    // By title.
    let response = get_json(&app, "/sessions/search?q=ARCHAEOLOGY", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response, StatusCode::OK, "GET /api/v1/sessions/search").await;
    let found: Vec<_> = body["data"]
        .as_array()
        .expect("search response should include data")
        .iter()
        .filter(|s| s["id"] == archaeology.as_str())
        .collect();
    assert_eq!(
        found.len(),
        1,
        "the title match finds the archaeology session"
    );
    assert_eq!(found[0]["run_id"], first_run.as_str());

    // By run anchor.
    let response = get_json(&app, &format!("/sessions/search?q={first_run}"), None).await;
    let body = response_json(
        response,
        StatusCode::OK,
        "GET /api/v1/sessions/search?q=run",
    )
    .await;
    let ids: Vec<_> = body["data"]
        .as_array()
        .expect("search response should include data")
        .iter()
        .map(|s| s["id"].as_str().expect("session id should be a string"))
        .collect();
    assert_eq!(
        ids,
        vec![archaeology.as_str()],
        "only the anchored run's session matches"
    );

    // By recorded message text.
    let response = get_json(&app, "/sessions/search?q=nightly%20stall", None).await;
    let body = response_json(
        response,
        StatusCode::OK,
        "GET /api/v1/sessions/search?q=text",
    )
    .await;
    let ids: Vec<_> = body["data"]
        .as_array()
        .expect("search response should include data")
        .iter()
        .map(|s| s["id"].as_str().expect("session id should be a string"))
        .collect();
    assert_eq!(
        ids,
        vec![archaeology.as_str()],
        "the recorded message text matches"
    );

    // A keyword matching nothing leaves the page empty.
    let response = get_json(&app, "/sessions/search?q=zzz-no-such-thing", None).await;
    let body = response_json(
        response,
        StatusCode::OK,
        "GET /api/v1/sessions/search?q=none",
    )
    .await;
    assert!(
        body["data"].as_array().expect("data array").is_empty(),
        "no keyword match, empty page"
    );
    assert_eq!(body["meta"]["has_more"], false);
}

/// The session archaeology transcript door (work#1679): the fold turns a
/// session's events into user and assistant messages (the failed turn
/// lands as a visible note), and the token budget drops whole messages
/// past the cap and says so.
#[tokio::test]
async fn transcript_door_folds_events_and_caps_the_budget() {
    let app = fabro_server::test_support::build_test_router(test_app_state());
    let (run_id, _run_id_workspace) = create_run(&app).await;

    let long_text = "x".repeat(1000);
    let session_id = create_titled_session(&app, &run_id, "Ask Fabro").await;
    delete_openai_credential(&app).await;
    submit_failing_turn(&app, &session_id, &long_text).await;

    // Uncapped: the fold yields the user message and the failed turn's
    // visible note.
    let response = get_json(&app, &format!("/sessions/{session_id}/transcript"), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let transcript = response_json(
        response,
        StatusCode::OK,
        "GET /api/v1/sessions/{id}/transcript",
    )
    .await;
    assert_eq!(transcript["session_id"], session_id.as_str());
    assert_eq!(transcript["run_id"], run_id.as_str());
    let messages = transcript["messages"]
        .as_array()
        .expect("transcript should include messages");
    assert_eq!(messages.len(), 2, "user message + the failure note");
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[0]["text"], long_text);
    assert_eq!(messages[1]["role"], "assistant");
    let failure_note = messages[1]["text"]
        .as_str()
        .expect("the failure note should be text");
    assert!(
        failure_note.starts_with("Turn failed:"),
        "the failed turn is visible: {failure_note}"
    );
    assert_eq!(transcript["tool_calls"]["total"], 0);
    assert_eq!(transcript["budget"]["truncated"], false);

    // Capped at the floor: only the (long) opening message survives.
    let response = get_json(
        &app,
        &format!("/sessions/{session_id}/transcript?max_tokens=200"),
        None,
    )
    .await;
    let capped = response_json(
        response,
        StatusCode::OK,
        "GET /api/v1/sessions/{id}/transcript?max_tokens",
    )
    .await;
    let capped_messages = capped["messages"]
        .as_array()
        .expect("capped transcript should include messages");
    assert_eq!(capped_messages.len(), 1, "the budget drops the tail");
    assert_eq!(capped_messages[0]["role"], "user");
    assert_eq!(capped["budget"]["truncated"], true);
    assert_eq!(capped["budget"]["omitted_messages"], 1);
    assert_eq!(capped["budget"]["max_tokens"], 200);
}

/// The archaeology doors' auth (work#1679): a user passes; a supervisor
/// worker token passes; a plain run worker — and a read-only analyst's
/// agent:run_tools token — are refused on both doors.
#[tokio::test]
async fn archaeology_doors_refuse_workers_without_the_supervisor_scope() {
    let state = test_app_state();
    let app = fabro_server::test_support::build_test_router(state.clone());
    let (run_id, _run_id_workspace) = create_run(&app).await;
    let session_id = create_titled_session(&app, &run_id, "Ask Fabro").await;
    delete_openai_credential(&app).await;
    submit_failing_turn(&app, &session_id, "hello there").await;
    let run: fabro_types::RunId = run_id.parse().unwrap();

    let plain = state.test_issue_worker_token(&run);
    let analyst = state.test_issue_worker_token_with_run_tools(&run);
    let supervisor = state.test_issue_worker_token_supervisor(&run);
    for (label, token) in [
        ("plain run worker", plain.as_str()),
        ("read-only analyst token", analyst.as_str()),
    ] {
        let response = get_json(&app, "/sessions/search?q=hello", Some(token)).await;
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "{label} must not reach the search door"
        );
        let response = get_json(
            &app,
            &format!("/sessions/{session_id}/transcript"),
            Some(token),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "{label} must not reach the transcript door"
        );
    }

    // The supervisor worker passes both doors.
    let response = get_json(&app, "/sessions/search?q=hello", Some(&supervisor)).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the supervisor scope reaches the search door"
    );
    let response = get_json(
        &app,
        &format!("/sessions/{session_id}/transcript"),
        Some(&supervisor),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the supervisor scope reaches the transcript door"
    );

    // So does the user (no bearer in the test principal).
    let response = get_json(&app, "/sessions/search?q=hello", None).await;
    assert_eq!(response.status(), StatusCode::OK);
}
