// Re-export the sandbox types the agent works with from fabro-sandbox.
pub use fabro_sandbox::{
    CommandOutputCallback, DirEntry, ExecResult, ExecStreamingRequest, ExecStreamingResult,
    FileKind, GrepMatch, GrepOptions, OutputCaptureStats, RefreshOutcome, RemoteCredentialAction,
    RunSandbox, SandboxFile, StderrCollector, StdioProcess, StdioProcessHandle,
    StdioProcessTermination, TokenProvenance, TokenSnapshot, WalkOptions, format_lines_numbered,
    shell_quote,
};
