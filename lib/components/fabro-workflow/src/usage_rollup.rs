pub use fabro_types::usage_rollup::{
    ProjectionUsageByModel, ProjectionUsageRollup, ProjectionUsageStage,
    usage_rollup_from_projection,
};

#[cfg(test)]
mod tests {
    use fabro_types::{
        AttrValue, Graph, ModelRef, Node, RunProjection, RunSpec, StageCompletion, StageOutcome,
        first_event_seq, test_support,
    };
    use lithos_llm::catalog::{ModelId, builtin};
    use lithos_llm::types::{Cost, CostSource, TokenCounts, Usage};

    use super::usage_rollup_from_projection;
    use crate::test_support::test_usage;

    fn test_projection() -> RunProjection {
        RunProjection::new(
            "Test run".to_string(),
            run_spec_with_boundary_nodes(),
            chrono::Utc::now(),
        )
    }

    #[test]
    fn by_model_splits_a_completed_stage_by_its_usage_rows() {
        let mut projection = test_projection();
        let root = test_usage("gpt-root", 100, 10);
        let child = test_usage("gpt-child", 7, 1);
        let stage = projection.stage_entry("work", 1, first_event_seq(1));
        stage.timing = Some(fabro_types::StageTiming::wall_only(100));
        stage.usage = root.usage.saturating_add(child.usage);
        stage.model = Some(root.model().clone());
        stage.usage_by_model = vec![root.clone(), child.clone()];
        stage.completion = Some(StageCompletion {
            outcome:        StageOutcome::Succeeded,
            notes:          None,
            failure_reason: None,
            timestamp:      chrono::Utc::now(),
        });

        let rollup = usage_rollup_from_projection(&projection);

        assert_eq!(rollup.totals.tokens.input, 107);
        assert_eq!(rollup.stages[0].model.as_ref(), Some(root.model()));
        assert_eq!(rollup.by_model.len(), 2, "{:?}", rollup.by_model);
        let entry = |model_id: &str| {
            rollup
                .by_model
                .iter()
                .find(|entry| entry.model.model_id.as_str() == model_id)
                .unwrap_or_else(|| panic!("a row for {model_id}"))
        };
        assert_eq!(entry("gpt-root").stages, 1);
        assert_eq!(entry("gpt-root").usage.tokens.input, 100);
        assert_eq!(entry("gpt-root").usage.cost, root.usage.cost);
        assert_eq!(entry("gpt-child").stages, 1);
        assert_eq!(entry("gpt-child").usage.tokens.input, 7);
        assert_eq!(entry("gpt-child").usage.cost, child.usage.cost);
    }

    #[test]
    fn rollup_groups_stage_rows_by_node_and_sums_retry_visit_usage() {
        let mut projection = test_projection();
        let failed_usage = test_usage("gpt-old", 100, 10);
        let success_usage = test_usage("gpt-new", 200, 20);
        let first = projection.stage_entry("verify", 1, first_event_seq(1));
        first.timing = Some(fabro_types::StageTiming::wall_only(1200));
        first.usage = failed_usage.usage;
        first.model = Some(failed_usage.model().clone());
        first.completion = Some(StageCompletion {
            outcome:        StageOutcome::Failed {
                retry_requested: true,
            },
            notes:          None,
            failure_reason: Some("try again".to_string()),
            timestamp:      chrono::Utc::now(),
        });
        let second = projection.stage_entry("verify", 2, first_event_seq(2));
        second.timing = Some(fabro_types::StageTiming::wall_only(800));
        second.usage = success_usage.usage;
        second.model = Some(success_usage.model().clone());
        second.completion = Some(StageCompletion {
            outcome:        StageOutcome::Succeeded,
            notes:          None,
            failure_reason: None,
            timestamp:      chrono::Utc::now(),
        });

        let rollup = usage_rollup_from_projection(&projection);

        assert_eq!(rollup.stages.len(), 1);
        assert_eq!(rollup.stages[0].node_id, "verify");
        assert_eq!(
            rollup.stages[0]
                .model
                .as_ref()
                .map(|model| model.model_id.as_str()),
            Some("gpt-new")
        );
        assert_eq!(rollup.stages[0].timing.wall_time_ms, 2000);
        assert_eq!(rollup.stages[0].usage.tokens.input, 300);
        assert_eq!(rollup.stages[0].usage.tokens.output, 30);
        assert_eq!(
            rollup.stages[0].usage.cost,
            Some(Cost {
                usd_micros: 330,
                source:     CostSource::Catalog,
            })
        );

        assert_eq!(rollup.timing.wall_time_ms, 2000);
        assert_eq!(rollup.totals.tokens.input, 300);
        assert_eq!(rollup.totals.tokens.output, 30);
        assert_eq!(rollup.totals.cost.map(|cost| cost.usd_micros), Some(330));
        assert_eq!(rollup.usage_visit_count, 2);

        assert_eq!(rollup.by_model.len(), 2);
        assert_eq!(rollup.by_model[0].model.model_id.as_str(), "gpt-new");
        assert_eq!(rollup.by_model[0].stages, 1);
        assert_eq!(rollup.by_model[0].usage.tokens.input, 200);
        assert_eq!(rollup.by_model[1].model.model_id.as_str(), "gpt-old");
        assert_eq!(rollup.by_model[1].stages, 1);
        assert_eq!(rollup.by_model[1].usage.tokens.input, 100);
    }

    #[test]
    fn rollup_includes_completed_non_llm_stage_rows_with_zero_usage() {
        let mut projection = test_projection();
        let stage = projection.stage_entry("build", 1, first_event_seq(1));
        stage.timing = Some(fabro_types::StageTiming::wall_only(25));
        stage.completion = Some(StageCompletion {
            outcome:        StageOutcome::Succeeded,
            notes:          None,
            failure_reason: None,
            timestamp:      chrono::Utc::now(),
        });

        let rollup = usage_rollup_from_projection(&projection);

        assert_eq!(rollup.stages.len(), 1);
        assert_eq!(rollup.stages[0].node_id, "build");
        assert_eq!(rollup.stages[0].timing.wall_time_ms, 25);
        assert!(rollup.stages[0].model.is_none());
        assert_eq!(rollup.stages[0].usage, Usage::default());
        assert_eq!(rollup.timing.wall_time_ms, 25);
        assert!(rollup.by_model.is_empty());
        assert!(rollup.usage_if_present().is_none());
    }

    #[test]
    fn rollup_excludes_workflow_boundary_stage_rows() {
        let mut projection = test_projection();
        projection.spec = run_spec_with_boundary_nodes();
        let start = projection.stage_entry("start", 1, first_event_seq(1));
        start.timing = Some(fabro_types::StageTiming::wall_only(25));
        start.completion = Some(StageCompletion {
            outcome:        StageOutcome::Succeeded,
            notes:          None,
            failure_reason: None,
            timestamp:      chrono::Utc::now(),
        });
        let exit = projection.stage_entry("exit", 1, first_event_seq(2));
        exit.timing = Some(fabro_types::StageTiming::wall_only(7));
        exit.completion = Some(StageCompletion {
            outcome:        StageOutcome::Succeeded,
            notes:          None,
            failure_reason: None,
            timestamp:      chrono::Utc::now(),
        });

        let rollup = usage_rollup_from_projection(&projection);

        assert_eq!(rollup.stages.len(), 0);
        assert_eq!(rollup.timing.wall_time_ms, 0);
    }

    #[test]
    fn rollup_keeps_in_flight_stage_usage_unpriced() {
        let mut projection = test_projection();
        let model = ModelRef::new(builtin::openai(), ModelId::new("gpt-5.4"));
        let stage = projection.stage_entry("agent", 1, first_event_seq(1));
        stage.started_at = Some(chrono::Utc::now());
        stage.usage = Usage::from(TokenCounts {
            input: 500_000,
            output: 125_000,
            ..TokenCounts::default()
        });
        stage.model = Some(model.clone());

        let rollup = usage_rollup_from_projection(&projection);

        // The rollup keeps the shape of what the events recorded. Costs come
        // from the events themselves; an in-flight stage that has recorded no
        // cost yet stays unpriced rather than being re-estimated here.
        assert_eq!(rollup.stages.len(), 1);
        assert_eq!(rollup.stages[0].node_id, "agent");
        assert_eq!(rollup.stages[0].usage.cost, None);
        assert_eq!(rollup.stages[0].usage.tokens.input, 500_000);
        assert_eq!(rollup.totals.cost, None);
        assert_eq!(rollup.by_model.len(), 1);
        assert_eq!(rollup.by_model[0].usage.tokens.input, 500_000);
    }

    #[test]
    fn rollup_totals_lose_their_cost_once_an_unpriced_stage_used_tokens() {
        let mut projection = test_projection();
        let priced = test_usage("gpt-priced", 100, 10);
        let first = projection.stage_entry("plan", 1, first_event_seq(1));
        first.usage = priced.usage;
        first.model = Some(priced.model().clone());
        first.completion = Some(StageCompletion {
            outcome:        StageOutcome::Succeeded,
            notes:          None,
            failure_reason: None,
            timestamp:      chrono::Utc::now(),
        });
        let second = projection.stage_entry("work", 1, first_event_seq(2));
        second.usage = Usage::from(TokenCounts {
            input: 5,
            ..TokenCounts::default()
        });
        second.model = Some(ModelRef::new(builtin::openai(), ModelId::new("mystery")));
        second.completion = Some(StageCompletion {
            outcome:        StageOutcome::Succeeded,
            notes:          None,
            failure_reason: None,
            timestamp:      chrono::Utc::now(),
        });

        let rollup = usage_rollup_from_projection(&projection);

        // A total cost is known only when every part is priced; the per-stage
        // rows keep their own.
        assert_eq!(rollup.totals.tokens.input, 105);
        assert_eq!(rollup.totals.cost, None);
        assert_eq!(rollup.stages[0].usage.cost, priced.usage.cost);
        assert_eq!(rollup.stages[1].usage.cost, None);
        assert_eq!(
            rollup.usage_if_present().map(|usage| usage.cost),
            Some(None)
        );
    }

    fn run_spec_with_boundary_nodes() -> RunSpec {
        let mut graph = Graph::new("test");
        graph.nodes.insert("start".to_string(), {
            let mut node = Node::new("start");
            node.attrs.insert(
                "shape".to_string(),
                AttrValue::String("Mdiamond".to_string()),
            );
            node
        });
        graph.nodes.insert("exit".to_string(), {
            let mut node = Node::new("exit");
            node.attrs.insert(
                "shape".to_string(),
                AttrValue::String("Msquare".to_string()),
            );
            node
        });

        RunSpec {
            graph,
            ..test_support::test_run_spec()
        }
    }
}
