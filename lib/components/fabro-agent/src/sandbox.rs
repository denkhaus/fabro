// Re-export the sandbox types the agent works with from fabro-sandbox.
pub use fabro_sandbox::{
    CaptureStats, DirEntry, ExecControls, ExecResult, ExecResultExt, ExecSpec, ExecStreamingResult,
    FileKind, GrepMatch, GrepOptions, OutputSink, OutputStream, RefreshOutcome,
    RemoteCredentialAction, RunSandbox, SandboxFile, StderrTail, StdioProcess, StdioProcessHandle,
    Termination, TokenProvenance, TokenSnapshot, WalkOptions, command_termination,
    format_lines_numbered, program_exit_code, shell_quote,
};
