use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::BlobHash;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct WorkflowVersionId(BlobHash);

impl From<BlobHash> for WorkflowVersionId {
    fn from(value: BlobHash) -> Self {
        Self(value)
    }
}

impl From<WorkflowVersionId> for BlobHash {
    fn from(value: WorkflowVersionId) -> Self {
        value.0
    }
}

impl fmt::Display for WorkflowVersionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl From<WorkflowVersionId> for String {
    fn from(value: WorkflowVersionId) -> Self {
        value.to_string()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
#[error("workflow version ID must be exactly 64 hexadecimal characters")]
pub struct WorkflowVersionIdParseError;

impl FromStr for WorkflowVersionId {
    type Err = WorkflowVersionIdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value
            .parse::<BlobHash>()
            .map(Self)
            .map_err(|_| WorkflowVersionIdParseError)
    }
}

impl TryFrom<String> for WorkflowVersionId {
    type Error = WorkflowVersionIdParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

#[cfg(test)]
mod tests {
    use crate::{BlobHash, WorkflowVersionId};

    #[test]
    fn conversion_preserves_digest_and_display() {
        let blob_hash = BlobHash::new(b"workflow");
        let version_id = WorkflowVersionId::from(blob_hash);
        assert_eq!(version_id.to_string(), blob_hash.to_string());
        assert_eq!(BlobHash::from(version_id), blob_hash);
    }

    #[test]
    fn parse_accepts_any_case_and_serializes_lowercase() {
        let value = BlobHash::new(b"workflow").to_string();
        let id: WorkflowVersionId = value.parse().unwrap();
        assert_eq!(serde_json::to_value(id).unwrap(), value);
        assert_eq!(
            value.to_uppercase().parse::<WorkflowVersionId>().unwrap(),
            id
        );
        for invalid in [
            String::new(),
            "0".repeat(63),
            "0".repeat(65),
            "g".repeat(64),
        ] {
            assert!(invalid.parse::<WorkflowVersionId>().is_err());
        }
        assert_eq!(
            serde_json::from_value::<WorkflowVersionId>(serde_json::json!(value.to_uppercase()))
                .unwrap()
                .to_string(),
            value
        );
    }
}
