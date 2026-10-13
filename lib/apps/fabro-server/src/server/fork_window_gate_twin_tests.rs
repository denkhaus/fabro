//! Fork-only twin-scenario smoke for the provider-window gate (fabro-2919,
//! ADR-0028's probe-only gate): the three gate behaviors a deploy touches
//! are proven against a REAL scripted provider on the twin — before any
//! deploy, in seconds, in every plain `cargo nextest run -p fabro-server`
//! round (the `it` target's `test-support` feature gap does not apply to
//! this src-level file).
//!
//! Why twin scenarios and not another unit pin: the unit pins prove the
//! gate's decision core on hand-written probe outcomes; they cannot see a
//! regression in the REAL probe path (the llm client's error rendering of
//! a 429 answer, the catalog probe selector, the retry middleware) — the
//! exact chain the 2026-10-12 re-arm wedge needed a 40-minute deploy to
//! observe. The scenarios:
//!
//! 1. a scripted usage-window 429 holds the scheduled tick: no run is created,
//!    the hold is VISIBLE on the trigger's window facts, and a manual fire
//!    during the closed window is refused with the actionable message
//!    (fabro-b869 step 4);
//! 2. the twin flipping back to success re-opens the window on the next recheck
//!    tick and the held fire goes out through the REAL scheduled fire path —
//!    the announce-style forever-hold class (fabro-f73b, fabro-ac3f, ADR-0028)
//!    REDs here.
//!
//! Time control: the tick functions take `now`, so the 600-second recheck
//! cadence is crossed synthetically (t0 + 660s) instead of being waited
//! out; the probes and the fired run are real.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use chrono::{Duration as ChronoDuration, Utc};
use fabro_automation::{
    AutomationDraft, AutomationId, AutomationTrigger, AutomationTriggerId,
    ModelProviderRequirement, ScheduleTrigger,
};
use fabro_static::EnvVars;
use fabro_test::{TwinScenario, TwinScenarios, twin_openai};
use fabro_types::{GitRunTarget, RunTarget};
use serde_json::json;
use tower::ServiceExt;

use super::fork_line_recovery::ProviderWindow;
use crate::test_support::{
    TestAppStateBuilder, TestAutomationRunMaterializer, build_test_router,
    test_register_workflow_version,
};

/// The live incident's 429 body (2026-10-10 zai, fabro-986b evidence): the
/// classifier must close the gate on what a provider actually sends.
const USAGE_WINDOW_MESSAGE: &str = "Usage limit reached for 5 hour. Your limit will reset at \
                                    2026-10-13 20:15:00";

/// An intent whose admission requires the twin-backed provider, so a manual
/// create reaches the window gate (fabro-b869 step 4).
const OPENAI_AGENT_DOT: &str = r#"digraph Test {
    graph [goal="Test"]
    start [shape=Mdiamond]
    work [shape=box, prompt="Do the work", model="gpt-5.4", provider="openai"]
    exit  [shape=Msquare]
    start -> work -> exit
}"#;

/// One twin-backed gate line: state with the scheduler running, one
/// schedule automation whose admission requires `openai`, and the twin
/// answering every probe for the catalog's own probe model with the
/// scripted usage-window 429.
struct GateLine {
    state:       Arc<super::AppState>,
    app:         Router,
    automation:  AutomationId,
    trigger:     AutomationTriggerId,
    twin:        &'static fabro_test::TwinOpenAi,
    namespace:   String,
    probe_model: String,
}

async fn gate_line(namespace: String) -> GateLine {
    let twin = twin_openai().await;
    // The API key doubles as the twin's bearer namespace (e2e_openai
    // pattern), so every request the client makes lands in this test's
    // namespace and its scenarios.
    let api_key = namespace.clone();
    let env_api_key = api_key.clone();
    let state = TestAppStateBuilder::new()
        .provider_base_url("openai", twin.base_url.clone())
        .vault_entries([(EnvVars::OPENAI_API_KEY, api_key)])
        .env_lookup(move |name| (name == EnvVars::OPENAI_API_KEY).then(|| env_api_key.clone()))
        .in_process_execution()
        .automation_materializer(TestAutomationRunMaterializer::succeed(GitRunTarget {
            repo:   "fabro-sh/fabro".to_string(),
            branch: "main".to_string(),
            tag:    None,
            sha:    None,
        }))
        .build();
    // Script the model the gate will actually probe: the same catalog call
    // `probe_selector` makes, so a catalog probe-row change cannot silently
    // desync the twin scripts from the gate.
    let probe_model = state
        .catalog()
        .enabled_provider("openai")
        .and_then(lithos_llm::catalog::CatalogProvider::probe_offering)
        .expect("the test catalog offers an openai probe model")
        .model
        .id()
        .to_string();

    // The automation: a never-run scheduled line whose recorded admission
    // (fabro-0611) requires openai. The schedule stays out of any real
    // minute — this test drives the gate tick directly, never the planner.
    let automation = AutomationId::new("window-gate-twin").expect("id is valid");
    let trigger = AutomationTriggerId::new("tick").expect("trigger id is valid");
    state
        .automation_store()
        .create(AutomationDraft {
            id:              automation.clone(),
            name:            "window-gate-twin".to_string(),
            description:     None,
            environment_id:  Some("default".to_string()),
            target:          RunTarget::Git(GitRunTarget {
                repo:   "fabro-sh/fabro".to_string(),
                branch: "main".to_string(),
                tag:    None,
                sha:    None,
            }),
            workflow:        "demo".to_string(),
            workflow_source: None,
            on_overlap:      None,
            triggers:        vec![AutomationTrigger::Schedule(ScheduleTrigger {
                id:                trigger.clone(),
                enabled:           true,
                expression:        "0 3 * * *".to_string(),
                breaker_threshold: None,
                breaker:           None,
                provider_window:   None,
            })],
        })
        .await
        .expect("test automation should store");
    state
        .automation_store()
        .set_model_providers(
            &automation,
            Some(&[ModelProviderRequirement {
                provider: "openai".to_string(),
                model:    None,
            }]),
        )
        .await
        .expect("recording the admission's providers should store");

    // Sticky: the retry middleware re-sends a 429; every attempt must meet
    // the scripted answer, never a spent scenario's fallback.
    TwinScenarios::new(namespace.clone())
        .scenario(
            TwinScenario::responses(&probe_model)
                .error(429, USAGE_WINDOW_MESSAGE)
                .sticky(),
        )
        .scenario(
            TwinScenario::chat_completions(&probe_model)
                .error(429, USAGE_WINDOW_MESSAGE)
                .sticky(),
        )
        .load(twin)
        .await;

    super::spawn_scheduler(Arc::clone(&state));
    let app = build_test_router(state.clone());
    GateLine {
        state,
        app,
        automation,
        trigger,
        twin,
        namespace,
        probe_model,
    }
}

/// Apply the tick's deferred fact writes the way the scheduler loop does,
/// in order, after the gate lock is released.
async fn apply_fact_writes(state: &super::AppState, tick: &super::fork_line_recovery::GateTick) {
    for write in &tick.fact_writes {
        let automation = AutomationId::new(&write.automation_id).expect("tick writes valid ids");
        state
            .automation_store()
            .apply_schedule_provider_window(&automation, &write.trigger_id, write.facts.as_ref())
            .await
            .expect("window fact write should store");
    }
}

/// The stored automation, fresh from the store.
async fn stored_automation(
    state: &super::AppState,
    id: &AutomationId,
) -> fabro_automation::Automation {
    state
        .automation_store()
        .get(id)
        .await
        .expect("automation store reads")
        .expect("the test automation exists")
}

/// The automation's schedule trigger with this id, from a stored record.
fn schedule_trigger<'a>(
    automation: &'a fabro_automation::Automation,
    trigger: &AutomationTriggerId,
) -> &'a fabro_automation::ScheduleTrigger {
    automation
        .triggers
        .iter()
        .find_map(|candidate| match candidate {
            AutomationTrigger::Schedule(schedule) if schedule.id == *trigger => Some(schedule),
            _ => None,
        })
        .expect("the schedule trigger is stored")
}

async fn automation_runs(state: &super::AppState, id: &AutomationId) -> Vec<fabro_types::RunId> {
    state
        .stores
        .run_summaries
        .list_all(Utc::now())
        .await
        .expect("run summaries read")
        .into_iter()
        .filter(|run| {
            run.automation
                .as_ref()
                .is_some_and(|automation| automation.id == id.as_str())
        })
        .map(|run| run.id)
        .collect()
}

async fn manual_intent(app: &Router, source: &str) -> serde_json::Value {
    let entrypoint = fabro_types::WorkflowPath::new("workflow.fabro").expect("entrypoint valid");
    let files = std::collections::BTreeMap::from([(entrypoint.clone(), source.to_string())]);
    let version =
        fabro_types::WorkflowVersion::new(entrypoint, files, std::collections::BTreeMap::new())
            .expect("test workflow version should build");
    let id = test_register_workflow_version(app, &version, None).await;
    json!({"workflow_version_id": id, "target": {"kind": "none"}, "args": {}})
}

async fn post_run(app: &Router, intent: &serde_json::Value) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/runs")
                .header("content-type", "application/json")
                .body(Body::from(intent.to_string()))
                .expect("request builds"),
        )
        .await
        .expect("oneshot runs")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_scripted_usage_window_holds_the_tick_and_refuses_manual_fires() {
    let line = gate_line(format!("{}::{}", module_path!(), "hold")).await;
    let t0 = Utc::now();

    let automation = stored_automation(&line.state, &line.automation).await;
    let tick =
        super::fork_line_recovery::provider_gate_tick(Arc::clone(&line.state), &[automation], t0)
            .await;
    assert!(
        tick.held.contains(line.automation.as_str()),
        "a 429 usage-window answer must hold the scheduled fire"
    );
    apply_fact_writes(&line.state, &tick).await;

    // The hold is visible on the trigger's read model (the WINDOW column's
    // source), naming the closed provider.
    let stored = stored_automation(&line.state, &line.automation).await;
    let trigger = schedule_trigger(&stored, &line.trigger);
    let facts = trigger
        .provider_window
        .as_ref()
        .expect("the hold is visible");
    let openai = facts
        .providers
        .iter()
        .find(|fact| fact.provider == "openai")
        .expect("the closed provider is named");
    assert_eq!(openai.window, ProviderWindow::Closed);

    // No run was created for the automation.
    assert!(
        automation_runs(&line.state, &line.automation)
            .await
            .is_empty(),
        "a held tick creates no run"
    );

    // The probe really reached the provider — a gate that holds on stale
    // claims without probing (claim-only regression) answers zero requests.
    let logs = line.twin.request_logs(&line.namespace).await;
    let probe_count = logs["requests"].as_array().map_or(0, Vec::len);
    assert!(
        probe_count > 0,
        "the gate probed the provider before holding"
    );

    // A manual fire of the same provider is refused before any run record
    // exists, with the actionable message (fabro-b869 step 4).
    let intent = manual_intent(&line.app, OPENAI_AGENT_DOT).await;
    let response = post_run(&line.app, &intent).await;
    let body =
        fabro_test::expect_axum_json(response, axum::http::StatusCode::CONFLICT, file!()).await;
    assert_eq!(
        body["errors"][0]["code"].as_str(),
        Some("provider_window_closed"),
        "{body}"
    );
    let detail = body["errors"][0]["detail"].as_str().unwrap_or_default();
    assert!(
        detail.contains("openai") && detail.contains("--force"),
        "the refusal names the provider and the override: {detail}"
    );
    assert!(
        automation_runs(&line.state, &line.automation)
            .await
            .is_empty(),
        "the refused manual fire created no run"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_window_reopen_fires_on_the_next_recheck_tick_and_clears_the_facts() {
    let line = gate_line(format!("{}::{}", module_path!(), "reopen")).await;
    let t0 = Utc::now();

    // The window closes and the line holds (scenario 1, compressed).
    let automation = stored_automation(&line.state, &line.automation).await;
    let tick =
        super::fork_line_recovery::provider_gate_tick(Arc::clone(&line.state), &[automation], t0)
            .await;
    assert!(tick.held.contains(line.automation.as_str()));
    apply_fact_writes(&line.state, &tick).await;

    // The provider reopens: the twin now answers the probe model normally.
    TwinScenarios::new(line.namespace.clone())
        .scenario(TwinScenario::responses(&line.probe_model).text("open"))
        .scenario(TwinScenario::chat_completions(&line.probe_model).text("open"))
        .load(line.twin)
        .await;

    // One recheck cadence later (600s) the tick re-probes, ends the hold,
    // and fires through the REAL scheduled fire path — an announce-style
    // forever-hold (fabro-f73b/fabro-ac3f, ADR-0028) REDs here.
    let reopen_at = t0 + ChronoDuration::seconds(660);
    let automation = stored_automation(&line.state, &line.automation).await;
    let tick = super::fork_line_recovery::provider_gate_tick(
        Arc::clone(&line.state),
        &[automation],
        reopen_at,
    )
    .await;
    assert!(
        !tick.held.contains(line.automation.as_str()),
        "an open window releases the hold"
    );
    apply_fact_writes(&line.state, &tick).await;

    // The stale facts are cleared — the read model never says closed for a
    // schedule that just fired (fabro-b869 step 5b).
    let stored = stored_automation(&line.state, &line.automation).await;
    let trigger = schedule_trigger(&stored, &line.trigger);
    assert!(
        trigger.provider_window.is_none(),
        "the reopen tick clears the trigger's window facts"
    );

    // The held fire went out: a run of this automation exists. The fire is
    // spawned by the tick, so poll briefly for its record.
    let mut runs = Vec::new();
    for _ in 0..100 {
        runs = automation_runs(&line.state, &line.automation).await;
        if !runs.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        !runs.is_empty(),
        "the reopen tick fires the held automation run"
    );
}
