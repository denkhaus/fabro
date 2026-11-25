use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::settings::{
    AskNamespace, CliNamespace, ObjectStoreSettings, ProjectNamespace, RunNamespace,
    ServerNamespace, WorkflowNamespace,
};

/// The server's resolved settings: the `[server]` namespace and the
/// runless-chat `[ask]` namespace (work#1690). This is the payload the
/// `GET /settings` door serves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerSettings {
    pub server: ServerNamespace,
    pub ask:    AskNamespace,
}

impl ServerSettings {
    #[must_use]
    pub fn with_storage_override(mut self, path: &Path) -> Self {
        self.server.storage.root = path.display().to_string();
        override_local_object_store_root(&mut self.server.artifacts.store, path, "artifacts");
        self
    }
}

fn override_local_object_store_root(
    store: &mut ObjectStoreSettings,
    storage_root: &Path,
    domain: &str,
) {
    let ObjectStoreSettings::Local { root } = store else {
        return;
    };
    *root = storage_root
        .join("objects")
        .join(domain)
        .display()
        .to_string();
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UserSettings {
    pub cli: CliNamespace,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkflowSettings {
    pub project:  ProjectNamespace,
    pub workflow: WorkflowNamespace,
    pub run:      RunNamespace,
}

impl WorkflowSettings {
    #[must_use]
    pub fn combined_labels(&self) -> HashMap<String, String> {
        let mut labels = self.project.metadata.clone();
        labels.extend(self.workflow.metadata.clone());
        labels.extend(self.run.metadata.clone());
        labels
    }
}
