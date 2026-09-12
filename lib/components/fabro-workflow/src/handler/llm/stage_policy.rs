//! Fork stage tool policy on pebble's middleware seam.
//!
//! Two node-level stage-envelope features (ADR-0009) that upstream's pebble
//! adoption does not carry natively, re-anchored from the retired
//! `fabro-agent` tool layer onto [`pebble_agent::ToolMiddleware`] (merge of
//! upstream v0.354.0-nightly.0):
//!
//! - **fabro-47b5** — the node `tools` attribute: an allow-list of tool names
//!   the stage may discover and execute. Question tools (`request_user_input`,
//!   `AskUserQuestion`) are exempt both ways: HITL is a workflow contract, like
//!   the engine-stamped context keys. Unset attribute = no policy = the
//!   default-open full catalog.
//! - **fabro-ba96** — the node `fs_hide`/`fs_write` attributes compiled into an
//!   [`FsScope`]: hidden workspace paths behave as if they did not exist for
//!   the stage (reads fail, discovery results are filtered, writes are denied),
//!   and a set `fs_write` narrows writes to its globs. The shell tool remains
//!   the documented escape hatch.
//!
//! The middleware composes around the run's hook middleware
//! ([`fabro_hooks::WorkflowToolHookCallback`]): policy refusals happen
//! before hooks observe the call, and a call the policy admits still passes
//! through the hook chain. Pebble propagates middleware to subagent
//! sessions, which is the inheritance the old subagent factory provided.
//!
//! Matching is by [`pebble_agent::ToolDescriptor`] stable id — the
//! canonical native name regardless of the profile vocabulary the model
//! sees — with a small fork-side alias table so a `tools` entry may name a
//! tool the way a profile vocabulary spells it (`Read` for `read_file`).

use std::sync::Arc;

use async_trait::async_trait;
use fabro_graphviz::graph::Node;
use fabro_sandbox::fs_scope::ScopeDenial;
use fabro_sandbox::{Error as SandboxError, FsScope};
use pebble_agent::{
    ToolCallNext, ToolCallRequest, ToolCatalog, ToolDiscoveryNext, ToolErrorKind, ToolMiddleware,
    ToolOutcome, TurnContext,
};
use pebble_coding_agent::tools::grep_result_path;

use crate::error::{Error, Result};

/// The question tools exempt from the `tools` allow-list (HITL contract).
const QUESTION_TOOLS: [&str; 2] = ["request_user_input", "AskUserQuestion"];

/// Resolve a `tools` attribute entry to the canonical name the policy
/// matches against. Covers the alias spellings pebble's profile
/// vocabularies expose; anything else passes through unchanged (MCP tools,
/// run tools, `spawn_agent`).
pub fn canonical_tool_name(name: &str) -> &str {
    match name {
        "Read" => "read_file",
        "Write" => "write_file",
        "Edit" => "edit_file",
        "Bash" => "shell",
        "Grep" => "grep",
        "Glob" => "glob",
        "WebSearch" => "web_search",
        "WebFetch" | "FetchURL" => "web_fetch",
        "Skill" => "use_skill",
        "Agent" => "background_agent",
        "TaskOutput" => "agent_output",
        "TaskStop" => "stop_agent",
        "SendMessage" => "message_agent",
        other => other,
    }
}

/// Compile a node's `fs_hide`/`fs_write` attributes into a filesystem
/// scope (fabro-ba96). `None` when the node declares nothing. Fail-closed
/// on invalid globs.
pub fn node_fs_scope(node: &Node) -> Result<Option<Arc<FsScope>>> {
    let hide = node.fs_hide();
    let write = node.fs_write();
    if hide.is_empty() && write.is_none() {
        return Ok(None);
    }
    FsScope::try_new(&hide, write.as_deref())
        .map(Arc::new)
        .map(Some)
        .map_err(|error| {
            Error::handler(format!(
                "node '{}' declares an invalid {} glob '{}'",
                node.id,
                error.attribute(),
                error.pattern(),
            ))
        })
}

/// The per-stage tool policy middleware (fabro-47b5 + fabro-ba96).
pub struct StageToolMiddleware {
    /// Canonical names the node's `tools` attribute admits. `None` = no
    /// policy, the default-open full catalog.
    allowed_tools: Option<Vec<String>>,
    fs_scope:      Option<Arc<FsScope>>,
    working_dir:   String,
    /// The run's hook middleware, wrapped so hooks still observe every
    /// call the policy admits.
    hooks:         Option<Arc<dyn ToolMiddleware>>,
}

impl StageToolMiddleware {
    /// Build the middleware for one stage invocation. `sandbox_dir` is the
    /// stage sandbox's working directory, the root `fs_hide`/`fs_write`
    /// globs are relative to.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the node declares an invalid glob.
    pub fn new(
        node: &Node,
        sandbox_dir: String,
        hooks: Option<Arc<dyn ToolMiddleware>>,
    ) -> Result<Self> {
        let allowed_tools = {
            let tools = node.tools();
            if tools.is_empty() {
                None
            } else {
                Some(
                    tools
                        .iter()
                        .map(|tool| canonical_tool_name(tool).to_owned())
                        .collect(),
                )
            }
        };
        Ok(Self {
            allowed_tools,
            fs_scope: node_fs_scope(node)?,
            working_dir: sandbox_dir,
            hooks,
        })
    }

    /// Whether the tool identified by `id` (a descriptor's stable id) is
    /// admitted by the allow-list.
    fn policy_admits(&self, id: &str) -> bool {
        if QUESTION_TOOLS.contains(&id) {
            return true;
        }
        self.allowed_tools
            .as_ref()
            .is_none_or(|allowed| allowed.iter().any(|name| name == id))
    }

    /// Narrow a discovered catalog to the tools the policy admits.
    fn retain_admitted(&self, catalog: &mut ToolCatalog) {
        if self.allowed_tools.is_some() {
            catalog.retain(|descriptor| self.policy_admits(descriptor.id().as_str()));
        }
    }

    /// Deny a call the policy refuses, with the message the model sees.
    fn deny(request: &ToolCallRequest, message: String) -> ToolOutcome {
        tracing::debug!(
            tool = %request.call().name,
            "stage tool policy refused a call: {message}"
        );
        ToolOutcome::failure(ToolErrorKind::Denied, message)
    }

    /// One path-bearing argument as a string.
    fn str_arg<'a>(arguments: &'a serde_json::Value, key: &str) -> Option<&'a str> {
        arguments.get(key).and_then(serde_json::Value::as_str)
    }

    /// The read-side denial message for a hidden path (nonexistent
    /// semantics, fabro-ba96).
    fn hidden_message(path: &str) -> String {
        format!(
            "path '{path}' is hidden from this stage by fs_hide and behaves as if it did not exist"
        )
    }

    /// The `fs_write` denial message for a path outside the write globs.
    fn outside_write_message(path: &str) -> String {
        format!(
            "path '{path}' is not writable by this stage: fs_write is set and the path is outside \
             its globs"
        )
    }

    /// Enforce the scope's read side for one path.
    fn check_read(&self, path: &str) -> std::result::Result<(), String> {
        let Some(scope) = self.fs_scope.as_ref() else {
            return Ok(());
        };
        scope
            .check_read(&self.working_dir, path)
            .map_err(|_| Self::hidden_message(path))
    }

    /// Enforce the scope's write side for one path.
    fn check_write(&self, path: &str) -> std::result::Result<(), String> {
        let Some(scope) = self.fs_scope.as_ref() else {
            return Ok(());
        };
        scope.check_write(&self.working_dir, path).map_err(|error| {
            if matches!(&error, SandboxError::StageFsScope {
                reason: ScopeDenial::HiddenByFsHide,
                ..
            }) {
                Self::hidden_message(path)
            } else {
                Self::outside_write_message(path)
            }
        })
    }

    /// Every target path an `apply_patch` text touches, in patch order.
    fn patch_target_paths(patch: &str) -> Vec<&str> {
        patch
            .lines()
            .filter_map(|line| {
                ["*** Add File: ", "*** Delete File: ", "*** Update File: "]
                    .into_iter()
                    .find_map(|marker| line.strip_prefix(marker))
            })
            .collect()
    }

    /// Policy checks for one resolved call. `Ok(())` admits the call
    /// through to the hook chain; the message denies it.
    fn enforce_call(&self, request: &ToolCallRequest) -> std::result::Result<(), String> {
        let id = request.descriptor().id().as_str();
        if !self.policy_admits(id) {
            return Err(format!(
                "tool '{}' is denied by tool access policy for this stage: the node's 'tools' \
                 attribute does not list it",
                request.call().name,
            ));
        }
        if self.fs_scope.is_none() {
            return Ok(());
        }
        let arguments = request_arguments(request);
        match id {
            "read_file" => {
                if let Some(path) = Self::str_arg(&arguments, "file_path") {
                    self.check_read(path)?;
                }
            }
            "edit_file" | "write_file" => {
                if let Some(path) = Self::str_arg(&arguments, "file_path") {
                    self.check_write(path)?;
                }
            }
            "read_many_files" => {
                if let Some(paths) = arguments.get("paths").and_then(serde_json::Value::as_array) {
                    for path in paths.iter().filter_map(serde_json::Value::as_str) {
                        self.check_read(path)?;
                    }
                }
            }
            "list_dir" | "grep" => {
                let path = Self::str_arg(&arguments, "path").unwrap_or(".");
                self.check_read(path)?;
            }
            "apply_patch" => {
                if let Some(patch) = arguments.as_str() {
                    for path in Self::patch_target_paths(patch) {
                        self.check_write(path)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Filter a discovery tool's successful text output so hidden paths do
    /// not leak (fabro-ba96 nonexistent semantics).
    fn filter_success(&self, id: &str, arguments: &serde_json::Value, outcome: &mut ToolOutcome) {
        let Some(scope) = self.fs_scope.as_ref() else {
            return;
        };
        let ToolOutcome::Success { output, .. } = outcome else {
            return;
        };
        let text = output.text();
        let filtered = match id {
            "grep" => {
                let searched = Self::str_arg(arguments, "path").unwrap_or(".");
                text.lines()
                    .filter(|line| {
                        !scope.is_path_hidden(&self.working_dir, grep_result_path(line, searched))
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            "glob" => text
                .lines()
                .filter(|line| !scope.is_path_hidden(&self.working_dir, line))
                .collect::<Vec<_>>()
                .join("\n"),
            "list_dir" => {
                let dir = Self::str_arg(arguments, "path").unwrap_or(".");
                text.lines()
                    .filter(|line| {
                        let name = line.strip_suffix('/').unwrap_or(line);
                        let candidate = join_dir_entry(dir, name);
                        !scope.is_path_hidden(&self.working_dir, &candidate)
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            "read_many_files" => {
                // Keep only the `=== path ===` blocks whose path is
                // visible; a hidden path's block is dropped whole.
                let mut kept: Vec<&str> = Vec::new();
                let mut hidden = false;
                for line in text.lines() {
                    if let Some(path) = line
                        .strip_prefix("=== ")
                        .and_then(|r| r.strip_suffix(" ==="))
                    {
                        hidden = scope.is_path_hidden(&self.working_dir, path);
                    }
                    if !hidden {
                        kept.push(line);
                    }
                }
                kept.join("\n")
            }
            _ => return,
        };
        if filtered != text {
            let metadata = output.metadata().clone();
            *output = pebble_agent::ToolOutput::from(filtered).with_metadata(metadata);
        }
    }
}

/// The arguments of a resolved call: the model's call input as JSON. A
/// custom (freeform) tool's input becomes the raw string, which is the
/// `apply_patch` shape.
fn request_arguments(request: &ToolCallRequest) -> serde_json::Value {
    request
        .call()
        .input
        .to_value()
        .unwrap_or(serde_json::Value::Null)
}

/// Compose a listed directory and an entry name into one scope candidate.
fn join_dir_entry(dir: &str, name: &str) -> String {
    if name.starts_with('/') {
        return name.to_string();
    }
    let trimmed = dir.trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "." {
        name.to_string()
    } else {
        format!("{trimmed}/{name}")
    }
}

#[async_trait]
impl ToolMiddleware for StageToolMiddleware {
    async fn discover(
        &self,
        context: TurnContext<'_>,
        next: ToolDiscoveryNext<'_>,
    ) -> std::result::Result<ToolCatalog, pebble_agent::ToolSystemError> {
        let mut catalog = match self.hooks.as_ref() {
            Some(hooks) => hooks.discover(context, next).await?,
            None => next.run(context).await?,
        };
        self.retain_admitted(&mut catalog);
        Ok(catalog)
    }

    async fn call(
        &self,
        request: ToolCallRequest,
        next: ToolCallNext<'_>,
    ) -> std::result::Result<ToolOutcome, pebble_agent::ToolSystemError> {
        let id = request.descriptor().id().as_str().to_owned();
        let arguments = request_arguments(&request);
        if let Err(message) = self.enforce_call(&request) {
            return Ok(Self::deny(&request, message));
        }
        let mut outcome = match self.hooks.as_ref() {
            Some(hooks) => hooks.call(request, next).await?,
            None => next.run(request).await?,
        };
        self.filter_success(&id, &arguments, &mut outcome);
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use fabro_graphviz::graph::{AttrValue, Node};
    use lithos_llm::types::ToolDefinition;
    use pebble_agent::ToolId;

    use super::*;

    fn node_with_attrs(attrs: &[(&str, &str)]) -> Node {
        let mut node = Node::new("stage");
        for (key, value) in attrs {
            node.attrs
                .insert((*key).to_string(), AttrValue::String((*value).to_string()));
        }
        node
    }

    #[test]
    fn node_fs_scope_compiles_node_attributes_and_defaults_to_none() {
        let plain = node_with_attrs(&[("prompt", "work")]);
        assert!(node_fs_scope(&plain).unwrap().is_none());

        let scoped = node_with_attrs(&[
            ("fs_hide", ".fabro/**, .seeds/**"),
            ("fs_write", "*.go, go.mod"),
        ]);
        let scope = node_fs_scope(&scoped).unwrap().expect("scope is active");
        assert!(scope.is_hidden(".fabro/workflow.toml"));
        assert!(scope.is_hidden(".seeds/1/issues.md"));
        assert!(!scope.is_hidden("src/main.go"));

        // An explicitly empty fs_write list makes the stage read-only.
        let read_only = node_with_attrs(&[("fs_write", "")]);
        let scope = node_fs_scope(&read_only).unwrap().expect("scope is active");
        assert!(scope.check_write("/w", "any/path").is_err());

        let broken = node_with_attrs(&[("fs_hide", "../*")]);
        let error = node_fs_scope(&broken).expect_err("invalid glob fails the stage");
        assert!(error.to_string().contains("fs_hide"));
    }

    #[test]
    fn canonical_tool_name_resolves_vocabulary_aliases() {
        assert_eq!(canonical_tool_name("Read"), "read_file");
        assert_eq!(canonical_tool_name("Bash"), "shell");
        assert_eq!(canonical_tool_name("FetchURL"), "web_fetch");
        assert_eq!(canonical_tool_name("spawn_agent"), "spawn_agent");
        assert_eq!(canonical_tool_name("fabro_run_create"), "fabro_run_create");
    }

    #[test]
    fn policy_admits_question_tools_and_listed_names() {
        let node = node_with_attrs(&[("tools", "read_file, Grep")]);
        let mw = StageToolMiddleware::new(&node, "/w".to_string(), None).unwrap();
        assert!(mw.policy_admits("read_file"));
        assert!(mw.policy_admits("grep"), "alias resolves on the way in");
        assert!(mw.policy_admits("request_user_input"));
        assert!(mw.policy_admits("AskUserQuestion"));
        assert!(!mw.policy_admits("write_file"));
        assert!(!mw.policy_admits("spawn_agent"));

        let open = StageToolMiddleware::new(&node_with_attrs(&[]), "/w".to_string(), None).unwrap();
        assert!(open.policy_admits("write_file"));
    }

    #[test]
    fn patch_target_paths_extracts_every_marker() {
        let patch = "*** Begin Patch\n\
                     *** Update File: src/main.go\n\
                     @@\n-old\n+new\n\
                     *** Add File: docs/new.md\n\
                     +hello\n\
                     *** Delete File: stale.txt\n\
                     *** End Patch";
        assert_eq!(StageToolMiddleware::patch_target_paths(patch), vec![
            "src/main.go",
            "docs/new.md",
            "stale.txt"
        ],);
    }

    #[test]
    fn fs_checks_use_scope_semantics() {
        let node = node_with_attrs(&[("fs_hide", ".seeds/**"), ("fs_write", "*.go")]);
        let mw = StageToolMiddleware::new(&node, "/w".to_string(), None).unwrap();

        assert!(mw.check_read("/w/.seeds/1.md").is_err());
        assert!(mw.check_read("/w/src/main.go").is_ok());
        assert!(mw.check_write("/w/src/main.go").is_ok());
        assert!(mw.check_write("/w/README.md").is_err());
        assert!(mw.check_write("/etc/passwd").is_err());
        // The denial message distinguishes hidden from outside-write.
        assert!(
            mw.check_write("/w/.seeds/x")
                .unwrap_err()
                .contains("fs_hide")
        );
        assert!(
            mw.check_write("/w/README.md")
                .unwrap_err()
                .contains("fs_write")
        );
    }

    #[test]
    fn list_dir_entries_compose_with_their_directory() {
        assert_eq!(join_dir_entry(".", "src"), "src");
        assert_eq!(join_dir_entry("/w", "src"), "/w/src");
        assert_eq!(join_dir_entry("/w/", "src"), "/w/src");
        assert_eq!(join_dir_entry("/w", "/abs"), "/abs");
    }

    #[test]
    fn discover_retention_narrows_the_catalog() {
        let node = node_with_attrs(&[("tools", "read_file")]);
        let mw = StageToolMiddleware::new(&node, "/w".to_string(), None).unwrap();
        let mut catalog = ToolCatalog::new([
            pebble_agent::ToolDescriptor::new(
                ToolId::try_new("read_file").unwrap(),
                ToolDefinition::function(
                    "read_file",
                    "read",
                    serde_json::json!({"type": "object"}),
                ),
            ),
            pebble_agent::ToolDescriptor::new(
                ToolId::try_new("write_file").unwrap(),
                ToolDefinition::function(
                    "write_file",
                    "write",
                    serde_json::json!({"type": "object"}),
                ),
            ),
        ]);
        mw.retain_admitted(&mut catalog);
        let names: Vec<&str> = catalog
            .visible_tools()
            .map(|descriptor| descriptor.id().as_str())
            .collect();
        assert_eq!(names, vec!["read_file"]);
    }

    #[test]
    fn graph_level_attributes_still_exist() {
        // The engine-side accessors the middleware compiles from: this is
        // the regression canary for the merged graph module.
        let node = node_with_attrs(&[("fs_hide", ".fabro/**"), ("tools", "read_file")]);
        assert_eq!(node.fs_hide(), vec![".fabro/**"]);
        assert_eq!(node.tools(), vec!["read_file"]);
    }
}
