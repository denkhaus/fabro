pub mod config;
pub mod error;
pub mod from_environment;
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
pub mod plugin;

pub mod daytona;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use daytona::{DaytonaConfig, attach_daytona, daytona_sandbox};
pub use details::sandbox_details;
pub use docker::{DockerSandboxOptions, attach_docker, check_docker_daemon, docker_sandbox};
pub use driver::{DaytonaCredentials, ProviderAccess};
pub use driver_sandbox::{DriverSandbox, local_sandbox};
pub use error::{Error, Result, default_redacted_output_tail, display_for_log};
pub use exec::{ExplicitEnvPolicy, SandboxExec, is_sensitive_env_var};
pub use fabro_github::token_source::{
    InstallationTokenSource, ResolvedToken, TokenProvenance, TokenSnapshot,
};
pub use fabro_types::{RunSandboxInstance, SandboxProviderKind};
pub use git_retry::{
    CredentialContext, GitRetryReason, RetryPlan, classify_failure, retry_git_operation,
};
pub use plugin::{PluginSandboxOptions, attach_plugin, plugin_sandbox};
pub use provider::driver::DriverInventoryProvider;
pub use provider::{
    LocalSandboxProvider, SandboxLookupError, SandboxProvider, SandboxProviderRegistry,
};
pub use push_credentials::RefreshErrorKind;
pub use reconnect::{
    reconnect, reconnect_driver_for_run, reconnect_for_run, reconnect_for_run_with_callback,
};
pub use sandbox::{
    CommandOutputCallback, DEFAULT_EXEC_OUTPUT_TAIL_BYTES, DirEntry, ExecResult,
    ExecStreamingRequest, ExecStreamingResult, GitRunInfo, GitSetupIntent, GrepOptions,
    OutputCaptureStats, PushAttempt, PushError, PushReport, RefreshOutcome, RemoteCredentialAction,
    Sandbox, SandboxEvent, SandboxEventCallback, SandboxFile, SandboxWorkspaceLayout,
    StderrCollector, StdioProcess, StdioProcessHandle, StdioProcessTermination, WalkOptions,
    format_lines_numbered, redacted_output_tail, setup_git_via_exec, shell_quote,
};
pub use sandbox_spec::SandboxSpec;
pub use terminal::{DriverTerminalSession, TerminalSession, TerminalSize, open_terminal_for_run};
