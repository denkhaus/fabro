use fabro_graphviz::Error as GraphvizError;
use fabro_types::diagnostic::Diagnostic;
use fabro_util::error::{SharedError, collect_chain, render_with_causes};
use thiserror::Error as ThisError;

#[derive(ThisError, Debug, Clone)]
pub enum Error {
    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Validation failed")]
    ValidationFailed { diagnostics: Vec<Diagnostic> },

    /// Fabro's own platform work around a run failed: a store call, a
    /// serialization, a spawned task, a Git command.
    #[error("Engine error: {message}")]
    Engine {
        message: String,
        #[source]
        source:  Option<SharedError>,
    },

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
            _ => Vec::new(),
        }
    }

    #[must_use]
    pub fn display_with_causes(&self) -> String {
        render_with_causes(&self.to_string(), &self.causes())
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
