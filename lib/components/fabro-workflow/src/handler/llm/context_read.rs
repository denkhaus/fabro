//! The `context_read` stage tool (fabro-e804, ADR-0009 pull axis).
//!
//! Agent stages get read-only pull access to workflow context values by
//! key. The readable set is the node's window on the context: engine
//! keys are never served, and `preamble_allow_keys` gates pull access
//! exactly as it gates preamble rendering. Values over the stage's
//! inline budget return the same preview-plus-path marker the preamble
//! demote pass produces, so the tool reuses [`crate::artifact`]'s
//! materialization instead of inventing a second mechanism.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use fabro_agent::Sandbox;
use fabro_agent::tool_registry::{RegisteredTool, ToolContext, ToolSource};
use fabro_graphviz::graph::{Graph, Node};
use fabro_llm::types::ToolDefinition as LlmToolDefinition;
use fabro_types::graph::ATTR_LIST_WILDCARD;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::artifact;
use crate::context::{Context, keys};
use crate::error::Result;
use crate::runtime_store::RunStoreHandle;

/// Canonical name of the stage context-pull tool.
pub const CONTEXT_READ_TOOL_NAME: &str = "context_read";

/// Stage tools this module registers beyond fabro-agent's native set.
/// The validate crate's catalog drift test cross-checks against this list.
#[must_use]
pub fn workflow_tool_names() -> &'static [&'static str] {
    &[CONTEXT_READ_TOOL_NAME]
}

const CONTEXT_READ_TOOL_DESCRIPTION: &str = "Read one workflow context value by key, read-only. \
Values over the stage's inline budget return a preview plus a sandbox file path instead of the \
full value.";

/// Cap on how many key names the unknown-key error lists, keeping tool
/// errors bounded on contexts with very many keys.
const UNKNOWN_KEY_LIST_MAX: usize = 20;

const VIEW_LOCK_POISONED: &str =
    "context_read view lock is never poisoned: no code panics while holding it";

/// The per-node set of context values the tool may serve.
///
/// Built once per stage execution from the resolved context the handler
/// receives; sharing it with the registered tool freezes the node's view
/// for the duration of its session (deterministic input contract:
/// parallel-branch updates that land mid-run are not visible).
#[derive(Clone, Debug)]
pub(crate) struct ContextReadView {
    values:     HashMap<String, Value>,
    inline_max: usize,
}

impl ContextReadView {
    /// Filter `context` down to the keys this node may pull.
    ///
    /// Engine plumbing keys ([`keys::is_preamble_hidden_key`]) are removed
    /// first and cannot be re-admitted by an allowlist entry. A set
    /// `preamble_allow_keys` then narrows the window to the listed keys,
    /// with the lone `*` keeping the default-open posture.
    #[must_use]
    pub(crate) fn new(context: &Context, node: &Node, graph: &Graph) -> Self {
        let mut values = context.snapshot();
        values.retain(|key, _| !keys::is_preamble_hidden_key(key));
        if let Some(allow) = node.preamble_allow_keys() {
            if !allow.contains(&ATTR_LIST_WILDCARD) {
                values.retain(|key, _| allow.contains(&key.as_str()));
            }
        }
        Self {
            values,
            inline_max: artifact::resolve_inline_max_bytes(node, graph),
        }
    }
}

/// Run-scoped materialization inputs plus the per-node view.
///
/// Flows through the stage request to the session factory; the run-scoped
/// parts are identical across stages of one run, the view is per node.
#[derive(Clone)]
pub struct ContextReadServices {
    run_store: RunStoreHandle,
    sandbox:   Arc<dyn Sandbox>,
    run_dir:   PathBuf,
    view:      ContextReadView,
}

impl ContextReadServices {
    #[must_use]
    pub fn new(
        context: &Context,
        node: &Node,
        graph: &Graph,
        run_store: RunStoreHandle,
        sandbox: Arc<dyn Sandbox>,
        run_dir: PathBuf,
    ) -> Self {
        Self {
            run_store,
            sandbox,
            run_dir,
            view: ContextReadView::new(context, node, graph),
        }
    }
}

/// Shared, refreshable state captured by the registered tool executor.
///
/// The view swaps per node execution (a full-fidelity session is reused
/// across nodes on one thread); the run-scoped materialization inputs are
/// stable for the run's lifetime.
pub(crate) struct ContextReadState {
    view:      RwLock<ContextReadView>,
    run_store: RunStoreHandle,
    sandbox:   Arc<dyn Sandbox>,
    run_dir:   PathBuf,
    locality:  Mutex<artifact::SandboxLocality>,
}

impl ContextReadState {
    pub(crate) fn new(services: ContextReadServices) -> Self {
        let ContextReadServices {
            run_store,
            sandbox,
            run_dir,
            view,
        } = services;
        Self {
            view: RwLock::new(view),
            run_store,
            sandbox,
            run_dir,
            locality: Mutex::new(artifact::SandboxLocality::default()),
        }
    }

    /// Replace the served view for the node about to execute.
    pub(crate) fn update(&self, services: &ContextReadServices) {
        *self.view.write().expect(VIEW_LOCK_POISONED) = services.view.clone();
    }

    /// Look up one key under the current view, in a single read-lock hold.
    ///
    /// Returns the cloned value plus the stage's inline budget, or the
    /// model-facing unknown-key error naming the available keys.
    fn fetch(&self, key: &str) -> std::result::Result<(Value, usize), String> {
        let view = self.view.read().expect(VIEW_LOCK_POISONED);
        match view.values.get(key) {
            Some(value) => Ok((value.clone(), view.inline_max)),
            None => Err(unknown_key_error(key, &view.values)),
        }
    }

    /// Serve one fetched value: full JSON, or the demote marker when the
    /// serialized value exceeds the stage's inline budget.
    ///
    /// Blob references surviving the dispatch-time resolution pass (the
    /// handler's context is normally already rehydrated by
    /// `resolve_context_for_execution`) are hydrated first, so the model
    /// never receives a bare `blob://` pointer it cannot dereference
    /// (key-only input). No view lock is held; the locality lock spans the
    /// materialization awaits because it memoizes sandbox probes.
    async fn serve(&self, mut value: Value, inline_max: usize) -> Result<String> {
        value = artifact::resolve_json_value(value, &self.run_store).await?;
        let mut locality = self.locality.lock().await;
        artifact::demote_value_for_prompt(
            &mut value,
            inline_max,
            &self.run_store,
            self.sandbox.as_ref(),
            &self.run_dir,
            &mut locality,
        )
        .await?;
        Ok(match value {
            // Bare text for strings: JSON quoting a plain string adds noise
            // without adding information; every other shape stays JSON.
            Value::String(text) => text,
            other => other.to_string(),
        })
    }
}

/// Build the registered `context_read` tool closing over `state`.
pub(crate) fn context_read_tool(state: Arc<ContextReadState>) -> RegisteredTool {
    RegisteredTool {
        definition: LlmToolDefinition {
            name:        CONTEXT_READ_TOOL_NAME.to_string(),
            description: CONTEXT_READ_TOOL_DESCRIPTION.to_string(),
            parameters:  serde_json::json!({
                "type": "object",
                "properties": {
                    "key": {
                        "type": "string",
                        "description": "Context key to read, e.g. 'plan.outline'"
                    }
                },
                "required": ["key"],
                "additionalProperties": false,
            }),
        },
        executor:   Arc::new(move |args, _context: ToolContext| {
            let state = Arc::clone(&state);
            Box::pin(async move { execute_context_read(args, state).await })
        }),
        source:     ToolSource::Native,
    }
}

async fn execute_context_read(
    args: Value,
    state: Arc<ContextReadState>,
) -> std::result::Result<String, String> {
    let Some(key) = args.get("key").and_then(Value::as_str) else {
        return Err("context_read requires a string 'key' argument".to_string());
    };
    let (value, inline_max) = state.fetch(key)?;
    state
        .serve(value, inline_max)
        .await
        .map_err(|err| err.to_string())
}

/// Model-facing error naming the requested key and the available ones.
fn unknown_key_error(requested: &str, values: &HashMap<String, Value>) -> String {
    let mut available: Vec<&str> = values.keys().map(String::as_str).collect();
    available.sort_unstable();
    let shown: Vec<&str> = available
        .iter()
        .take(UNKNOWN_KEY_LIST_MAX)
        .copied()
        .collect();
    let suffix = if available.len() > UNKNOWN_KEY_LIST_MAX {
        format!(", … ({} more)", available.len() - UNKNOWN_KEY_LIST_MAX)
    } else {
        String::new()
    };
    format!(
        "unknown context key '{requested}'; available keys: {}{suffix}",
        shown.join(", ")
    )
}

/// In-memory blob store shared by this module's tests and the crate-wide
/// `for_tests` request-literal helper.
#[cfg(test)]
mod test_support {
    use std::collections::HashMap;

    use anyhow::Result;
    use fabro_store::{EventEnvelope, RunProjection};
    use fabro_types::{BlobHash, RunEvent};

    use crate::runtime_store::RunStoreBackend;

    pub(super) struct MemoryBlobBackend {
        blobs: std::sync::Mutex<HashMap<BlobHash, bytes::Bytes>>,
    }

    impl MemoryBlobBackend {
        pub(super) fn new() -> Self {
            Self {
                blobs: std::sync::Mutex::new(HashMap::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl RunStoreBackend for MemoryBlobBackend {
        async fn load_state(&self) -> Result<RunProjection> {
            unreachable!("context_read tests never load run state")
        }

        async fn list_events(&self) -> Result<Vec<EventEnvelope>> {
            unreachable!("context_read tests never list events")
        }

        async fn append_run_event(&self, _event: &RunEvent) -> Result<()> {
            unreachable!("context_read tests never append events")
        }

        async fn write_blob(&self, data: &[u8]) -> Result<BlobHash> {
            let hash = BlobHash::new(data);
            self.blobs
                .lock()
                .expect("blob map lock is never poisoned: no code panics while holding it")
                .insert(hash, bytes::Bytes::copy_from_slice(data));
            Ok(hash)
        }

        async fn read_blob(&self, blob_hash: &BlobHash) -> Result<Option<bytes::Bytes>> {
            Ok(self
                .blobs
                .lock()
                .expect("blob map lock is never poisoned: no code panics while holding it")
                .get(blob_hash)
                .cloned())
        }

        async fn read_run_log(&self) -> Result<Option<Vec<u8>>> {
            unreachable!("context_read tests never read run logs")
        }
    }
}

/// Minimal services for `CodergenRunRequest` literals in tests that never
/// invoke the tool.
#[cfg(test)]
impl ContextReadServices {
    pub(crate) fn for_tests() -> Self {
        use fabro_agent::LocalSandbox;
        let run_dir = std::env::temp_dir().join("fabro-context-read-tests");
        Self::new(
            &Context::new(),
            &plain_test_node(),
            &plain_test_graph(),
            crate::runtime_store::RunStoreHandle::new(std::sync::Arc::new(
                test_support::MemoryBlobBackend::new(),
            )),
            std::sync::Arc::new(LocalSandbox::new(run_dir.clone())),
            run_dir,
        )
    }
}

#[cfg(test)]
fn plain_test_graph() -> Graph {
    let mut graph = Graph::new("test");
    graph.nodes.insert("a".to_string(), plain_test_node());
    graph
}

#[cfg(test)]
fn plain_test_node() -> Node {
    Node::new("a")
}

#[cfg(test)]
mod tests {
    use fabro_agent::LocalSandbox;
    use serde_json::json;

    use super::test_support::MemoryBlobBackend;
    use super::*;

    fn node_with_attrs(attrs: &[(&str, &str)]) -> Node {
        let mut node = Node::new("a");
        for (key, value) in attrs {
            node.attrs.insert(
                (*key).to_string(),
                fabro_graphviz::graph::AttrValue::String((*value).to_string()),
            );
        }
        node
    }

    fn plain_graph() -> Graph {
        plain_test_graph()
    }

    fn graph_with_inline_max(kb: i64) -> Graph {
        let mut graph = Graph::new("test");
        graph.attrs.insert(
            "preamble_inline_max_kb".to_string(),
            fabro_graphviz::graph::AttrValue::Integer(kb),
        );
        graph.nodes.insert("a".to_string(), Node::new("a"));
        graph
    }

    fn services_for(
        context: &Context,
        node: &Node,
        graph: &Graph,
        run_dir: &std::path::Path,
    ) -> ContextReadServices {
        ContextReadServices::new(
            context,
            node,
            graph,
            RunStoreHandle::new(Arc::new(MemoryBlobBackend::new())),
            Arc::new(LocalSandbox::new(run_dir.to_path_buf())),
            run_dir.to_path_buf(),
        )
    }

    #[test]
    fn view_drops_engine_keys() {
        let context = Context::new();
        context.set("user.key", json!("value"));
        context.set(keys::INTERNAL_FIDELITY, json!("compact"));
        context.set(keys::response_key("plan"), json!("spoofable"));
        context.set(keys::OUTCOME, json!({}));

        let view = ContextReadView::new(&context, &node_with_attrs(&[]), &plain_graph());
        assert!(!view.values.contains_key(keys::INTERNAL_FIDELITY));
        assert!(!view.values.contains_key(&keys::response_key("plan")));
        assert!(!view.values.contains_key(keys::OUTCOME));
        assert_eq!(view.values.get("user.key"), Some(&json!("value")));
    }

    #[test]
    fn view_honors_allow_keys_and_wildcard() {
        let context = Context::new();
        context.set("a", json!(1));
        context.set("b", json!(2));

        let scoped = ContextReadView::new(
            &context,
            &node_with_attrs(&[("preamble_allow_keys", "a")]),
            &plain_graph(),
        );
        assert_eq!(scoped.values.keys().collect::<Vec<_>>(), [&"a"]);

        let wildcard = ContextReadView::new(
            &context,
            &node_with_attrs(&[("preamble_allow_keys", "*")]),
            &plain_graph(),
        );
        assert_eq!(wildcard.values.len(), 2);
    }

    #[test]
    fn view_allow_list_cannot_re_admit_engine_keys() {
        let context = Context::new();
        context.set(keys::OUTCOME, json!({}));
        let view = ContextReadView::new(
            &context,
            &node_with_attrs(&[("preamble_allow_keys", "outcome")]),
            &plain_graph(),
        );
        assert!(view.values.is_empty());
    }

    #[test]
    fn view_inline_max_node_over_graph_over_default() {
        let context = Context::new();

        let default = ContextReadView::new(&context, &node_with_attrs(&[]), &plain_graph());
        assert_eq!(default.inline_max, artifact::PROMPT_INLINE_VALUE_MAX);

        let graph = graph_with_inline_max(4);
        let graph_ceiling = ContextReadView::new(&context, &node_with_attrs(&[]), &graph);
        assert_eq!(graph_ceiling.inline_max, 4 * 1024);

        let mut node = Node::new("a");
        node.attrs.insert(
            "preamble_inline_max_kb".to_string(),
            fabro_graphviz::graph::AttrValue::Integer(2),
        );
        let node_override = ContextReadView::new(&context, &node, &graph);
        assert_eq!(node_override.inline_max, 2 * 1024);
    }

    #[tokio::test]
    async fn read_returns_small_value_as_json() {
        let context = Context::new();
        context.set("plan.outline", json!({"steps": ["a", "b"]}));
        let run_dir = tempfile::tempdir().expect("tempdir");
        let services = services_for(
            &context,
            &node_with_attrs(&[]),
            &plain_graph(),
            run_dir.path(),
        );
        let state = Arc::new(ContextReadState::new(services));

        let answer = execute_context_read(json!({"key": "plan.outline"}), Arc::clone(&state))
            .await
            .expect("small value serves");
        assert_eq!(answer, r#"{"steps":["a","b"]}"#);
    }

    #[tokio::test]
    async fn read_demotes_large_value_to_marker_with_path() {
        let big = "x".repeat(9 * 1024);
        let context = Context::new();
        context.set("evidence", json!(big));
        let run_dir = tempfile::tempdir().expect("tempdir");
        let services = services_for(
            &context,
            &node_with_attrs(&[]),
            &plain_graph(),
            run_dir.path(),
        );
        let state = Arc::new(ContextReadState::new(services));

        let answer = execute_context_read(json!({"key": "evidence"}), Arc::clone(&state))
            .await
            .expect("large value serves");
        let marker: Value = serde_json::from_str(&answer).expect("marker is JSON");
        let marker = marker.get("fabroLargeValue").expect("demote marker shape");
        let path = marker.get("path").and_then(Value::as_str).expect("path");
        assert!(
            run_dir.path().join("runtime/blobs").exists(),
            "materialized blob directory exists under the run dir"
        );
        assert!(
            std::path::Path::new(path).exists(),
            "materialized blob file exists at {path}"
        );
    }

    #[tokio::test]
    async fn read_unknown_key_lists_available() {
        let context = Context::new();
        context.set("alpha", json!(1));
        let run_dir = tempfile::tempdir().expect("tempdir");
        let services = services_for(
            &context,
            &node_with_attrs(&[]),
            &plain_graph(),
            run_dir.path(),
        );
        let state = Arc::new(ContextReadState::new(services));

        let message = execute_context_read(json!({"key": "missing"}), Arc::clone(&state))
            .await
            .expect_err("missing key errors");
        assert!(
            message.contains("unknown context key 'missing'"),
            "{message}"
        );
        assert!(message.contains("alpha"), "{message}");
    }

    #[tokio::test]
    async fn update_swaps_the_served_view() {
        let context = Context::new();
        context.set("before", json!(1));
        let run_dir = tempfile::tempdir().expect("tempdir");
        let services = services_for(
            &context,
            &node_with_attrs(&[]),
            &plain_graph(),
            run_dir.path(),
        );
        let state = Arc::new(ContextReadState::new(services));
        assert_eq!(
            execute_context_read(json!({"key": "before"}), Arc::clone(&state))
                .await
                .expect("serves"),
            "1"
        );

        let next = Context::new();
        next.set("after", json!(2));
        let next_services =
            services_for(&next, &node_with_attrs(&[]), &plain_graph(), run_dir.path());
        state.update(&next_services);

        assert!(
            execute_context_read(json!({"key": "before"}), Arc::clone(&state))
                .await
                .is_err()
        );
        assert_eq!(
            execute_context_read(json!({"key": "after"}), Arc::clone(&state))
                .await
                .expect("serves"),
            "2"
        );
    }

    #[tokio::test]
    async fn read_hydrates_a_surviving_blob_ref() {
        // Simulate a raw offloaded value reaching the view: the store holds
        // the blob, the context holds only its canonical reference.
        let contents = format!("[\"{}\"]", "y".repeat(9 * 1024));
        let run_dir = tempfile::tempdir().expect("tempdir");
        let run_store = RunStoreHandle::new(Arc::new(MemoryBlobBackend::new()));
        let hash = run_store
            .write_blob(contents.as_bytes())
            .await
            .expect("blob write");
        let blob_ref = fabro_types::format_blob_ref(&hash);

        let context = Context::new();
        context.set("offloaded", json!(blob_ref));
        let services = ContextReadServices::new(
            &context,
            &node_with_attrs(&[]),
            &plain_graph(),
            run_store,
            Arc::new(LocalSandbox::new(run_dir.path().to_path_buf())),
            run_dir.path().to_path_buf(),
        );
        let state = Arc::new(ContextReadState::new(services));

        // 9KB content: over the 8KB default budget, so the hydrated value
        // must come back as a demote marker, not a bare reference.
        let answer = execute_context_read(json!({"key": "offloaded"}), Arc::clone(&state))
            .await
            .expect("offloaded value serves");
        let marker: Value = serde_json::from_str(&answer).expect("marker is JSON");
        let marker = marker.get("fabroLargeValue").expect("demote marker shape");
        let path = marker.get("path").and_then(Value::as_str).expect("path");
        assert!(
            std::path::Path::new(path).exists(),
            "materialized blob file exists at {path}"
        );
        assert!(
            !answer.contains("blob://"),
            "no raw blob reference reaches the model: {answer}"
        );
    }

    #[tokio::test]
    async fn read_returns_string_values_bare() {
        let context = Context::new();
        context.set("note", json!("plain text"));
        let run_dir = tempfile::tempdir().expect("tempdir");
        let services = services_for(
            &context,
            &node_with_attrs(&[]),
            &plain_graph(),
            run_dir.path(),
        );
        let state = Arc::new(ContextReadState::new(services));

        let answer = execute_context_read(json!({"key": "note"}), Arc::clone(&state))
            .await
            .expect("string serves");
        assert_eq!(answer, "plain text");
    }

    #[test]
    fn tool_definition_name_and_schema() {
        let run_dir = tempfile::tempdir().expect("tempdir");
        let services = services_for(
            &Context::new(),
            &node_with_attrs(&[]),
            &plain_graph(),
            run_dir.path(),
        );
        let tool = context_read_tool(Arc::new(ContextReadState::new(services)));
        assert_eq!(tool.definition.name, "context_read");
        let required = tool
            .definition
            .parameters
            .get("required")
            .and_then(Value::as_array)
            .expect("required array");
        assert_eq!(required.first().and_then(Value::as_str), Some("key"));
    }
}
