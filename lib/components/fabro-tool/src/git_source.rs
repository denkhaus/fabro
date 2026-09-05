//! Git-source run creation (fabro-e297): map a validated create spec with
//! a [`WorkflowSourceRef`] onto the API's `GitSourceRunIntent`. The server
//! resolves and registers the workflow versions; the caller's filesystem
//! never participates.

use std::collections::HashMap;

use fabro_api::types as api;

use crate::{ValidatedCreateRunSpec, WorkflowSourceRef};

pub(crate) fn intent_from_spec(
    spec: &ValidatedCreateRunSpec,
    source: &WorkflowSourceRef,
    parent_id: Option<fabro_types::RunId>,
) -> anyhow::Result<api::GitSourceRunIntent> {
    // The source workflow name wins over the plain workflow selector: a
    // source form without it reuses the selector (both may be present when
    // a caller migrates a path form to the source form).
    let workflow = source
        .workflow
        .clone()
        .unwrap_or_else(|| spec.workflow.clone());
    let repo = source.repo.trim().to_string();
    let branch = source.branch.trim().to_string();
    if repo.is_empty() || branch.is_empty() {
        anyhow::bail!("workflow_source.repo and workflow_source.branch must not be blank");
    }
    let git_target = api::GitRunTarget {
        repo:   repo.clone(),
        branch: branch.clone(),
        tag:    source.tag.clone(),
        sha:    source.sha.clone(),
    };
    let mut inputs = HashMap::new();
    for (key, value) in &spec.inputs {
        inputs.insert(key.clone(), toml_to_json(value));
    }
    let intent = api::GitSourceRunIntent {
        repo,
        branch,
        tag: source.tag.clone(),
        sha: source.sha.clone(),
        workflow,
        target: api::RunTarget::Git(git_target),
        args: api::RunIntentArgs {
            model: spec.model.clone(),
            provider: spec.provider.clone(),
            inputs,
            labels: spec.labels.clone(),
            dry_run: spec.dry_run,
            auto_approve: spec.auto_approve,
            preserve_sandbox: spec.preserve_sandbox,
        },
        environment_id: spec.environment.clone(),
        parent_id: parent_id.map(|run_id| run_id.to_string()),
        title: None,
        goal: spec.goal.clone(),
    };
    Ok(intent)
}

fn toml_to_json(value: &toml::Value) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or(serde_json::Value::Null)
}
