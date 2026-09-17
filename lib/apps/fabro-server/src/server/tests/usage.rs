use super::*;

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
