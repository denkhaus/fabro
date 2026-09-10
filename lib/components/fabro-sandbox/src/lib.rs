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
pub use exec::{
    DEFAULT_RETAINED_OUTPUT_BYTES, DEFAULT_STOP_GRACE, ExecResultExt, ExplicitEnvPolicy,
    SandboxExec, command_termination, is_sensitive_env_var, program_exit_code,
};
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
pub use provider::{SandboxInventory, SandboxLookupError};
pub use provider_sandbox::{attach_provider_sandbox, provider_sandbox};
pub use push_credentials::RefreshErrorKind;
pub use reconnect::{
    open_terminal_for_run, reconnect, reconnect_driver_for_run, reconnect_for_run,
    reconnect_for_run_with_events,
};
pub use sandbox::{
    DEFAULT_EXEC_OUTPUT_TAIL_BYTES, GitRunInfo, GitSetupIntent, PushAttempt, PushError, PushReport,
    RefreshOutcome, RemoteCredentialAction, SandboxFile, SandboxWorkspaceLayout,
    format_lines_numbered, redacted_output_tail, setup_git, shell_quote,
};
/// Driver types a run sandbox speaks: what a command is and how it ended,
/// what the file and search operations return, and the network policy a
/// [`SandboxOptions`] asks for. Re-exported so consumers need no direct
/// driver dependency.
pub use sandbox_driver::{
    CaptureStats, DirEntry, ExecControls, ExecFailure, ExecResult, ExecSpec, ExecStreamingResult,
    FileKind, GrepMatch, GrepOptions, NetworkPolicy, OutputSink, OutputStream, PtySession, PtySize,
    StderrTail, StdioProcess, StdioProcessHandle, Termination, TransportError, WalkOptions,
};
pub use sandbox_spec::{ProviderSandboxSpec, SandboxSpec};
