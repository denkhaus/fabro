use std::collections::HashMap;

use fabro_types::{
    RunIntent, RunIntentArgs, RunTarget, WorkflowVersionId, normalize_git_commit_sha,
};
use serde_json::json;

fn version_id() -> WorkflowVersionId {
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        .parse()
        .expect("fixture version ID should be valid")
}

fn intent() -> RunIntent {
    RunIntent {
        workflow_version_id: version_id(),
        target:              RunTarget::Git {
            repo:   "fabro-sh/fabro".to_string(),
            branch: "feature/run-intent".to_string(),
            sha:    Some("ABCDEF0123456789ABCDEF0123456789ABCDEF01".to_string()),
        },
        args:                RunIntentArgs {
            model:    Some("gpt-5.6".to_string()),
            provider: Some("openai".to_string()),
            inputs:   HashMap::from([
                ("attempts".to_string(), json!(3)),
                ("enabled".to_string(), json!(true)),
            ]),
            labels:   HashMap::from([("team".to_string(), "platform".to_string())]),
        },
        environment_id:      Some("production".to_string()),
        parent_id:           None,
        title:               Some("Add RunIntent".to_string()),
        goal:                Some("Implement the endpoint".to_string()),
    }
}

#[test]
fn run_intent_round_trips_the_strict_git_shape() {
    let intent = intent();
    let value = serde_json::to_value(&intent).expect("intent should serialize");

    assert_eq!(value["target"]["kind"], "git");
    assert_eq!(value["target"]["repo"], "fabro-sh/fabro");
    assert_eq!(value["target"]["branch"], "feature/run-intent");
    assert_eq!(
        serde_json::from_value::<RunIntent>(value).expect("intent should deserialize"),
        intent
    );
}

#[test]
fn run_intent_rejects_unknown_fields_at_every_object_boundary() {
    let value = serde_json::to_value(intent()).expect("intent should serialize");

    for path in ["root", "args", "target"] {
        let mut candidate = value.clone();
        match path {
            "root" => candidate["unexpected"] = json!(true),
            "args" => candidate["args"]["unexpected"] = json!(true),
            "target" => candidate["target"]["unexpected"] = json!(true),
            _ => unreachable!(),
        }
        assert!(
            serde_json::from_value::<RunIntent>(candidate).is_err(),
            "{path} should reject unknown fields"
        );
    }
}

#[test]
fn run_intent_requires_args_and_git_branch() {
    let mut missing_args = serde_json::to_value(intent()).expect("intent should serialize");
    missing_args.as_object_mut().unwrap().remove("args");
    assert!(serde_json::from_value::<RunIntent>(missing_args).is_err());

    let mut missing_branch = serde_json::to_value(intent()).expect("intent should serialize");
    missing_branch["target"]
        .as_object_mut()
        .unwrap()
        .remove("branch");
    assert!(serde_json::from_value::<RunIntent>(missing_branch).is_err());
}

#[test]
fn git_commit_sha_normalization_is_exact_and_pure() {
    assert_eq!(
        normalize_git_commit_sha("ABCDEF0123456789ABCDEF0123456789ABCDEF01"),
        Some("abcdef0123456789abcdef0123456789abcdef01".to_string())
    );
    for invalid in [
        "abcdef0123456789abcdef0123456789abcdef0",
        "abcdef0123456789abcdef0123456789abcdef012",
        "abcdef0123456789abcdef0123456789abcdef0g",
        " abcdef0123456789abcdef0123456789abcdef01",
    ] {
        assert_eq!(normalize_git_commit_sha(invalid), None);
    }
}

#[test]
fn target_validation_normalizes_sha_without_network_resolution() {
    let validated = RunTarget::Git {
        repo:   "fabro-sh/fabro".to_string(),
        branch: "feature/run-intent".to_string(),
        sha:    Some("ABCDEF0123456789ABCDEF0123456789ABCDEF01".to_string()),
    }
    .validate()
    .unwrap();

    assert_eq!(
        validated.git.sha.as_deref(),
        Some("abcdef0123456789abcdef0123456789abcdef01")
    );
    assert_eq!(
        validated.git.origin_url,
        "https://github.com/fabro-sh/fabro"
    );
    assert_eq!(validated.target, RunTarget::Git {
        repo:   "fabro-sh/fabro".to_string(),
        branch: "feature/run-intent".to_string(),
        sha:    Some("abcdef0123456789abcdef0123456789abcdef01".to_string()),
    });
}

#[test]
fn target_validation_rejects_invalid_grammar() {
    use fabro_types::TargetValidationError;

    let validate = |repo: &str, branch: &str, sha: Option<&str>| {
        RunTarget::Git {
            repo:   repo.to_string(),
            branch: branch.to_string(),
            sha:    sha.map(str::to_string),
        }
        .validate()
    };

    assert_eq!(
        validate("not-a-slug", "main", None).unwrap_err(),
        TargetValidationError::Repository
    );
    for branch in [
        "",
        "HEAD",
        "-main",
        ".main",
        "heads/main",
        "tags/v1",
        "refs/heads/main",
        "abcdef0123456789abcdef0123456789abcdef01",
        "bad..branch",
    ] {
        assert_eq!(
            validate("fabro-sh/fabro", branch, None).unwrap_err(),
            TargetValidationError::Branch,
            "{branch:?}"
        );
    }
    assert_eq!(
        validate("fabro-sh/fabro", "main", Some("short")).unwrap_err(),
        TargetValidationError::Sha
    );
}
