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
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FabroWorkflowVersionCreateParams {
    /// Exact package-relative key of the graph or workflow configuration
    /// file, including its workflow directory (`<slug>/workflow.toml`). A
    /// bare filename is rejected: runs derive their workflow slug from the
    /// entrypoint's parent directory.
    #[schemars(with = "String")]
    pub entrypoint: WorkflowPath,
    /// All local dependencies, keyed by package-relative path. Values are
    /// text contents. Tool arguments arrive as an already-parsed JSON value
    /// on every production route, so duplicate keys have collapsed (last
    /// wins) before this type sees them; there is no byte-level guard to
    /// add here. Optional ONLY together with `files_from` (the run sandbox
    /// supplies the contents); inline and sandbox sources are mutually
    /// exclusive.
    #[serde(default)]
    #[schemars(with = "BTreeMap<String, String>")]
    pub files:      BTreeMap<WorkflowPath, String>,
    /// Workspace-relative directory whose contents become the file map —
    /// the run sandbox reads the closure, so the model never transcribes
    /// file contents through tool arguments (the 8-minute, 76k-output-token
    /// survey cost of passes like 01M2GJXV71TD). File keys are the paths
    /// RELATIVE TO THE DIRECTORY'S PARENT, so `.fabro/workflows/develop`
    /// yields the mandated `develop/…` keys. Requires a run sandbox; the
    /// MCP surface and ask-fabro sessions support inline `files` only.
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub files_from: Option<WorkflowPath>,
}

/// A supplied source tree whose entrypoint, budgets, and portable path
/// collisions have been checked, so it is safe to stage on a filesystem.
#[derive(Clone, Debug)]
pub struct ValidatedWorkflowVersionCreate {
    pub entrypoint: WorkflowPath,
    pub files:      BTreeMap<WorkflowPath, String>,
}

/// The entrypoint must name its parent directory: a run created from the
/// version derives its workflow slug from that directory, and a bare
/// conventional name (`workflow.toml`) collapses the slug to the ambiguous
/// fallback `workflow` (fabro-9cb0).
/// `<slug>`-style placeholders pass path validation, so a verbatim copy from
/// documentation would register a version whose runs carry the literal slug
/// `<slug>`. Angle brackets never occur in real workflow directories, so they
/// are rejected loudly instead.
fn looks_like_placeholder(path: &WorkflowPath) -> bool {
    path.as_str().contains('<') || path.as_str().contains('>')
}

fn has_directory_component(path: &WorkflowPath) -> bool {
    std::path::Path::new(path.as_str())
        .parent()
        .is_some_and(|parent| !parent.as_os_str().is_empty())
}

impl TryFrom<FabroWorkflowVersionCreateParams> for ValidatedWorkflowVersionCreate {
    type Error = ToolError;

    fn try_from(params: FabroWorkflowVersionCreateParams) -> Result<Self, Self::Error> {
        let FabroWorkflowVersionCreateParams {
            entrypoint,
            files,
            files_from,
        } = params;
        if files_from.is_some() {
            return Err(ToolError::message(
                "files_from requires this run's sandbox filesystem; only the in-run \
                 fabro_workflow_version_create tool resolves it (inline `files` here)",
            ));
        }
        fabro_types::validate_workflow_files(&entrypoint, &files)
            .map_err(|err| ToolError::message(err.to_string()))?;
        if looks_like_placeholder(&entrypoint) || files.keys().any(looks_like_placeholder) {
            return Err(ToolError::message(
                "workflow version paths contain a placeholder (<...>): replace it with the \
                 workflow's actual directory name before registering",
            ));
        }
        if !has_directory_component(&entrypoint) {
            return Err(ToolError::message(format!(
                "entrypoint `{ep}` has no directory component: runs derive their workflow slug \
                 from the entrypoint's parent directory, so a bare name collapses every run to \
                 the slug \"workflow\" and hides it from workflow=<slug> filters. Collect the \
                 closure from the repository root and prefix the entrypoint and every file key \
                 with the workflow's directory (`<slug>/workflow.toml`)",
                ep = entrypoint.as_str()
            )));
        }
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

/// Workspace filesystem access for `files_from` registration: the run
/// sandbox supplies the closure so the model never transcribes file
/// contents through tool arguments. Implemented by the workflow engine
/// over its sandbox handle with the stage's `fs_hide` policy applied.
#[async_trait]
pub trait WorkflowFilesSource: Send + Sync {
    /// Sorted, workspace-relative paths of the regular text files under
    /// `dir` (also workspace-relative), one entry per file.
    async fn list_text_files(&self, dir: &str) -> Result<Vec<String>, ToolError>;

    /// Reads one workspace-relative text file.
    async fn read_text_file(&self, path: &str) -> Result<String, ToolError>;
}

/// Resolves `files_from` into an inline file map through `source`: every
/// listed file becomes `<dir-basename>/<relative-path>` — the directory
/// name is part of every key, so the mandated `<slug>/` prefix (and with
/// it the run's workflow slug) survives without any transcription.
///
/// # Errors
///
/// Budget and UTF-8 failures name the offending path.
pub async fn expand_files_from(
    source: &dyn WorkflowFilesSource,
    dir: &WorkflowPath,
) -> Result<BTreeMap<WorkflowPath, String>, ToolError> {
    let dir_str = dir.as_str();
    let prefix = std::path::Path::new(dir_str)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| unreachable!("a validated WorkflowPath always has a final component"));
    let mut files = BTreeMap::new();
    for path in source.list_text_files(dir_str).await? {
        let relative = path.strip_prefix(&format!("{dir_str}/")).unwrap_or(&path);
        let key: WorkflowPath = format!("{prefix}/{relative}").parse().map_err(|error| {
            ToolError::message(format!("files_from produced `{relative}`: {error}"))
        })?;
        if files.len() >= fabro_types::MAX_WORKFLOW_VERSION_FILES {
            return Err(ToolError::message(format!(
                "files_from exceeds {} files",
                fabro_types::MAX_WORKFLOW_VERSION_FILES
            )));
        }
        let contents = source.read_text_file(&path).await?;
        if contents.len() > fabro_types::MAX_WORKFLOW_VERSION_FILE_BYTES {
            return Err(ToolError::message(format!(
                "files_from file `{key}` exceeds {} bytes",
                fabro_types::MAX_WORKFLOW_VERSION_FILE_BYTES
            )));
        }
        files.insert(key, contents);
    }
    if files.is_empty() {
        return Err(ToolError::message(format!(
            "files_from directory `{dir_str}` contains no files"
        )));
    }
    Ok(files)
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
        let valid =
            json!({"entrypoint": "demo/workflow", "files": {"demo/workflow": "digraph W {}"}});
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
                json!({"entrypoint":"demo/workflow","files":{path:"x"}}),
            ] {
                assert!(serde_json::from_value::<FabroWorkflowVersionCreateParams>(value).is_err());
            }
        }
    }

    #[test]
    fn workflow_version_source_enforces_presence_collisions_and_budgets() {
        assert!(
            validate(json!({"entrypoint":"demo/missing","files":{"demo/workflow":"digraph W {}"}}))
                .is_err()
        );
        for files in [
            json!({"A":"x","a":"y"}),
            json!({"A":"x","a/b.md":"y"}),
            json!({"a":"x","A/b.md":"y"}),
        ] {
            let mut files = files.as_object().unwrap().clone();
            files.insert("demo/workflow".into(), json!("digraph W {}"));
            assert!(validate(json!({"entrypoint":"demo/workflow","files":files})).is_err());
        }
        let oversized_file = FabroWorkflowVersionCreateParams {
            files_from: None,
            entrypoint: "demo/workflow".parse().unwrap(),
            files:      BTreeMap::from([(
                "demo/workflow".parse().unwrap(),
                "x".repeat(MAX_WORKFLOW_VERSION_FILE_BYTES + 1),
            )]),
        };
        assert!(ValidatedWorkflowVersionCreate::try_from(oversized_file).is_err());
        let mut too_many_files = FabroWorkflowVersionCreateParams {
            files_from: None,
            entrypoint: "demo/workflow".parse().unwrap(),
            files:      (0..MAX_WORKFLOW_VERSION_FILES)
                .map(|i| (format!("file{i}").parse().unwrap(), String::new()))
                .collect(),
        };
        too_many_files
            .files
            .insert(too_many_files.entrypoint.clone(), String::new());
        assert!(ValidatedWorkflowVersionCreate::try_from(too_many_files).is_err());
        let mut oversized_total = FabroWorkflowVersionCreateParams {
            files_from: None,
            entrypoint: "demo/workflow".parse().unwrap(),
            files:      (0..5)
                .map(|i| {
                    (
                        format!("file{i}").parse().unwrap(),
                        "x".repeat(MAX_WORKFLOW_VERSION_FILE_BYTES),
                    )
                })
                .collect(),
        };
        oversized_total
            .files
            .insert(oversized_total.entrypoint.clone(), String::new());
        assert!(ValidatedWorkflowVersionCreate::try_from(oversized_total).is_err());
    }

    #[test]
    fn workflow_version_request_rejects_placeholder_paths() {
        // Documentation uses `<slug>/…` placeholders; a verbatim copy must
        // fail loudly instead of registering runs under the literal slug
        // `<slug>`.
        for placeholder in ["<slug>/workflow.toml", "<folder>/graph.fabro"] {
            let error =
                validate(json!({"entrypoint":placeholder,"files":{placeholder:"digraph W {}"}}))
                    .expect_err("a placeholder path must be rejected");
            assert!(error.as_str().contains("placeholder"), "{}", error.as_str());
        }
        let error = validate(
            json!({"entrypoint":"demo/workflow","files":{"demo/workflow":"digraph W {}","<slug>/prompt.md":"x"}}),
        )
        .expect_err("placeholder file keys must be rejected too");
        assert!(error.as_str().contains("placeholder"));
    }

    #[test]
    fn workflow_version_request_rejects_entrypoint_without_directory() {
        // fabro-9cb0: a bare entrypoint collapses every derived run slug to
        // the ambiguous fallback "workflow", so registration must reject it
        // with the corrective teaching in the error text.
        for bare in ["workflow.toml", "workflow.fabro", "workflow", "graph.dot"] {
            let error = validate(json!({"entrypoint":bare,"files":{bare:"digraph W {}"}}))
                .expect_err("a bare entrypoint must be rejected");
            let message = error.as_str();
            assert!(message.contains("no directory component"), "{message}");
            assert!(message.contains("workflow=<slug>"), "{message}");
            assert!(message.contains(bare), "{message}");
        }
        for prefixed in ["develop/workflow.toml", "demo/workflow", "a/b/graph.dot"] {
            validate(json!({"entrypoint":prefixed,"files":{prefixed:"digraph W {}"}}))
                .unwrap_or_else(|error| panic!("dir-prefixed entrypoint must pass: {error}"));
        }
    }

    struct FakeSandboxFiles {
        files: Vec<(&'static str, &'static str)>,
    }

    #[async_trait]
    impl WorkflowFilesSource for FakeSandboxFiles {
        async fn list_text_files(&self, dir: &str) -> Result<Vec<String>, ToolError> {
            Ok(self
                .files
                .iter()
                .filter(|(path, _)| path.starts_with(&format!("{dir}/")))
                .map(|(path, _)| (*path).to_string())
                .collect())
        }

        async fn read_text_file(&self, path: &str) -> Result<String, ToolError> {
            self.files
                .iter()
                .find(|(candidate, _)| *candidate == path)
                .map(|(_, contents)| (*contents).to_string())
                .ok_or_else(|| ToolError::message(format!("no such file `{path}`")))
        }
    }

    #[tokio::test]
    async fn files_from_expands_to_slug_prefixed_keys() {
        let source = FakeSandboxFiles {
            files: vec![
                (".fabro/workflows/develop/workflow.toml", "_version = 1"),
                (".fabro/workflows/develop/workflow.fabro", "digraph D {}"),
                (".fabro/workflows/develop/prompts/plan.md", "plan"),
            ],
        };
        let files = expand_files_from(&source, &".fabro/workflows/develop".parse().unwrap())
            .await
            .unwrap();
        let keys: Vec<&str> = files.keys().map(WorkflowPath::as_str).collect();
        assert_eq!(keys, [
            "develop/prompts/plan.md",
            "develop/workflow.fabro",
            "develop/workflow.toml"
        ]);
        // The expanded map satisfies inline validation unchanged: the
        // entrypoint resolves and the 9cb0 directory guard passes.
        validate(json!({"entrypoint":"develop/workflow.toml","files":files})).unwrap();
    }

    #[tokio::test]
    async fn files_from_empty_directory_is_an_error() {
        let source = FakeSandboxFiles { files: vec![] };
        let error = expand_files_from(&source, &".fabro/workflows/develop".parse().unwrap())
            .await
            .unwrap_err();
        assert!(error.as_str().contains("contains no files"));
    }

    #[test]
    fn files_from_without_a_sandbox_refuses_with_teaching() {
        let error = validate(
            json!({"entrypoint":"develop/workflow.toml","files_from":".fabro/workflows/develop"}),
        )
        .unwrap_err();
        assert!(error.as_str().contains("requires this run's sandbox"));
    }

    #[tokio::test]
    async fn workflow_version_same_run_backend_denies_before_packaging() {
        let client = fabro_client::Client::new_no_proxy("http://127.0.0.1:1").unwrap();
        let backend = ClientBackend::new(Arc::new(client))
            .with_workflow_version_packager(Arc::new(UnreachablePackager))
            .with_run_scope("01KRBZW4DW0000000000000002".parse().unwrap());
        let source = validate(
            json!({"entrypoint":"demo/workflow","files":{"demo/workflow":"digraph W {}"}}),
        )
        .unwrap();
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
