//! Project memory files for agent and prompt stages.
//!
//! Pebble loads memory from explicit paths and looks in no conventional
//! location. Fabro supplies the convention: each coding harness reads the
//! instruction files its vendor's own agent reads, found in the sandbox
//! working directory.

use fabro_sandbox::RunSandbox;
use fabro_types::AgentProfileKind;
use tokio_util::sync::CancellationToken;

use crate::error::Error;

/// The most memory text a stage loads into its system prompt.
pub const MEMORY_BUDGET_BYTES: usize = 32_768;

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
/// candidate file's contents, deduplicated and cut to the budget.
///
/// # Errors
///
/// Returns [`Error::Cancelled`] when `cancel` fires between reads.
pub async fn load_memory_text(
    sandbox: &RunSandbox,
    working_dir: &str,
    profile_kind: AgentProfileKind,
    cancel: &CancellationToken,
) -> Result<Option<String>, Error> {
    let mut documents = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut budget = MEMORY_BUDGET_BYTES;
    for path in memory_paths(working_dir, profile_kind) {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let Ok(content) = sandbox.read_file_text(&path).await else {
            continue;
        };
        if content.is_empty() || !seen.insert(content.clone()) {
            continue;
        }
        if budget == 0 {
            break;
        }
        if content.len() <= budget {
            budget -= content.len();
            documents.push(content);
        } else {
            documents.push(truncate_to_budget(&content, budget));
            budget = 0;
        }
    }
    if documents.is_empty() {
        Ok(None)
    } else {
        Ok(Some(documents.join("\n\n")))
    }
}

fn truncate_to_budget(content: &str, budget: usize) -> String {
    const MARKER: &str = "[Project instructions truncated at 32KB]";
    if budget <= MARKER.len() {
        return MARKER[..budget].to_string();
    }
    let keep = content.floor_char_boundary(budget - MARKER.len());
    format!("{}{MARKER}", &content[..keep])
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
