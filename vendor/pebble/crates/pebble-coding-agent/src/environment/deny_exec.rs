//! An [`Environment`] that refuses every file and command operation.
//!
//! Built for tools-only agents — the Ask Fabro supervisor (ADR-0046
//! phase 2): the agent acts through client tools over the network, and
//! its own file/exec surface is a denial with a model-readable reason.
//! Session init tolerates the refusals: memory loads, the git root
//! probe, and skill discovery all skip when the environment errors.

use async_trait::async_trait;

use super::{
    DirEntry, EnvResult, Environment, EnvironmentError, EnvironmentErrorKind, ExecOutcome,
    ExecRequest, GrepOptions,
};

/// The refusal every operation reports. Written for the model: it names
/// the situation and the way out.
const DENIED: &str = "This agent is tools-only: file and command operations are not available. \
     Use the provided client tools instead.";

/// An [`Environment`] that denies every operation.
///
/// The working directory is a display value only: nothing can read,
/// write, or execute through this environment.
pub struct DenyExecEnvironment {
    working_directory: String,
}

impl DenyExecEnvironment {
    /// Builds the environment; `working_directory` is display-only.
    #[must_use]
    pub fn new(working_directory: impl Into<String>) -> Self {
        Self {
            working_directory: working_directory.into(),
        }
    }
}

fn denied() -> EnvironmentError {
    EnvironmentError::new(EnvironmentErrorKind::Unsupported, DENIED)
}

#[async_trait]
impl Environment for DenyExecEnvironment {
    fn working_directory(&self) -> &str {
        &self.working_directory
    }

    fn platform(&self) -> &str {
        "unknown"
    }

    fn os_version(&self) -> String {
        "unavailable".to_owned()
    }

    async fn read_file_bytes(&self, _path: &str) -> EnvResult<Vec<u8>> {
        Err(denied())
    }

    async fn write_file(&self, _path: &str, _content: &str) -> EnvResult<()> {
        Err(denied())
    }

    async fn rename_file(&self, _source: &str, _destination: &str) -> EnvResult<()> {
        Err(denied())
    }

    async fn delete_file(&self, _path: &str) -> EnvResult<()> {
        Err(denied())
    }

    async fn file_exists(&self, _path: &str) -> EnvResult<bool> {
        Err(denied())
    }

    async fn list_directory(&self, _path: &str, _depth: Option<usize>) -> EnvResult<Vec<DirEntry>> {
        Err(denied())
    }

    async fn grep(
        &self,
        _pattern: &str,
        _path: &str,
        _options: &GrepOptions,
    ) -> EnvResult<Vec<String>> {
        Err(denied())
    }

    async fn glob(&self, _pattern: &str, _path: Option<&str>) -> EnvResult<Vec<String>> {
        Err(denied())
    }

    async fn exec(&self, _request: ExecRequest<'_>) -> EnvResult<ExecOutcome> {
        Err(denied())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every operation denies with the tools-only reason, and the prompt
    /// accessors return their display values.
    #[tokio::test]
    async fn every_operation_denies_with_the_tools_only_reason() {
        let env = DenyExecEnvironment::new("/supervisor");

        assert_eq!(env.working_directory(), "/supervisor");
        assert_eq!(env.platform(), "unknown");
        assert_eq!(env.os_version(), "unavailable");

        assert!(env.read_file_bytes("/x").await.is_err());
        assert!(env.read_file_text("/x").await.is_err());
        assert!(env.read_file("/x", None, None).await.is_err());
        assert!(env.write_file("/x", "y").await.is_err());
        assert!(env.write_existing_file("/x", "y").await.is_err());
        assert!(env.rename_file("/x", "/y").await.is_err());
        assert!(env.delete_file("/x").await.is_err());
        assert!(env.file_exists("/x").await.is_err());
        assert!(env.list_directory("/x", None).await.is_err());
        assert!(env.grep("p", "/x", &GrepOptions::default()).await.is_err());
        assert!(env.glob("*", Some("/x")).await.is_err());
        assert!(env.exec(ExecRequest::new("true")).await.is_err());

        let error = env.read_file_bytes("/x").await.unwrap_err();
        assert_eq!(error.kind, EnvironmentErrorKind::Unsupported);
        assert!(error.message.contains("tools-only"));
    }
}
