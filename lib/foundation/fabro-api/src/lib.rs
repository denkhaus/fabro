#[allow(
    clippy::absolute_paths,
    clippy::all,
    clippy::derivable_impls,
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::needless_lifetimes,
    clippy::unwrap_used,
    unreachable_pub,
    unused_imports,
    reason = "Generated OpenAPI client code intentionally preserves codegen output."
)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/codegen.rs"));
}
pub mod types {
    pub use fabro_automation::{
        Automation,
        AutomationDraft as CreateAutomationRequest,
        AutomationReplace as ReplaceAutomationRequest,
        AutomationTrigger,
        // Fork (fabro-b869 step 5b): the provider-window read model reuses
        // the domain types, so the gate's facts and the API payload cannot
        // drift into parallel shapes.
        ModelProviderRequirement as AutomationModelProvider,
        ProviderWindowFact as AutomationProviderWindowFact,
        ProviderWindowKind,
        ProviderWindowState as AutomationProviderWindowState,
        // Fork (fabro-3d97): the breaker read model rides the domain type
        // like the window facts, so the scheduler's bookkeeping and the API
        // payload cannot drift into parallel shapes.
        ScheduleBreakerState as AutomationScheduleBreakerState,
    };
    pub use fabro_environment::Environment;
    pub use fabro_types::settings::run::{
        McpHttpProtocol, RunIntegrationsGithubSettings, RunIntegrationsSettings, RunModelControls,
        RunModelSettings,
    };
    pub use fabro_types::settings::server::{
        GithubIntegrationSettings, GithubIntegrationStrategy, IntegrationWebhooksSettings,
        LogDestination, ObjectStoreSettings, ServerApiSettings, ServerArtifactsSettings,
        ServerAuthGithubSettings, ServerAuthMethod, ServerAuthSettings, ServerIntegrationsSettings,
        ServerListenSettings, ServerLoggingSettings, ServerSandboxProviderSettings,
        ServerSandboxProvidersSettings, ServerSandboxSettings, ServerSchedulerSettings,
        ServerStorageSettings, ServerWebSettings, SlackIntegrationSettings, WebhookStrategy,
    };
    pub use fabro_types::settings::{AskNamespace, McpTransport, ServerNamespace};
    pub use fabro_types::status::{
        BlockedReason, FailureReason, PendingReason, RunControlAction, RunStatus, SuccessReason,
    };
    pub use fabro_types::{
        AgentEventProps, AgentSessionActivatedProps, AgentToolsAvailableProps, AskFabro,
        AskFabroCapability, AuthMethod, AutomationRef, BlobHash, CommandTermination, Conclusion,
        ContextWindowBreakdownItem, ContextWindowCategory, ContextWindowCountMethod,
        ContextWindowSnapshot, ContextWindowStaleness, ContextWindowWarning, CreateVariableRequest,
        DiffStats, DiffSummary, DirtyStatus, ExecOutputTail, FailureCategory, FailureDetail,
        FailureSignature, GitContext, GitRunTarget, GitRunTarget as AutomationGitWorkflowSource,
        IdpIdentity, IntegrationConnectionKind, IntegrationConnectionState,
        IntegrationConnectionStatus, IntegrationProvider, IntegrationStatus, InterviewOption,
        InterviewQuestionRecord, LlmOutputKind, McpServerDraft as CreateMcpServerRequest,
        McpServerReplace as ReplaceMcpServerRequest, McpServerView as McpServer, McpTransportView,
        Model, ModelControls, ModelCosts, ModelFeatures, ModelLimits, ModelRef as UsageModelRef,
        ModelTestMode, ModelUsage, ObservationRef, OrchestrationField, OrchestrationVerdict,
        PairId, PairMessageId, PairMessageRecord, PairMessageRequest, PairRecord, PairStartRequest,
        PairStatus, PairTarget, PairTranscriptEntry, PairTranscriptResponse, ParallelBranchId,
        ParallelBranchResult, PendingInterviewRecord, PermissionLevel, PetriAdmission,
        PetriGraphRef, Principal, Provider, PullRequest, PullRequestCreation,
        PullRequestCreationId, PullRequestCreationStatus, PullRequestDetails,
        PullRequestDetailsStatus, PullRequestDetailsUnavailableReason, PullRequestLink,
        PullRequestMeta, PullRequestResponse, QuestionType, RepositoryRef, ReviewTarget,
        ReviewTargetKind, Run, RunApproval, RunApprovalState, RunClientProvenance, RunFailure,
        RunGraph, RunGraphEdge, RunGraphNode, RunIntent, RunIntentArgs, RunPairStatusResponse,
        RunProjection, RunProvenance, RunRunnableSource, RunSandbox, RunSandboxFailure,
        RunSandboxInstance, RunSandboxKind, RunSandboxPlan, RunSandboxRuntime, RunServerProvenance,
        RunSessionMetadata, RunSize, RunStreamItem, RunStreamItemKind, RunTarget, SandboxDetails,
        SandboxInfo, SandboxListMeta, SandboxListResponse, SandboxProviderKind,
        SandboxProviderLookupError, SandboxService, SandboxServiceListResponse, SecretMetadata,
        SecretType, ServerSettings, SessionDetail, SessionEvent, SessionEventBody, SessionId,
        SessionStatus, SessionSummary, SessionTurn, SetSessionCapabilityRequest,
        SkillActivationSource, SkillSummary, StageCompletion, StageContextWindow,
        StageContextWindowUnavailableReason, StageHandler, StageId, StageInferenceProjection,
        StageModelUsage, StageOutcome, StageProjection, StageState, StageToolBatchProjection,
        SystemActorKind, SystemIntegrationStatus, SystemIntegrationsResponse, TodoListProjection,
        ToolCategory, ToolSource, ToolSummary, TurnId, UpdateVariableRequest, UserPrincipal,
        Variable, VariableListResponse, WorkItemComment, WorkflowPath, WorkflowSettings,
        WorkflowVersion, WorkflowVersionId,
    };
    pub use lithos_llm::catalog::{ModelHandle, ProviderId};
    pub use lithos_llm::types::{
        ContentPart, Cost, CostSource, ErrorKind as LlmErrorKind, Message, ReasoningEffort,
        ReasoningOutput, ResponseFormat as CompletionResponseFormat,
        RetryClassification as LlmRetryClassification, Role, Speed, TokenCounts,
        ToolChoice as CompletionToolChoice, ToolDefinition as CompletionToolDefinition,
        ToolDefinitionKind as CompletionToolDefinitionKind, Usage,
    };
    /// `StageProjection.agent` is the coding agent's own fold of the stage's
    /// events; the API reuses pebble's types under the schema names.
    pub use pebble_coding_agent::events::{
        CompactionReason, ErrorData as AgentErrorData, ErrorKind as AgentErrorKind,
        FailoverContinuation, FailoverStop, McpToolSummary,
    };
    pub use pebble_coding_agent::projection::{
        ActivatedSkill as AgentSessionActivatedSkill,
        CompactionProjection as AgentSessionCompaction,
        DescendantAccount as AgentSessionDescendantAccount,
        FailoverStopProjection as AgentSessionFailoverStop,
        McpServerProjection as AgentSessionMcpServer, PromptDelta as AgentSessionPromptDelta,
        RouteFailoverProjection as AgentSessionRouteFailover, RouteProjection as AgentSessionRoute,
        SessionActivity as AgentSessionActivity, SessionProjection as AgentSessionProjection,
        SkillsProjection as AgentSessionSkills, SubagentCounts as AgentSessionSubagentCounts,
        SubagentProjection as AgentSessionSubagent, SubagentStatus as AgentSessionSubagentStatus,
        ToolActivity as AgentSessionToolActivity,
    };
    /// A sandbox's status on the API is the sandbox driver's own type.
    pub use sandbox_driver::{
        NetworkPolicy as SandboxNetworkPolicy, Resources as SandboxResources, SandboxId,
        SandboxKind, SandboxState, SandboxStatus, WorkspaceOwnership as SandboxWorkspaceOwnership,
    };

    pub use crate::generated::types::*;
}

/// The single run-status-kind → board-column mapping (work#1460).
///
/// `BoardColumn` is the API's board vocabulary — the buckets the runs
/// list and the kanban board share — and this function is its one source
/// of status-kind assignments. It lives in fabro-api as a free function
/// over the foreign [`fabro_types::RunStatusKind`] because the dependency
/// edge only works this way: fabro-api already imports fabro-types, so
/// fabro-types cannot host the mapping without a cycle. The spec's
/// `BoardColumn` enum, [`RunStatusKind::board_rank`]'s ranks, and the
/// server's status-filter lowering all conform to this mapping;
/// `tests/board_column_conformance.rs` holds the conformance proof.
#[must_use]
pub fn board_column(kind: fabro_types::RunStatusKind) -> types::BoardColumn {
    use fabro_types::RunStatusKind;

    match kind {
        RunStatusKind::Submitted | RunStatusKind::Pending => types::BoardColumn::Pending,
        RunStatusKind::Runnable => types::BoardColumn::Runnable,
        RunStatusKind::Starting => types::BoardColumn::Initializing,
        RunStatusKind::Running | RunStatusKind::Paused => types::BoardColumn::Running,
        RunStatusKind::Blocked => types::BoardColumn::Blocked,
        RunStatusKind::HumanNeeded => types::BoardColumn::HumanNeeded,
        RunStatusKind::Succeeded => types::BoardColumn::Succeeded,
        RunStatusKind::Failed | RunStatusKind::Dead => types::BoardColumn::Failed,
        RunStatusKind::Removing => types::BoardColumn::Removing,
    }
}

/// The status kinds that board in `column`, in enum order — the inverse
/// of [`board_column`], derived from it by walking
/// [`fabro_types::RunStatusKind`]'s variants so no second table can
/// drift. `None` for `archived`: that column selects on the archival
/// overlay, not on a status bucket.
#[must_use]
pub fn board_column_kinds(column: types::BoardColumn) -> Option<Vec<fabro_types::RunStatusKind>> {
    use strum::VariantArray;

    if column == types::BoardColumn::Archived {
        return None;
    }
    Some(
        fabro_types::RunStatusKind::VARIANTS
            .iter()
            .copied()
            .filter(|kind| board_column(*kind) == column)
            .collect(),
    )
}
pub use generated::Client as ApiClient;
