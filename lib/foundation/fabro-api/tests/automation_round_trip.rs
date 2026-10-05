use fabro_api::types::{
    Automation as ApiAutomation, AutomationGitWorkflowSource as ApiAutomationGitWorkflowSource,
    AutomationProviderWindowFact as ApiProviderWindowFact,
    AutomationProviderWindowState as ApiProviderWindowState,
    AutomationTrigger as ApiAutomationTrigger,
    CreateAutomationRequest as ApiCreateAutomationRequest,
    ReplaceAutomationRequest as ApiReplaceAutomationRequest,
};
use fabro_automation::{
    Automation, AutomationDraft, AutomationGitWorkflowSource, AutomationReplace, AutomationTrigger,
    ProviderWindowFact, ProviderWindowState,
};
use serde_json::json;

// Compile-time witnesses that the generated API types resolve to the same
// types as the `fabro-automation` domain types via `with_replacement(...)`.
// If progenitor stops reusing the domain type, these functions stop type-
// checking and the build fails.
const _: fn(ApiAutomation) -> Automation = |value| value;
const _: fn(ApiAutomationTrigger) -> AutomationTrigger = |value| value;
const _: fn(ApiAutomationGitWorkflowSource) -> AutomationGitWorkflowSource = |value| value;
const _: fn(ApiCreateAutomationRequest) -> AutomationDraft = |value| value;
const _: fn(ApiReplaceAutomationRequest) -> AutomationReplace = |value| value;
// Fork (fabro-b869 step 5b): the provider-window read model reuses the
// domain types, so the gate's facts and the API payload cannot drift.
const _: fn(ApiProviderWindowState) -> ProviderWindowState = |value| value;
const _: fn(ApiProviderWindowFact) -> ProviderWindowFact = |value| value;

#[test]
fn automation_response_round_trips_public_json_shape() {
    let value = json!({
        "id": "nightly-deps",
        "revision": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "name": "Nightly dependency update",
        "description": null,
        "environment_id": "daytona-smoke",
        "last_error": null,
        "target": {
            "kind": "git",
            "repo": "fabro-sh/fabro",
            "branch": "main",
            "tag": "v1.2.3",
            "sha": "0123456789abcdef0123456789abcdef01234567"
        },
        "workflow": "dependency-update",
        "triggers": [
            {
                "id": "manual",
                "type": "api",
                "enabled": true
            },
            {
                "id": "nightly",
                "type": "schedule",
                "enabled": true,
                "expression": "0 3 * * *"
            }
        ]
    });

    let api: ApiAutomation = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(api).unwrap(), value);
}

#[test]
fn create_automation_request_round_trips_public_json_shape() {
    let value = json!({
        "id": "nightly-deps",
        "name": "Nightly dependency update",
        "description": "Keep dependencies fresh",
        "environment_id": "daytona-smoke",
        "target": {
            "kind": "git",
            "repo": "fabro-sh/fabro",
            "branch": "main"
        },
        "workflow": "dependency-update",
        "triggers": [
            {
                "id": "manual",
                "type": "api",
                "enabled": false
            }
        ]
    });

    let api: ApiCreateAutomationRequest = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(api).unwrap(), value);
}

#[test]
fn replace_automation_request_round_trips_public_json_shape() {
    let value = json!({
        "name": "Nightly dependency update",
        "description": "Keep dependencies fresh",
        "environment_id": "daytona-smoke",
        "target": {
            "kind": "git",
            "repo": "fabro-sh/fabro",
            "branch": "release",
            "tag": "v2"
        },
        "workflow": "dependency-update",
        "triggers": [
            {
                "id": "nightly",
                "type": "schedule",
                "enabled": true,
                "expression": "0 3 * * *"
            }
        ]
    });

    let api: ApiReplaceAutomationRequest = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(api).unwrap(), value);
}

#[test]
fn automation_workflow_sources_round_trip_each_public_json_shape() {
    for source in [
        json!({"repo": "fabro-sh/workflows", "branch": "main"}),
        json!({
            "repo": "fabro-sh/workflows",
            "branch": "main",
            "tag": "release/v1"
        }),
        json!({
            "repo": "fabro-sh/workflows",
            "branch": "context-only",
            "tag": "release/v1",
            "sha": "abcdef0123456789abcdef0123456789abcdef01"
        }),
    ] {
        let value = json!({
            "id": "nightly-deps",
            "name": "Nightly dependency update",
            "environment_id": "daytona-smoke",
            "target": {
                "kind": "git",
                "repo": "fabro-sh/app",
                "branch": "main"
            },
            "workflow": "dependency-update",
            "workflow_source": source,
            "triggers": []
        });

        let api: ApiCreateAutomationRequest = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(api).unwrap(), value);
    }
}

#[test]
fn automation_workflow_source_rejects_unknown_or_incomplete_coordinates() {
    for source in [
        json!({"repo": "fabro-sh/workflows"}),
        json!({"branch": "main"}),
        json!({"repo": "fabro-sh/workflows", "branch": "main", "extra": true}),
    ] {
        assert!(serde_json::from_value::<ApiAutomationGitWorkflowSource>(source).is_err());
    }

    let invalid_commit: ApiAutomationGitWorkflowSource = serde_json::from_value(json!({
        "repo": "fabro-sh/workflows",
        "branch": "main",
        "sha": "short"
    }))
    .unwrap();
    assert!(fabro_automation::validate_workflow_source(invalid_commit).is_err());
}

#[test]
fn provider_window_facts_round_trip_the_public_json_shape() {
    // The trigger read model as the fork's scheduler writes it (fabro-b869
    // step 5b): the window is the catalog-spelled lowercase value.
    let value = json!({
        "providers": [
            {
                "provider": "zai",
                "window": "closed",
                "last_probe_at": "2026-10-05T09:00:00Z",
                "next_probe_at": "2026-10-05T09:10:00Z"
            }
        ]
    });

    let state: ApiProviderWindowState = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(
        state.providers[0].window,
        fabro_automation::ProviderWindowKind::Closed
    );
    assert_eq!(serde_json::to_value(state).unwrap(), value);
}
