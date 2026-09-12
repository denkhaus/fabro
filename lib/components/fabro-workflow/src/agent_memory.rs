//! Project memory files for agent and prompt stages.
//!
//! Pebble loads memory from explicit paths and looks in no conventional
//! location. Fabro supplies the convention: each coding harness reads the
//! instruction files its vendor's own agent reads, found in the sandbox
//! working directory. The loading itself — the 32 KB budget, duplicate text
//! skipped, the file that crosses the budget cut and marked — is pebble's
//! [`ProjectMemory`], the same loader an agent stage runs over
//! `with_memory_files`, so a prompt stage reads by the agent's rules.

use fabro_sandbox::RunSandbox;
use fabro_types::AgentProfileKind;
use pebble_coding_agent::environment::Environment;
use pebble_coding_agent::{InterruptReason, ProjectMemory};
use tokio_util::sync::CancellationToken;

use crate::error::Error;

/// The instruction filenames a harness reads, in load order.
#[must_use]
pub fn memory_filenames(profile_kind: AgentProfileKind) -> &'static [&'static str] {
    // `AgentProfileKind` is non-exhaustive: a profile pebble adds later reads
    // the shared AGENTS.md until fabro says otherwise.
    match profile_kind {
        AgentProfileKind::Anthropic | AgentProfileKind::Claude5 => &["AGENTS.md", "CLAUDE.md"],
        AgentProfileKind::OpenAi | AgentProfileKind::Gpt56 | AgentProfileKind::Gpt6 => {
            &["AGENTS.md", ".codex/instructions.md"]
        }
        AgentProfileKind::Gemini => &["AGENTS.md", "GEMINI.md"],
        // Kimi Code reads only AGENTS.md; it has no vendor-specific
        // instruction filename of its own.
        AgentProfileKind::Kimi | _ => &["AGENTS.md"],
    }
}

/// The candidate memory paths for a harness working in `working_dir`.
///
/// Missing and empty files are skipped by the loader, so every candidate can
/// be named without checking the sandbox first.
#[must_use]
pub fn memory_paths(working_dir: &str, profile_kind: AgentProfileKind) -> Vec<String> {
    let root = working_dir.trim_end_matches('/');
    memory_filenames(profile_kind)
        .iter()
        .map(|filename| format!("{root}/{filename}"))
        .collect()
}

/// The memory text a prompt stage inlines into its system prompt: every
/// candidate file's contents, loaded by pebble's [`ProjectMemory`] rules.
///
/// # Errors
///
/// Returns [`Error::Cancelled`] when `cancel` fires around a read.
pub async fn load_memory_text(
    sandbox: &RunSandbox,
    working_dir: &str,
    profile_kind: AgentProfileKind,
    cancel: &CancellationToken,
) -> Result<Option<String>, Error> {
    let paths = memory_paths(working_dir, profile_kind);
    let memory = ProjectMemory::load(sandbox as &dyn Environment, &paths, cancel)
        .await
        .map_err(|error| match error {
            pebble_coding_agent::Error::Interrupted(InterruptReason::Cancelled) => Error::Cancelled,
            other => Error::handler_with_source("Failed to load project memory", other),
        })?;
    Ok((!memory.is_empty()).then(|| memory.text()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_paths_follow_the_profile() {
        assert_eq!(memory_paths("/work/", AgentProfileKind::Claude5), vec![
            "/work/AGENTS.md".to_string(),
            "/work/CLAUDE.md".to_string()
        ]);
        assert_eq!(memory_paths("/work", AgentProfileKind::Gpt56), vec![
            "/work/AGENTS.md".to_string(),
            "/work/.codex/instructions.md".to_string()
        ]);
        assert_eq!(memory_paths("/work", AgentProfileKind::Kimi), vec![
            "/work/AGENTS.md".to_string()
        ]);
    }

    #[tokio::test]
    async fn memory_text_dedupes_and_skips_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("AGENTS.md"), "shared")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("CLAUDE.md"), "shared")
            .await
            .unwrap();
        let sandbox = fabro_sandbox::local_sandbox(dir.path().to_path_buf())
            .await
            .unwrap();

        let text = load_memory_text(
            &sandbox,
            sandbox.working_directory(),
            AgentProfileKind::Anthropic,
            &CancellationToken::new(),
        )
        .await
        .unwrap();

        assert_eq!(text.as_deref(), Some("shared"));
        let none = load_memory_text(
            &sandbox,
            sandbox.working_directory(),
            AgentProfileKind::Gemini,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(none.as_deref(), Some("shared"));
    }
}
