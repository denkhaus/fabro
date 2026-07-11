use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::Instant;

use async_trait::async_trait;
use fabro_agent::Sandbox;
use fabro_agent::tool_registry::ToolContext;
use fabro_auth::CredentialSource;
use fabro_llm::client::Client as LlmClient;
use fabro_llm::generate::{GenerateParams, generate_object};
use fabro_llm::types::{Message, Request, ToolResult};
use fabro_model::Catalog;
use fabro_redact::redacted_url_for_log;
use tokio::process::Command as TokioCommand;
use tokio::time::timeout as tokio_timeout;
use tokio_util::sync::CancellationToken;

use crate::config::{RuntimeHookDefinition, RuntimeHookType, RuntimeHttpHook, TlsMode};
use crate::types::{
    HookContext, HookDecision, HookExecutionContext, HookResult, PromptHookResponse,
};

const HOOK_EVALUATOR_SYSTEM_PROMPT: &str = "You are a hook evaluator for a workflow engine. Given context about a workflow event, evaluate the condition.";

static HOOK_RESPONSE_SCHEMA: LazyLock<serde_json::Value> = LazyLock::new(|| {
    serde_json::json!({
        "type": "object",
        "properties": {
            "ok": { "type": "boolean" },
            "reason": { "type": "string" }
        },
        "required": ["ok"],
        "additionalProperties": false
    })
});

fn duration_ms(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Trait for executing hooks via different transports.
///
/// Definitions arrive fully resolved from the run boundary
/// ([`RuntimeHookDefinition`]): the executor formats, dispatches, and merges
/// decisions — it resolves nothing.
#[async_trait]
pub trait HookExecutor: Send + Sync {
    async fn execute(
        &self,
        definition: &RuntimeHookDefinition,
        context: &HookContext,
        sandbox: Arc<dyn Sandbox>,
        execution_context: &HookExecutionContext,
        llm_source: &dyn CredentialSource,
        catalog: Arc<Catalog>,
    ) -> HookResult;
}

/// Executes hooks via shell commands or HTTP POST.
pub struct HookExecutorImpl;

impl HookExecutorImpl {
    /// Parse a hook decision from JSON stdout and exit code.
    fn parse_decision(exit_code: i32, stdout: &str) -> HookDecision {
        if exit_code == 0 {
            // Try parsing JSON response for explicit decision
            if let Ok(decision) = serde_json::from_str::<HookDecision>(stdout.trim()) {
                return decision;
            }
            HookDecision::Proceed
        } else if exit_code == 2 {
            // Exit 2 = block/skip
            if let Ok(decision) = serde_json::from_str::<HookDecision>(stdout.trim()) {
                return decision;
            }
            HookDecision::Block {
                reason: Some("hook exited with code 2".to_string()),
            }
        } else {
            HookDecision::Block {
                reason: Some(format!("hook exited with code {exit_code}")),
            }
        }
    }

    /// Execute a command hook (sandbox or host).
    async fn execute_command(
        definition: &RuntimeHookDefinition,
        command: &str,
        context: &HookContext,
        sandbox: &Arc<dyn Sandbox>,
        execution_context: &HookExecutionContext,
    ) -> HookDecision {
        let context_json = serde_json::to_string(context).unwrap_or_default();
        let timeout_ms = duration_ms(definition.timeout());

        let mut env_vars = HashMap::new();
        env_vars.insert("FABRO_EVENT".to_string(), context.event.to_string());
        env_vars.insert("FABRO_RUN_ID".to_string(), context.run_id.to_string());
        env_vars.insert("FABRO_WORKFLOW".to_string(), context.workflow_name.clone());
        if let Some(ref node_id) = context.node_id {
            env_vars.insert("FABRO_NODE_ID".to_string(), node_id.clone());
        }

        if definition.runs_in_sandbox() {
            let ctx_path = format!(
                "/tmp/fabro-hook-context-{}.json",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            );
            if sandbox.write_file(&ctx_path, &context_json).await.is_ok() {
                env_vars.insert("FABRO_HOOK_CONTEXT".to_string(), ctx_path.clone());
            }
            let sandbox_work_dir = execution_context
                .command_cwd_for(definition)
                .map(|path| path.to_string_lossy().to_string());
            match sandbox
                .exec_command(
                    command,
                    timeout_ms,
                    sandbox_work_dir.as_deref(),
                    Some(&env_vars),
                    None,
                )
                .await
            {
                Ok(result) => Self::parse_decision(result.exit_code.unwrap_or(-1), &result.stdout),
                Err(e) => HookDecision::Block {
                    reason: Some(format!("sandbox exec failed: {e}")),
                },
            }
        } else {
            let mut cmd = TokioCommand::new("sh");
            cmd.arg("-c").arg(command);
            if let Some(wd) = execution_context.command_cwd_for(definition) {
                cmd.current_dir(wd);
            }
            for (k, v) in &env_vars {
                cmd.env(k, v);
            }
            cmd.stdin(std::process::Stdio::piped());
            cmd.stdout(std::process::Stdio::piped());
            cmd.stderr(std::process::Stdio::piped());

            match cmd.spawn() {
                Ok(mut child) => {
                    if let Some(mut stdin) = child.stdin.take() {
                        use tokio::io::AsyncWriteExt;
                        let _ = stdin.write_all(context_json.as_bytes()).await;
                    }
                    match child.wait_with_output().await {
                        Ok(output) => {
                            let exit_code = output.status.code().unwrap_or(1);
                            let stdout = String::from_utf8_lossy(&output.stdout);
                            Self::parse_decision(exit_code, &stdout)
                        }
                        Err(e) => HookDecision::Block {
                            reason: Some(format!("command wait failed: {e}")),
                        },
                    }
                }
                Err(e) => HookDecision::Block {
                    reason: Some(format!("command spawn failed: {e}")),
                },
            }
        }
    }

    /// Strip markdown code fences from LLM responses.
    ///
    /// LLMs often wrap JSON in ```json ... ``` blocks.
    fn strip_code_fences(text: &str) -> &str {
        let trimmed = text.trim();
        let inner = trimmed
            .strip_prefix("```json")
            .or_else(|| trimmed.strip_prefix("```"))
            .unwrap_or(trimmed);
        let inner = inner.strip_suffix("```").unwrap_or(inner);
        inner.trim()
    }

    /// Parse a prompt/agent hook LLM response into a `HookDecision`.
    ///
    /// Fail-open: invalid JSON or missing fields → `Proceed`.
    pub fn parse_prompt_response(response_text: &str) -> HookDecision {
        let cleaned = Self::strip_code_fences(response_text);
        match serde_json::from_str::<PromptHookResponse>(cleaned) {
            Ok(resp) if resp.ok => HookDecision::Proceed,
            Ok(resp) => HookDecision::Block {
                reason: resp.reason,
            },
            Err(e) => {
                tracing::warn!(error = %e, "prompt hook response parse failed, proceeding");
                HookDecision::Proceed
            }
        }
    }

    /// Resolve a model alias (e.g. "haiku") to a concrete model ID.
    fn resolve_model(model: Option<&str>, catalog: &Catalog) -> String {
        let model_id = model.unwrap_or("haiku");
        let model_info = catalog.get(model_id);
        model_info.map_or(model_id, |m| m.id.as_str()).to_string()
    }

    /// Build the user message for prompt/agent hooks.
    fn build_hook_user_message(prompt: &str, context: &HookContext) -> String {
        let context_json = serde_json::to_string(context).unwrap_or_default();
        format!("Hook prompt: {prompt}\n\nEvent context:\n{context_json}")
    }

    /// Execute an LLM hook with a timeout, failing open on error or timeout.
    async fn execute_llm_with_timeout<F, Fut>(
        timeout: std::time::Duration,
        hook_kind: &str,
        f: F,
    ) -> HookDecision
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = HookDecision>,
    {
        if let Ok(decision) = tokio_timeout(timeout, f()).await {
            decision
        } else {
            tracing::warn!("{hook_kind} hook timed out, proceeding");
            HookDecision::Proceed
        }
    }

    /// Execute a prompt hook: single-turn LLM call returning ok/block.
    async fn execute_prompt(
        definition: &RuntimeHookDefinition,
        prompt: &str,
        model: Option<&str>,
        context: &HookContext,
        llm_source: &dyn CredentialSource,
        catalog: Arc<Catalog>,
    ) -> HookDecision {
        let resolved_model = Self::resolve_model(model, catalog.as_ref());
        let user_msg = Self::build_hook_user_message(prompt, context);

        Self::execute_llm_with_timeout(definition.timeout(), "prompt", || async move {
            let client = match LlmClient::from_source(llm_source, catalog).await {
                Ok(client) => Arc::new(client),
                Err(e) => {
                    tracing::warn!(error = %e, "prompt hook client creation failed, proceeding");
                    return HookDecision::Proceed;
                }
            };

            let params = GenerateParams::new(&resolved_model, client)
                .system(HOOK_EVALUATOR_SYSTEM_PROMPT)
                .prompt(user_msg)
                .max_tokens(1024);

            match generate_object(params, HOOK_RESPONSE_SCHEMA.clone()).await {
                Ok(result) => if let Some(obj) = result.output { match serde_json::from_value::<PromptHookResponse>(obj) {
                    Ok(resp) if resp.ok => HookDecision::Proceed,
                    Ok(resp) => HookDecision::Block {
                        reason: resp.reason,
                    },
                    Err(e) => {
                        tracing::warn!(error = %e, "prompt hook response deserialize failed, proceeding");
                        HookDecision::Proceed
                    }
                } } else {
                    tracing::warn!("prompt hook returned no structured output, proceeding");
                    HookDecision::Proceed
                },
                Err(e) => {
                    tracing::warn!(error = %e, "prompt hook LLM call failed, proceeding");
                    HookDecision::Proceed
                }
            }
        })
        .await
    }

    /// Execute an agent hook: multi-turn LLM call with sandbox tool access.
    ///
    /// Reuses the core `ToolRegistry` from `fabro_agent` so the agent hook has
    /// the same tools (read_file, write_file, shell, grep, glob, etc.) as
    /// a normal agent session.
    async fn execute_agent(
        definition: &RuntimeHookDefinition,
        prompt: &str,
        model: Option<&str>,
        max_tool_rounds: Option<u32>,
        context: &HookContext,
        sandbox: Arc<dyn Sandbox>,
        llm_source: &dyn CredentialSource,
        catalog: Arc<Catalog>,
    ) -> HookDecision {
        let resolved_model = Self::resolve_model(model, catalog.as_ref());
        let user_msg = Self::build_hook_user_message(prompt, context);

        Self::execute_llm_with_timeout(definition.timeout(), "agent", || async move {
            let client = match LlmClient::from_source(llm_source, catalog).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(error = %e, "agent hook client creation failed, proceeding");
                    return HookDecision::Proceed;
                }
            };

            let config = fabro_agent::SessionOptions::default();
            let mut registry = fabro_agent::ToolRegistry::new();
            fabro_agent::register_core_tools(&mut registry, &config, None);
            let tool_defs = registry.definitions();

            let mut messages = vec![
                Message::system(HOOK_EVALUATOR_SYSTEM_PROMPT),
                Message::user(user_msg),
            ];

            let rounds = max_tool_rounds.unwrap_or(50);
            let cancel = CancellationToken::new();

            for _ in 0..rounds {
                let request = Request {
                    model:            resolved_model.clone(),
                    messages:         messages.clone(),
                    provider:         None,
                    tools:            Some(tool_defs.clone()),
                    tool_choice:      None,
                    response_format:  None,
                    temperature:      None,
                    top_p:            None,
                    max_tokens:       None,
                    stop_sequences:   None,
                    reasoning_effort: None,
                    speed:            None,
                    metadata:         None,
                    provider_options: None,
                };

                let response = match client.complete(&request).await {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!(error = %e, "agent hook LLM call failed, proceeding");
                        return HookDecision::Proceed;
                    }
                };

                let tool_calls = response.tool_calls();
                if tool_calls.is_empty() {
                    return Self::parse_prompt_response(&response.text());
                }

                messages.push(response.message.clone());

                for tc in &tool_calls {
                    let tool = registry.get(&tc.name).cloned();
                    let ctx = ToolContext {
                        env:                 sandbox.clone(),
                        cancel:              cancel.child_token(),
                        tool_env_provider:   None,
                        session_id:          None,
                        root_session_id:     None,
                        tool_call_id:        Some(tc.id.clone()),
                        agent_event_emitter: None,
                    };
                    let result = match tool {
                        Some(t) => match (t.executor)(tc.arguments.clone(), ctx).await {
                            Ok(output) => {
                                ToolResult::success(tc.id.clone(), serde_json::json!(output))
                            }
                            Err(err) => ToolResult::error(tc.id.clone(), err),
                        },
                        None => {
                            ToolResult::error(tc.id.clone(), format!("Unknown tool: {}", tc.name))
                        }
                    };
                    messages.push(Message::tool_result(
                        result.tool_call_id,
                        result.content,
                        result.is_error,
                    ));
                }
            }

            tracing::warn!("agent hook exhausted max tool rounds, proceeding");
            HookDecision::Proceed
        })
        .await
    }

    /// Build an HTTP client for the given TLS mode.
    fn build_http_client(tls: TlsMode) -> fabro_http::HttpClient {
        let accept_invalid = matches!(tls, TlsMode::NoVerify | TlsMode::Off);
        #[cfg(test)]
        {
            fabro_http::HttpClientBuilder::new()
                .danger_accept_invalid_certs(accept_invalid)
                .no_proxy()
                .build()
                .expect("hook HTTP client should build")
        }
        #[cfg(not(test))]
        {
            fabro_http::HttpClientBuilder::new()
                .danger_accept_invalid_certs(accept_invalid)
                .build()
                .expect("hook HTTP client should build")
        }
    }

    /// Execute an HTTP hook: POST context JSON and parse the response.
    ///
    /// Transport outcomes (non-2xx, connection errors, unparseable body) are
    /// fail-open and return `Proceed`. Logging uses `http.url_source` — the
    /// unresolved config source carried on the runtime type — so the resolved
    /// URL, which may embed secret material, is never logged.
    async fn execute_http(
        client: &fabro_http::HttpClient,
        http: &RuntimeHttpHook,
        context: &HookContext,
        timeout: std::time::Duration,
    ) -> HookDecision {
        // Enforce URL scheme based on TLS mode
        match http.tls {
            TlsMode::Verify | TlsMode::NoVerify => {
                if !http.url.starts_with("https://") {
                    return HookDecision::Block {
                        reason: Some(format!(
                            "HTTP hook URL must use https:// (tls mode is {:?})",
                            http.tls
                        )),
                    };
                }
            }
            TlsMode::Off => {}
        }

        let mut request = client.post(&http.url).timeout(timeout).json(context);

        if let Some(hdrs) = &http.headers {
            for (key, value) in hdrs {
                request = request.header(key, value);
            }
        }

        let response = match request.send().await {
            Ok(resp) => resp,
            Err(e) => {
                tracing::warn!(
                    url_source = %redacted_url_for_log(&http.url_source),
                    error = %e,
                    "HTTP hook request failed, proceeding"
                );
                return HookDecision::Proceed;
            }
        };

        if !response.status().is_success() {
            tracing::warn!(
                url_source = %redacted_url_for_log(&http.url_source),
                status = response.status().as_u16(),
                "HTTP hook returned non-2xx, proceeding"
            );
            return HookDecision::Proceed;
        }

        let body = match response.text().await {
            Ok(text) => text,
            Err(e) => {
                tracing::warn!(
                    url_source = %redacted_url_for_log(&http.url_source),
                    error = %e,
                    "HTTP hook body read failed, proceeding"
                );
                return HookDecision::Proceed;
            }
        };

        if body.trim().is_empty() {
            return HookDecision::Proceed;
        }

        match serde_json::from_str::<HookDecision>(body.trim()) {
            Ok(decision) => decision,
            Err(e) => {
                tracing::warn!(
                    url_source = %redacted_url_for_log(&http.url_source),
                    error = %e,
                    "HTTP hook response parse failed, proceeding"
                );
                HookDecision::Proceed
            }
        }
    }
}

/// Cached HTTP clients keyed by TLS mode.
struct HttpClientCache {
    verify:    fabro_http::HttpClient,
    no_verify: fabro_http::HttpClient,
    off:       fabro_http::HttpClient,
}

impl HttpClientCache {
    fn new() -> Self {
        Self {
            verify:    HookExecutorImpl::build_http_client(TlsMode::Verify),
            no_verify: HookExecutorImpl::build_http_client(TlsMode::NoVerify),
            off:       HookExecutorImpl::build_http_client(TlsMode::Off),
        }
    }

    fn get(&self, tls: TlsMode) -> &fabro_http::HttpClient {
        match tls {
            TlsMode::Verify => &self.verify,
            TlsMode::NoVerify => &self.no_verify,
            TlsMode::Off => &self.off,
        }
    }
}

impl Default for HttpClientCache {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HookExecutor for HookExecutorImpl {
    async fn execute(
        &self,
        definition: &RuntimeHookDefinition,
        context: &HookContext,
        sandbox: Arc<dyn Sandbox>,
        execution_context: &HookExecutionContext,
        llm_source: &dyn CredentialSource,
        catalog: Arc<Catalog>,
    ) -> HookResult {
        use std::sync::OnceLock;
        static HTTP_CLIENTS: OnceLock<HttpClientCache> = OnceLock::new();

        let start = Instant::now();

        let decision = match &definition.hook_type {
            Some(RuntimeHookType::Command { command }) => {
                Self::execute_command(definition, command, context, &sandbox, execution_context)
                    .await
            }
            Some(RuntimeHookType::Http(http)) => {
                let clients = HTTP_CLIENTS.get_or_init(HttpClientCache::new);
                Self::execute_http(clients.get(http.tls), http, context, definition.timeout()).await
            }
            Some(RuntimeHookType::Prompt { prompt, model }) => {
                Self::execute_prompt(
                    definition,
                    prompt,
                    model.as_deref(),
                    context,
                    llm_source,
                    Arc::clone(&catalog),
                )
                .await
            }
            Some(RuntimeHookType::Agent {
                prompt,
                model,
                max_tool_rounds,
            }) => {
                Self::execute_agent(
                    definition,
                    prompt,
                    model.as_deref(),
                    *max_tool_rounds,
                    context,
                    sandbox,
                    llm_source,
                    Arc::clone(&catalog),
                )
                .await
            }
            None => HookDecision::Block {
                reason: Some("no hook type specified".into()),
            },
        };

        let duration_ms = duration_ms(start.elapsed());
        HookResult {
            hook_name: definition.name.clone(),
            decision,
            duration_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use fabro_auth::{CredentialSource, EnvCredentialSource};
    use fabro_types::fixtures;

    use super::*;
    use crate::types::HookEvent;

    fn make_context() -> HookContext {
        HookContext::new(HookEvent::StageStart, fixtures::RUN_1, "test-wf".into())
    }

    fn make_sandbox() -> Arc<dyn Sandbox> {
        Arc::new(fabro_agent::LocalSandbox::new(
            std::env::current_dir().unwrap(),
        ))
    }

    fn test_llm_source() -> Arc<dyn CredentialSource> {
        Arc::new(EnvCredentialSource::new())
    }

    fn test_catalog() -> Arc<Catalog> {
        Arc::new(Catalog::from_builtin().unwrap())
    }

    fn test_http_client() -> fabro_http::HttpClient {
        HookExecutorImpl::build_http_client(TlsMode::Off)
    }

    fn make_typed_definition(hook_type: Option<RuntimeHookType>) -> RuntimeHookDefinition {
        RuntimeHookDefinition {
            name: Some("test-hook".into()),
            event: HookEvent::StageStart,
            hook_type,
            matcher: None,
            blocking: None,
            timeout_ms: Some(5000),
            sandbox: Some(false), // host execution for tests
            effective_name: "test-hook".into(),
        }
    }

    fn make_definition(command: &str) -> RuntimeHookDefinition {
        make_typed_definition(Some(RuntimeHookType::Command {
            command: command.to_string(),
        }))
    }

    #[test]
    fn parse_decision_exit_0_proceed() {
        assert_eq!(
            HookExecutorImpl::parse_decision(0, ""),
            HookDecision::Proceed
        );
    }

    #[test]
    fn parse_decision_exit_0_with_json() {
        let json = r#"{"decision": "skip", "reason": "not needed"}"#;
        assert_eq!(
            HookExecutorImpl::parse_decision(0, json),
            HookDecision::Skip {
                reason: Some("not needed".into()),
            }
        );
    }

    #[test]
    fn parse_decision_exit_2_block() {
        assert!(matches!(
            HookExecutorImpl::parse_decision(2, ""),
            HookDecision::Block { .. }
        ));
    }

    #[test]
    fn parse_decision_exit_2_with_json() {
        let json = r#"{"decision": "skip", "reason": "skipping"}"#;
        assert_eq!(
            HookExecutorImpl::parse_decision(2, json),
            HookDecision::Skip {
                reason: Some("skipping".into()),
            }
        );
    }

    #[test]
    fn parse_decision_exit_1_block() {
        assert!(matches!(
            HookExecutorImpl::parse_decision(1, ""),
            HookDecision::Block { .. }
        ));
    }

    #[test]
    fn parse_decision_exit_0_override() {
        let json = r#"{"decision": "override", "edge_to": "node_b"}"#;
        assert_eq!(
            HookExecutorImpl::parse_decision(0, json),
            HookDecision::Override {
                edge_to: "node_b".into(),
            }
        );
    }

    #[tokio::test]
    async fn command_executor_host_success() {
        let executor = HookExecutorImpl;
        let def = make_definition("exit 0");
        let ctx = make_context();
        let sandbox = make_sandbox();
        let source = test_llm_source();
        let result = executor
            .execute(
                &def,
                &ctx,
                sandbox,
                &HookExecutionContext::default(),
                source.as_ref(),
                test_catalog(),
            )
            .await;
        assert_eq!(result.decision, HookDecision::Proceed);
        assert_eq!(result.hook_name.as_deref(), Some("test-hook"));
    }

    #[tokio::test]
    async fn command_executor_host_failure() {
        let executor = HookExecutorImpl;
        let def = make_definition("exit 1");
        let ctx = make_context();
        let sandbox = make_sandbox();
        let source = test_llm_source();
        let result = executor
            .execute(
                &def,
                &ctx,
                sandbox,
                &HookExecutionContext::default(),
                source.as_ref(),
                test_catalog(),
            )
            .await;
        assert!(matches!(result.decision, HookDecision::Block { .. }));
    }

    #[tokio::test]
    async fn command_executor_host_skip_via_exit_2() {
        let executor = HookExecutorImpl;
        let def = make_definition("exit 2");
        let ctx = make_context();
        let sandbox = make_sandbox();
        let source = test_llm_source();
        let result = executor
            .execute(
                &def,
                &ctx,
                sandbox,
                &HookExecutionContext::default(),
                source.as_ref(),
                test_catalog(),
            )
            .await;
        assert!(matches!(result.decision, HookDecision::Block { .. }));
    }

    #[tokio::test]
    async fn command_executor_host_json_decision() {
        let executor = HookExecutorImpl;
        let def = make_definition(r#"echo '{"decision": "skip", "reason": "test skip"}'"#);
        let ctx = make_context();
        let sandbox = make_sandbox();
        let source = test_llm_source();
        let result = executor
            .execute(
                &def,
                &ctx,
                sandbox,
                &HookExecutionContext::default(),
                source.as_ref(),
                test_catalog(),
            )
            .await;
        assert_eq!(result.decision, HookDecision::Skip {
            reason: Some("test skip".into()),
        });
    }

    #[tokio::test]
    async fn command_executor_env_vars_set() {
        let executor = HookExecutorImpl;
        // Print env vars to stdout for verification
        let def = make_definition("echo $ARC_EVENT:$ARC_RUN_ID:$ARC_WORKFLOW");
        let mut ctx = make_context();
        ctx.node_id = Some("plan".into());
        let sandbox = make_sandbox();
        let source = test_llm_source();
        let result = executor
            .execute(
                &def,
                &ctx,
                sandbox,
                &HookExecutionContext::default(),
                source.as_ref(),
                test_catalog(),
            )
            .await;
        assert_eq!(result.decision, HookDecision::Proceed);
    }

    #[tokio::test]
    async fn no_hook_type_blocks() {
        let executor = HookExecutorImpl;
        let def = make_typed_definition(None);
        let ctx = make_context();
        let sandbox = make_sandbox();
        let source = test_llm_source();
        let result = executor
            .execute(
                &def,
                &ctx,
                sandbox,
                &HookExecutionContext::default(),
                source.as_ref(),
                test_catalog(),
            )
            .await;
        assert!(matches!(result.decision, HookDecision::Block { .. }));
    }

    // --- parse_prompt_response tests ---

    #[test]
    fn parse_prompt_response_ok_true() {
        assert_eq!(
            HookExecutorImpl::parse_prompt_response(r#"{"ok": true}"#),
            HookDecision::Proceed,
        );
    }

    #[test]
    fn parse_prompt_response_ok_false() {
        assert_eq!(
            HookExecutorImpl::parse_prompt_response(r#"{"ok": false, "reason": "tests failing"}"#),
            HookDecision::Block {
                reason: Some("tests failing".into()),
            },
        );
    }

    #[test]
    fn parse_prompt_response_ok_false_no_reason() {
        assert_eq!(
            HookExecutorImpl::parse_prompt_response(r#"{"ok": false}"#),
            HookDecision::Block { reason: None },
        );
    }

    #[test]
    fn parse_prompt_response_invalid_json() {
        assert_eq!(
            HookExecutorImpl::parse_prompt_response("not json"),
            HookDecision::Proceed,
        );
    }

    #[test]
    fn parse_prompt_response_strips_code_fences() {
        assert_eq!(
            HookExecutorImpl::parse_prompt_response(
                "```json\n{\"ok\": false, \"reason\": \"no\"}\n```"
            ),
            HookDecision::Block {
                reason: Some("no".into()),
            },
        );
    }

    #[test]
    fn strip_code_fences_plain() {
        assert_eq!(
            HookExecutorImpl::strip_code_fences(r#"{"ok": true}"#),
            r#"{"ok": true}"#
        );
    }

    #[test]
    fn strip_code_fences_json() {
        assert_eq!(
            HookExecutorImpl::strip_code_fences("```json\n{\"ok\": true}\n```"),
            "{\"ok\": true}"
        );
    }

    #[test]
    fn strip_code_fences_bare() {
        assert_eq!(
            HookExecutorImpl::strip_code_fences("```\n{\"ok\": true}\n```"),
            "{\"ok\": true}"
        );
    }

    // Token resolution tests live with the boundary resolver
    // (`HookDefinition::resolve_env` in fabro-types); the executor only ever
    // sees resolved strings.

    // --- HTTP hook execution tests ---

    /// Call `execute_http` with the URL doubling as its own (already literal)
    /// source, which is exactly what the boundary produces for token-free
    /// config URLs.
    async fn execute_http_for_test(
        url: &str,
        headers: Option<HashMap<String, String>>,
        tls: TlsMode,
        timeout: std::time::Duration,
    ) -> HookDecision {
        let client = test_http_client();
        let http = RuntimeHttpHook {
            url: url.to_string(),
            url_source: url.to_string(),
            headers,
            tls,
        };
        HookExecutorImpl::execute_http(&client, &http, &make_context(), timeout).await
    }

    #[tokio::test]
    async fn http_hook_posts_json_and_parses_decision() {
        let server = httpmock::MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method("POST")
                    .path("/hook")
                    .header("content-type", "application/json");
                then.status(200)
                    .body(r#"{"decision": "skip", "reason": "not needed"}"#);
            })
            .await;

        let decision = execute_http_for_test(
            &server.url("/hook"),
            None,
            TlsMode::Off,
            std::time::Duration::from_secs(5),
        )
        .await;

        mock.assert_async().await;
        assert_eq!(decision, HookDecision::Skip {
            reason: Some("not needed".into()),
        });
    }

    #[tokio::test]
    async fn http_hook_empty_2xx_returns_proceed() {
        let server = httpmock::MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method("POST").path("/hook");
                then.status(200).body("");
            })
            .await;

        let decision = execute_http_for_test(
            &server.url("/hook"),
            None,
            TlsMode::Off,
            std::time::Duration::from_secs(5),
        )
        .await;

        mock.assert_async().await;
        assert_eq!(decision, HookDecision::Proceed);
    }

    #[tokio::test]
    async fn http_hook_non_2xx_returns_proceed() {
        let server = httpmock::MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method("POST").path("/hook");
                then.status(500).body("Internal Server Error");
            })
            .await;

        let decision = execute_http_for_test(
            &server.url("/hook"),
            None,
            TlsMode::Off,
            std::time::Duration::from_secs(5),
        )
        .await;

        mock.assert_async().await;
        assert_eq!(decision, HookDecision::Proceed);
    }

    #[tokio::test]
    async fn http_hook_connection_failure_returns_proceed() {
        let decision = execute_http_for_test(
            "http://127.0.0.1:1",
            None,
            TlsMode::Off,
            std::time::Duration::from_secs(1),
        )
        .await;

        assert_eq!(decision, HookDecision::Proceed);
    }

    #[tokio::test]
    async fn http_hook_sends_configured_headers() {
        let server = httpmock::MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method("POST")
                    .path("/hook")
                    .header("authorization", "Bearer my-secret");
                then.status(200).body("");
            })
            .await;

        let headers =
            HashMap::from([("Authorization".to_string(), "Bearer my-secret".to_string())]);

        let decision = execute_http_for_test(
            &server.url("/hook"),
            Some(headers),
            TlsMode::Off,
            std::time::Duration::from_secs(5),
        )
        .await;

        mock.assert_async().await;
        assert_eq!(decision, HookDecision::Proceed);
    }

    // --- TLS mode enforcement tests ---

    #[tokio::test]
    async fn http_hook_rejects_http_url_when_tls_verify() {
        let decision = execute_http_for_test(
            "http://example.com/hook",
            None,
            TlsMode::Verify,
            std::time::Duration::from_secs(5),
        )
        .await;

        assert!(matches!(decision, HookDecision::Block { .. }));
    }

    #[tokio::test]
    async fn http_hook_rejects_http_url_when_tls_no_verify() {
        let decision = execute_http_for_test(
            "http://example.com/hook",
            None,
            TlsMode::NoVerify,
            std::time::Duration::from_secs(5),
        )
        .await;

        assert!(matches!(decision, HookDecision::Block { .. }));
    }

    #[tokio::test]
    async fn http_hook_allows_http_url_when_tls_off() {
        let server = httpmock::MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method("POST").path("/hook");
                then.status(200).body("");
            })
            .await;

        let decision = execute_http_for_test(
            &server.url("/hook"),
            None,
            TlsMode::Off,
            std::time::Duration::from_secs(5),
        )
        .await;

        mock.assert_async().await;
        assert_eq!(decision, HookDecision::Proceed);
    }

    #[tokio::test]
    async fn executor_dispatches_http_hook() {
        let server = httpmock::MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method("POST").path("/hook");
                then.status(200).body(r#"{"decision": "proceed"}"#);
            })
            .await;

        let executor = HookExecutorImpl;
        let def = RuntimeHookDefinition {
            name:           Some("http-test".into()),
            event:          HookEvent::StageStart,
            hook_type:      Some(RuntimeHookType::Http(RuntimeHttpHook {
                url:        server.url("/hook"),
                url_source: server.url("/hook"),
                headers:    None,
                tls:        TlsMode::Off,
            })),
            matcher:        None,
            blocking:       None,
            timeout_ms:     Some(5000),
            sandbox:        Some(false),
            effective_name: "http-test".into(),
        };
        let ctx = make_context();
        let sandbox = make_sandbox();
        let source = test_llm_source();
        let result = executor
            .execute(
                &def,
                &ctx,
                sandbox,
                &HookExecutionContext::default(),
                source.as_ref(),
                test_catalog(),
            )
            .await;

        mock.assert_async().await;
        assert_eq!(result.decision, HookDecision::Proceed);
        assert_eq!(result.hook_name.as_deref(), Some("http-test"));
    }
}
