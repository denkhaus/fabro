// Re-export the sandbox types the agent works with from fabro-sandbox.
pub use fabro_sandbox::{
    CaptureStats, DirEntry, DriverSpec, ExecControls, ExecResult, ExecResultExt, ExecSpec,
    ExecStreamingResult, FileKind, FsScope, FsScopeError, GrepMatch, GrepOptions, OutputSink,
    OutputStream, RunSandbox, SandboxFile, SandboxSource, ScopeDenial, StderrTail, StdioProcess,
    StdioProcessHandle, Termination, TokenProvenance, TokenSnapshot, WalkOptions,
    command_termination, program_exit_code,
};
