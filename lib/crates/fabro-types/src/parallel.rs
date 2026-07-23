use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParallelBranchResult {
    pub id:              String,
    pub status:          String,
    pub context_updates: BTreeMap<String, Value>,
}
