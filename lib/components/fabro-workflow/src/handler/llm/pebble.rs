//! The API backend for LLM stages: pebble's `CodingAgent` bound to the
//! workflow's sandbox, events, steering, hooks, and human input.
//!
//! One agent serves one stage invocation. At `full` fidelity, stages sharing a
//! `thread_id` continue one conversation: the agent is exported when a stage
//! ends and resumed by the next, which binds its own event scope, hooks, and
//! interviewer. Model failover is pebble's: the stage hands it the resolved
//! fallback routes, pebble keeps the conversation as it stands and asks the
//! next route to continue it, and reports each move as its own
//! `agent.route.failover` event, stored like every other.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use fabro_graphviz::graph::Node;
use fabro_llm::credentials::CredentialProvider;
use fabro_llm::lithos_catalog::Catalog;
use fabro_llm::types::ResponseFormat;
use fabro_llm::{Client, ClientOptions, Request, Response};
use fabro_mcp::config::McpServerSettings;
use fabro_mcp::pebble::pebble_servers;
use fabro_sandbox::{FsScope, RunSandbox, SecretRedactor, WalkOptions};
use fabro_types::settings::run::RunModelControls;
use fabro_types::{
    AgentProfileKind, BilledModelUsage, ModelRef, PermissionLevel, SessionCapability, StageId,
    StageTiming, UsdMicros, billing,
};
use fabro_util::home::Home;
use lithos_llm::catalog::{ModelId, ProviderId};
use lithos_llm::types::{Message as LlmMessage, Role, TokenCounts};
use pebble_agent::ToolMiddleware;
use pebble_coding_agent::environment::Environment;
use pebble_coding_agent::events::{CodingAgentEvent, EventSink, EventSinkError};
use pebble_coding_agent::extensions::HumanInputProvider;
use pebble_coding_agent::projection::{DescendantAccount, SessionProjection};
use pebble_coding_agent::state::Message;
use pebble_coding_agent::steering::SteerableSession;
use pebble_coding_agent::subagents::SubagentOptions;
use pebble_coding_agent::tools::{RegisteredTool, ToolEnvProvider};
use pebble_coding_agent::{
    CodingAgent, CodingAgentBuilder, CodingAgentControlHandle, CodingAgentExport,
    CodingAgentOptions, CodingInput, InterruptReason, MemoryDiscovery, ShutdownReason,
    SkillDiscovery,
};
use tokio_util::sync::CancellationToken;

use super::super::agent::{
    CodergenBackend, CodergenResult, CodergenRunRequest, OneShotRequest, continuation_message,
    validate_agent_output_sources,
};
use super::super::structured_output;
use super::activation_lease::{ActivationLease, ActivationLeaseOptions};
use super::context_read::{self, ContextReadState};
use super::controls::{
    EffectiveRequestControls, effective_request_controls, node_max_output_tokens,
};
use super::fabro_tools::{register_fabro_run_tools, register_named_fabro_run_tools};
use super::fallback::{self, FallbackPlan, LlmRoute};
use super::routing::{self, ProviderContext};
use super::stage_policy::{node_admits_tool, node_fs_scope};
use crate::context::WorkflowContext;
use crate::context::keys::Fidelity;
use crate::error::Error;
use crate::event::{Emitter, Event, StageScope};
use crate::model_fallback::{ModelFallbackNotice, ModelFallbackPolicy};
use crate::outcome::{Outcome, billed_model_usage_from_llm};
use crate::services::FabroRunToolServices;
use crate::steering_hub::SteeringHub;
use crate::web_search::{self, SearchSecrets};

/// The share of the model's context window at which an agent stage compacts
/// its conversation. Fabro's own agent loop used this value; pebble's default
/// is the same, and it is set here so the stage's policy is fabro's to state.
pub const COMPACTION_THRESHOLD_PERCENT: usize = 80;

/// How many recent turns compaction leaves verbatim, as fabro's agent loop
/// did.
pub const COMPACTION_PRESERVE_TURNS: usize = 6;

/// The API backend: pebble coding agents over the workflow's LLM client.
pub struct PebbleBackend {
    model:                String,
    provider_id:          ProviderId,
    fallbacks:            ModelFallbackPolicy,
    /// Exported conversations keyed by thread, waiting for the next stage.
    threads:              Mutex<HashMap<String, CachedThread>>,
    /// Messages of fallback-plan notices already emitted for this run, so the
    /// same configuration warning is not repeated on every LLM call.
    emitted_plan_notices: Mutex<HashSet<String>>,
    tool_env:             Option<Arc<dyn ToolEnvProvider>>,
    mcp_servers:          Vec<McpServerSettings>,
    search_secrets:       SearchSecrets,
    skill_dirs:           Option<Vec<String>>,
    run_model_controls:   RunModelControls,
    source:               Arc<dyn CredentialProvider>,
    steering_hub:         Arc<SteeringHub>,
    catalog:              Arc<Catalog>,
    fabro_run_tools:      Option<FabroRunToolServices>,
}

/// A conversation between stages: what the next stage resumes from. The
/// successor starts the stage's MCP servers again; the same settings give
/// the same tool names, so the conversation's earlier calls stay valid.
struct CachedThread {
    export:        CodingAgentExport,
    fallback_plan: FallbackPlan,
}

/// How the backend reports a failed prompt.
///
/// A model error reaches this after pebble has followed every fallback route
/// the stage gave it, so it is terminal here whatever its kind.
enum AgentErrorDisposition {
    /// The run's token cancelled the prompt; surface as `Error::Cancelled`.
    Cancelled,
    /// Terminal error; abort the invocation with this workflow `Error`.
    Terminal(Error),
}

fn classify_agent_error(error: pebble_coding_agent::Error) -> AgentErrorDisposition {
    if let Some(llm) = error.llm_source() {
        return AgentErrorDisposition::Terminal(Error::from(llm.data()));
    }
    match error {
        pebble_coding_agent::Error::Interrupted(InterruptReason::Cancelled) => {
            AgentErrorDisposition::Cancelled
        }
        pebble_coding_agent::Error::Interrupted(InterruptReason::WallClockTimeout) => {
            AgentErrorDisposition::Terminal(Error::Precondition(
                "Agent session exceeded its wall-clock timeout".to_string(),
            ))
        }
        pebble_coding_agent::Error::Interrupted(InterruptReason::TurnLimit) => {
            AgentErrorDisposition::Terminal(Error::Precondition(
                "Agent session used every model turn it was allowed".to_string(),
            ))
        }
        // Stages set no round budget; the arm names the outcome should one
        // ever be configured.
        pebble_coding_agent::Error::ToolRoundsExhausted { limit } => {
            AgentErrorDisposition::Terminal(Error::Precondition(format!(
                "Agent session reached its limit of {limit} tool rounds"
            )))
        }
        pebble_coding_agent::Error::EventSink(sink) => AgentErrorDisposition::Terminal(Error::Io(
            format!("Failed to persist agent events: {sink:#}"),
        )),
        pebble_coding_agent::Error::FallbackRoute { route, source } => {
            AgentErrorDisposition::Terminal(Error::Precondition(format!(
                "Fallback route {route} could not be started: {source:#}"
            )))
        }
        // `InterruptReason` may grow; a reason this build does not know still
        // ended the prompt.
        pebble_coding_agent::Error::Interrupted(_) => AgentErrorDisposition::Terminal(
            Error::Precondition(format!("Agent session was interrupted: {error}")),
        ),
        other => AgentErrorDisposition::Terminal(Error::Precondition(format!(
            "Agent session failed: {other:#}"
        ))),
    }
}

// --- Event sink -----------------------------------------------------------

/// Pebble's durable event sink for one stage: every agent event becomes a
/// run event in the run's log before the agent goes on, so the stage's
/// `SessionProjection` rebuilt from the log sees what the live one saw.
/// Pebble's stream is the agent event contract; fabro emits an agent event
/// of its own only for a fact pebble cannot know.
struct WorkflowEventSink {
    emitter:    Arc<Emitter>,
    node_id:    String,
    scope:      StageScope,
    /// Pebble's fold of every event this sink recorded: the stage's one
    /// account of what its agent and subagents spent, wrote, and ran. The
    /// store folds the same events the same way, so the stage's billing at
    /// its end is the usage the run showed live.
    projection: Mutex<SessionProjection>,
}

impl WorkflowEventSink {
    /// The account as it stands.
    fn snapshot(&self) -> SessionProjection {
        self.projection
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl EventSink for WorkflowEventSink {
    async fn record(&self, event: &CodingAgentEvent) -> Result<(), EventSinkError> {
        // Every event, including streaming deltas, resets the run's activity
        // watchdog.
        self.emitter.touch();
        // Streaming deltas are not run history. `ProcessingEnd` is: pebble's
        // `SessionProjection` reads it to complete the prompt and mark the
        // session idle, so a projection rebuilt from the run's log needs it.
        if event.event.is_streaming_noise() {
            return Ok(());
        }
        self.projection
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .apply(event);
        self.emitter
            .emit_durable(
                &Event::Agent {
                    stage: self.node_id.clone(),
                    visit: self.scope.visit,
                    event: event.clone(),
                },
                Some(&self.scope),
            )
            .await
            .map_err(|error| {
                EventSinkError::new(format!("failed to persist agent event: {error}"))
                    .with_source(error)
            })
    }
}

// --- Live invocation ------------------------------------------------------

/// One stage invocation's live agent, its timing, and the sink that
/// accounts for it.
///
/// A stage may run several prompts on one agent (the prompt, output repairs,
/// late steering). What every one of them spent and wrote, subagents
/// included and across whatever routes pebble moved through, is the sink's
/// fold of the events it recorded; the prompt reports here contribute their
/// timing and the route the prompt ended on.
struct LiveAgent {
    agent:              CodingAgent,
    handle:             CodingAgentControlHandle,
    lease:              Option<Arc<ActivationLease>>,
    sink:               Arc<WorkflowEventSink>,
    inference_duration: Duration,
    tool_duration:      Duration,
}

impl LiveAgent {
    fn new(
        agent: CodingAgent,
        handle: CodingAgentControlHandle,
        sink: Arc<WorkflowEventSink>,
    ) -> Self {
        Self {
            agent,
            handle,
            lease: None,
            sink,
            inference_duration: Duration::ZERO,
            tool_duration: Duration::ZERO,
        }
    }

    fn record_report(&mut self, report: &pebble_coding_agent::PromptReport) {
        self.inference_duration = self
            .inference_duration
            .saturating_add(report.timing.inference);
        self.tool_duration = self.tool_duration.saturating_add(report.timing.tool);
        for compaction in &report.compactions {
            // The summary call's usage is already in the stage's account; this
            // is the breakdown, for anyone asking why a stage cost what it did.
            tracing::debug!(
                reason = ?compaction.reason,
                original_turns = compaction.original_turn_count,
                preserved_turns = compaction.preserved_turn_count,
                usage = ?compaction.usage,
                cost_usd_micros = ?compaction.cost_usd_micros,
                "agent stage compacted its conversation"
            );
        }
    }

    /// What the stage's prompts have spent and written so far.
    fn account(&self) -> SessionProjection {
        self.sink.snapshot()
    }

    /// The path written or edited most recently, when any was.
    fn last_file_touched(&self) -> Option<String> {
        self.sink
            .projection
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .last_file_touched
            .clone()
    }

    fn release_lease(&mut self) {
        if let Some(lease) = self.lease.take() {
            lease.release();
        }
    }

    /// End the agent for a prompt that will not continue on it.
    async fn discard(&mut self, reason: ShutdownReason) {
        self.release_lease();
        if let Err(error) = self.agent.shutdown(reason).await {
            tracing::debug!(error = %error, "agent session did not shut down cleanly");
        }
    }

    /// The text of the agent's last answer, when the report carried none.
    fn last_assistant_text(&self) -> String {
        self.agent
            .history()
            .turns()
            .iter()
            .rev()
            .find_map(|turn| match turn {
                Message::Assistant { content, .. } if !content.is_empty() => Some(content.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }
}

/// The route as billing names it: provider, model, and the speed tier the
/// stage asked for.
fn route_model(route: &LlmRoute) -> ModelRef {
    ModelRef::new(
        route.target.provider.clone(),
        ModelId::new(route.target.model.as_str()),
    )
    .with_speed(route.controls.speed)
}

/// A stage's billing from its account: the whole tree under the root's
/// route, and the rows that split it by model.
struct StageBilling {
    total:    BilledModelUsage,
    by_model: Vec<BilledModelUsage>,
}

/// Bills the stage's account from the catalog: the root session at
/// `root_model`, its route, and each descendant at its own route where the
/// catalog knows it and at the root's otherwise, so a subagent on a cheaper
/// or dearer model is priced as what it ran. A descendant on the root's
/// route joins the root's row. Where pebble carried a provider-reported
/// cost, that cost stands in for the catalog's estimate.
fn stage_billing(
    catalog: &Catalog,
    root_model: &ModelRef,
    account: &SessionProjection,
) -> Result<StageBilling, Error> {
    let mut groups: Vec<(ModelRef, TokenCounts, Option<u64>)> = vec![(
        root_model.clone(),
        TokenCounts::from(account.usage),
        account.cost_usd_micros,
    )];
    for descendant in account.descendants.values() {
        let model = descendant_model(catalog, root_model, descendant);
        match groups.iter_mut().find(|(grouped, _, _)| *grouped == model) {
            Some((_, tokens, cost)) => {
                billing::add_usage(tokens, TokenCounts::from(descendant.usage));
                add_reported_cost(cost, descendant.cost_usd_micros);
            }
            None => groups.push((
                model,
                TokenCounts::from(descendant.usage),
                descendant.cost_usd_micros,
            )),
        }
    }
    // The root's row first, then the others by model.
    groups[1..].sort_by(|left, right| left.0.sort_key().cmp(&right.0.sort_key()));

    let mut by_model = Vec::with_capacity(groups.len());
    let mut total_tokens = TokenCounts::default();
    let mut total_cost = None;
    for (model, tokens, reported) in groups {
        let row = billed_model_usage_from_llm(catalog, &model, tokens)?
            .with_reported_cost(reported.map(usd_micros));
        billing::add_usage(&mut total_tokens, row.tokens);
        UsdMicros::accumulate(&mut total_cost, row.total_usd_micros.map(UsdMicros));
        by_model.push(row);
    }
    Ok(StageBilling {
        total: BilledModelUsage {
            model:            root_model.clone(),
            tokens:           total_tokens,
            total_usd_micros: total_cost.map(|cost| cost.0),
        },
        by_model,
    })
}

/// The route a descendant is billed at: its own where its start named one
/// the catalog knows, else the root's. A descendant whose start was not seen
/// names only its answers' model, taken to be on the root's provider.
fn descendant_model(
    catalog: &Catalog,
    root_model: &ModelRef,
    account: &DescendantAccount,
) -> ModelRef {
    let Some(model) = account.model.as_deref() else {
        return root_model.clone();
    };
    let provider = account
        .provider
        .as_deref()
        .unwrap_or(root_model.provider.as_str());
    if provider == root_model.provider.as_str() && model == root_model.model_id.as_str() {
        return root_model.clone();
    }
    if catalog.enabled_provider(provider).is_none() {
        return root_model.clone();
    }
    ModelRef::new(ProviderId::new(provider), ModelId::new(model))
}

/// Folds a reported cost into a total that stays `None` until one is seen.
fn add_reported_cost(total: &mut Option<u64>, cost: Option<u64>) {
    if let Some(cost) = cost {
        *total = Some(total.unwrap_or(0).saturating_add(cost));
    }
}

fn usd_micros(micros: u64) -> UsdMicros {
    UsdMicros(i64::try_from(micros).unwrap_or(i64::MAX))
}

/// Everything one stage binds to an agent it builds or resumes.
struct StageBindings<'a> {
    node_id:         &'a str,
    stage_scope:     &'a StageScope,
    emitter:         &'a Arc<Emitter>,
    sandbox:         &'a Arc<RunSandbox>,
    tool_middleware: Option<&'a Arc<dyn ToolMiddleware>>,
    human_input:     Option<&'a Arc<dyn HumanInputProvider>>,
    /// Fork (fabro-e804): the per-node `context_read` view the stage's
    /// agents serve.
    context_read:    &'a Arc<context_read::ContextReadState>,
}

impl PebbleBackend {
    #[must_use]
    pub fn new(
        model: String,
        provider_id: impl Into<ProviderId>,
        fallbacks: ModelFallbackPolicy,
        source: Arc<dyn CredentialProvider>,
        steering_hub: Arc<SteeringHub>,
    ) -> Self {
        let catalog = Arc::new(fabro_llm::default_catalog());
        Self::new_with_catalog(
            model,
            provider_id.into(),
            fallbacks,
            source,
            steering_hub,
            catalog,
        )
    }

    #[must_use]
    pub fn new_with_catalog(
        model: String,
        provider_id: ProviderId,
        fallbacks: ModelFallbackPolicy,
        source: Arc<dyn CredentialProvider>,
        steering_hub: Arc<SteeringHub>,
        catalog: Arc<Catalog>,
    ) -> Self {
        Self {
            model,
            provider_id,
            fallbacks,
            threads: Mutex::new(HashMap::new()),
            emitted_plan_notices: Mutex::new(HashSet::new()),
            tool_env: None,
            mcp_servers: Vec::new(),
            search_secrets: SearchSecrets::default(),
            skill_dirs: None,
            run_model_controls: RunModelControls::default(),
            source,
            steering_hub,
            catalog,
            fabro_run_tools: None,
        }
    }

    #[must_use]
    pub fn with_tool_env_provider(mut self, provider: Arc<dyn ToolEnvProvider>) -> Self {
        self.tool_env = Some(provider);
        self
    }

    #[must_use]
    pub fn with_mcp_servers(mut self, servers: Vec<McpServerSettings>) -> Self {
        self.mcp_servers = servers;
        self
    }

    #[must_use]
    pub fn with_search_secrets(mut self, secrets: SearchSecrets) -> Self {
        self.search_secrets = secrets;
        self
    }

    /// Directories searched for skills, replacing the defaults (the user's
    /// Fabro skills directory plus `.fabro/skills` and `skills` under the
    /// sandbox working directory).
    #[must_use]
    pub fn with_skill_dirs(mut self, dirs: Vec<String>) -> Self {
        self.skill_dirs = Some(dirs);
        self
    }

    #[must_use]
    pub fn with_run_model_controls(mut self, controls: RunModelControls) -> Self {
        self.run_model_controls = controls;
        self
    }

    #[must_use]
    pub fn with_fabro_run_tools(mut self, services: FabroRunToolServices) -> Self {
        self.fabro_run_tools = Some(services);
        self
    }

    fn resolve_effective_request_controls(
        &self,
        node: &Node,
    ) -> Result<EffectiveRequestControls, Error> {
        effective_request_controls(&self.run_model_controls, node)
    }

    fn resolve_provider_context(
        &self,
        model: &str,
        provider_attr: Option<&str>,
    ) -> Result<ProviderContext, Error> {
        routing::resolve_provider_context(
            self.catalog.as_ref(),
            &self.provider_id,
            model,
            provider_attr,
        )
    }

    fn fallback_plan(
        &self,
        model: &str,
        provider: &ProviderId,
        requested_controls: EffectiveRequestControls,
    ) -> (FallbackPlan, Vec<ModelFallbackNotice>) {
        fallback::fallback_plan(
            self.catalog.as_ref(),
            &self.fallbacks,
            model,
            provider,
            requested_controls,
        )
    }

    fn emit_fallback_plan_notices(
        &self,
        notices: &[ModelFallbackNotice],
        emitter: &Emitter,
        stage_scope: &StageScope,
    ) {
        let mut emitted = self
            .emitted_plan_notices
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for notice in notices {
            let message = notice.message();
            if emitted.insert(message.clone()) {
                emitter.notice_scoped(notice.level(), notice.code(), message, stage_scope);
            }
        }
    }

    async fn build_llm_client(&self) -> Result<Client, Error> {
        build_llm_client(&self.catalog, Arc::clone(&self.source)).await
    }

    /// Where a stage's skills come from: the directories the backend was
    /// given, else fabro's convention — the user's skills directory, then
    /// `.fabro/skills` and `skills` under the repository root — which pebble
    /// resolves and searches. Applies only to nodes that opt in with
    /// `skills = "discover"` (fabro-4dd8): stage input is harness-assembled
    /// and legitimately carries bare slash-paths, which a non-empty skill
    /// set turns into session-killing "Unknown skill" expansion errors.
    fn skill_options(&self, node: &Node, options: CodingAgentOptions) -> CodingAgentOptions {
        if !node.skills_discovery() {
            return options;
        }
        match &self.skill_dirs {
            Some(dirs) => options.with_skill_dirs(dirs.clone()),
            None => options.with_skill_discovery(
                SkillDiscovery::new()
                    .search(Home::from_env().skills_dir().to_string_lossy().into_owned())
                    .search_under_git_root(".fabro/skills")
                    .search_under_git_root("skills"),
            ),
        }
    }

    fn agent_options(&self, node: &Node, controls: EffectiveRequestControls) -> CodingAgentOptions {
        // The profile's own instruction files, from the repository root down
        // to the working directory: pebble knows the files and does the walk.
        let options = CodingAgentOptions::default()
            .with_reasoning_effort(controls.reasoning_effort)
            .with_speed(controls.speed)
            .with_max_tokens(node_max_output_tokens(node).map(i64::from))
            .with_memory_discovery(MemoryDiscovery::from_git_root());
        self.skill_options(node, options)
            .with_recorded_permission_level(PermissionLevel::Full)
            .with_context_compaction(true)
            .with_compaction_threshold_percent(COMPACTION_THRESHOLD_PERCENT)
            .with_compaction_preserve_turns(COMPACTION_PRESERVE_TURNS)
    }

    /// The application tools a stage agent gets beyond pebble's own and the
    /// MCP servers'.
    /// The application tools a stage agent gets beyond pebble's own and the
    /// MCP servers'.
    ///
    /// Fork additions: the node's `fabro_tools` attribute opts a stage into
    /// a NAMED subset of the run tools (unset = the full run-wide set), and
    /// every stage agent serves the `context_read` tool (fabro-e804) on the
    /// invocation's per-node view. Both are subagent-inheritable, matching
    /// the retired fabro-agent session factory.
    fn stage_tools(&self, node: &Node, bindings: &StageBindings<'_>) -> Vec<RegisteredTool> {
        let mut tools: Vec<RegisteredTool> = match &self.fabro_run_tools {
            Some(services) => {
                // files_from registration reads through this stage's
                // sandbox with its fs_hide policy applied.
                let mut services = services.clone();
                services.files = Some(Arc::new(SandboxWorkflowFiles {
                    sandbox:  Arc::clone(bindings.sandbox),
                    fs_scope: node_fs_scope(node).ok().flatten(),
                }));
                let named: Vec<String> = node
                    .fabro_tools()
                    .iter()
                    .map(|tool| (*tool).to_owned())
                    .collect();
                if named.is_empty() {
                    register_fabro_run_tools(&services)
                } else {
                    let names: Vec<&str> = named.iter().map(String::as_str).collect();
                    register_named_fabro_run_tools(&services, &names)
                }
            }
            None => Vec::new(),
        }
        .into_iter()
        .map(RegisteredTool::allow_in_subagents)
        .collect();
        tools.push(context_read::context_read_tool(Arc::clone(
            bindings.context_read,
        )));
        tools
    }

    /// Bind the stage's services, the plan's current route, and the routes
    /// left to fail over to, to `builder`.
    fn bind_builder(
        &self,
        mut builder: CodingAgentBuilder,
        node: &Node,
        plan: &FallbackPlan,
        provider: &ProviderContext,
        bindings: &StageBindings<'_>,
    ) -> (CodingAgentBuilder, Arc<WorkflowEventSink>) {
        let route = plan.current();
        let max_tokens = node_max_output_tokens(node).map(i64::from);
        let sink = Arc::new(WorkflowEventSink {
            emitter:    Arc::clone(bindings.emitter),
            node_id:    bindings.node_id.to_string(),
            scope:      bindings.stage_scope.clone(),
            projection: Mutex::new(SessionProjection::new()),
        });
        let event_sink = Arc::clone(&sink) as Arc<dyn EventSink>;
        builder = builder
            .tools(self.stage_tools(node, bindings))
            .mcp_servers(pebble_servers(&self.mcp_servers))
            .permission_level(PermissionLevel::Full)
            .options(self.agent_options(node, route.controls))
            .fallback_routes(plan.pebble_routes(max_tokens))
            .event_sink(event_sink)
            .redactor(Arc::new(SecretRedactor))
            .subagents(SubagentOptions::enabled());
        if let Some(routes) = bindings.sandbox.port_routes() {
            builder = builder.port_routes(routes);
        }
        if let Some(provider) = &self.tool_env {
            builder = builder.tool_env_provider(Arc::clone(provider));
        }
        if let Some(middleware) = bindings.tool_middleware {
            builder = builder.tool_middleware(Arc::clone(middleware));
        }
        if let Some(human_input) = bindings.human_input {
            builder = builder.human_input(Arc::clone(human_input));
        }
        if let Some(search) = web_search::search_provider(&self.search_secrets) {
            builder = builder.search_provider(search);
        }
        if provider.profile_kind == AgentProfileKind::Claude5 {
            builder = builder.web_fetch_summarizer(route.selector());
        }
        (builder, sink)
    }

    /// A new agent on the plan's current route.
    async fn build_agent(
        &self,
        node: &Node,
        plan: &FallbackPlan,
        provider: &ProviderContext,
        bindings: &StageBindings<'_>,
    ) -> Result<(CodingAgent, Arc<WorkflowEventSink>), Error> {
        let client = self.build_llm_client().await?;
        let environment: Arc<dyn Environment> =
            Arc::clone(bindings.sandbox) as Arc<dyn Environment>;
        let builder = CodingAgent::builder(client, environment).model(plan.current().selector());
        let (builder, sink) = self.bind_builder(builder, node, plan, provider, bindings);
        let agent = builder
            .build()
            .await
            .map_err(|error| Error::handler_with_source("Failed to start agent session", error))?;
        Ok((agent, sink))
    }

    /// The exported conversation of an earlier stage, continued on the
    /// route it was on, with the routes it had left.
    async fn resume_exported_agent(
        &self,
        export: CodingAgentExport,
        node: &Node,
        plan: &FallbackPlan,
        provider: &ProviderContext,
        bindings: &StageBindings<'_>,
    ) -> Result<(CodingAgent, Arc<WorkflowEventSink>), Error> {
        let client = self.build_llm_client().await?;
        let environment: Arc<dyn Environment> =
            Arc::clone(bindings.sandbox) as Arc<dyn Environment>;
        let builder = CodingAgent::resume_from_export(client, environment, export);
        let (builder, sink) = self.bind_builder(builder, node, plan, provider, bindings);
        let agent = builder
            .build()
            .await
            .map_err(|error| Error::handler_with_source("Failed to resume agent session", error))?;
        Ok((agent, sink))
    }

    /// Register `live` with the steering hub so steers reach it, and tell
    /// the run which tools it has.
    fn activate(
        &self,
        live: &mut LiveAgent,
        node: &Node,
        route: &LlmRoute,
        stage_id: &StageId,
        thread_id: Option<&str>,
        bindings: &StageBindings<'_>,
    ) -> Result<(), Error> {
        let session: Arc<dyn SteerableSession> = Arc::new(live.handle.clone());
        let lease = ActivationLease::activate(
            ActivationLeaseOptions {
                stage_id:         stage_id.clone(),
                session_id:       live.agent.id().to_string(),
                thread_id:        thread_id.map(str::to_string),
                provider:         Some(route.target.provider.to_string()),
                model:            Some(route.target.model.to_string()),
                reasoning_effort: route.controls.reasoning_effort,
                speed:            route.controls.speed,
                permission_level: Some(PermissionLevel::Full),
                capabilities:     vec![SessionCapability::Steer],
                hub:              Arc::clone(&self.steering_hub),
                emitter:          Arc::clone(bindings.emitter),
            },
            session,
        )?;
        live.lease = Some(lease);
        // The event must mirror what the stage policy admits on the wire,
        // not the full registry: pebble's `CodingAgent::tools` lists every
        // registered tool, while the middleware narrows each turn's catalog
        // (fabro-ba96/47b5).
        let tools = live
            .agent
            .snapshot()
            .tools()
            .iter()
            .filter(|tool| node_admits_tool(node, &tool.name))
            .cloned()
            .collect();
        bindings.emitter.emit(&Event::AgentToolsAvailable {
            node_id: bindings.node_id.to_string(),
            visit: stage_id.visit(),
            session_id: live.agent.id().to_string(),
            tools,
        });
        Ok(())
    }

    /// Run `input` on `live`. Pebble follows the stage's fallback routes
    /// itself; the plan here follows the route the prompt ended on, so a
    /// later prompt of this stage and a successor on the thread start there,
    /// and the run hears which route the session is on now.
    async fn prompt_live(
        &self,
        live: &mut LiveAgent,
        node: &Node,
        input: CodingInput,
        fallback_plan: &mut FallbackPlan,
        stage_id: &StageId,
        thread_id: Option<&str>,
        bindings: &StageBindings<'_>,
        cancel_token: &CancellationToken,
    ) -> Result<String, Error> {
        let report = live
            .agent
            .prompt_with_cancellation(input, cancel_token)
            .await;
        live.record_report(&report);
        if fallback_plan.advance_to(&report.route) {
            live.release_lease();
            self.activate(
                live,
                node,
                fallback_plan.current(),
                stage_id,
                thread_id,
                bindings,
            )?;
        }
        match report.result {
            Ok(output) => Ok(output.text.unwrap_or_else(|| live.last_assistant_text())),
            Err(error) => match classify_agent_error(error) {
                AgentErrorDisposition::Cancelled => Err(Error::Cancelled),
                AgentErrorDisposition::Terminal(error) => Err(error),
            },
        }
    }

    /// The failed outcome of an agent stage that spent before it failed: the
    /// failure itself, with the session tree's usage, the files it wrote, and
    /// its active time, so the run bills what the stage spent. A billing the
    /// catalog cannot price is logged and left off.
    fn failed_outcome(&self, error: &Error, live: &LiveAgent, plan: &FallbackPlan) -> Outcome {
        let mut outcome = error.to_fail_outcome();
        let account = live.account();
        match stage_billing(
            self.catalog.as_ref(),
            &route_model(plan.current()),
            &account,
        ) {
            Ok(billing) => {
                outcome.usage = Some(billing.total);
                outcome.usage_by_model = billing.by_model;
            }
            Err(billing_error) => {
                tracing::debug!(
                    error = %billing_error,
                    "failed agent stage could not be billed"
                );
            }
        }
        outcome.files_touched = account.files_touched;
        outcome.timing = Some(StageTiming::active_only(
            crate::millis_u64(live.inference_duration),
            crate::millis_u64(live.tool_duration),
        ));
        outcome
    }

    /// Steers that landed between the answer and the hub's close-the-door
    /// check run as further prompts, so the stage never ends with a steer
    /// nobody saw.
    async fn drain_late_steering(
        &self,
        live: &mut LiveAgent,
        node: &Node,
        fallback_plan: &mut FallbackPlan,
        stage_id: &StageId,
        thread_id: Option<&str>,
        bindings: &StageBindings<'_>,
        cancel_token: &CancellationToken,
        mut response: String,
    ) -> Result<String, Error> {
        loop {
            let released = live
                .lease
                .as_ref()
                .is_none_or(|lease| lease.release_if_idle());
            if released {
                live.lease.take();
                return Ok(response);
            }
            let (steering, follow_ups) = live.handle.take_pending_input().into_parts();
            for message in steering.into_iter().chain(follow_ups) {
                response = self
                    .prompt_live(
                        live,
                        node,
                        CodingInput::from(message.content().clone()),
                        fallback_plan,
                        stage_id,
                        thread_id,
                        bindings,
                        cancel_token,
                    )
                    .await?;
            }
        }
    }

    fn take_thread(&self, key: &str) -> Option<CachedThread> {
        self.threads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(key)
    }

    fn store_thread(&self, key: String, thread: CachedThread) {
        self.threads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key, thread);
    }

    // --- One-shot completions -------------------------------------------

    fn route_max_tokens(&self, node: &Node, route: &LlmRoute) -> Option<u32> {
        node_max_output_tokens(node).or_else(|| {
            self.catalog
                .enabled_provider(route.target.provider.as_str())
                .and_then(|provider| provider.offering(route.target.model.as_str()))
                .and_then(|entry| entry.model.limits())
                .map(|limits| u32::try_from(limits.max_output_tokens).unwrap_or(u32::MAX))
        })
    }

    /// Build a one-shot completion request addressed to `route`.
    fn route_request(
        &self,
        node: &Node,
        route: &LlmRoute,
        messages: Vec<LlmMessage>,
        response_format: Option<ResponseFormat>,
    ) -> Result<Request, Error> {
        let mut builder = Request::builder().model(route.selector());
        for message in messages {
            builder = builder.message(message);
        }
        if let Some(format) = response_format {
            builder = builder.response_format(format);
        }
        if let Some(max_tokens) = self.route_max_tokens(node, route) {
            builder = builder.max_output_tokens(max_tokens);
        }
        if let Some(effort) = route.controls.reasoning_effort {
            builder = builder.reasoning_effort(effort);
        }
        if let Some(speed) = route.controls.speed {
            builder = builder.speed(speed);
        }
        builder
            .build()
            .map_err(|err| Error::handler(format!("invalid LLM request: {err}")))
    }

    async fn complete_one_shot_request(
        &self,
        client: &Client,
        node: &Node,
        emitter: &Arc<Emitter>,
        stage_scope: &StageScope,
        mut request: Request,
        plan: &mut FallbackPlan,
    ) -> Result<OneShotCompletion, Error> {
        loop {
            match client.complete(request.clone()).await {
                Ok(response) => {
                    let route = plan.current();
                    return Ok(OneShotCompletion {
                        response,
                        model: ModelRef::new(
                            route.target.provider.clone(),
                            route.target.model.clone(),
                        )
                        .with_speed(route.controls.speed),
                    });
                }
                Err(error) if error.failover_eligible() && plan.has_next() => {
                    let error_message = error.to_string();
                    plan.advance();
                    fallback::emit_failover(node, emitter, stage_scope, plan, &error_message);
                    request = self.route_request(
                        node,
                        plan.current(),
                        request.messages().to_vec(),
                        request.response_format().cloned(),
                    )?;
                }
                Err(error) => return Err(Error::from(error)),
            }
        }
    }
}

struct OneShotCompletion {
    response: Response,
    model:    ModelRef,
}

/// Build the LLM client a stage session dispatches through.
async fn build_llm_client(
    catalog: &Arc<Catalog>,
    source: Arc<dyn CredentialProvider>,
) -> Result<Client, Error> {
    fabro_llm::build_client(Catalog::clone(catalog), source, ClientOptions::standard())
        .await
        .map(|built| built.client)
        .map_err(|e| Error::handler_with_source("Failed to create LLM client", e))
}

#[async_trait]
impl CodergenBackend for PebbleBackend {
    async fn shutdown(&self, _emitter: &Arc<Emitter>) {
        // Exported conversations were shut down when their stages ended, and
        // their MCP servers with them.
        self.threads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    fn effective_request_controls(&self, node: &Node) -> Result<EffectiveRequestControls, Error> {
        self.resolve_effective_request_controls(node)
    }

    async fn one_shot(&self, request: OneShotRequest<'_>) -> Result<CodergenResult, Error> {
        let node = request.node;
        let prompt = request.prompt;
        let system_prompt = request.system_prompt;
        let emitter = request.emitter;
        let stage_scope = request.stage_scope;

        let client = self.build_llm_client().await?;

        let model = node.model().unwrap_or(&self.model);
        let provider = self.resolve_provider_context(model, node.provider())?;
        let controls = self.resolve_effective_request_controls(node)?;
        let (mut fallback_plan, notices) =
            self.fallback_plan(model, &provider.provider_id, controls);
        self.emit_fallback_plan_notices(&notices, emitter, stage_scope);

        let mut messages = Vec::new();
        if let Some(sys) = system_prompt {
            messages.push(LlmMessage::text(Role::System, sys));
        }
        messages.push(LlmMessage::text(Role::User, prompt));

        let output_schema = structured_output::parse_node_output_schema(request.graph, node)?;
        let response_format = output_schema
            .as_ref()
            .map(structured_output::prompt_response_format);
        let mut repair_attempts = 0_i64;
        let mut previous_validation_error = None;
        let mut total_usage = TokenCounts::default();
        let mut total_cost = None;
        let mut inference_duration = Duration::ZERO;

        loop {
            let request = self.route_request(
                node,
                fallback_plan.current(),
                messages.clone(),
                response_format.clone(),
            )?;

            let inference_start = Instant::now();
            let completion_result = self
                .complete_one_shot_request(
                    &client,
                    node,
                    emitter,
                    stage_scope,
                    request,
                    &mut fallback_plan,
                )
                .await;
            inference_duration = inference_duration.saturating_add(inference_start.elapsed());
            let completion = completion_result?;
            billing::add_usage(&mut total_usage, completion.response.usage);
            UsdMicros::accumulate(
                &mut total_cost,
                completion.response.cost.as_ref().map(UsdMicros::from_cost),
            );
            let response_text = completion.response.text();

            let validation_error = if let Some(schema) = &output_schema {
                match structured_output::validate_response_text(schema, &response_text) {
                    Ok(_) => None,
                    Err(error) => Some((schema, error)),
                }
            } else {
                None
            };

            if let Some((schema, error)) = validation_error {
                if repair_attempts >= node.output_retries() {
                    if !error.is_truncated() {
                        return Err(Error::OutputSchemaValidation(
                            structured_output::exhausted_failure_reason(node.output_retries()),
                        ));
                    }
                    // fabro-274d: the output was cut off at the length cap
                    // even after length-aware repair — size overflow is not a
                    // correctness failure. Fall through with the best-effort
                    // text; handler-level validation persists it with an
                    // explicit truncation marker.
                } else {
                    let repair_message =
                        error.repair_message(schema, previous_validation_error.as_ref());
                    previous_validation_error = Some(error);
                    messages.push(LlmMessage::text(Role::Assistant, response_text));
                    messages.push(LlmMessage::text(Role::User, repair_message));
                    repair_attempts += 1;
                    continue;
                }
            }

            let stage_usage =
                billed_model_usage_from_llm(self.catalog.as_ref(), &completion.model, total_usage)?
                    .with_reported_cost(total_cost);

            return Ok(CodergenResult::Text {
                text:              response_text,
                usage_by_model:    Vec::new(),
                usage:             Some(stage_usage),
                files_touched:     Vec::new(),
                last_file_touched: None,
                timing:            StageTiming::active_only(
                    crate::millis_u64(inference_duration),
                    0,
                ),
            });
        }
    }

    async fn run(&self, request: CodergenRunRequest<'_>) -> Result<CodergenResult, Error> {
        let node = request.node;
        let emitter = request.emitter;
        let cancel_token = &request.cancel_token;
        let output_schema = structured_output::parse_node_output_schema(request.graph, node)?;

        let fidelity = request.context.fidelity();
        let reuse_key = if fidelity == Fidelity::Full {
            request.thread_id.map(String::from)
        } else {
            None
        };

        if cancel_token.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let stage_scope = StageScope::for_handler(request.context, &node.id);
        let stage_id = stage_scope.stage_id();
        // Fork (fabro-e804): the stage's context-pull view, served by the
        // `context_read` tool for the whole invocation.
        let context_read = Arc::new(ContextReadState::new(request.context_read.clone()));
        let bindings = StageBindings {
            node_id: &node.id,
            stage_scope: &stage_scope,
            emitter,
            sandbox: request.sandbox,
            tool_middleware: request.tool_middleware.as_ref(),
            human_input: request.human_input.as_ref(),
            context_read: &context_read,
        };

        let cached = reuse_key
            .as_ref()
            .and_then(|key| self.take_thread(key))
            .or_else(|| {
                // fabro-183f: a stage without a thread that failed
                // retryably parked its conversation under its stage id, so
                // the retry continues it instead of restarting.
                self.take_thread(&stage_id.to_string())
            });
        let is_reused = cached.is_some();
        let ((agent, sink), mut fallback_plan) = if let Some(thread) = cached {
            let route = thread.fallback_plan.current().clone();
            let provider = self.resolve_provider_context(
                route.target.model.as_str(),
                Some(route.target.provider.as_str()),
            )?;
            let session = self
                .resume_exported_agent(
                    thread.export,
                    node,
                    &thread.fallback_plan,
                    &provider,
                    &bindings,
                )
                .await?;
            (session, thread.fallback_plan)
        } else {
            let model = node.model().unwrap_or(&self.model);
            let provider = routing::resolve_node_provider_context(
                self.catalog.as_ref(),
                &self.provider_id,
                &self.model,
                node,
            )?;
            let controls = self.resolve_effective_request_controls(node)?;
            let (fallback_plan, notices) =
                self.fallback_plan(model, &provider.provider_id, controls);
            self.emit_fallback_plan_notices(&notices, emitter, &stage_scope);
            let route = fallback_plan.current().clone();
            let route_provider = self.resolve_provider_context(
                route.target.model.as_str(),
                Some(route.target.provider.as_str()),
            )?;
            let session = self
                .build_agent(node, &fallback_plan, &route_provider, &bindings)
                .await?;
            (session, fallback_plan)
        };
        if cancel_token.is_cancelled() {
            let mut agent = agent;
            let _ = agent.shutdown(ShutdownReason::Cancelled).await;
            return Err(Error::Cancelled);
        }

        tracing::info!(
            node = %node.id,
            fidelity = %fidelity,
            reused = is_reused,
            "Agent session ready"
        );

        let handle = agent.control_handle();
        let mut live = LiveAgent::new(agent, handle, sink);
        let route = fallback_plan.current().clone();
        if let Err(error) = self.activate(
            &mut live,
            node,
            &route,
            &stage_id,
            request.thread_id,
            &bindings,
        ) {
            live.discard(ShutdownReason::Error).await;
            return Err(error);
        }

        let result = async {
            // fabro-183f: a retry (or resumed execution) that found a
            // continuable session sends the minimal continuation message,
            // not the stage prompt; a fresh session — e.g. a cross-process
            // resume with no parked conversation — still needs the full
            // prompt to know the task.
            let input = match (&request.retry_continuation, is_reused) {
                (Some(continuation), true) => CodingInput::text(continuation_message(continuation)),
                _ => CodingInput::text(request.prompt),
            };
            let mut response = self
                .prompt_live(
                    &mut live,
                    node,
                    input,
                    &mut fallback_plan,
                    &stage_id,
                    request.thread_id,
                    &bindings,
                    cancel_token,
                )
                .await?;

            if let Some(schema) = &output_schema {
                let mut repair_attempts = 0_i64;
                let mut previous_validation_error = None;
                loop {
                    let last_file_touched = live.last_file_touched();
                    match validate_agent_output_sources(
                        schema,
                        &response,
                        request.sandbox,
                        last_file_touched.as_deref(),
                    )
                    .await
                    {
                        Ok(_) => break,
                        Err(error) => {
                            if repair_attempts >= node.output_retries() {
                                if !error.is_truncated() {
                                    return Err(Error::OutputSchemaValidation(
                                        structured_output::exhausted_failure_reason(
                                            node.output_retries(),
                                        ),
                                    ));
                                }
                                // fabro-274d: exhausted repairs on a truncated
                                // output are size overflow, not a correctness
                                // failure. Break with the best-effort text;
                                // handler-level validation persists it with an
                                // explicit truncation marker.
                                break;
                            }
                            let repair_message =
                                error.repair_message(schema, previous_validation_error.as_ref());
                            // Only once the model has seen the repair can a later
                            // identical failure mean it ignored the correction.
                            previous_validation_error = Some(error);
                            response = self
                                .prompt_live(
                                    &mut live,
                                    node,
                                    CodingInput::text(repair_message),
                                    &mut fallback_plan,
                                    &stage_id,
                                    request.thread_id,
                                    &bindings,
                                    cancel_token,
                                )
                                .await?;
                            repair_attempts += 1;
                        }
                    }
                }
            }

            self.drain_late_steering(
                &mut live,
                node,
                &mut fallback_plan,
                &stage_id,
                request.thread_id,
                &bindings,
                cancel_token,
                response,
            )
            .await
        }
        .await;

        let response = match result {
            Ok(response) => response,
            Err(error) => {
                let reason = if matches!(error, Error::Cancelled) {
                    ShutdownReason::Cancelled
                } else {
                    ShutdownReason::Error
                };
                if error.is_retryable() {
                    // fabro-183f: park the conversation so the retry (or an
                    // in-process resume) continues this session instead of
                    // re-sending the stage prompt to a fresh one. Under the
                    // thread key when the stage runs one, else under the
                    // stage id.
                    live.release_lease();
                    match live.agent.export_for_reuse(ShutdownReason::Error).await {
                        Ok(export) => {
                            let key = reuse_key.clone().unwrap_or_else(|| stage_id.to_string());
                            self.store_thread(key, CachedThread {
                                export,
                                fallback_plan: fallback_plan.clone(),
                            });
                        }
                        Err(export_error) => {
                            tracing::debug!(
                                error = %export_error,
                                "agent session did not shut down cleanly"
                            );
                            live.discard(reason).await;
                        }
                    }
                } else {
                    live.discard(reason).await;
                }
                // Cancellation and a retryable failure go up as the error, so
                // the engine cancels or retries as before. A terminal failure
                // becomes the stage's failed outcome, carrying what the
                // session tree spent and wrote before it failed.
                if matches!(error, Error::Cancelled) || error.is_retryable() {
                    return Err(error);
                }
                return Ok(CodergenResult::Full(Box::new(self.failed_outcome(
                    &error,
                    &live,
                    &fallback_plan,
                ))));
            }
        };

        let account = live.account();
        let billing = stage_billing(
            self.catalog.as_ref(),
            &route_model(fallback_plan.current()),
            &account,
        )?;

        live.release_lease();
        match reuse_key {
            // The thread's successor continues from an export whose cursor is
            // already past this session's close.
            Some(key) => match live.agent.export_for_reuse(ShutdownReason::Completed).await {
                Ok(export) => self.store_thread(key, CachedThread {
                    export,
                    fallback_plan: fallback_plan.clone(),
                }),
                Err(error) => {
                    tracing::debug!(error = %error, "agent session did not shut down cleanly");
                }
            },
            None => {
                if let Err(error) = live.agent.shutdown(ShutdownReason::Completed).await {
                    tracing::debug!(error = %error, "agent session did not shut down cleanly");
                }
            }
        }

        Ok(CodergenResult::Text {
            text:              response,
            usage:             Some(billing.total),
            usage_by_model:    billing.by_model,
            files_touched:     account.files_touched,
            last_file_touched: account.last_file_touched,
            timing:            StageTiming::active_only(
                crate::millis_u64(live.inference_duration),
                crate::millis_u64(live.tool_duration),
            ),
        })
    }
}

/// `files_from` file source over the stage's sandbox with the node's
/// `fs_hide` policy applied to both listing and reads: a hidden path
/// behaves as if it did not exist, matching the stage's other tools.
struct SandboxWorkflowFiles {
    sandbox:  Arc<RunSandbox>,
    fs_scope: Option<Arc<FsScope>>,
}

impl SandboxWorkflowFiles {
    fn admit(&self, path: &str) -> Result<(), fabro_tool::ToolError> {
        if self
            .fs_scope
            .as_ref()
            .is_some_and(|scope| scope.is_hidden(path))
        {
            return Err(fabro_tool::ToolError::message(format!(
                "`{path}` is hidden from this stage by fs_hide"
            )));
        }
        Ok(())
    }
}

#[async_trait]
impl fabro_tool::WorkflowFilesSource for SandboxWorkflowFiles {
    async fn list_text_files(&self, dir: &str) -> Result<Vec<String>, fabro_tool::ToolError> {
        self.admit(dir)?;
        let files = self
            .sandbox
            .walk_files(
                self.sandbox.working_directory(),
                dir,
                &WalkOptions::default(),
            )
            .await
            .map_err(|error| {
                fabro_tool::ToolError::message(format!(
                    "files_from walk below `{dir}` failed: {error}"
                ))
            })?;
        Ok(files.into_iter().map(|file| file.path).collect())
    }

    async fn read_text_file(&self, path: &str) -> Result<String, fabro_tool::ToolError> {
        self.admit(path)?;
        self.sandbox.read_file_text(path).await.map_err(|error| {
            fabro_tool::ToolError::message(format!("read `{path}` failed: {error}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use fabro_llm::test_support::test_catalog;
    use lithos_llm::catalog::builtin;
    use pebble_coding_agent::events::{CodingAgentEvent, CodingEvent, InputSource, TokenUsage};

    use super::*;

    fn root(event: CodingEvent) -> CodingAgentEvent {
        CodingAgentEvent::new("ses_root".to_string(), event, SystemTime::UNIX_EPOCH)
    }

    fn child(session_id: &str, event: CodingEvent) -> CodingAgentEvent {
        CodingAgentEvent::new(session_id.to_string(), event, SystemTime::UNIX_EPOCH)
            .with_parent_session_id("ses_root".to_string())
    }

    fn started(provider: &str, model: &str) -> CodingEvent {
        CodingEvent::SessionStarted {
            provider: Some(provider.to_string()),
            model:    Some(model.to_string()),
        }
    }

    fn message(model: &str, input: u64, output: u64, cost: Option<u64>) -> CodingEvent {
        CodingEvent::AssistantMessage {
            text:            "ok".to_string(),
            model:           model.to_string(),
            usage:           TokenUsage {
                input,
                output,
                ..TokenUsage::default()
            },
            cost_usd_micros: cost,
            cost_source:     None,
            tool_call_count: 0,
            context_window:  None,
            reasoning:       None,
        }
    }

    fn root_model() -> ModelRef {
        ModelRef::new(builtin::openai(), ModelId::new("gpt-5.4"))
    }

    fn account(events: &[CodingAgentEvent]) -> SessionProjection {
        let mut account = SessionProjection::new();
        account.apply_all(events);
        account
    }

    #[test]
    fn stage_billing_prices_the_root_at_its_route_and_each_descendant_at_its_own() {
        let catalog = test_catalog();
        let account = account(&[
            root(started("openai", "gpt-5.4")),
            root(CodingEvent::UserInput {
                text:    "go".to_string(),
                content: None,
                source:  InputSource::Prompt,
            }),
            root(message("gpt-5.4", 100_000, 25_000, None)),
            // A child on the parent's route joins the parent's row.
            child("ses_same", started("openai", "gpt-5.4")),
            child("ses_same", message("gpt-5.4", 10_000, 1_000, None)),
            // A child on another route is its own row, at that route's rate.
            child("ses_other", started("anthropic", "claude-sonnet-5")),
            child("ses_other", message("claude-sonnet-5", 20_000, 2_000, None)),
            // A child on a route the catalog does not know bills at the root's.
            child("ses_unknown", started("nowhere", "mystery")),
            child("ses_unknown", message("mystery", 1_000, 100, None)),
            root(CodingEvent::ProcessingEnd),
        ]);

        let billing = stage_billing(&catalog, &root_model(), &account).unwrap();

        assert_eq!(billing.by_model.len(), 2, "{:?}", billing.by_model);
        let root_row = &billing.by_model[0];
        assert_eq!(root_row.model, root_model());
        assert_eq!(
            root_row.tokens.input, 111_000,
            "the root, the same-route child, and the unknown-route child"
        );
        assert_eq!(root_row.tokens.output, 26_100);
        let root_priced =
            billed_model_usage_from_llm(&catalog, &root_model(), root_row.tokens).unwrap();
        assert_eq!(root_row.total_usd_micros, root_priced.total_usd_micros);

        let other_model = ModelRef::new(
            ProviderId::new("anthropic"),
            ModelId::new("claude-sonnet-5"),
        );
        let other_row = &billing.by_model[1];
        assert_eq!(other_row.model, other_model);
        assert_eq!(other_row.tokens.input, 20_000);
        assert_eq!(other_row.tokens.output, 2_000);
        let other_priced =
            billed_model_usage_from_llm(&catalog, &other_model, other_row.tokens).unwrap();
        assert_eq!(other_row.total_usd_micros, other_priced.total_usd_micros);
        assert_ne!(
            other_row.total_usd_micros,
            billed_model_usage_from_llm(&catalog, &root_model(), other_row.tokens)
                .unwrap()
                .total_usd_micros,
            "priced at its own rate, not the root's"
        );

        // The total is the tree's tokens under the root's route, at the rows' summed
        // cost.
        assert_eq!(billing.total.model, root_model());
        assert_eq!(billing.total.tokens.input, 131_000);
        assert_eq!(billing.total.tokens.output, 28_100);
        assert_eq!(
            billing.total.total_usd_micros,
            Some(root_priced.total_usd_micros.unwrap() + other_priced.total_usd_micros.unwrap())
        );
    }

    #[test]
    fn a_provider_reported_cost_stands_in_for_the_catalogs_estimate() {
        let catalog = test_catalog();
        let account = account(&[
            root(started("openai", "gpt-5.4")),
            root(message("gpt-5.4", 1_000, 100, Some(4_321))),
            child("ses_child", started("anthropic", "claude-sonnet-5")),
            child("ses_child", message("claude-sonnet-5", 500, 50, None)),
        ]);

        let billing = stage_billing(&catalog, &root_model(), &account).unwrap();

        assert_eq!(billing.by_model[0].total_usd_micros, Some(4_321));
        let child_priced = billed_model_usage_from_llm(
            &catalog,
            &billing.by_model[1].model,
            billing.by_model[1].tokens,
        )
        .unwrap();
        assert_eq!(
            billing.by_model[1].total_usd_micros,
            child_priced.total_usd_micros
        );
        assert_eq!(
            billing.total.total_usd_micros,
            Some(4_321 + child_priced.total_usd_micros.unwrap())
        );
    }

    #[test]
    fn a_descendant_seen_only_through_its_answers_bills_on_the_roots_provider() {
        let catalog = test_catalog();
        let mut account = account(&[root(started("openai", "gpt-5.4"))]);
        // No `SessionStarted` for the child: only its answer names a model.
        account.apply(&child(
            "ses_quiet",
            message("gpt-5.4-mini", 1_000, 100, None),
        ));

        let billing = stage_billing(&catalog, &root_model(), &account).unwrap();

        let child_row = billing
            .by_model
            .iter()
            .find(|row| row.model.model_id.as_str() == "gpt-5.4-mini")
            .expect("the child is billed as its answers' model on the root's provider");
        assert_eq!(child_row.model.provider, root_model().provider);
        assert_eq!(child_row.tokens.input, 1_000);
    }
}
