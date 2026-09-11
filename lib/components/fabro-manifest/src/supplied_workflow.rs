//! Package workflow versions from caller-supplied file contents instead of a
//! checkout on disk.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use fabro_config::project::WorkflowLocation;
use fabro_types::WorkflowPath;
use tempfile::TempDir;

use crate::CollectedWorkflowClosure;

/// Stage `files` in a private temporary directory and collect the workflow
/// closure rooted at `entrypoint` with the same collector used for checkouts.
/// Only supplied files can satisfy references; the staging directory is
/// removed on every return path. Every dependency is validated before this
/// returns and nothing is registered.
pub fn collect_supplied_workflow_versions(
    entrypoint: &WorkflowPath,
    files: &BTreeMap<WorkflowPath, String>,
) -> Result<CollectedWorkflowClosure> {
    let staging = tempfile::Builder::new()
        .prefix("fabro-workflow-version-")
        .tempdir()?;
    collect_in_staging(entrypoint, files, &staging)
}

fn collect_in_staging(
    entrypoint: &WorkflowPath,
    files: &BTreeMap<WorkflowPath, String>,
    staging: &TempDir,
) -> Result<CollectedWorkflowClosure> {
    let root = staging.path().canonicalize()?;
    for (path, contents) in files {
        let destination = root.join(path.as_str());
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(destination, contents)?;
    }
    let entrypoint = Path::new(entrypoint.as_str());
    let location = WorkflowLocation::from_exact_path(entrypoint, &root)?;
    let closure = crate::collect_workflow_versions_at_location(&location, &root, entrypoint)?;
    // A case-insensitive host must not satisfy a reference that is missing
    // from the supplied tree under its exact key.
    for (_, version) in closure.versions() {
        for path in version.version().files().keys() {
            anyhow::ensure!(
                files.contains_key(path),
                "collected file `{path}` was not supplied"
            );
        }
    }
    Ok(closure)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    struct Supplied {
        entrypoint: WorkflowPath,
        files:      BTreeMap<WorkflowPath, String>,
    }

    fn supplied(entrypoint: &str, files: &[(&str, &str)]) -> Supplied {
        Supplied {
            entrypoint: entrypoint.parse().unwrap(),
            files:      files
                .iter()
                .map(|(path, content)| (path.parse().unwrap(), (*content).to_string()))
                .collect(),
        }
    }

    fn fixture() -> Supplied {
        supplied("workflow.toml", &[
            (
                "workflow.toml",
                "_version = 1\n[workflow]\ngraph = \"workflow.fabro\"\n",
            ),
            (
                "workflow.fabro",
                r#"digraph W { p [prompt="@prompt.md"] child [stack.child_workflow="child.fabro"] }"#,
            ),
            (
                "prompt.md",
                "Keep {{ secrets.TEST }} and {{ env.TEST }} for runtime.",
            ),
            ("child.fabro", "digraph Child {}"),
        ])
    }

    fn collect(input: &Supplied) -> CollectedWorkflowClosure {
        collect_supplied_workflow_versions(&input.entrypoint, &input.files).unwrap()
    }

    /// Consumes `staging` so the tests can assert cleanup after return.
    fn collect_with_staging(
        input: &Supplied,
        staging: TempDir,
    ) -> Result<CollectedWorkflowClosure> {
        let result = collect_in_staging(&input.entrypoint, &input.files, &staging);
        drop(staging);
        result
    }

    #[test]
    fn supplied_content_matches_checkout_collector_and_cleans_staging() {
        for input in [
            supplied("workflow.fabro", &[("workflow.fabro", "digraph W {}")]),
            fixture(),
        ] {
            let source = tempfile::tempdir().unwrap();
            for (path, content) in &input.files {
                std::fs::write(source.path().join(path.as_str()), content).unwrap();
            }
            let expected = crate::collect_workflow_versions(
                Path::new(input.entrypoint.as_str()),
                source.path(),
            )
            .unwrap();
            let staging = tempfile::tempdir().unwrap();
            let path = staging.path().to_owned();
            let actual = collect_with_staging(&input, staging).unwrap();
            assert!(!path.exists());
            assert_eq!(actual.root_id(), expected.root_id());
            assert_eq!(
                actual
                    .versions()
                    .map(|(_, v)| v.version())
                    .collect::<Vec<_>>(),
                expected
                    .versions()
                    .map(|(_, v)| v.version())
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn exact_extensionless_entrypoint_and_child_ignore_selectors() {
        let input = supplied("workflow", &[
            (
                "workflow",
                r#"digraph W { child [stack.child_workflow="child"] }"#,
            ),
            ("child", "digraph Child {}"),
            (
                ".fabro/project.toml",
                "malformed project config must not be read",
            ),
            (
                ".fabro/workflows/workflow/workflow.toml",
                "misleading named workflow",
            ),
        ]);
        let closure = collect(&input);
        let versions = closure.versions().collect::<Vec<_>>();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[1].1.version().entrypoint().as_str(), "workflow");
        assert_eq!(versions[0].1.version().entrypoint().as_str(), "child");
    }

    #[test]
    fn rejects_missing_and_escaping_references_and_cleans_failure() {
        let parent = tempfile::tempdir().unwrap();
        std::fs::write(
            parent.path().join("outside.md"),
            "host content must never satisfy a reference",
        )
        .unwrap();
        std::fs::write(parent.path().join("child.fabro"), "digraph Host {}").unwrap();
        // Malformed on purpose: a parser that reaches this file would quote it.
        std::fs::write(
            parent.path().join("secret.toml"),
            "HOST_SECRET = [unterminated",
        )
        .unwrap();
        std::fs::write(
            parent.path().join("workflow.toml"),
            "HOST_SECRET = [unterminated",
        )
        .unwrap();
        for (index, input) in [
            supplied("workflow.fabro", &[
                ("workflow.fabro", r#"digraph W { p [prompt="@prompt.md"] }"#),
                ("Prompt.md", "wrong case"),
            ]),
            supplied("workflow.fabro", &[(
                "workflow.fabro",
                r#"digraph W { p [prompt="@../outside.md"] }"#,
            )]),
            supplied("workflow.fabro", &[(
                "workflow.fabro",
                r#"digraph W { p [prompt="@sub/../../outside.md"] }"#,
            )]),
            supplied("workflow.fabro", &[(
                "workflow.fabro",
                r#"digraph W { p [prompt="@outside.md"] }"#,
            )]),
            supplied("workflow.fabro", &[(
                "workflow.fabro",
                r#"digraph W { p [output_schema="@../outside.md"] }"#,
            )]),
            supplied("workflow.fabro", &[(
                "workflow.fabro",
                r#"digraph W { p [stack.child_workflow="../child.fabro"] }"#,
            )]),
            supplied("workflow.fabro", &[(
                "workflow.fabro",
                r#"digraph W { p [stack.child_workflow="sub/../../child.fabro"] }"#,
            )]),
            supplied("workflow.fabro", &[(
                "workflow.fabro",
                r#"digraph W { p [stack.child_workflow="../secret.toml"] }"#,
            )]),
            supplied("workflow.fabro", &[(
                "workflow.fabro",
                r#"digraph W { p [stack.child_workflow="../graph.fabro"] }"#,
            )]),
            supplied("workflow.fabro", &[(
                "workflow.fabro",
                r#"digraph W { p [stack.child_workflow="missing"] }"#,
            )]),
            supplied("workflow.toml", &[(
                "workflow.toml",
                "_version = 1\n[workflow]\ngraph = \"../child.fabro\"\n",
            )]),
            supplied("workflow.fabro", &[(
                "workflow.fabro",
                "invalid source PRIVATE_CONTENT",
            )]),
        ]
        .into_iter()
        .enumerate()
        {
            let staging = tempfile::tempdir_in(parent.path()).unwrap();
            let path = staging.path().to_owned();
            let error = collect_with_staging(&input, staging)
                .err()
                .unwrap_or_else(|| panic!("accepted invalid fixture {index}"));
            let rendered = format!("{error:#}");
            // Escaping references must fail before any host file is opened,
            // so no host diagnostic (parse error, exists-vs-missing) leaks.
            assert!(
                !rendered.contains("HOST_SECRET") && !rendered.contains("secret.toml:"),
                "fixture {index} read a host file: {rendered}"
            );
            assert!(!path.exists());
        }
    }

    #[test]
    fn preserves_literal_scripts_without_executing() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("must-not-exist");
        // Script is literal command text in Fabro, not an @file import.
        let graph = format!(
            "digraph W {{ command [script=\"touch {}\"] }}",
            marker.display()
        );
        let input = supplied("workflow", &[("workflow", &graph)]);
        let closure = collect(&input);
        let root = closure.versions().last().unwrap().1.version();
        assert_eq!(root.files()[&"workflow".parse().unwrap()], graph);
        assert!(!marker.exists());
    }

    #[test]
    fn map_order_is_irrelevant_and_reachable_changes_change_ids() {
        let input = fixture();
        let first = collect(&input);
        let mut reordered = Supplied {
            entrypoint: input.entrypoint.clone(),
            files:      input.files.into_iter().rev().collect(),
        };
        assert_eq!(first.root_id(), collect(&reordered).root_id());
        reordered
            .files
            .insert("prompt.md".parse().unwrap(), "changed".into());
        let changed = collect(&reordered);
        assert_ne!(first.root_id(), changed.root_id());
        assert_eq!(
            first.versions().next().unwrap().0,
            changed.versions().next().unwrap().0
        );
        reordered
            .files
            .insert("child.fabro".parse().unwrap(), "digraph Changed {}".into());
        let changed_child = collect(&reordered);
        assert_ne!(changed.root_id(), changed_child.root_id());
        assert_ne!(
            changed.versions().next().unwrap().0,
            changed_child.versions().next().unwrap().0
        );
    }
}
