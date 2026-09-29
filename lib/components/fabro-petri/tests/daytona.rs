//! Fabro's Daytona selection reaches the provider through Petri's real
//! executor.

mod support;

use std::sync::Arc;

use fabro_petri::check::Launch;
use fabro_petri::engine::{self, RunStatus};
use fabro_petri::providers::{DaytonaCredentials, SandboxProviderConfig};
use fabro_petri::runtime::RuntimeSpec;
use fabro_types::SandboxProviderKind;
use fabro_types::settings::run::EnvironmentResourcesSettings;
use httpmock::prelude::*;
use petri_store::MemoryRunStore;
use serde_json::json;

#[tokio::test]
async fn daytona_runs_request_container_runner_snapshots() {
    assert_snapshot_request(EnvironmentResourcesSettings::default(), 2, 4, None).await;
}

#[tokio::test]
async fn daytona_runs_forward_configured_resources_in_provider_units() {
    assert_snapshot_request(
        EnvironmentResourcesSettings {
            cpu:    Some(4),
            memory: Some("6GB".parse().unwrap()),
            disk:   Some("8GB".parse().unwrap()),
        },
        4,
        6,
        Some(8),
    )
    .await;
}

#[tokio::test]
async fn daytona_decimal_memory_meets_the_runner_minimum_without_defaulting_disk() {
    assert_snapshot_request(
        EnvironmentResourcesSettings {
            cpu:    Some(2),
            memory: Some("4GB".parse().unwrap()),
            disk:   None,
        },
        2,
        4,
        None,
    )
    .await;
}

async fn assert_snapshot_request(
    resources: EnvironmentResourcesSettings,
    cpu: u32,
    memory: u64,
    disk: Option<u64>,
) {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(GET).path("/api-keys/current");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(json!({
                    "name": "test-key",
                    "organizationId": "test-org",
                    "permissions": [
                        "write:snapshots", "delete:snapshots",
                        "write:sandboxes", "delete:sandboxes"
                    ]
                }));
        })
        .await;
    server
        .mock_async(|when, then| {
            when.method(GET).path("/sandbox");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(json!({"items": [], "nextCursor": null}));
        })
        .await;
    server
        .mock_async(|when, then| {
            when.method(GET).path_matches(r"^/snapshots/[^/]+$");
            then.status(404)
                .header("content-type", "application/json")
                .json_body(json!({"message": "not found"}));
        })
        .await;
    let create = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/snapshots")
                .json_body_includes(
                    json!({"sandboxClass": "container", "cpu": cpu, "memory": memory}).to_string(),
                )
                .is_true(move |request| {
                    let Ok(body) = serde_json::from_slice::<serde_json::Value>(request.body_ref())
                    else {
                        return false;
                    };
                    match disk {
                        Some(disk) => body.get("disk") == Some(&json!(disk)),
                        None => body.get("disk").is_none(),
                    }
                });
            // Stop at the provider boundary: this test proves the wire
            // contract without creating a sandbox or simulating its shell.
            then.status(400)
                .header("content-type", "application/json")
                .json_body(json!({"message": "snapshot creation stopped by test"}));
        })
        .await;
    let credentials = DaytonaCredentials::new("test-key".to_string())
        .with_api_url(Some(server.base_url()))
        .with_http_client(Some(fabro_test::test_http_client()));
    let runtime = RuntimeSpec {
        sandbox: SandboxProviderConfig::from_lookup(Some(credentials), |_| None),
        ..RuntimeSpec::default()
    };
    let graphs = support::admit(
        &[
            ("workflow.toml", support::SETTINGS),
            (
                "workflow.fabro",
                r#"digraph Smoke {
                    start [shape=Mdiamond]
                    work [shape=parallelogram, script="echo smoke"]
                    exit [shape=Msquare]
                    start -> work -> exit
                }"#,
            ),
        ],
        Launch::default(),
        &runtime,
    );
    let root = tempfile::tempdir().expect("a temporary run directory");
    let mut request = support::run_request(
        "daytona-container",
        root.path(),
        graphs,
        Arc::new(MemoryRunStore::new()),
        runtime,
        support::no_questions(Arc::new(support::Silent)),
    );
    request.provider = SandboxProviderKind::DAYTONA;
    request.resources = resources;

    let outcome = engine::run(request)
        .await
        .expect("the run records its failure");

    assert_eq!(create.calls_async().await, 1, "{outcome:?}");
    assert_eq!(outcome.status, RunStatus::Failed);
}
