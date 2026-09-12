//! `fabro exec`: one agentic coding session in the current directory.
//!
//! The session is pebble's coding agent over a local sandbox. Model calls go
//! either straight to the provider with the CLI's credentials or through a
//! Fabro server's completions endpoint when a server target is set.

use std::collections::HashMap;
use std::io::IsTerminal as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result as AnyResult};
use async_trait::async_trait;
use fabro_llm::credentials::CredentialProvider;
use fabro_llm::gateway::{GatewayAdapter, GatewayError, GatewayTransport};
use fabro_llm::lithos_catalog::{Catalog, CatalogProvider};
use fabro_llm::middleware::{Call, Middleware, Next, Output};
use fabro_llm::{Client, ClientOptions, Error as LlmError, ErrorKind};
use fabro_mcp::config::McpServerSettings;
use fabro_mcp::pebble::pebble_servers;
use fabro_sandbox::{RunSandbox, SecretRedactor, local_sandbox};
use fabro_static::EnvVars;
use fabro_types::PermissionLevel;
use fabro_types::settings::cli::OutputFormat as SettingsOutputFormat;
use fabro_types::settings::run::ResolvedMcpEntry;
use fabro_util::exit::{self, ErrorExt, ExitClass};
use fabro_util::home::Home;
use fabro_util::terminal::Styles;
use fabro_workflow::web_search::{self, SearchSecrets};
use lithos_llm::catalog::ProviderId;
use pebble_agent::{ToolCallRequest, ToolSystemError};
use pebble_coding_agent::environment::Environment;
use pebble_coding_agent::events::{CodingAgentEvent, CodingEvent};
use pebble_coding_agent::state::Message;
use pebble_coding_agent::subagents::SubagentOptions;
use pebble_coding_agent::tools::{
    ApprovalDecision, PermissionLevelPolicy, PermissionMiddleware, ToolApprovalService,
};
use pebble_coding_agent::{
    CodingAgent, CodingAgentOptions, MemoryDiscovery, ShutdownReason, SkillDiscovery,
};
use tokio::io::{AsyncWriteExt, stdout};
use tokio::signal;
use tokio::task::spawn_blocking;
use tokio_util::sync::CancellationToken;

use crate::args::{AgentArgs, ExecArgs, ExecOutputFormat};
use crate::command_context::CommandContext;
#[cfg(feature = "sleep_inhibitor")]
use crate::sleep_inhibitor;
use crate::{server_client, user_config};

/// Posts completions to a Fabro server through the authenticated CLI client.
struct ServerCompletionTransport {
    client:   server_client::Client,
    base_url: String,
}

impl ServerCompletionTransport {
    fn new(client: server_client::Client) -> Self {
        let base_url = client.base_url();
        Self { client, base_url }
    }
}

#[async_trait]
impl GatewayTransport for ServerCompletionTransport {
    async fn post_completion(
        &self,
        body: serde_json::Value,
    ) -> Result<fabro_http::Response, GatewayError> {
        let url = format!("{}/api/v1/completions", self.base_url);
        let response = self
            .client
            .send_http_response(|http_client| {
                let body = body.clone();
                let url = url.clone();
                async move { http_client.post(url).json(&body).send().await }
            })
            .await
            .map_err(|err| GatewayError::Transport {
                auth:    exit::exit_class_for(&err) == Some(ExitClass::AuthRequired),
                message: err.to_string(),
            })?;
        response.map_err(|failure| GatewayError::Status {
            status:  failure.status.as_u16(),
            headers: failure.headers,
            body:    failure.body,
        })
    }
}

/// How a failed session is reported: a model failure by what the provider
/// said, everything else by the agent's own description.
#[derive(Debug, thiserror::Error)]
enum SessionError {
    #[error("LLM error: {0}")]
    Llm(fabro_llm::ErrorData),
    #[error(transparent)]
    Agent(pebble_coding_agent::Error),
}

impl From<pebble_coding_agent::Error> for SessionError {
    fn from(error: pebble_coding_agent::Error) -> Self {
        match error.llm_source() {
            Some(llm) => Self::Llm(llm.data()),
            None => Self::Agent(error),
        }
    }
}

fn classify_server_agent_auth(err: anyhow::Error) -> anyhow::Error {
    let is_auth = err.chain().any(|cause| {
        cause
            .downcast_ref::<SessionError>()
            .is_some_and(|error| {
                matches!(error, SessionError::Llm(data) if data.kind() == ErrorKind::Authentication)
            })
    });
    if is_auth {
        err.classify(ExitClass::AuthRequired)
    } else {
        err
    }
}

fn run_mcp_servers_for_exec(
    mcps: &HashMap<String, ResolvedMcpEntry>,
) -> AnyResult<Vec<McpServerSettings>> {
    mcps.iter()
        .map(|(key, entry)| match entry {
            ResolvedMcpEntry::Resolved(server) => Ok(server.clone()),
            ResolvedMcpEntry::Reference(reference) => {
                anyhow::bail!(
                    "fabro exec cannot resolve run.agent.mcps.{key} catalog reference \
                     (id `{}`); define an inline server under [cli.exec.agent.mcps.{key}] or \
                     remove the run-level reference",
                    reference.id
                );
            }
        })
        .collect()
}

pub(crate) async fn execute(mut args: ExecArgs, ctx: &CommandContext) -> AnyResult<()> {
    let cli = &ctx.user_settings().cli;
    #[cfg(feature = "sleep_inhibitor")]
    let _sleep_guard = sleep_inhibitor::guard(cli.exec.prevent_idle_sleep);
    let provider_str = cli.exec.model.provider.as_deref();
    let model_str = cli.exec.model.name.as_deref();
    let permissions = cli.exec.agent.permissions;
    let output_format = Some(match cli.output.format {
        SettingsOutputFormat::Text => ExecOutputFormat::Text,
        SettingsOutputFormat::Json => ExecOutputFormat::Json,
    });
    args.agent
        .apply_cli_defaults(provider_str, model_str, permissions, output_format);
    let server_target = user_config::exec_server_target(&args.server)?;
    // v2 MCPs live under `cli.exec.agent.mcps` (owner-specific) or
    // `run.agent.mcps`. For `fabro exec` we use the cli.exec path, falling
    // back to run.agent.mcps if unset.
    let mcp_servers: Vec<McpServerSettings> = match cli.exec.agent.mcps.as_ref() {
        Some(mcps) => mcps.values().cloned().collect(),
        None => ctx
            .run_settings()
            .ok()
            .map(|settings| run_mcp_servers_for_exec(&settings.agent.mcps))
            .transpose()?
            .unwrap_or_default(),
    };
    // Fully validate MCP transport config at the exec boundary. `fabro exec`
    // has no server vault, so secret and unsupported tokens fail instead of
    // reaching the transport.
    let mcp_servers = mcp_servers
        .into_iter()
        .map(|settings| {
            settings
                .resolve_transport_secrets(|_| None)
                .with_context(|| format!("failed to resolve MCP server {:?}", settings.name))
        })
        .collect::<AnyResult<Vec<_>>>()?;
    // Resolve color support once, leak to get 'static lifetime for use across
    // threads.
    let styles: &'static Styles = Box::leak(Box::new(Styles::detect_stderr()));
    if let Some(target) = server_target {
        tracing::info!(transport = "server", "Agent session starting");
        let provider_name = args
            .agent
            .provider
            .clone()
            .unwrap_or_else(|| "anthropic".to_string());
        let catalog = ctx.catalog()?;
        let provider_id = catalog.enabled_provider(&provider_name).map_or_else(
            || ProviderId::new(provider_name.as_str()),
            |provider| provider.id().clone(),
        );
        let server_client = server_client::connect_server_target(&target).await?;
        let adapter = Arc::new(GatewayAdapter::new(Box::new(
            ServerCompletionTransport::new(server_client),
        )));
        // The server inlines attachments and is the billing authority, so the
        // local client only routes and reports diagnostics.
        let mut options = cli_client_options(&args.agent, styles);
        options.inline_attachments = false;
        let client = fabro_llm::build_offline_client(
            Catalog::clone(&catalog),
            options.with_adapter(provider_id, adapter),
        )
        .context("Failed to register fabro server adapter")?
        .client;
        run_session(args.agent, client, mcp_servers, catalog, styles)
            .await
            .map_err(classify_server_agent_auth)?;
    } else {
        tracing::info!(transport = "direct", "Agent session starting");
        let llm_source = ctx.llm_source().await?;
        let catalog = ctx.catalog()?;
        let client = build_direct_client(&args.agent, llm_source, &catalog, styles).await?;
        run_session(args.agent, client, mcp_servers, catalog, styles).await?;
    }

    Ok(())
}

#[allow(
    clippy::print_stderr,
    reason = "Provider build issues are diagnostics for the person running the CLI."
)]
async fn build_direct_client(
    args: &AgentArgs,
    llm_source: Arc<dyn CredentialProvider>,
    catalog: &Arc<Catalog>,
    styles: &'static Styles,
) -> AnyResult<Client> {
    let built = fabro_llm::build_client(
        Catalog::clone(catalog),
        llm_source,
        cli_client_options(args, styles),
    )
    .await
    .context("Failed to create LLM client")?;
    for issue in &built.build_issues {
        eprintln!(
            "{}",
            styles.dim.apply_to(format!(
                "[llm] provider '{}' is unavailable: {}",
                issue.provider, issue.cause
            ))
        );
    }
    Ok(built.client)
}

/// Client options for the session: standard retries plus the requested
/// diagnostic middleware.
fn cli_client_options(args: &AgentArgs, styles: &'static Styles) -> ClientOptions {
    let options = ClientOptions::standard();
    if args.verbose {
        options.with_middleware(Arc::new(VerboseMiddleware { styles }))
    } else if args.debug {
        options.with_middleware(Arc::new(DebugMiddleware { styles }))
    } else {
        options
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "fabro exec passes search process-env credentials into the agent's search tool."
)]
fn cli_search_secrets() -> SearchSecrets {
    SearchSecrets {
        brave_search_api_key: std::env::var(EnvVars::BRAVE_SEARCH_API_KEY).ok(),
        venice_api_key:       std::env::var(EnvVars::VENICE_API_KEY).ok(),
    }
}

/// The provider the session runs on: the `--provider` flag, else the
/// highest-priority available provider offering `--model`, else the default.
fn resolve_provider_id(
    catalog: &Catalog,
    args: &AgentArgs,
    available: &std::collections::HashSet<ProviderId>,
) -> ProviderId {
    let requested = ProviderId::new(args.provider.as_deref().unwrap_or("anthropic"));
    if args.provider.is_some() {
        return canonical_provider_id(catalog, &requested);
    }
    if let Some(model_id) = args.model.as_deref() {
        let matches = catalog.offerings_matching(model_id);
        if let Some(entry) = matches
            .iter()
            .find(|entry| available.contains(entry.provider.id()))
            .or_else(|| matches.first())
        {
            return entry.provider.id().clone();
        }
    }
    canonical_provider_id(catalog, &requested)
}

/// The catalog id for `requested`, resolving aliases; the request itself when
/// the catalog does not know it, so the error names what the caller typed.
fn canonical_provider_id(catalog: &Catalog, requested: &ProviderId) -> ProviderId {
    catalog
        .enabled_provider(requested.as_str())
        .map_or_else(|| requested.clone(), |provider| provider.id().clone())
}

/// The model that summarizes fetched web pages: the provider's small default,
/// else its default model, else the session's own model.
fn summarizer_model(catalog: &Catalog, provider_id: &ProviderId, selected_model: &str) -> String {
    let model = catalog
        .small_default_for([provider_id])
        .filter(|entry| entry.provider.id() == provider_id)
        .or_else(|| {
            catalog
                .enabled_provider(provider_id.as_str())?
                .default_offering()
        })
        .map_or_else(
            || selected_model.to_string(),
            |entry| entry.model.id().to_string(),
        );
    format!("{provider_id}/{model}")
}

/// Interactive approval for tools the permission level does not allow
/// outright. Without a terminal, or with `--auto-approve`, such tools are
/// refused.
struct CliApproval {
    level:          Mutex<PermissionLevel>,
    is_interactive: bool,
    styles:         &'static Styles,
}

#[async_trait]
impl ToolApprovalService for CliApproval {
    async fn approve(
        &self,
        request: &ToolCallRequest,
    ) -> Result<ApprovalDecision, ToolSystemError> {
        let tool_name = request.call().name.clone();
        let current_level = *self
            .level
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if current_level.auto_approves_tool(&tool_name) {
            return Ok(ApprovalDecision::Allow);
        }
        if !self.is_interactive {
            return Ok(ApprovalDecision::Deny {
                reason: format!("{tool_name} tool denied at current permission level"),
            });
        }
        let styles = self.styles;
        let answer = spawn_blocking(move || prompt_for_approval(&tool_name, styles))
            .await
            .map_err(|error| ToolSystemError::new(format!("approval prompt failed: {error}")))?;
        match answer {
            Ok(ApprovalAnswer::Allow) => Ok(ApprovalDecision::Allow),
            Ok(ApprovalAnswer::AllowAlways) => {
                *self
                    .level
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = PermissionLevel::Full;
                Ok(ApprovalDecision::Allow)
            }
            Ok(ApprovalAnswer::Deny { tool_name }) => Ok(ApprovalDecision::Deny {
                reason: format!("{tool_name} tool denied by user"),
            }),
            Err(reason) => Ok(ApprovalDecision::Deny { reason }),
        }
    }
}

enum ApprovalAnswer {
    Allow,
    AllowAlways,
    Deny { tool_name: String },
}

#[allow(
    clippy::print_stderr,
    reason = "Interactive approval prompts belong on stderr, not assistant output."
)]
#[expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "Interactive tool approval blocks on stdin and stderr by design, on a blocking task."
)]
fn prompt_for_approval(tool_name: &str, styles: &Styles) -> Result<ApprovalAnswer, String> {
    use std::io::Write as _;

    eprint!(
        "Allow {}? [y]es / [n]o / [a]lways: ",
        styles.bold.apply_to(tool_name),
    );
    std::io::stderr().flush().ok();
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(|e| format!("Failed to read input: {e}"))?;
    Ok(match input.trim().to_lowercase().as_str() {
        "y" | "yes" => ApprovalAnswer::Allow,
        "a" | "always" => ApprovalAnswer::AllowAlways,
        _ => ApprovalAnswer::Deny {
            tool_name: tool_name.to_string(),
        },
    })
}

fn format_tool_args(args: &serde_json::Value, cwd: &str) -> String {
    let cwd_prefix = if cwd.ends_with('/') {
        cwd.to_string()
    } else {
        format!("{cwd}/")
    };
    let Some(obj) = args.as_object() else {
        return args.to_string();
    };
    obj.iter()
        .map(|(k, v)| match v {
            serde_json::Value::String(s) => {
                let s = s.strip_prefix(&cwd_prefix).unwrap_or(s);
                let display = if s.len() > 80 {
                    format!("{}...", &s[..s.floor_char_boundary(77)])
                } else {
                    s.to_string()
                };
                format!("{k}={display:?}")
            }
            other => format!("{k}={other}"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[allow(
    clippy::print_stdout,
    reason = "Assistant responses are the CLI's primary stdout output."
)]
fn print_output(agent: &CodingAgent, styles: &Styles) {
    for turn in agent.history().turns() {
        if let Message::Assistant { content, .. } = turn {
            if !content.is_empty() {
                println!("{}", styles.render_markdown(content));
            }
        }
    }
}

#[allow(
    clippy::print_stderr,
    reason = "Session summaries are diagnostic metadata, not assistant output."
)]
fn print_summary(agent: &CodingAgent, styles: &Styles) {
    let (mut turn_count, mut tool_call_count, mut total_tokens) = (0usize, 0usize, 0u64);
    for turn in agent.history().turns() {
        if let Message::Assistant {
            tool_calls, usage, ..
        } = turn
        {
            turn_count += 1;
            tool_call_count += tool_calls.len();
            total_tokens = total_tokens.saturating_add(usage.input.saturating_add(usage.output));
        }
    }
    let token_str = if total_tokens >= 1_000_000 {
        format!("{:.1}m", total_tokens as f64 / 1_000_000.0)
    } else if total_tokens >= 1000 {
        format!("{}k", total_tokens / 1000)
    } else {
        total_tokens.to_string()
    };
    eprintln!(
        "{}",
        styles.dim.apply_to(format!(
            "Done ({turn_count} turns, {tool_call_count} tools, {token_str} toks)"
        )),
    );
}

/// Middleware that logs LLM request/response summaries to stderr.
struct DebugMiddleware {
    styles: &'static Styles,
}

#[async_trait]
impl Middleware for DebugMiddleware {
    #[allow(
        clippy::print_stderr,
        reason = "Debug middleware logs request and response summaries to stderr."
    )]
    async fn handle(&self, call: Call, next: Next) -> Result<Output, LlmError> {
        let s = self.styles;
        eprintln!(
            "{}",
            s.dim.apply_to(format!(
                "[debug] request: model={} messages={} tools={}",
                call.route().handle(),
                call.request().messages().len(),
                call.request().tools().len(),
            )),
        );
        let output = next.run(call).await?;
        if let Output::Complete(response) = &output {
            eprintln!(
                "{}",
                s.dim.apply_to(format!(
                    "[debug] response: model={} finish={:?} usage=({}/{}/{})",
                    response.model,
                    response.finish_reason,
                    response.usage.input,
                    response.usage.output,
                    response.usage.total(),
                )),
            );
        }
        Ok(output)
    }
}

/// Middleware that logs full LLM request/response JSON to stderr.
struct VerboseMiddleware {
    styles: &'static Styles,
}

#[async_trait]
impl Middleware for VerboseMiddleware {
    #[allow(
        clippy::print_stderr,
        reason = "Verbose middleware dumps full request and response JSON to stderr."
    )]
    async fn handle(&self, call: Call, next: Next) -> Result<Output, LlmError> {
        let s = self.styles;
        eprintln!(
            "{}\n{}",
            s.dim.apply_to("[verbose] request:"),
            serde_json::to_string_pretty(call.request())
                .unwrap_or_else(|e| format!("<serialize error: {e}>"))
        );
        let output = next.run(call).await?;
        if let Output::Complete(response) = &output {
            eprintln!(
                "{}\n{}",
                s.dim.apply_to("[verbose] response:"),
                serde_json::to_string_pretty(response)
                    .unwrap_or_else(|e| format!("<serialize error: {e}>"))
            );
        }
        Ok(output)
    }
}

#[allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "Assistant output stays on stdout while prompts and diagnostics use stderr."
)]
async fn run_session(
    args: AgentArgs,
    client: Client,
    mcp_servers: Vec<McpServerSettings>,
    catalog: Arc<Catalog>,
    styles: &'static Styles,
) -> AnyResult<()> {
    let available: std::collections::HashSet<ProviderId> =
        client.available_providers().iter().cloned().collect();
    let provider_id = resolve_provider_id(&catalog, &args, &available);
    if !available.contains(&provider_id) {
        anyhow::bail!("LLM credentials not configured for provider '{provider_id}'");
    }
    let model = if let Some(model) = args.model.clone() {
        model
    } else {
        catalog
            .enabled_provider(provider_id.as_str())
            .and_then(CatalogProvider::default_offering)
            .map(|entry| entry.model.id().to_string())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "provider '{provider_id}' has no default model in the catalog; pass --model explicitly"
                )
            })?
    };
    eprintln!("{}", styles.dim.apply_to(format!("Using model: {model}")));

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let cwd_str = cwd.to_string_lossy().to_string();
    let sandbox: Arc<RunSandbox> = Arc::new(
        local_sandbox(cwd)
            .await
            .context("failed to create the local sandbox")?,
    );

    let permissions = args.permission_level();
    #[expect(
        clippy::disallowed_methods,
        reason = "is_terminal() on stdin is a non-blocking fstat; no actual I/O performed"
    )]
    let is_interactive = std::io::stdin().is_terminal() && !args.auto_approve;
    let approval = Arc::new(CliApproval {
        level: Mutex::new(permissions),
        is_interactive,
        styles,
    });
    let permission_middleware =
        PermissionMiddleware::new(Arc::new(PermissionLevelPolicy::new(permissions)))
            .with_approval(approval);

    // The profile's own instruction files from the repository root down, and
    // fabro's skill directories: pebble knows the files and does the walk.
    let mut options = CodingAgentOptions::default()
        .with_memory_discovery(MemoryDiscovery::from_git_root())
        .with_recorded_permission_level(permissions);
    options = match &args.skills_dir {
        Some(skills_dir) => options.with_skill_dirs([skills_dir.clone()]),
        None => options.with_skill_discovery(
            SkillDiscovery::new()
                .search(Home::from_env().skills_dir().to_string_lossy().into_owned())
                .search_under_git_root(".fabro/skills")
                .search_under_git_root("skills"),
        ),
    };

    let environment: Arc<dyn Environment> = Arc::clone(&sandbox) as Arc<dyn Environment>;
    let mut builder = CodingAgent::builder(client, environment)
        .model(format!("{provider_id}/{model}"))
        .options(options)
        .tool_middleware(Arc::new(permission_middleware))
        .redactor(Arc::new(SecretRedactor))
        .web_fetch_summarizer(summarizer_model(&catalog, &provider_id, &model))
        .mcp_servers(pebble_servers(&mcp_servers))
        .subagents(SubagentOptions::enabled());
    if let Some(routes) = sandbox.port_routes() {
        builder = builder.port_routes(routes);
    }
    if let Some(search) = web_search::search_provider(&cli_search_secrets()) {
        builder = builder.search_provider(search);
    }
    let mut agent = builder
        .build()
        .await
        .context("failed to start the agent session")?;
    if matches!(
        args.output_format.unwrap_or(ExecOutputFormat::Text),
        ExecOutputFormat::Text
    ) {
        print_mcp_servers(&agent, styles);
    }

    // SIGINT ends the prompt; the session shuts down as cancelled.
    let cancel_token = CancellationToken::new();
    let sigint_token = cancel_token.clone();
    tokio::spawn(async move {
        signal::ctrl_c().await.ok();
        sigint_token.cancel();
    });

    let verbose = args.verbose;
    let output_format = args.output_format.unwrap_or(ExecOutputFormat::Text);
    let mut rx = agent.subscribe();
    let printer = tokio::spawn(async move {
        match output_format {
            ExecOutputFormat::Json => {
                let mut stdout = stdout();
                while let Ok(event) = rx.recv().await {
                    if let Ok(json) = serde_json::to_string(&event) {
                        let _ = stdout.write_all(json.as_bytes()).await;
                        let _ = stdout.write_all(b"\n").await;
                        let _ = stdout.flush().await;
                    }
                }
            }
            ExecOutputFormat::Text => {
                while let Ok(event) = rx.recv().await {
                    print_progress(&event, verbose, &cwd_str, styles);
                }
            }
        }
    });

    let report = agent
        .prompt_with_cancellation(args.prompt.as_str(), &cancel_token)
        .await;
    let shutdown_reason = match &report.result {
        Ok(_) => ShutdownReason::Completed,
        Err(_) if cancel_token.is_cancelled() => ShutdownReason::Cancelled,
        Err(_) => ShutdownReason::Error,
    };
    if let Err(error) = agent.shutdown(shutdown_reason).await {
        tracing::debug!(error = %error, "agent session did not shut down cleanly");
    }
    // The stream ends with the shutdown, so the printer drains everything.
    let _ = printer.await;

    if matches!(output_format, ExecOutputFormat::Text) {
        print_output(&agent, styles);
        print_summary(&agent, styles);
    }

    report
        .result
        .map(|_| ())
        .map_err(|error| anyhow::Error::new(SessionError::from(error)))
}

/// Report what became of each configured MCP server on stderr: pebble
/// started them while the agent was built, so the outcomes are read from the
/// agent rather than from a stream that had no subscriber yet.
#[allow(
    clippy::print_stderr,
    reason = "MCP connection outcomes are diagnostics for the person running the CLI."
)]
fn print_mcp_servers(agent: &CodingAgent, styles: &Styles) {
    for status in agent.snapshot().mcp_servers() {
        match &status.error {
            None => eprintln!(
                "{}",
                styles.dim.apply_to(format!(
                    "[mcp] {}: {} tools",
                    status.server,
                    status.tools.len()
                ))
            ),
            Some(error) => eprintln!(
                "{}",
                styles
                    .red
                    .apply_to(format!("[mcp] {} failed: {error}", status.server))
            ),
        }
    }
}

#[allow(
    clippy::print_stderr,
    reason = "Progress lines are diagnostics on stderr; assistant output stays on stdout."
)]
fn print_progress(event: &CodingAgentEvent, verbose: bool, cwd: &str, s: &Styles) {
    let child_prefix = if event.parent_session_id.is_some() {
        format!("[child {}] ", event.session_id)
    } else {
        String::new()
    };
    match &event.event {
        CodingEvent::ToolCallStarted {
            tool_name,
            arguments,
            ..
        } => {
            eprintln!(
                "  {} {}{}",
                s.dim.apply_to("\u{25cf}"),
                s.bold_cyan.apply_to(format!("{child_prefix}{tool_name}")),
                s.dim
                    .apply_to(format!("({})", format_tool_args(arguments, cwd))),
            );
        }
        CodingEvent::ToolCallCompleted {
            tool_name,
            output,
            is_error,
            ..
        } if verbose => {
            let label = if *is_error {
                "tool error"
            } else {
                "tool result"
            };
            eprintln!(
                "  {}\n{}",
                s.dim
                    .apply_to(format!("[{label}] {child_prefix}{tool_name}:")),
                serde_json::to_string_pretty(output).unwrap_or_else(|_| output.to_string()),
            );
        }
        CodingEvent::Error { error } => {
            eprintln!(
                "  {}",
                s.red
                    .apply_to(format!("\u{2717} {child_prefix}{}", error.message)),
            );
        }
        CodingEvent::SubAgentSpawned {
            agent_id,
            depth,
            task,
            generation,
        }
        | CodingEvent::SubAgentTurnStarted {
            agent_id,
            depth,
            task,
            generation,
        } => {
            let started = if matches!(event.event, CodingEvent::SubAgentSpawned { .. }) {
                "spawned"
            } else {
                "turn started"
            };
            let task_preview = if task.len() > 60 {
                &task[..task.floor_char_boundary(60)]
            } else {
                task
            };
            eprintln!(
                "  {}",
                s.dim.apply_to(format!(
                    "{child_prefix}\u{25b6} subagent {agent_id} {started} (depth={depth}, generation={generation}) task={task_preview:?}"
                )),
            );
        }
        CodingEvent::SubAgentCompleted {
            agent_id,
            depth,
            generation,
            success,
            turns_used,
        } => {
            eprintln!(
                "  {}",
                s.dim.apply_to(format!(
                    "{child_prefix}\u{25a0} subagent {agent_id} completed (depth={depth}, generation={generation}, success={success}, turns={turns_used})"
                )),
            );
        }
        CodingEvent::SubAgentFailed {
            agent_id,
            depth,
            generation,
            error,
        } => {
            eprintln!(
                "  {}",
                s.red.apply_to(format!(
                    "{child_prefix}\u{2717} subagent {agent_id} failed (depth={depth}, generation={generation}): {}",
                    error.message
                )),
            );
        }
        CodingEvent::SubAgentClosed {
            agent_id,
            depth,
            generation,
        } => {
            eprintln!(
                "  {}",
                s.dim.apply_to(format!(
                    "{child_prefix}\u{25a0} subagent {agent_id} closed (depth={depth}, generation={generation})"
                )),
            );
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use fabro_llm::test_support::{test_catalog, test_catalog_with_overlay};
    use fabro_types::settings::run::{McpServerRef, McpServerSettings, ResolvedMcpEntry};
    use lithos_llm::catalog::builtin;

    use super::{
        AgentArgs, format_tool_args, resolve_provider_id, run_mcp_servers_for_exec,
        summarizer_model,
    };
    use crate::args::{ExecOutputFormat, PermissionsArg};

    fn args(provider: Option<&str>, model: Option<&str>) -> AgentArgs {
        AgentArgs {
            prompt:        "task".to_string(),
            provider:      provider.map(str::to_string),
            model:         model.map(str::to_string),
            permissions:   Some(PermissionsArg::Full),
            auto_approve:  true,
            debug:         false,
            verbose:       false,
            skills_dir:    None,
            output_format: Some(ExecOutputFormat::Text),
        }
    }

    #[test]
    fn run_mcp_servers_for_exec_rejects_catalog_references() {
        let err = run_mcp_servers_for_exec(&HashMap::from([(
            "sentry".to_string(),
            ResolvedMcpEntry::Reference(McpServerRef {
                id:      "catalog/sentry".to_string(),
                enabled: None,
            }),
        )]))
        .expect_err("fabro exec should reject unresolved run-level MCP references");

        assert!(
            err.to_string()
                .contains("fabro exec cannot resolve run.agent.mcps.sentry catalog reference"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn run_mcp_servers_for_exec_keeps_resolved_servers() {
        let servers = run_mcp_servers_for_exec(&HashMap::from([(
            "inline".to_string(),
            ResolvedMcpEntry::Resolved(McpServerSettings {
                name: "inline".to_string(),
                ..McpServerSettings::default()
            }),
        )]))
        .expect("resolved inline server should be usable by fabro exec");

        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "inline");
    }

    #[test]
    fn explicit_provider_wins_over_model_matching() {
        let catalog = test_catalog_with_overlay("[providers.openrouter]\nenabled = true\n");
        let available = [builtin::openai()].into_iter().collect();

        let provider = resolve_provider_id(&catalog, &args(Some("openrouter"), None), &available);

        assert_eq!(provider.as_str(), "openrouter");
    }

    #[test]
    fn a_bare_model_picks_an_available_provider_offering_it() {
        let catalog = test_catalog();
        let available = [builtin::openai()].into_iter().collect();

        let provider = resolve_provider_id(&catalog, &args(None, Some("gpt-5.4")), &available);

        assert_eq!(provider, builtin::openai());
    }

    #[test]
    fn summarizer_uses_the_providers_small_default() {
        let catalog = test_catalog();

        let selector = summarizer_model(&catalog, &builtin::anthropic(), "claude-opus-4-6");

        assert!(selector.starts_with("anthropic/"), "{selector}");
        assert_ne!(selector, "anthropic/claude-opus-4-6");
    }

    #[test]
    fn tool_args_strip_the_working_directory_prefix() {
        let rendered = format_tool_args(
            &serde_json::json!({"file_path": "/work/src/main.rs", "limit": 20}),
            "/work",
        );

        assert!(rendered.contains("file_path=\"src/main.rs\""), "{rendered}");
        assert!(rendered.contains("limit=20"), "{rendered}");
    }
}
