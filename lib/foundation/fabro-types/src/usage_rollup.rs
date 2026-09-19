use std::collections::HashMap;

use lithos_llm::types::Usage;

use crate::usage::usage_is_empty;
use crate::{ModelRef, RunProjection, RunTiming, StageProjection, StageSummary, StageTiming};

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectionUsageStage {
    pub node_id: String,
    pub usage:   Usage,
    /// Per-node timing summed across every visit of that node within this
    /// projection. `wall_time_ms`, `inference_time_ms`, `tool_time_ms`, and
    /// `active_time_ms` are all summed in lockstep.
    pub timing:  StageTiming,
    pub model:   Option<ModelRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionUsageByModel {
    pub model:  ModelRef,
    pub stages: i64,
    pub usage:  Usage,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectionUsageRollup {
    pub stages:            Vec<ProjectionUsageStage>,
    pub totals:            Usage,
    pub by_model:          Vec<ProjectionUsageByModel>,
    /// Run-level timing summed across every stage visit. `wall_time_ms` is
    /// the sum of stage visit wall times (not the run clock duration).
    pub timing:            RunTiming,
    /// Stage visits that used tokens or carried a cost.
    pub usage_visit_count: usize,
}

impl ProjectionUsageRollup {
    /// The totals, once at least one stage visit used tokens; `None` for a
    /// run that made no model calls.
    #[must_use]
    pub fn usage_if_present(&self) -> Option<Usage> {
        (self.usage_visit_count > 0).then_some(self.totals)
    }

    /// The conclusion's per-node summaries: one row per node the run
    /// visited, ordered by the node's first stage event, with the usage
    /// and timing summed over its visits and the retries counted past the
    /// first visit.
    #[must_use]
    pub fn conclusion_stages(&self, projection: &RunProjection) -> (Vec<StageSummary>, u32) {
        let projection_order = stage_projection_order(projection);
        let usage_by_node = self
            .stages
            .iter()
            .map(|stage| (stage.node_id.as_str(), stage))
            .collect::<HashMap<_, _>>();
        let mut nodes = projection
            .iter_stages()
            .map(|(stage_id, _)| stage_id.node_id())
            .collect::<Vec<_>>();
        nodes.dedup();
        let mut seen = std::collections::HashSet::new();
        let mut retries_sum: u32 = 0;
        let mut stage_rows = Vec::new();
        for node_id in nodes {
            if !seen.insert(node_id) {
                continue;
            }
            let visits = projection.list_node_visits(node_id).len();
            let retries = u32::try_from(visits.saturating_sub(1)).unwrap_or(u32::MAX);
            retries_sum = retries_sum.saturating_add(retries);
            let row = usage_by_node.get(node_id);
            let summary = StageSummary {
                stage_id: node_id.to_string(),
                stage_label: node_id.to_string(),
                timing: row.map_or_else(StageTiming::default, |stage| stage.timing),
                usage: row.map_or_else(Usage::default, |stage| stage.usage),
                retries,
            };
            stage_rows.push((
                projection_order.get(node_id).copied().unwrap_or(u32::MAX),
                summary,
            ));
        }
        stage_rows.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| left.1.stage_id.cmp(&right.1.stage_id))
        });
        let stages = stage_rows.into_iter().map(|(_, summary)| summary).collect();
        (stages, retries_sum)
    }
}

#[must_use]
pub fn usage_rollup_from_projection(projection: &RunProjection) -> ProjectionUsageRollup {
    let mut stage_indices = HashMap::<String, usize>::new();
    let mut stages = Vec::<ProjectionUsageStage>::new();
    let mut by_model = HashMap::<ModelRef, ProjectionUsageByModel>::new();
    let mut totals = Usage::default();
    let mut run_timing = RunTiming::default();
    let mut usage_visit_count = 0_usize;

    for (stage_id, stage) in projection.iter_stages() {
        if projection.is_boundary_stage(stage_id.node_id()) {
            continue;
        }
        let usage = stage.usage;
        if stage.completion.is_none() && stage.timing.is_none() && usage_is_empty(&usage) {
            continue;
        }

        let node_id = stage_id.node_id();
        let index = *stage_indices.entry(node_id.to_string()).or_insert_with(|| {
            let index = stages.len();
            stages.push(ProjectionUsageStage {
                node_id: node_id.to_string(),
                usage:   Usage::default(),
                timing:  StageTiming::default(),
                model:   None,
            });
            index
        });
        let row = &mut stages[index];

        if let Some(timing) = stage.timing {
            row.timing = row.timing.saturating_add(&timing);
            run_timing = run_timing.saturating_add(&RunTiming::from(timing));
        }

        if !usage_is_empty(&usage) {
            usage_visit_count += 1;
            row.usage = row.usage.saturating_add(usage);
            totals = totals.saturating_add(usage);

            if let Some(model) = &stage.model {
                row.model = Some(model.clone());
            }
            // A completed agent stage says which model used which tokens:
            // the root's route and each subagent's own. Until then, and for
            // a stage without a coding agent, `usage` goes under `model`.
            for (model, usage) in model_rows(stage) {
                let model_entry =
                    by_model
                        .entry(model.clone())
                        .or_insert_with(|| ProjectionUsageByModel {
                            model,
                            stages: 0,
                            usage: Usage::default(),
                        });
                model_entry.stages += 1;
                model_entry.usage = model_entry.usage.saturating_add(usage);
            }
        }
    }

    let mut by_model = by_model.into_values().collect::<Vec<_>>();
    by_model.sort_by(|left, right| left.model.sort_key().cmp(&right.model.sort_key()));

    ProjectionUsageRollup {
        stages,
        totals,
        by_model,
        timing: run_timing,
        usage_visit_count,
    }
}

/// The stage's usage by model: its `usage_by_model` rows when the stage
/// completed with them, else its `usage` under its `model`.
fn model_rows(stage: &StageProjection) -> Vec<(ModelRef, Usage)> {
    if stage.usage_by_model.is_empty() {
        return stage
            .model
            .iter()
            .map(|model| (model.clone(), stage.usage))
            .collect();
    }
    stage
        .usage_by_model
        .iter()
        .map(|row| (row.model.clone(), row.usage))
        .collect()
}

fn stage_projection_order(state: &RunProjection) -> HashMap<String, u32> {
    let mut order = HashMap::new();
    for (stage_id, stage) in state.iter_stages() {
        order
            .entry(stage_id.node_id().to_string())
            .and_modify(|first_seq: &mut u32| {
                *first_seq = (*first_seq).min(stage.first_event_seq.get());
            })
            .or_insert_with(|| stage.first_event_seq.get());
    }
    order
}

#[cfg(test)]
mod tests {
    use lithos_llm::catalog::{ModelId, builtin};
    use lithos_llm::types::{Cost, CostSource, TokenCounts, Usage};

    use super::usage_rollup_from_projection;
    use crate::test_support::{self, test_usage};
    use crate::{
        AttrValue, Graph, ModelRef, Node, RunProjection, RunSpec, StageCompletion, StageOutcome,
        first_event_seq,
    };

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
        stage.timing = Some(crate::StageTiming::wall_only(100));
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
        first.timing = Some(crate::StageTiming::wall_only(1200));
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
        second.timing = Some(crate::StageTiming::wall_only(800));
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
        stage.timing = Some(crate::StageTiming::wall_only(25));
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
        start.timing = Some(crate::StageTiming::wall_only(25));
        start.completion = Some(StageCompletion {
            outcome:        StageOutcome::Succeeded,
            notes:          None,
            failure_reason: None,
            timestamp:      chrono::Utc::now(),
        });
        let exit = projection.stage_entry("exit", 1, first_event_seq(2));
        exit.timing = Some(crate::StageTiming::wall_only(7));
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
