use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use fabro_api::types::CreateWorkflowVersionResponse;
use fabro_types::{
    MAX_WORKFLOW_VERSION_BYTES, MAX_WORKFLOW_VERSION_FILE_BYTES, MAX_WORKFLOW_VERSION_FILES,
    WorkflowPath, WorkflowVersionId,
};
use schemars::JsonSchema;
use serde::de::{Error as _, MapAccess, Visitor};
use serde::{Deserialize, Serialize};

use crate::{FabroToolBackend, ToolError, ToolResult};

/// Caller-supplied source content, before packaging resolves workflow
/// dependencies.
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FabroWorkflowVersionCreateParams {
    /// Exact package-relative key of the graph or workflow configuration file.
    #[schemars(with = "String")]
    pub entrypoint: WorkflowPath,
    /// All local dependencies, keyed by package-relative path. Values are text
    /// contents.
    #[serde(deserialize_with = "deserialize_files")]
    #[schemars(with = "BTreeMap<String, String>")]
    pub files:      BTreeMap<WorkflowPath, String>,
}

fn deserialize_files<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<WorkflowPath, String>, D::Error> {
    struct FilesVisitor;
    impl<'de> Visitor<'de> for FilesVisitor {
        type Value = BTreeMap<WorkflowPath, String>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("workflow files with unique path keys and text contents")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut files = BTreeMap::new();
            while let Some((path, content)) = map.next_entry()? {
                if files.insert(path, content).is_some() {
                    return Err(A::Error::custom("duplicate workflow file key"));
                }
            }
            Ok(files)
        }
    }
    deserializer.deserialize_map(FilesVisitor)
}

impl FabroWorkflowVersionCreateParams {
    /// Validate the complete supplied tree before any filesystem writes.
    pub fn validate(&self) -> ToolResult<()> {
        if !self.files.contains_key(&self.entrypoint) {
            return Err(ToolError::message(
                "entrypoint must be an exact supplied file key",
            ));
        }
        if self.files.len() > MAX_WORKFLOW_VERSION_FILES {
            return Err(ToolError::message("workflow source exceeds 512 files"));
        }
        let mut total = 0;
        for content in self.files.values() {
            if content.len() > MAX_WORKFLOW_VERSION_FILE_BYTES {
                return Err(ToolError::message("workflow source file exceeds 512 KiB"));
            }
            total += content.len();
        }
        if total > MAX_WORKFLOW_VERSION_BYTES {
            return Err(ToolError::message("workflow source exceeds 2 MiB"));
        }
        fabro_types::validate_workflow_source_paths(self.files.keys())
            .map_err(|_| ToolError::message("workflow source paths collide"))?;
        Ok(())
    }
}

/// Application seam for packaging and registering content without a dependency
/// cycle. Implementations must validate before staging, confine reads to
/// supplied files, validate the entire closure before uploading, and register
/// dependencies first.
#[async_trait]
pub trait WorkflowVersionCreateAdapter: Send + Sync {
    async fn create_workflow_version(
        &self,
        params: FabroWorkflowVersionCreateParams,
        client: &fabro_client::Client,
    ) -> anyhow::Result<WorkflowVersionId>;
}

pub async fn create_workflow_version(
    backend: Arc<dyn FabroToolBackend>,
    params: FabroWorkflowVersionCreateParams,
) -> ToolResult<CreateWorkflowVersionResponse> {
    params.validate()?;
    let workflow_version_id = backend
        .create_workflow_version(params)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))?;
    Ok(CreateWorkflowVersionResponse {
        workflow_version_id,
    })
}

#[must_use]
pub fn workflow_version_create_text(result: &CreateWorkflowVersionResponse) -> String {
    serde_json::to_string(result).expect("workflow version response should serialize")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fabro_client::ClientBackend;

    #[test]
    fn workflow_version_request_rejects_unknown_fields_and_invalid_paths() {
        let valid = json!({"entrypoint": "workflow", "files": {"workflow": "digraph W {}"}});
        let params: FabroWorkflowVersionCreateParams =
            serde_json::from_value(valid.clone()).unwrap();
        params.validate().unwrap();
        assert!(
            serde_json::from_str::<FabroWorkflowVersionCreateParams>(
                r#"{"entrypoint":"workflow","files":{"workflow":"a","workflow":"b"}}"#
            )
            .is_err()
        );
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
    fn workflow_version_source_enforces_presence_collisions_and_budgets() {
        for files in [
            json!({}),
            json!({"A":"x","a":"y"}),
            json!({"A":"x","a/b.md":"y"}),
            json!({"a":"x","A/b.md":"y"}),
        ] {
            let mut files = files.as_object().unwrap().clone();
            files.insert("workflow".into(), json!("digraph W {}"));
            let mut params: FabroWorkflowVersionCreateParams =
                serde_json::from_value(json!({"entrypoint":"workflow","files":files})).unwrap();
            if params.files.len() == 1 {
                params.entrypoint = "missing".parse().unwrap();
            }
            assert!(params.validate().is_err());
        }
        let mut params = FabroWorkflowVersionCreateParams {
            entrypoint: "workflow".parse().unwrap(),
            files:      BTreeMap::from([(
                "workflow".parse().unwrap(),
                "x".repeat(MAX_WORKFLOW_VERSION_FILE_BYTES + 1),
            )]),
        };
        assert!(params.validate().is_err());
        params.files = (0..MAX_WORKFLOW_VERSION_FILES)
            .map(|i| (format!("file{i}").parse().unwrap(), String::new()))
            .collect();
        params
            .files
            .insert(params.entrypoint.clone(), String::new());
        assert!(params.validate().is_err());
        params.files = (0..5)
            .map(|i| {
                (
                    format!("file{i}").parse().unwrap(),
                    "x".repeat(MAX_WORKFLOW_VERSION_FILE_BYTES),
                )
            })
            .collect();
        params
            .files
            .insert(params.entrypoint.clone(), String::new());
        assert!(params.validate().is_err());
    }

    #[tokio::test]
    async fn workflow_version_same_run_backend_denies_before_adapter() {
        let client = fabro_client::Client::new_no_proxy("http://127.0.0.1:1").unwrap();
        let backend = ClientBackend::new(Arc::new(client))
            .with_workflow_version_create_adapter(Arc::new(UnreachableAdapter))
            .with_run_scope("01KRBZW4DW0000000000000002".parse().unwrap());
        let params = serde_json::from_value(
            json!({"entrypoint":"workflow","files":{"workflow":"digraph W {}"}}),
        )
        .unwrap();
        let error = create_workflow_version(Arc::new(backend), params)
            .await
            .unwrap_err();
        assert!(error.as_str().contains("run scope"));
    }

    struct UnreachableAdapter;
    #[async_trait]
    impl WorkflowVersionCreateAdapter for UnreachableAdapter {
        async fn create_workflow_version(
            &self,
            _: FabroWorkflowVersionCreateParams,
            _: &fabro_client::Client,
        ) -> anyhow::Result<WorkflowVersionId> {
            panic!("scoped backend must not invoke the adapter")
        }
    }
}
