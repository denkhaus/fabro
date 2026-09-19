//! Tests that paginated list endpoints return `{ data, meta: { has_more } }`.

#![allow(
    clippy::absolute_paths,
    reason = "This test module prefers explicit type paths over extra imports."
)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::helpers::{response_json, test_app_state};

async fn get_json(app: axum::Router, uri: &str) -> serde_json::Value {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .header("x-fabro-demo", "1")
        .body(Body::empty())
        .expect("pagination request should build");
    let response = app.clone().oneshot(req).await.unwrap();
    response_json(response, StatusCode::OK, format!("GET {uri}")).await
}

/// Assert that a value has the paginated shape: `{ data: [...], meta: {
/// has_more: bool } }`
fn assert_paginated_shape(json: &serde_json::Value, context: &str) {
    assert!(json.get("data").is_some(), "{context}: missing 'data' key");
    assert!(json["data"].is_array(), "{context}: 'data' is not an array");
    assert!(json.get("meta").is_some(), "{context}: missing 'meta' key");
    assert!(
        json["meta"].get("has_more").is_some(),
        "{context}: missing 'meta.has_more'"
    );
    assert!(
        json["meta"]["has_more"].is_boolean(),
        "{context}: 'meta.has_more' is not boolean"
    );
}

struct PaginatedEndpoint {
    path: &'static str,
    name: &'static str,
}

const ENDPOINTS: &[PaginatedEndpoint] = &[
    PaginatedEndpoint {
        path: "/api/v1/insights/queries",
        name: "listSavedQueries",
    },
    PaginatedEndpoint {
        path: "/api/v1/insights/history",
        name: "listQueryHistory",
    },
    PaginatedEndpoint {
        path: "/api/v1/models",
        name: "listModels",
    },
    PaginatedEndpoint {
        path: "/api/v1/runs/run-1/questions",
        name: "listRunQuestions",
    },
    PaginatedEndpoint {
        path: "/api/v1/runs/run-1/stages",
        name: "listRunStages",
    },
];

#[tokio::test]
async fn paginated_endpoints_return_correct_shape() {
    let state = test_app_state();
    let app = fabro_server::test_support::build_test_router(state);

    for ep in ENDPOINTS {
        // Walk the collection at the largest page the API allows: every page
        // has the paginated shape, and the last one reports has_more = false.
        // The built-in model catalog is larger than one page, so the walk
        // follows `page[offset]` rather than assuming one page fits.
        let mut offset = 0;
        loop {
            let json = get_json(
                app.clone(),
                &format!("{}?page[limit]=100&page[offset]={offset}", ep.path),
            )
            .await;
            assert_paginated_shape(&json, &format!("{} offset={offset}", ep.name));
            let page_len = json["data"].as_array().unwrap().len();
            assert!(page_len <= 100, "{}: page exceeded the limit", ep.name);
            if json["meta"]["has_more"] == false {
                break;
            }
            assert!(
                page_len == 100,
                "{}: has_more=true on a page shorter than the limit",
                ep.name
            );
            offset += page_len;
            assert!(offset < 10_000, "{}: has_more never turned false", ep.name);
        }

        // limit=1: at most 1 item, has_more = true (all fixtures have >1 item)
        let json = get_json(app.clone(), &format!("{}?page[limit]=1", ep.path)).await;
        assert_paginated_shape(&json, &format!("{} limit=1", ep.name));
        assert!(
            json["data"].as_array().unwrap().len() <= 1,
            "{}: limit=1 returned more than 1 item",
            ep.name
        );
        assert_eq!(
            json["meta"]["has_more"], true,
            "{}: limit=1 should have has_more=true",
            ep.name
        );
    }
}
