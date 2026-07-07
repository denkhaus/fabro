use std::collections::HashMap;
use std::sync::Arc;

use fabro_agent::Sandbox;
use fabro_auth::CredentialSource;
#[cfg(test)]
use fabro_auth::EnvCredentialSource;
use fabro_model::Catalog;
use fabro_redact::SecretRedactor;

use crate::config::{HookDefinition, HookSettings};
use crate::executor::{HookExecutor, HookExecutorImpl};
use crate::secrets::HookSecretResolver;
use crate::types::{HookContext, HookDecision, HookExecutionContext, HookResult};

/// Central orchestrator: filters matching hooks, executes them, merges
/// decisions.
pub struct HookRunner {
    config:            HookSettings,
    executor:          Arc<dyn HookExecutor>,
    llm_source:        Arc<dyn CredentialSource>,
    catalog:           Arc<Catalog>,
    secrets:           HookSecretResolver,
    /// Pre-compiled regexes keyed by matcher pattern string.
    compiled_matchers: HashMap<String, regex::Regex>,
}

fn redact_hook_result(mut result: HookResult, redactor: &SecretRedactor) -> HookResult {
    result.decision = redact_hook_decision(result.decision, redactor);
    result
}

fn redact_hook_decision(decision: HookDecision, redactor: &SecretRedactor) -> HookDecision {
    match decision {
        HookDecision::Skip { reason } => HookDecision::Skip {
            reason: reason.map(|reason| redactor.redact_into(&reason)),
        },
        HookDecision::Block { reason } => HookDecision::Block {
            reason: reason.map(|reason| redactor.redact_into(&reason)),
        },
        // `Override.edge_to` is a structural graph edge id, not free-form text,
        // so it is intentionally left unredacted.
        HookDecision::Proceed | HookDecision::Override { .. } => decision,
    }
}

impl HookRunner {
    #[must_use]
    pub fn new(
        config: HookSettings,
        llm_source: Arc<dyn CredentialSource>,
        catalog: Arc<Catalog>,
        secrets: HookSecretResolver,
    ) -> Self {
        let compiled_matchers = Self::compile_matchers(&config);
        Self {
            config,
            executor: Arc::new(HookExecutorImpl),
            llm_source,
            catalog,
            secrets,
            compiled_matchers,
        }
    }

    /// Create a HookRunner with a custom executor (for testing).
    #[cfg(test)]
    pub fn with_executor(config: HookSettings, executor: Arc<dyn HookExecutor>) -> Self {
        let compiled_matchers = Self::compile_matchers(&config);
        Self {
            config,
            executor,
            llm_source: Arc::new(EnvCredentialSource::new()),
            catalog: Arc::new(Catalog::from_builtin().expect("default catalog should build")),
            secrets: HookSecretResolver::default(),
            compiled_matchers,
        }
    }

    fn compile_matchers(config: &HookSettings) -> HashMap<String, regex::Regex> {
        let mut map = HashMap::new();
        for hook in &config.hooks {
            let Some(pattern) = hook.matcher.as_ref() else {
                continue;
            };
            if map.contains_key(pattern) {
                continue;
            }
            match regex::Regex::new(pattern) {
                Ok(re) => {
                    map.insert(pattern.clone(), re);
                }
                Err(error) => {
                    tracing::warn!(
                        pattern = %pattern,
                        error = %error,
                        "hook matcher pattern failed to compile; hooks using it will never match"
                    );
                }
            }
        }
        map
    }

    /// Run all matching hooks for the given event and return the merged
    /// decision.
    pub async fn run(
        &self,
        context: &HookContext,
        sandbox: Arc<dyn Sandbox>,
        execution_context: HookExecutionContext,
    ) -> HookDecision {
        let matching = self.filter_hooks(context);
        if matching.is_empty() {
            return HookDecision::Proceed;
        }

        let hooks_matched = matching.len();
        tracing::info!(
            event = %context.event,
            hooks_matched,
            "Running hooks"
        );

        let any_blocking = matching.iter().any(|h| h.is_blocking());

        let decision = if any_blocking {
            // Sequential execution for blocking hooks, short-circuit on first Block
            self.run_sequential(&matching, context, sandbox, &execution_context)
                .await
        } else {
            // Non-blocking: run all, ignore decisions
            self.run_non_blocking(&matching, context, sandbox, &execution_context)
                .await
        };

        tracing::info!(
            event = %context.event,
            decision = decision.as_str(),
            "Hooks complete"
        );

        decision
    }

    /// Filter hooks that match the given event and context.
    fn filter_hooks(&self, context: &HookContext) -> Vec<&HookDefinition> {
        self.config
            .hooks
            .iter()
            .filter(|h| h.event == context.event)
            .filter(|h| self.matches(h, context))
            .collect()
    }

    /// Check if a hook's matcher applies to this context.
    fn matches(&self, hook: &HookDefinition, context: &HookContext) -> bool {
        let Some(ref pattern) = hook.matcher else {
            return true;
        };
        let Some(re) = self.compiled_matchers.get(pattern) else {
            // Pattern failed to compile at construction (warned there); treat as
            // no match so a broken matcher never silently matches everything.
            return false;
        };
        [
            context.node_id.as_deref(),
            context.handler_type.as_deref(),
            context.edge_to.as_deref(),
            context.edge_from.as_deref(),
            context.tool_name.as_deref(),
        ]
        .iter()
        .any(|field| field.is_some_and(|v| re.is_match(v)))
    }

    /// Resolve secrets, run a single hook through the executor, and redact its
    /// result. Shared by the blocking and non-blocking loops.
    async fn execute_one(
        &self,
        hook: &HookDefinition,
        context: &HookContext,
        sandbox: Arc<dyn Sandbox>,
        execution_context: &HookExecutionContext,
    ) -> HookResult {
        tracing::debug!(
            hook = %hook.effective_name(),
            event = %context.event,
            "Executing hook"
        );
        let secrets = self.secrets.resolve_for_definition(hook).await;
        let result = self
            .executor
            .execute(
                hook,
                context,
                sandbox,
                execution_context,
                self.llm_source.as_ref(),
                Arc::clone(&self.catalog),
                &secrets,
            )
            .await;
        let result = redact_hook_result(result, secrets.redactor());
        tracing::debug!(
            hook = %hook.effective_name(),
            duration_ms = result.duration_ms,
            decision = result.decision.as_str(),
            "Hook complete"
        );
        result
    }

    async fn run_sequential(
        &self,
        hooks: &[&HookDefinition],
        context: &HookContext,
        sandbox: Arc<dyn Sandbox>,
        execution_context: &HookExecutionContext,
    ) -> HookDecision {
        let mut merged = HookDecision::Proceed;
        for hook in hooks {
            let result = self
                .execute_one(hook, context, sandbox.clone(), execution_context)
                .await;

            if hook.is_blocking() {
                merged = merged.merge(result.decision);
                // Short-circuit on Block
                if matches!(merged, HookDecision::Block { .. }) {
                    tracing::error!(
                        hook = %hook.effective_name(),
                        event = %context.event,
                        decision = merged.as_str(),
                        "Hook blocked execution"
                    );
                    return merged;
                }
            } else if !result.decision.is_proceed() {
                tracing::warn!(
                    hook = %hook.effective_name(),
                    event = %context.event,
                    decision = result.decision.as_str(),
                    "Non-blocking hook returned non-proceed, ignoring"
                );
            }
        }
        merged
    }

    async fn run_non_blocking(
        &self,
        hooks: &[&HookDefinition],
        context: &HookContext,
        sandbox: Arc<dyn Sandbox>,
        execution_context: &HookExecutionContext,
    ) -> HookDecision {
        for hook in hooks {
            let result = self
                .execute_one(hook, context, sandbox.clone(), execution_context)
                .await;
            if !result.decision.is_proceed() {
                tracing::warn!(
                    hook = %hook.effective_name(),
                    event = %context.event,
                    decision = result.decision.as_str(),
                    "Non-blocking hook failed, continuing"
                );
            }
        }
        HookDecision::Proceed
    }
}

#[cfg(test)]
mod tests {
    use fabro_auth::EnvCredentialSource;
    use fabro_types::fixtures;

    use super::*;
    use crate::config::HookSettings;
    use crate::types::{HookContext, HookEvent, HookResult};

    struct MockExecutor {
        decision: HookDecision,
    }

    #[async_trait::async_trait]
    impl HookExecutor for MockExecutor {
        async fn execute(
            &self,
            definition: &HookDefinition,
            _context: &HookContext,
            _sandbox: Arc<dyn Sandbox>,
            _execution_context: &HookExecutionContext,
            _llm_source: &dyn CredentialSource,
            _catalog: Arc<Catalog>,
            _secrets: &crate::ResolvedHookSecrets,
        ) -> HookResult {
            HookResult {
                hook_name:   definition.name.clone(),
                decision:    self.decision.clone(),
                duration_ms: 1,
            }
        }
    }

    fn make_sandbox() -> Arc<dyn Sandbox> {
        Arc::new(fabro_agent::LocalSandbox::new(
            std::env::current_dir().unwrap(),
        ))
    }

    fn make_context(event: HookEvent) -> HookContext {
        HookContext::new(event, fixtures::RUN_1, "test-wf".into())
    }

    fn test_llm_source() -> Arc<dyn CredentialSource> {
        Arc::new(EnvCredentialSource::new())
    }

    fn test_catalog() -> Arc<Catalog> {
        Arc::new(Catalog::from_builtin().expect("default catalog should build"))
    }

    fn make_hook(event: HookEvent, name: &str) -> HookDefinition {
        HookDefinition {
            name: Some(name.into()),
            event,
            command: Some("echo test".into()),
            hook_type: None,
            matcher: None,
            blocking: None,
            timeout_ms: None,
            sandbox: Some(false),
        }
    }

    #[tokio::test]
    async fn no_hooks_returns_proceed() {
        let runner = HookRunner::new(
            HookSettings::default(),
            test_llm_source(),
            test_catalog(),
            HookSecretResolver::default(),
        );
        let ctx = make_context(HookEvent::RunStart);
        let sandbox = make_sandbox();
        let decision = runner
            .run(&ctx, sandbox.clone(), HookExecutionContext::default())
            .await;
        assert_eq!(decision, HookDecision::Proceed);
    }

    #[tokio::test]
    async fn filters_by_event() {
        let config = HookSettings {
            hooks: vec![
                make_hook(HookEvent::RunStart, "a"),
                make_hook(HookEvent::StageStart, "b"),
            ],
        };
        let runner = HookRunner::with_executor(
            config,
            Arc::new(MockExecutor {
                decision: HookDecision::Proceed,
            }),
        );
        let ctx = make_context(HookEvent::RunStart);
        let matching = runner.filter_hooks(&ctx);
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].name.as_deref(), Some("a"));
    }

    #[tokio::test]
    async fn matcher_filters_by_node_id() {
        let mut hook = make_hook(HookEvent::StageStart, "filtered");
        hook.matcher = Some("agent".into());
        let config = HookSettings { hooks: vec![hook] };
        let runner = HookRunner::with_executor(
            config,
            Arc::new(MockExecutor {
                decision: HookDecision::Proceed,
            }),
        );

        // No node_id — no match
        let ctx = make_context(HookEvent::StageStart);
        assert!(runner.filter_hooks(&ctx).is_empty());

        // Matching node_id
        let mut ctx = make_context(HookEvent::StageStart);
        ctx.node_id = Some("agent_step".into());
        assert_eq!(runner.filter_hooks(&ctx).len(), 1);

        // Non-matching node_id
        let mut ctx = make_context(HookEvent::StageStart);
        ctx.node_id = Some("start".into());
        assert!(runner.filter_hooks(&ctx).is_empty());
    }

    #[tokio::test]
    async fn matcher_filters_by_handler_type() {
        let mut hook = make_hook(HookEvent::StageStart, "filtered");
        hook.matcher = Some("^agent$".into());
        let config = HookSettings { hooks: vec![hook] };
        let runner = HookRunner::with_executor(
            config,
            Arc::new(MockExecutor {
                decision: HookDecision::Proceed,
            }),
        );

        let mut ctx = make_context(HookEvent::StageStart);
        ctx.handler_type = Some("agent".into());
        assert_eq!(runner.filter_hooks(&ctx).len(), 1);

        let mut ctx = make_context(HookEvent::StageStart);
        ctx.handler_type = Some("command".into());
        assert!(runner.filter_hooks(&ctx).is_empty());
    }

    #[tokio::test]
    async fn matcher_filters_by_tool_name() {
        let mut hook = make_hook(HookEvent::PreToolUse, "tool-filter");
        hook.matcher = Some("shell".into());
        let config = HookSettings { hooks: vec![hook] };
        let runner = HookRunner::with_executor(
            config,
            Arc::new(MockExecutor {
                decision: HookDecision::Proceed,
            }),
        );

        // Matches tool_name "shell"
        let mut ctx = make_context(HookEvent::PreToolUse);
        ctx.tool_name = Some("shell".into());
        assert_eq!(runner.filter_hooks(&ctx).len(), 1);

        // Does not match tool_name "read_file"
        let mut ctx = make_context(HookEvent::PreToolUse);
        ctx.tool_name = Some("read_file".into());
        assert!(runner.filter_hooks(&ctx).is_empty());
    }

    #[tokio::test]
    async fn invalid_matcher_pattern_never_matches() {
        // A matcher that fails to compile is dropped at construction (with a
        // warning); the hook must then never match rather than silently match
        // everything.
        let mut hook = make_hook(HookEvent::StageStart, "bad-matcher");
        hook.matcher = Some("[unterminated(".into());
        let config = HookSettings { hooks: vec![hook] };
        let runner = HookRunner::with_executor(
            config,
            Arc::new(MockExecutor {
                decision: HookDecision::Proceed,
            }),
        );

        let mut ctx = make_context(HookEvent::StageStart);
        ctx.node_id = Some("agent".into());
        assert!(runner.filter_hooks(&ctx).is_empty());
    }

    #[tokio::test]
    async fn blocking_hook_block_decision() {
        let config = HookSettings {
            hooks: vec![make_hook(HookEvent::RunStart, "blocker")],
        };
        let runner = HookRunner::with_executor(
            config,
            Arc::new(MockExecutor {
                decision: HookDecision::Block {
                    reason: Some("denied".into()),
                },
            }),
        );
        let ctx = make_context(HookEvent::RunStart);
        let sandbox = make_sandbox();
        let decision = runner
            .run(&ctx, sandbox.clone(), HookExecutionContext::default())
            .await;
        assert!(matches!(decision, HookDecision::Block { .. }));
    }

    #[tokio::test]
    async fn blocking_hook_skip_decision() {
        let mut hook = make_hook(HookEvent::StageStart, "skipper");
        hook.blocking = Some(true);
        let config = HookSettings { hooks: vec![hook] };
        let runner = HookRunner::with_executor(
            config,
            Arc::new(MockExecutor {
                decision: HookDecision::Skip {
                    reason: Some("skip it".into()),
                },
            }),
        );
        let ctx = make_context(HookEvent::StageStart);
        let sandbox = make_sandbox();
        let decision = runner
            .run(&ctx, sandbox.clone(), HookExecutionContext::default())
            .await;
        assert!(matches!(decision, HookDecision::Skip { .. }));
    }

    #[tokio::test]
    async fn non_blocking_hook_doesnt_block() {
        let mut hook = make_hook(HookEvent::StageComplete, "observer");
        hook.blocking = Some(false);
        let config = HookSettings { hooks: vec![hook] };
        let runner = HookRunner::with_executor(
            config,
            Arc::new(MockExecutor {
                decision: HookDecision::Block {
                    reason: Some("ignored".into()),
                },
            }),
        );
        let ctx = make_context(HookEvent::StageComplete);
        let sandbox = make_sandbox();
        let decision = runner
            .run(&ctx, sandbox.clone(), HookExecutionContext::default())
            .await;
        // Non-blocking hooks don't affect the decision
        assert_eq!(decision, HookDecision::Proceed);
    }

    #[tokio::test]
    async fn executor_integration_success() {
        let config = HookSettings {
            hooks: vec![{
                let mut h = make_hook(HookEvent::RunStart, "echo-hook");
                h.command = Some("exit 0".into());
                h
            }],
        };
        let runner = HookRunner::new(
            config,
            test_llm_source(),
            test_catalog(),
            HookSecretResolver::default(),
        );
        let ctx = make_context(HookEvent::RunStart);
        let sandbox = make_sandbox();
        let decision = runner
            .run(&ctx, sandbox.clone(), HookExecutionContext::default())
            .await;
        assert_eq!(decision, HookDecision::Proceed);
    }

    #[tokio::test]
    async fn executor_integration_block() {
        let config = HookSettings {
            hooks: vec![{
                let mut h = make_hook(HookEvent::RunStart, "fail-hook");
                h.command = Some("exit 1".into());
                h
            }],
        };
        let runner = HookRunner::new(
            config,
            test_llm_source(),
            test_catalog(),
            HookSecretResolver::default(),
        );
        let ctx = make_context(HookEvent::RunStart);
        let sandbox = make_sandbox();
        let decision = runner
            .run(&ctx, sandbox.clone(), HookExecutionContext::default())
            .await;
        assert!(matches!(decision, HookDecision::Block { .. }));
    }
}
