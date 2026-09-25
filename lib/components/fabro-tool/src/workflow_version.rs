use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use fabro_api::types::CreateWorkflowVersionResponse;
use fabro_types::{MAX_WORKFLOW_VERSION_BYTES, WorkflowPath};
use fabro_workflow_version::CollectedWorkflowClosure;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{FabroToolBackend, ToolError, ToolResult};

/// Caller-supplied source content, before packaging resolves workflow
/// dependencies.
///
/// The files arrive one of two ways: inline in `files`, or read from the
/// run's own sandbox when `files_from` names a workspace-relative directory
/// (fabro-4b29) — the caller passes two short strings and the tool collects
/// the closure, never transcribing file contents through tool arguments.
/// Exactly one of the two is set; [`Self::resolve`] enforces that and the
/// sandbox read's map flows through the same validation as inline files.
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FabroWorkflowVersionCreateParams {
    /// Exact package-relative key of the graph or workflow configuration file.
    #[schemars(with = "String")]
    pub entrypoint: WorkflowPath,
    /// All local dependencies, keyed by package-relative path. Values are
    /// text contents. Tool arguments arrive as an already-parsed JSON value
    /// on every production route, so duplicate keys have collapsed (last
    /// wins) before this type sees them; there is no byte-level guard to add
    /// here. Omit when `files_from` names a directory of the run's sandbox.
    #[serde(default)]
    #[schemars(with = "Option<BTreeMap<String, String>>")]
    pub files:      Option<BTreeMap<WorkflowPath, String>>,
    /// Workspace-relative directory of the run's sandbox whose tree supplies
    /// the files (for example `.fabro/workflows/develop`), for callers whose
    /// surface has the run's sandbox behind it. Keys arrive relative to the
    /// directory's parent, so `<slug>/workflow.toml` entrypoints work
    /// unchanged. Exclusive with `files`.
    #[serde(default)]
    pub files_from: Option<String>,
}

/// A supplied source tree whose entrypoint, budgets, and portable path
/// collisions have been checked, so it is safe to stage on a filesystem.
#[derive(Clone, Debug)]
pub struct ValidatedWorkflowVersionCreate {
    pub entrypoint: WorkflowPath,
    pub files:      BTreeMap<WorkflowPath, String>,
}

impl ValidatedWorkflowVersionCreate {
    /// Validate a resolved tree: the entrypoint must be among the files, the
    /// total must fit the version budget, and the keys must not collide when
    /// staged. Inline files and sandbox-collected files share this path —
    /// there is no second validation.
    pub fn new(
        entrypoint: WorkflowPath,
        files: BTreeMap<WorkflowPath, String>,
    ) -> Result<Self, ToolError> {
        fabro_types::validate_workflow_files(&entrypoint, &files)
            .map_err(|err| ToolError::message(err.to_string()))?;
        let total: usize = files.values().map(String::len).sum();
        if total > MAX_WORKFLOW_VERSION_BYTES {
            return Err(ToolError::message(format!(
                "workflow source exceeds {} MiB",
                MAX_WORKFLOW_VERSION_BYTES / (1024 * 1024)
            )));
        }
        fabro_types::validate_workflow_source_paths(files.keys())
            .map_err(|_| ToolError::message("workflow source paths collide"))?;
        Ok(Self { entrypoint, files })
    }
}

impl FabroWorkflowVersionCreateParams {
    /// Validate the inline form: `files` set, `files_from` absent.
    fn validate_exactly_one(&self) -> Result<(), ToolError> {
        match (&self.files, &self.files_from) {
            (Some(_), None) | (None, Some(_)) => Ok(()),
            (None, None) => Err(ToolError::message(
                "exactly one of `files` (inline contents) and `files_from` \
                 (a workspace-relative directory of the run's sandbox) is \
                 required; neither was set",
            )),
            (Some(_), Some(from)) => Err(ToolError::message(format!(
                "exactly one of `files` and `files_from` is required; both \
                 were set (files_from: `{from}`)"
            ))),
        }
    }

    /// Resolve the params against a sandbox-read result: when `files_from`
    /// names a directory, `sandbox_files` is the map the backend collected
    /// from the run's sandbox (keys relative to the directory's parent,
    /// contents inline) and the parsed map flows through the same validation
    /// as inline files.
    pub fn resolve(
        self,
        sandbox_files: BTreeMap<String, String>,
    ) -> Result<ValidatedWorkflowVersionCreate, ToolError> {
        self.validate_exactly_one()?;
        let files = match (self.files, &self.files_from) {
            (Some(files), _) => files,
            (None, Some(_)) => parse_sandbox_files(sandbox_files)?,
            (None, None) => unreachable!("validate_exactly_one rejected both-absent"),
        };
        ValidatedWorkflowVersionCreate::new(self.entrypoint, files)
    }
}

/// Parse a sandbox-collected map into workflow file keys. The server keys
/// every entry relative to the requested directory's parent, so the keys are
/// already package-relative; parsing enforces that shape instead of trusting
/// it.
fn parse_sandbox_files(
    sandbox_files: BTreeMap<String, String>,
) -> Result<BTreeMap<WorkflowPath, String>, ToolError> {
    if sandbox_files.is_empty() {
        return Err(ToolError::message(
            "the run sandbox's directory held no text files",
        ));
    }
    sandbox_files
        .into_iter()
        .map(|(key, contents)| {
            let path = key.parse().map_err(|err| {
                ToolError::message(format!(
                    "a sandbox file key is not a valid workflow path \
                     (`{key}`): {err}"
                ))
            })?;
            Ok((path, contents))
        })
        .collect()
}

impl TryFrom<FabroWorkflowVersionCreateParams> for ValidatedWorkflowVersionCreate {
    type Error = ToolError;

    fn try_from(params: FabroWorkflowVersionCreateParams) -> Result<Self, Self::Error> {
        params.validate_exactly_one()?;
        let files = params
            .files
            .expect("validate_exactly_one rejects a missing files map");
        Self::new(params.entrypoint, files)
    }
}

/// Application seam for packaging supplied content. The manifest crates that
/// own collection depend on this crate, so the packager is injected instead.
/// Implementations confine reads to supplied files and validate the entire
/// closure before returning.
#[async_trait]
pub trait WorkflowVersionPackager: Send + Sync {
    async fn package(
        &self,
        source: ValidatedWorkflowVersionCreate,
    ) -> anyhow::Result<CollectedWorkflowClosure>;
}

pub async fn create_workflow_version(
    backend: Arc<dyn FabroToolBackend>,
    source: ValidatedWorkflowVersionCreate,
) -> ToolResult<CreateWorkflowVersionResponse> {
    let workflow_version_id = backend
        .create_workflow_version(source)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))?;
    Ok(CreateWorkflowVersionResponse {
        workflow_version_id,
    })
}

#[must_use]
pub fn workflow_version_create_text(result: &CreateWorkflowVersionResponse) -> String {
    format!("Registered workflow version {}", result.workflow_version_id)
}

#[cfg(test)]
mod tests {
    use fabro_types::{MAX_WORKFLOW_VERSION_FILE_BYTES, MAX_WORKFLOW_VERSION_FILES};
    use serde_json::json;

    use super::*;
    use crate::fabro_client::ClientBackend;

    fn validate(value: serde_json::Value) -> ToolResult<ValidatedWorkflowVersionCreate> {
        let params: FabroWorkflowVersionCreateParams = serde_json::from_value(value).unwrap();
        ValidatedWorkflowVersionCreate::try_from(params)
    }

    #[test]
    fn workflow_version_request_rejects_unknown_fields_and_invalid_paths() {
        let valid = json!({"entrypoint": "workflow", "files": {"workflow": "digraph W {}"}});
        validate(valid.clone()).unwrap();
        for field in [
            "cwd",
            "url",
            "workflow",
            "environment",
            "parent_id",
            "workflow_dependencies",
        ] {
            let mut value = valid.clone();
            value[field] = json!("unexpected");
            assert!(serde_json::from_value::<FabroWorkflowVersionCreateParams>(value).is_err());
        }
        for path in [
            "../workflow",
            "/workflow",
            "a/../workflow",
            "a//b",
            "a\\b",
            "~/workflow",
            "",
        ] {
            for value in [
                json!({"entrypoint":path,"files":{"workflow":"x"}}),
                json!({"entrypoint":"workflow","files":{path:"x"}}),
            ] {
                assert!(serde_json::from_value::<FabroWorkflowVersionCreateParams>(value).is_err());
            }
        }
    }

    #[test]
    fn files_and_files_from_are_mutually_exclusive() {
        for (files, files_from, names_both) in [
            // neither set
            (None, None, false),
            // both set
            (
                Some(json!({"workflow": "digraph W {}"})),
                Some(json!(".fabro/workflows/develop")),
                true,
            ),
        ] {
            let mut value = json!({"entrypoint": "workflow"});
            if let Some(files) = files {
                value["files"] = files;
            }
            if let Some(from) = files_from {
                value["files_from"] = from;
            }
            let error = validate(value).unwrap_err();
            let text = error.as_str();
            assert!(text.contains("exactly one"), "got: {text}");
            if names_both {
                assert!(text.contains("both"), "got: {text}");
            }
        }
    }

    #[test]
    fn resolve_flows_sandbox_files_through_the_inline_validation() {
        fn fresh_params() -> FabroWorkflowVersionCreateParams {
            serde_json::from_value(json!({
                "entrypoint": "develop/workflow.toml",
                "files_from": ".fabro/workflows/develop"
            }))
            .unwrap()
        }

        let resolved = fresh_params()
            .resolve(BTreeMap::from([
                (
                    "develop/workflow.toml".to_string(),
                    "digraph W {}".to_string(),
                ),
                ("develop/prompts/planner.md".to_string(), "plan".to_string()),
            ]))
            .expect("the collected map is a valid closure");
        assert_eq!(resolved.files.len(), 2);
        assert_eq!(
            resolved.files.keys().next().map(WorkflowPath::as_str),
            Some("develop/prompts/planner.md"),
            "parsed keys keep map order"
        );

        // The entrypoint must still be present in the collected map.
        let error = fresh_params()
            .resolve(BTreeMap::from([(
                "develop/other.toml".to_string(),
                String::new(),
            )]))
            .unwrap_err();
        assert!(
            error.as_str().contains("develop/workflow.toml"),
            "got: {}",
            error.as_str()
        );

        // An empty collection is a routing error, not an empty closure.
        let error = fresh_params().resolve(BTreeMap::new()).unwrap_err();
        assert!(
            error.as_str().contains("no text files"),
            "got: {}",
            error.as_str()
        );

        // Keys must parse as workflow paths.
        let error = fresh_params()
            .resolve(BTreeMap::from([(
                "develop/../workflow.toml".to_string(),
                String::new(),
            )]))
            .unwrap_err();
        assert!(
            error.as_str().contains("not a valid workflow path"),
            "got: {}",
            error.as_str()
        );
    }

    #[test]
    fn workflow_version_source_enforces_presence_collisions_and_budgets() {
        assert!(
            validate(json!({"entrypoint":"missing","files":{"workflow":"digraph W {}"}})).is_err()
        );
        for files in [
            json!({"A":"x","a":"y"}),
            json!({"A":"x","a/b.md":"y"}),
            json!({"a":"x","A/b.md":"y"}),
        ] {
            let mut files = files.as_object().unwrap().clone();
            files.insert("workflow".into(), json!("digraph W {}"));
            assert!(validate(json!({"entrypoint":"workflow","files":files})).is_err());
        }
        let oversized_file = FabroWorkflowVersionCreateParams {
            entrypoint: "workflow".parse().unwrap(),
            files:      Some(BTreeMap::from([(
                "workflow".parse().unwrap(),
                "x".repeat(MAX_WORKFLOW_VERSION_FILE_BYTES + 1),
            )])),
            files_from: None,
        };
        assert!(ValidatedWorkflowVersionCreate::try_from(oversized_file).is_err());
        let mut too_many_files = FabroWorkflowVersionCreateParams {
            entrypoint: "workflow".parse().unwrap(),
            files:      Some(
                (0..MAX_WORKFLOW_VERSION_FILES)
                    .map(|i| (format!("file{i}").parse().unwrap(), String::new()))
                    .collect(),
            ),
            files_from: None,
        };
        too_many_files
            .files
            .as_mut()
            .expect("the files map is set")
            .insert(too_many_files.entrypoint.clone(), String::new());
        assert!(ValidatedWorkflowVersionCreate::try_from(too_many_files).is_err());
        let mut oversized_total = FabroWorkflowVersionCreateParams {
            entrypoint: "workflow".parse().unwrap(),
            files:      Some(
                (0..5)
                    .map(|i| {
                        (
                            format!("file{i}").parse().unwrap(),
                            "x".repeat(MAX_WORKFLOW_VERSION_FILE_BYTES),
                        )
                    })
                    .collect(),
            ),
            files_from: None,
        };
        oversized_total
            .files
            .as_mut()
            .expect("the files map is set")
            .insert(oversized_total.entrypoint.clone(), String::new());
        assert!(ValidatedWorkflowVersionCreate::try_from(oversized_total).is_err());
    }

    #[tokio::test]
    async fn workflow_version_same_run_backend_denies_before_packaging() {
        let client = fabro_client::Client::new_no_proxy("http://127.0.0.1:1").unwrap();
        let backend = ClientBackend::new(Arc::new(client))
            .with_workflow_version_packager(Arc::new(UnreachablePackager))
            .with_run_scope("01KRBZW4DW0000000000000002".parse().unwrap());
        let source =
            validate(json!({"entrypoint":"workflow","files":{"workflow":"digraph W {}"}})).unwrap();
        let error = create_workflow_version(Arc::new(backend), source)
            .await
            .unwrap_err();
        assert!(error.as_str().contains("run scope"));
    }

    struct UnreachablePackager;
    #[async_trait]
    impl WorkflowVersionPackager for UnreachablePackager {
        async fn package(
            &self,
            _: ValidatedWorkflowVersionCreate,
        ) -> anyhow::Result<CollectedWorkflowClosure> {
            panic!("scoped backend must not invoke the packager")
        }
    }
}
