use super::*;

#[tokio::test]
async fn run_usage_includes_live_stage_timing_in_rows_and_totals() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();
    create_durable_run_with_events(&state, run_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
        workflow_run_started_event(run_id),
    ])
    .await;
    append_scoped_stage_event(
        &state,
        run_id,
        "work",
        1,
        &stage_started_event("work", "command"),
    )
    .await;

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{run_id}/usage")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let stages = body["stages"].as_array().unwrap();

    assert_eq!(stages.len(), 1);
    let row_timing = &stages[0]["timing"];
    assert!(row_timing["active_time_ms"].as_u64().unwrap() > 0);
    assert_eq!(row_timing["tool_time_ms"], row_timing["active_time_ms"]);
    assert_eq!(&body["totals"]["timing"], row_timing);
}

/// `checkpoint.completed_nodes` records every visit, so a looped node appears
/// once per re-entry. Usage must dedup so a retried node renders as one row
/// and `runtime_secs` is summed across all visits exactly once.
#[tokio::test]
async fn run_usage_dedups_retried_nodes_and_sums_their_durations() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();

    create_durable_run_with_events(&state, run_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
    ])
    .await;

    // Visit 1 of `verify` — completed in 1.5s.
    append_scoped_stage_event(
        &state,
        run_id,
        "verify",
        1,
        &workflow_event::Event::StageCompleted {
            node_id: "verify".to_string(),
            name: "Verify".to_string(),
            index: 1,
            timing: fabro_types::StageTiming::wall_only(1500),
            status: "failed".to_string(),
            preferred_label: None,
            suggested_next_ids: Vec::new(),
            usage_by_model: Vec::new(),
            usage: None,
            failure: None,
            notes: None,
            files_touched: Vec::new(),
            context_updates: None,
            jump_to_node: None,
            context_values: None,
            node_visits: None,
            loop_failure_signatures: None,
            restart_failure_signatures: None,
            response: None,
            attempt: 1,
            max_attempts: 1,
        },
    )
    .await;

    // Visit 2 of `verify` — completed in 0.8s.
    append_scoped_stage_event(
        &state,
        run_id,
        "verify",
        2,
        &workflow_event::Event::StageCompleted {
            node_id: "verify".to_string(),
            name: "Verify".to_string(),
            index: 1,
            timing: fabro_types::StageTiming::wall_only(800),
            status: "succeeded".to_string(),
            preferred_label: None,
            suggested_next_ids: Vec::new(),
            usage_by_model: Vec::new(),
            usage: None,
            failure: None,
            notes: None,
            files_touched: Vec::new(),
            context_updates: None,
            jump_to_node: None,
            context_values: None,
            node_visits: None,
            loop_failure_signatures: None,
            restart_failure_signatures: None,
            response: None,
            attempt: 1,
            max_attempts: 1,
        },
    )
    .await;

    // Checkpoint records `verify` twice (once per visit) — this is what makes
    // the dedup necessary.
    let run_store = state.stores.runs.open_run(&run_id).await.unwrap();
    workflow_event::append_event(
        &run_store,
        &run_id,
        &workflow_event::Event::CheckpointCompleted {
            graph_visit: None,
            resumed_from_stage_id: None,
            node_id: "verify".to_string(),
            status: "running".to_string(),
            current_node: "verify".to_string(),
            completed_nodes: vec!["verify".to_string(), "verify".to_string()],
            node_retries: std::collections::BTreeMap::new(),
            context_values: std::collections::BTreeMap::new(),
            node_outcomes: std::collections::BTreeMap::from([(
                "verify".to_string(),
                Outcome::default(),
            )]),
            next_node_id: Some("done".to_string()),
            git_commit_sha: None,
            loop_failure_signatures: std::collections::BTreeMap::new(),
            restart_failure_signatures: std::collections::BTreeMap::new(),
            node_visits: std::collections::BTreeMap::from([("verify".to_string(), 2usize)]),
            diff: None,
            diff_summary: None,
        },
    )
    .await
    .unwrap();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{run_id}/usage")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;

    let stages = body["stages"].as_array().unwrap();
    assert_eq!(
        stages.len(),
        1,
        "expected one row for the retried verify node"
    );
    assert_eq!(stages[0]["stage"]["id"], "verify");
    // Duration on the row is the sum across visits (1.5s + 0.8s = 2.3s).
    assert!(
        stages[0]["timing"]["wall_time_ms"].as_u64().unwrap() == 2300,
        "row runtime_secs should sum visits, got {}",
        stages[0]["timing"]["wall_time_ms"]
    );

    // Totals must not double-count: a single 2.3s, not 4.6s.
    assert!(
        body["totals"]["timing"]["wall_time_ms"].as_u64().unwrap() == 2300,
        "totals.runtime_secs should sum visits exactly once, got {}",
        body["totals"]["timing"]["wall_time_ms"]
    );
}

#[tokio::test]
async fn run_usage_sums_usage_across_retry_visits_and_uses_latest_model() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();

    create_priced_retry_run(&state, run_id).await;
    let success_usage = test_priced_usage("gpt-new", 200, 20);
    let mut latest_outcome: Outcome<Option<fabro_types::ModelUsage>> = Outcome::success();
    latest_outcome.usage = Some(success_usage);
    latest_outcome.timing = Some(fabro_types::StageTiming::wall_only(800));
    let run_store = state.stores.runs.open_run(&run_id).await.unwrap();
    workflow_event::append_event(
        &run_store,
        &run_id,
        &workflow_event::Event::CheckpointCompleted {
            graph_visit: None,
            resumed_from_stage_id: None,
            node_id: "verify".to_string(),
            status: "running".to_string(),
            current_node: "verify".to_string(),
            completed_nodes: vec!["verify".to_string(), "verify".to_string()],
            node_retries: std::collections::BTreeMap::from([("verify".to_string(), 2)]),
            context_values: std::collections::BTreeMap::new(),
            node_outcomes: std::collections::BTreeMap::from([(
                "verify".to_string(),
                latest_outcome,
            )]),
            next_node_id: None,
            git_commit_sha: None,
            loop_failure_signatures: std::collections::BTreeMap::new(),
            restart_failure_signatures: std::collections::BTreeMap::new(),
            node_visits: std::collections::BTreeMap::from([("verify".to_string(), 2usize)]),
            diff: None,
            diff_summary: None,
        },
    )
    .await
    .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{run_id}/usage")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;

    let stages = body["stages"].as_array().unwrap();
    assert_eq!(stages.len(), 1);
    assert_eq!(stages[0]["stage"]["id"], "verify");
    assert_eq!(stages[0]["model"]["provider"], "openai");
    assert_eq!(stages[0]["model"]["model_id"], "gpt-new");
    assert_eq!(stages[0]["usage"]["tokens"]["input"], 300);
    assert_eq!(stages[0]["usage"]["tokens"]["output"], 30);
    assert_eq!(stages[0]["usage"]["cost"]["usd_micros"], 330);
    assert!(stages[0]["timing"]["wall_time_ms"].as_u64().unwrap() == 2000);

    assert_eq!(body["totals"]["usage"]["tokens"]["input"], 300);
    assert_eq!(body["totals"]["usage"]["tokens"]["output"], 30);
    assert_eq!(body["totals"]["usage"]["cost"]["usd_micros"], 330);
    assert!(body["totals"]["timing"]["wall_time_ms"].as_u64().unwrap() == 2000);

    let by_model = body["by_model"].as_array().unwrap();
    assert_eq!(by_model.len(), 2);
    let old_model = by_model
        .iter()
        .find(|entry| entry["model"]["model_id"] == "gpt-old")
        .unwrap();
    let new_model = by_model
        .iter()
        .find(|entry| entry["model"]["model_id"] == "gpt-new")
        .unwrap();
    assert_eq!(old_model["model"]["provider"], "openai");
    assert_eq!(new_model["model"]["provider"], "openai");
    assert_eq!(old_model["stages"], 1);
    assert_eq!(old_model["usage"]["tokens"]["input"], 100);
    assert_eq!(new_model["stages"], 1);
    assert_eq!(new_model["usage"]["tokens"]["input"], 200);
}

#[tokio::test]
async fn run_usage_retried_node_then_succeeded_emits_one_row_with_final_attempt_duration() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();

    create_durable_run_with_events(&state, run_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
        workflow_event::Event::StageStarted {
            graph_visit:           None,
            resumed_from_stage_id: None,
            node_id:               "work".to_string(),
            name:                  "Work".to_string(),
            index:                 0,
            handler_type:          "command".to_string(),
            attempt:               1,
            max_attempts:          3,
        },
        workflow_event::Event::StageFailed {
            node_id:        "work".to_string(),
            name:           "Work".to_string(),
            index:          0,
            failure:        FailureDetail::new("transient", FailureCategory::TransientInfra),
            will_retry:     true,
            timing:         fabro_types::StageTiming::wall_only(10),
            usage_by_model: Vec::new(),
            usage:          None,
            actor:          None,
        },
        workflow_event::Event::StageRetrying {
            node_id:      "work".to_string(),
            name:         "Work".to_string(),
            index:        0,
            attempt:      2,
            max_attempts: 3,
            delay_ms:     0,
        },
        workflow_event::Event::StageStarted {
            graph_visit:           None,
            resumed_from_stage_id: None,
            node_id:               "work".to_string(),
            name:                  "Work".to_string(),
            index:                 0,
            handler_type:          "command".to_string(),
            attempt:               2,
            max_attempts:          3,
        },
        workflow_event::Event::StageCompleted {
            node_id: "work".to_string(),
            name: "Work".to_string(),
            index: 0,
            timing: fabro_types::StageTiming::wall_only(25),
            status: "succeeded".to_string(),
            preferred_label: None,
            suggested_next_ids: Vec::new(),
            usage_by_model: Vec::new(),
            usage: None,
            failure: None,
            notes: None,
            files_touched: Vec::new(),
            context_updates: None,
            jump_to_node: None,
            context_values: None,
            node_visits: None,
            loop_failure_signatures: None,
            restart_failure_signatures: None,
            response: None,
            attempt: 2,
            max_attempts: 3,
        },
    ])
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{run_id}/usage")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let stages = body["stages"].as_array().unwrap();
    assert_eq!(stages.len(), 1, "retry collapses to one row per node_id");
    let row = &stages[0];
    assert_eq!(row["stage"]["id"], "work");
    assert_eq!(
        row["state"], "succeeded",
        "final state mirrors the latest StageCompleted"
    );
    let runtime = row["timing"]["wall_time_ms"].as_u64().unwrap();
    assert_eq!(
        runtime, 25,
        "runtime should equal final attempt's 25ms, got {runtime}"
    );
}

#[tokio::test]
async fn run_usage_revisited_node_collapses_to_two_rows_with_summed_visit_duration() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();

    create_durable_run_with_events(&state, run_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
        // A → B → A loop. Per-visit `node_visits` payload steers the reducer
        // to attribute each StageCompleted to the right visit.
        revisit_test_started("a"),
        revisit_test_completed_with_visit("a", 1, 1),
        revisit_test_started("b"),
        revisit_test_completed_with_visit("b", 2, 1),
        revisit_test_started("a"),
        revisit_test_completed_with_visit("a", 99, 2),
    ])
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{run_id}/usage")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let stages = body["stages"].as_array().unwrap();
    assert_eq!(stages.len(), 2, "two distinct node_ids → two rows");
    assert_eq!(
        stages[0]["stage"]["id"], "a",
        "A appeared first → A's row first"
    );
    assert_eq!(stages[1]["stage"]["id"], "b");
    let a_runtime = stages[0]["timing"]["wall_time_ms"].as_u64().unwrap();
    assert_eq!(
        a_runtime, 100,
        "A should sum both visit durations (1ms + 99ms), got {a_runtime}"
    );
    let b_runtime = stages[1]["timing"]["wall_time_ms"].as_u64().unwrap();
    assert_eq!(
        b_runtime, 2,
        "B should carry its single visit's duration (2ms), got {b_runtime}"
    );
}

#[tokio::test]
async fn get_aggregate_usage_returns_zeros_initially() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let req = Request::builder()
        .method("GET")
        .uri(api("/usage"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(body["totals"]["runs"].as_i64().unwrap(), 0);
    assert_eq!(
        body["totals"]["usage"]["tokens"]["input"].as_u64().unwrap(),
        0
    );
    assert_eq!(
        body["totals"]["usage"]["tokens"]["output"]
            .as_u64()
            .unwrap(),
        0
    );
    assert_eq!(
        body["totals"]["timing"]["wall_time_ms"].as_u64().unwrap(),
        0
    );
    assert!(body["totals"]["usage"].get("cost").is_none());
    assert!(body["by_model"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn get_aggregate_usage_returns_provider_model_speed_identity() {
    let state = test_app_state();
    {
        let mut agg = state.aggregate_usage.lock().expect("aggregate usage lock");
        agg.total_runs = 1;
        agg.by_model.insert(
            ModelRef::new(
                lithos_llm::catalog::builtin::anthropic(),
                ModelId::new("claude-opus-4-6"),
            ),
            ModelUsageTotals {
                stages: 1,
                usage:  test_priced_usage("claude-opus-4-6", 10, 1).usage,
            },
        );
        agg.by_model.insert(
            ModelRef::new(
                lithos_llm::catalog::builtin::anthropic(),
                ModelId::new("claude-opus-4-6"),
            )
            .with_speed(Some(Speed::Fast)),
            ModelUsageTotals {
                stages: 1,
                usage:  test_priced_usage("claude-opus-4-6", 20, 2).usage,
            },
        );
    }
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api("/usage"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let by_model = body["by_model"].as_array().unwrap();

    assert_eq!(by_model.len(), 2);
    let standard = by_model
        .iter()
        .find(|entry| entry["model"]["speed"].is_null())
        .unwrap();
    let fast = by_model
        .iter()
        .find(|entry| entry["model"]["speed"] == "fast")
        .unwrap();
    assert_eq!(standard["model"]["provider"], "anthropic");
    assert_eq!(standard["model"]["model_id"], "claude-opus-4-6");
    assert_eq!(standard["usage"]["tokens"]["input"], 10);
    assert_eq!(fast["model"]["provider"], "anthropic");
    assert_eq!(fast["model"]["model_id"], "claude-opus-4-6");
    assert_eq!(fast["usage"]["tokens"]["input"], 20);
}

#[tokio::test]
async fn get_aggregate_usage_saturates_total_cost_across_models() {
    let state = test_app_state();
    {
        let mut agg = state.aggregate_usage.lock().expect("aggregate usage lock");
        for (model_id, usd_micros) in [("maximum", u64::MAX), ("one", 1)] {
            agg.by_model.insert(
                ModelRef::new(
                    lithos_llm::catalog::builtin::openai(),
                    ModelId::new(model_id),
                ),
                ModelUsageTotals {
                    stages: 1,
                    usage:  Usage {
                        tokens: TokenCounts::default(),
                        cost:   Some(Cost {
                            usd_micros,
                            source: CostSource::Catalog,
                        }),
                    },
                },
            );
        }
    }
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api("/usage"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;

    assert_eq!(
        body["totals"]["usage"]["cost"]["usd_micros"].as_u64(),
        Some(u64::MAX)
    );
}

#[test]
fn aggregate_usage_counts_projection_rollup_usage_visits() {
    let mut accumulator = UsageAccumulator::default();
    let rollup = fabro_workflow::ProjectionUsageRollup {
        stages:            Vec::new(),
        totals:            test_priced_usage("gpt-5.4", 300, 30).usage,
        by_model:          vec![
            fabro_workflow::ProjectionUsageByModel {
                model:  ModelRef::new(
                    lithos_llm::catalog::builtin::openai(),
                    ModelId::new("gpt-5.4"),
                ),
                stages: 1,
                usage:  test_priced_usage("gpt-5.4", 100, 10).usage,
            },
            fabro_workflow::ProjectionUsageByModel {
                model:  ModelRef::new(
                    lithos_llm::catalog::builtin::openai(),
                    ModelId::new("gpt-5.4"),
                )
                .with_speed(Some(Speed::Fast)),
                stages: 1,
                usage:  test_priced_usage("gpt-5.4", 200, 20).usage,
            },
        ],
        timing:            fabro_types::RunTiming::wall_only(2000),
        usage_visit_count: 2,
    };

    accumulate_usage_rollup(&mut accumulator, &rollup);

    assert_eq!(accumulator.total_runs, 1);
    assert_eq!(accumulator.total_timing.wall_time_ms, 2000);
    assert_eq!(accumulator.by_model.len(), 2);
    assert_eq!(
        accumulator.by_model[&ModelRef::new(
            lithos_llm::catalog::builtin::openai(),
            ModelId::new("gpt-5.4")
        )]
            .stages,
        1
    );
    assert_eq!(
        accumulator.by_model[&ModelRef::new(
            lithos_llm::catalog::builtin::openai(),
            ModelId::new("gpt-5.4")
        )]
            .usage
            .tokens
            .input,
        100
    );
    assert_eq!(
        accumulator.by_model[&ModelRef::new(
            lithos_llm::catalog::builtin::openai(),
            ModelId::new("gpt-5.4")
        )
        .with_speed(Some(Speed::Fast))]
            .stages,
        1
    );
    assert_eq!(
        accumulator.by_model[&ModelRef::new(
            lithos_llm::catalog::builtin::openai(),
            ModelId::new("gpt-5.4")
        )
        .with_speed(Some(Speed::Fast))]
            .usage
            .tokens
            .input,
        200
    );
}
