use std::fmt;
use std::sync::Arc;

use fabro_graphviz::Error as GraphvizError;
use fabro_template::TemplateError;
use fabro_types::diagnostic::Diagnostic;
use fabro_types::settings::ResolveError;
use fabro_util::error::{SharedError, collect_chain, render_with_causes};
use thiserror::Error as ThisError;

/// A template error shared across clones of the workflow error that carries
/// it, so the miette diagnostic and the source chain survive cloning.
#[derive(Debug, Clone)]
pub struct SharedTemplateError(Arc<TemplateError>);

impl SharedTemplateError {
    #[must_use]
    pub fn new(error: TemplateError) -> Self {
        Self(Arc::new(error))
    }

    #[must_use]
    pub fn inner(&self) -> &TemplateError {
        &self.0
    }
}

impl fmt::Display for SharedTemplateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&*self.0, formatter)
    }
}

impl std::error::Error for SharedTemplateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        std::error::Error::source(&*self.0)
    }
}

impl miette::Diagnostic for SharedTemplateError {
    fn code<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        miette::Diagnostic::code(&*self.0)
    }

    fn help<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        miette::Diagnostic::help(&*self.0)
    }

    fn source_code(&self) -> Option<&dyn miette::SourceCode> {
        miette::Diagnostic::source_code(&*self.0)
    }

    fn labels(&self) -> Option<Box<dyn Iterator<Item = miette::LabeledSpan> + '_>> {
        miette::Diagnostic::labels(&*self.0)
    }

    fn diagnostic_source(&self) -> Option<&dyn miette::Diagnostic> {
        miette::Diagnostic::diagnostic_source(&*self.0)
    }
}

#[derive(ThisError, Debug, Clone)]
pub enum Error {
    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Validation failed")]
    ValidationFailed { diagnostics: Vec<Diagnostic> },

    #[error("Validation error: script interpolation failed in {owner}: {source} ({fix})")]
    ScriptInterpolation {
        owner:  String,
        fix:    String,
        #[source]
        source: ResolveError,
    },

    #[error("{message}")]
    Template {
        message: String,
        #[source]
        source:  SharedTemplateError,
    },

    /// Fabro's own platform work around a run failed: a store call, a
    /// serialization, a spawned task, a Git command.
    #[error("Engine error: {message}")]
    Engine {
        message: String,
        #[source]
        source:  Option<SharedError>,
    },

    #[error("Stylesheet error: {0}")]
    Stylesheet(String),

    #[error("I/O error: {0}")]
    Io(String),

    #[error("Precondition failed: {0}")]
    Precondition(String),

    #[error("Run not found: {0}")]
    RunNotFound(String),

    #[error("Pipeline cancelled")]
    Cancelled,
}

impl Error {
    pub fn template(message: impl Into<String>, source: TemplateError) -> Self {
        Self::Template {
            message: message.into(),
            source:  SharedTemplateError::new(source),
        }
    }

    pub fn engine(message: impl Into<String>) -> Self {
        Self::Engine {
            message: message.into(),
            source:  None,
        }
    }

    pub fn engine_with_source(
        message: impl Into<String>,
        source: impl Into<anyhow::Error>,
    ) -> Self {
        Self::Engine {
            message: message.into(),
            source:  Some(SharedError::new(source.into())),
        }
    }

    pub fn engine_with_anyhow(message: impl Into<String>, source: anyhow::Error) -> Self {
        Self::engine_with_source(message, source)
    }

    #[must_use]
    pub fn causes(&self) -> Vec<String> {
        match self {
            Self::Engine { source, .. } => source
                .as_ref()
                .map_or_else(Vec::new, |source| collect_chain(source)),
            Self::Template { source, .. } => collect_chain(source),
            Self::ScriptInterpolation { source, .. } => collect_chain(source),
            _ => Vec::new(),
        }
    }

    #[must_use]
    pub fn display_with_causes(&self) -> String {
        render_with_causes(&self.to_string(), &self.causes())
    }
}

impl miette::Diagnostic for Error {
    fn code<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        match self {
            Self::Template { source, .. } => miette::Diagnostic::code(source),
            _ => None,
        }
    }

    fn help<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        match self {
            Self::Template { source, .. } => miette::Diagnostic::help(source),
            _ => None,
        }
    }

    fn source_code(&self) -> Option<&dyn miette::SourceCode> {
        match self {
            Self::Template { source, .. } => miette::Diagnostic::source_code(source),
            _ => None,
        }
    }

    fn labels(&self) -> Option<Box<dyn Iterator<Item = miette::LabeledSpan> + '_>> {
        match self {
            Self::Template { source, .. } => miette::Diagnostic::labels(source),
            _ => None,
        }
    }

    fn diagnostic_source(&self) -> Option<&dyn miette::Diagnostic> {
        match self {
            Self::Template { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err.to_string())
    }
}

impl From<GraphvizError> for Error {
    fn from(e: GraphvizError) -> Self {
        match e {
            GraphvizError::Parse(msg) => Self::Parse(msg),
            GraphvizError::Stylesheet(msg) => Self::Stylesheet(msg),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct TestCause(&'static str);

    impl std::fmt::Display for TestCause {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }

    impl std::error::Error for TestCause {}

    #[derive(Debug)]
    struct TestOuterError {
        message: &'static str,
        source:  TestCause,
    }

    impl std::fmt::Display for TestOuterError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.message)
        }
    }

    impl std::error::Error for TestOuterError {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.source)
        }
    }

    #[test]
    fn parse_error_display() {
        let err = Error::Parse("unexpected token".to_string());
        assert_eq!(err.to_string(), "Parse error: unexpected token");
    }

    #[test]
    fn validation_error_display() {
        let err = Error::Validation("missing start node".to_string());
        assert_eq!(err.to_string(), "Validation error: missing start node");
    }

    #[test]
    fn validation_failed_display() {
        let err = Error::ValidationFailed {
            diagnostics: vec![Diagnostic {
                rule: "test".to_string(),
                severity: fabro_types::diagnostic::Severity::Error,
                message: "missing start node".to_string(),
                node_id: None,
                edge: None,
                fix: None,

                ..Diagnostic::default()
            }],
        };
        assert_eq!(err.to_string(), "Validation failed");
    }

    #[test]
    fn template_error_variant_preserves_source_chain() {
        let template_err = fabro_template::render_named(
            "workflow.fabro",
            "{{ inputs.missing }}",
            &fabro_template::TemplateContext::new(),
        )
        .unwrap_err();

        let err = Error::template("template expansion failed", template_err);
        let chain = collect_chain(&err);

        assert!(
            chain
                .iter()
                .any(|part| part.contains("template expansion failed"))
        );
        assert!(
            chain
                .iter()
                .any(|part| part.contains("undefined template variable"))
        );
    }

    #[test]
    fn engine_error_display() {
        let err = Error::engine("no outgoing edge");
        assert_eq!(err.to_string(), "Engine error: no outgoing edge");
    }

    #[test]
    fn engine_error_with_source_preserves_cause_chain() {
        let source = TestOuterError {
            message: "Failed to pull Docker image buildpack-deps:noble",
            source:  TestCause("connection refused"),
        };
        let err = Error::engine_with_source("Failed to initialize sandbox", source);

        assert_eq!(
            err.to_string(),
            "Engine error: Failed to initialize sandbox"
        );
        assert_eq!(err.causes(), vec![
            "Failed to pull Docker image buildpack-deps:noble".to_string(),
            "connection refused".to_string(),
        ]);
        assert_eq!(
            err.display_with_causes(),
            "Engine error: Failed to initialize sandbox\n  caused by: Failed to pull Docker image buildpack-deps:noble\n  caused by: connection refused"
        );
    }

    #[test]
    fn io_error_display() {
        let err = Error::Io("permission denied".to_string());
        assert_eq!(err.to_string(), "I/O error: permission denied");
    }

    #[test]
    fn io_error_from_std() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        let err = Error::from(io_err);
        assert!(matches!(err, Error::Io(_)));
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn result_type_alias_works() {
        let ok: Result<i32> = Ok(42);
        assert!(ok.is_ok());

        let err: Result<i32> = Err(Error::Parse("bad".to_string()));
        assert!(err.is_err());
    }

    #[test]
    fn cancelled_error_display() {
        let err = Error::Cancelled;
        assert_eq!(err.to_string(), "Pipeline cancelled");
    }

    #[test]
    fn error_clone_preserves_display_for_all_variants() {
        let errors: Vec<Error> = vec![
            Error::Parse("bad".into()),
            Error::Validation("bad".into()),
            Error::ValidationFailed {
                diagnostics: vec![Diagnostic {
                    rule: "test".into(),
                    severity: fabro_types::diagnostic::Severity::Error,
                    message: "bad".into(),
                    node_id: None,
                    edge: None,
                    fix: None,

                    ..Diagnostic::default()
                }],
            },
            Error::engine("engine err"),
            Error::engine_with_source("engine err", TestCause("cause")),
            Error::Stylesheet("style err".into()),
            Error::Io("io err".into()),
            Error::Precondition("precondition".into()),
            Error::RunNotFound("run".into()),
            Error::Cancelled,
        ];
        for err in errors {
            assert_eq!(err.to_string(), err.clone().to_string());
        }
    }
}
