//! The engine-side `x.tools` allowlist at the session's tool boundary
//! (fabro-1a41, the first re-landed member of the fabro-70af family).
//!
//! [`ToolPolicyHooks`] wraps Petri's local hook service as the run's
//! `HookService`: at `BeforeToolUse` it asks the node's stage envelope —
//! a node that declares `x.tools` may call only those session tools, and
//! a call outside the list is denied mechanically, the reason stated to
//! the model, before Pebble runs the tool. Every other point, and every
//! call of a node without the attribute, passes through to the local
//! service unchanged, so `[[run.hooks]]` keep running. The wrapper is
//! what [`RuntimeSpec::runtime`](crate::runtime::RuntimeSpec::runtime)
//! installs as the `HookServiceHandle` capability before
//! `petri_attractor_steps::register`, the documented host-replacement
//! seam; the driver's awaited hooks (`FabroHooks`) wrap it in turn and
//! forward every point.

use std::sync::Arc;

use petri_attractor_steps::hooks::LocalHooks;
use petri_execution::hooks::{HookDecision, HookPoint, HookReport, HookRequest, HookService};

use crate::fork_stage_envelope::StageEnvelopes;

/// The wrapped local hook service with the run's stage envelopes.
#[derive(Clone)]
pub struct ToolPolicyHooks {
    inner:     Arc<LocalHooks>,
    envelopes: Option<Arc<StageEnvelopes>>,
}

impl ToolPolicyHooks {
    /// Wrap `inner` (Petri's local `[[run.hooks]]` service) with the
    /// `x.tools` policy for `envelopes`; `None` is a pure passthrough,
    /// the runtime without envelopes.
    #[must_use]
    pub fn new(inner: Arc<LocalHooks>, envelopes: Option<Arc<StageEnvelopes>>) -> Self {
        Self { inner, envelopes }
    }
}

#[async_trait::async_trait]
impl HookService for ToolPolicyHooks {
    async fn run(&self, request: HookRequest) -> HookReport {
        if let (Some(envelopes), Some(view)) = (&self.envelopes, &request.view) {
            if matches!(request.point, HookPoint::BeforeToolUse) {
                if let Some(tool) = request
                    .payload
                    .get("tool_name")
                    .and_then(|value| value.as_str())
                {
                    if let Some(reason) = denied_tool(envelopes, view.node_name(), tool) {
                        return HookReport {
                            point:    request.point,
                            decision: HookDecision::Block { reason },
                            hooks:    Vec::new(),
                            warnings: Vec::new(),
                            activity: Vec::new(),
                        };
                    }
                }
            }
        }
        self.inner.run(request).await
    }

    fn configured_hooks(&self, point: HookPoint) -> Vec<String> {
        self.inner.configured_hooks(point)
    }
}

/// The denial for `tool` on `node`, when the node's `x.tools` allowlist
/// excludes it: `None` proceeds. A node without the attribute, or a run
/// without envelopes, never denies.
#[must_use]
pub fn denied_tool(envelopes: &StageEnvelopes, node: &str, tool: &str) -> Option<String> {
    let tools = envelopes.envelope(node)?.tools.as_ref()?;
    if tools.iter().any(|allowed| allowed == tool) {
        return None;
    }
    Some(if tools.is_empty() {
        format!(
            "node '{node}' declares x.tools=\"\" (no session tools); '{tool}' is denied at \
             the engine layer (fabro-1a41)"
        )
    } else {
        format!(
            "node '{node}' allows only the session tools [{}] (x.tools); '{tool}' is denied at \
             the engine layer (fabro-1a41)",
            tools.join(", ")
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelopes(source: &str) -> StageEnvelopes {
        StageEnvelopes::parse(source)
    }

    #[test]
    fn a_node_without_x_tools_never_denies() {
        let source = r#"digraph W { a [x.fs_write="lib/**"] }"#;
        assert_eq!(denied_tool(&envelopes(source), "a", "write_file"), None);
    }

    #[test]
    fn a_declared_tool_passes_and_an_undeclared_one_is_denied_with_reason() {
        let source = r#"digraph W { reader [x.tools="read_file, list_files"] }"#;
        let parsed = envelopes(source);
        assert_eq!(denied_tool(&parsed, "reader", "read_file"), None);
        let reason = denied_tool(&parsed, "reader", "write_file").expect("denied");
        assert!(reason.contains("read_file, list_files"));
        assert!(reason.contains("write_file"));
        assert!(reason.contains("fabro-1a41"));
    }

    #[test]
    fn an_empty_list_denies_everything_with_the_empty_posture() {
        let source = r#"digraph W { reviewer [x.tools=""] }"#;
        let reason = denied_tool(&envelopes(source), "reviewer", "read_file").expect("denied");
        assert!(reason.contains("no session tools"));
    }

    #[test]
    fn expansion_clones_carry_the_allowlist() {
        let source = r#"digraph W { reader [x.tools="read_file"] }"#;
        assert_eq!(
            denied_tool(&envelopes(source), "reader#2", "write_file").is_some(),
            true
        );
    }
}
