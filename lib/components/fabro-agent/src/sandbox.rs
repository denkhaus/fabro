// Re-export the sandbox types the agent works with from fabro-sandbox.
pub use fabro_sandbox::{
    CaptureStats, DirEntry, DriverSpec, ExecControls, ExecResult, ExecResultExt, ExecSpec,
    ExecStreamingResult, FileKind, GrepMatch, GrepOptions, OutputSink, OutputStream, RunSandbox,
    SandboxFile, SandboxSource, StderrTail, StdioProcess, StdioProcessHandle, Termination,
    TokenProvenance, TokenSnapshot, WalkOptions, command_termination, format_lines_numbered,
    program_exit_code, shell_quote,
};
