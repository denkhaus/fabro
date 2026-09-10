pub mod error;
pub mod options;
pub mod provider;
pub mod sandbox;
pub mod sandbox_spec;

mod clone_source;

mod git_retry;

mod managed_labels;

mod push_credentials;

pub mod redact;

pub mod details;

pub mod driver;
pub mod driver_sandbox;

pub mod exec;

pub mod reconnect;

pub mod terminal;

mod clone;
pub mod docker;
pub mod provider_sandbox;

pub mod daytona;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use details::sandbox_details;
pub use docker::check_docker_daemon;
pub use driver::{DaytonaCredentials, ProviderAccess};
pub use driver_sandbox::{RunSandbox, local_sandbox};
pub use error::{Error, Result, default_redacted_output_tail, display_for_log};
pub use exec::{ExplicitEnvPolicy, SandboxExec, is_sensitive_env_var};
pub use fabro_github::token_source::{
    InstallationTokenSource, ResolvedToken, TokenProvenance, TokenSnapshot,
};
pub use fabro_types::{RunSandboxInstance, SandboxProviderKind};
pub use git_retry::{
    CredentialContext, GitRetryReason, RetryPlan, classify_failure, retry_git_operation,
};
pub use options::{
    SandboxOptions, local_working_directory_from_environment, options_from_environment,
    unresolved_env,
};
pub use provider::driver::DriverInventoryProvider;
pub use provider::{
    LocalSandboxProvider, SandboxLookupError, SandboxProvider, SandboxProviderRegistry,
};
pub use provider_sandbox::{attach_provider_sandbox, provider_sandbox};
pub use push_credentials::RefreshErrorKind;
pub use reconnect::{
    reconnect, reconnect_driver_for_run, reconnect_for_run, reconnect_for_run_with_events,
};
pub use sandbox::{
    CommandOutputCallback, DEFAULT_EXEC_OUTPUT_TAIL_BYTES, ExecResult, ExecStreamingRequest,
    ExecStreamingResult, GitRunInfo, GitSetupIntent, OutputCaptureStats, PushAttempt, PushError,
    PushReport, RefreshOutcome, RemoteCredentialAction, SandboxFile, SandboxWorkspaceLayout,
    StderrCollector, StdioProcess, StdioProcessHandle, StdioProcessTermination,
    format_lines_numbered, redacted_output_tail, setup_git_via_exec, shell_quote,
};
/// Driver types a run sandbox's file and search operations speak, and the
/// network policy a [`SandboxOptions`] asks for, re-exported so consumers
/// need no direct driver dependency.
pub use sandbox_driver::{DirEntry, FileKind, GrepMatch, GrepOptions, NetworkPolicy, WalkOptions};
pub use sandbox_spec::{ProviderSandboxSpec, SandboxSpec};
pub use terminal::{DriverTerminalSession, TerminalSession, TerminalSize, open_terminal_for_run};
