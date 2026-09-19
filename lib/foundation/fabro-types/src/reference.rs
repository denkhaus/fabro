//! The kinds of static file reference a workflow package carries.
//!
//! A workflow names other files from its graph (`import`,
//! `stack.child_workflow`, `@`-prefixed `prompt`, `output_schema` and `goal`
//! values) and from its `workflow.toml` (a Dockerfile, a run goal file).
//! Every such reference is static: it is resolved before any template
//! renders, so it may not contain template syntax. The kind names which rule
//! a reference was read under, for error messages.

/// Kinds of static (non-templated) workflow-owned file references.
#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::Display)]
pub enum ReferenceKind {
    #[strum(to_string = "file inline reference")]
    FileInline,
    #[strum(to_string = "import reference")]
    Import,
    #[strum(to_string = "child workflow reference")]
    ChildWorkflow,
    #[strum(to_string = "Dockerfile reference")]
    Dockerfile,
    #[strum(to_string = "graph goal file reference")]
    GraphGoalFile,
    #[strum(to_string = "run goal file reference")]
    RunGoalFile,
}
