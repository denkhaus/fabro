//! Fork feature (fabro-43cf part 2): the platform docs corpus as a
//! read-only tool of the Ask-Fabro analyst.
//!
//! The analyst is attached to the TARGET run's sandbox, whose checkout
//! belongs to the target's repository — a run of any other repo (the
//! seeds line re-deriving engine semantics three times in one night was
//! the evidence) has no fabro docs to read. This module embeds the fabro
//! docs corpus in the server binary and serves it as one application
//! tool, so platform questions ("deadlock vs soft_stop parking",
//! "scheduler 3-strike", "interview mechanics") are answerable from the
//! source of truth on every Ask-Fabro session, read-only by policy.
//!
//! Corpus v1: `docs/agents`, `docs/internal`, `docs/lab` (markdown
//! only). `docs/public` (marketing + the generated API reference) and the
//! historical `docs/plans`/`brainstorms`/`superpowers` trees stay out.

use include_dir::{Dir, include_dir};
use pebble_coding_agent::tools::{RegisteredTool, ToolError, ToolSource};
use schemars::JsonSchema;

static AGENTS_DOCS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../../docs/agents");
static INTERNAL_DOCS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../../docs/internal");
static LAB_DOCS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../../docs/lab");

pub(crate) const FORK_ASK_DOCS_TOOL_NAME: &str = "fabro_docs";

/// One served document: at most this many characters; longer files are
/// truncated with a marker so the analyst knows to ask for the tail
/// through the workspace when the target checkout carries it.
const MAX_DOC_CHARS: usize = 64 * 1024;

fn corpus_roots() -> [(&'static Dir<'static>, &'static str); 3] {
    [
        (&AGENTS_DOCS, "agents"),
        (&INTERNAL_DOCS, "internal"),
        (&LAB_DOCS, "lab"),
    ]
}

/// Every markdown document in the corpus, as `root/path — # first title
/// line`, sorted by path.
pub(crate) fn corpus_index() -> Vec<String> {
    let mut entries = Vec::new();
    for (dir, prefix) in corpus_roots() {
        collect_md(dir, prefix, &mut entries);
    }
    entries.sort();
    entries
}

/// Walk `dir` recursively; `Dir::files()` alone lists only the immediate
/// directory, and the corpus nests (`lab/adr/`, `internal/product/`).
fn collect_md(dir: &Dir<'_>, prefix: &str, out: &mut Vec<String>) {
    for file in dir.files() {
        let path = file.path().to_string_lossy();
        if !path.ends_with(".md") {
            continue;
        }
        let title = first_title_line(file.contents())
            .map(|title| format!(" — {title}"))
            .unwrap_or_default();
        out.push(format!("{prefix}/{path}{title}"));
    }
    for subdirectory in dir.dirs() {
        let name = subdirectory.path().to_string_lossy().into_owned();
        collect_md(subdirectory, &format!("{prefix}/{name}"), out);
    }
}

fn first_title_line(contents: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(contents);
    text.lines()
        .map(str::trim)
        .find(|line| line.starts_with('#'))
        .map(|line| line.trim_start_matches('#').trim().to_string())
        .filter(|title| !title.is_empty())
}

/// Serve one document by corpus path, truncated at [`MAX_DOC_CHARS`].
pub(crate) fn read_doc(path: &str) -> Result<String, String> {
    let requested = path.trim();
    for (dir, prefix) in corpus_roots() {
        let Some(relative) = requested.strip_prefix(&format!("{prefix}/")) else {
            continue;
        };
        if let Some(file) = dir.get_file(relative) {
            let text = String::from_utf8_lossy(file.contents()).into_owned();
            return Ok(truncate_chars(text, MAX_DOC_CHARS));
        }
    }
    Err(format!(
        "unknown docs path `{requested}`; call {FORK_ASK_DOCS_TOOL_NAME} without a path for the          corpus index"
    ))
}

fn truncate_chars(text: String, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text;
    }
    let mut truncated = text.chars().take(max_chars).collect::<String>();
    truncated.push_str("\n…[truncated at 64k chars]");
    truncated
}

#[derive(Debug, serde::Deserialize, JsonSchema)]
pub(crate) struct FabroDocsParams {
    /// Corpus-relative docs path, e.g. `internal/events-strategy.md`.
    /// Omit to list the whole corpus index with each document's title.
    pub path: Option<String>,
}

/// The `fabro_docs` application tool for the Ask-Fabro analyst.
#[must_use]
pub(crate) fn docs_tool() -> RegisteredTool {
    RegisteredTool::function(
        FORK_ASK_DOCS_TOOL_NAME.to_string(),
        "Read the Fabro platform documentation corpus (agents, internal strategy, lab ADRs).          Omit `path` to list the corpus index with titles; pass a path like          `internal/events-strategy.md` to read one document."
            .to_string(),
        serde_json::to_value(schemars::schema_for!(FabroDocsParams))
            .expect("docs tool parameter schema should serialize"),
        move |_context, arguments| async move {
            let params: FabroDocsParams = serde_json::from_value(arguments)
                .map_err(|err| ToolError::execution(format!("invalid fabro_docs arguments: {err}")))?;
            match params.path.as_deref() {
                None | Some("") => Ok(corpus_index().join("\n")),
                Some(path) => read_doc(path).map_err(ToolError::execution),
            }
        },
    )
    .with_source(ToolSource::Application)
}

#[cfg(test)]
mod tests {
    use super::{FORK_ASK_DOCS_TOOL_NAME, corpus_index, read_doc};

    #[test]
    fn corpus_index_lists_strategy_docs_with_titles() {
        let index = corpus_index();
        assert!(
            index
                .iter()
                .any(|entry| entry.starts_with("internal/events-strategy.md")),
            "the events strategy doc must be in the embedded corpus"
        );
        assert!(
            index.iter().any(|entry| entry.starts_with("lab/adr/")),
            "the ADRs must be in the embedded corpus"
        );
        assert!(
            index.iter().all(|entry| !entry.starts_with("public/")),
            "the marketing/API tree must stay out of the corpus"
        );
    }

    #[test]
    fn read_doc_serves_content_and_rejects_unknown_paths() {
        let content = read_doc("internal/events-strategy.md").expect("known doc should read");
        assert!(!content.is_empty());

        let error = read_doc("internal/does-not-exist.md").expect_err("unknown path must fail");
        assert!(error.contains("corpus index"), "{error}");
        // Traversal-shaped paths cannot escape the embedded map.
        let error = read_doc("../../Cargo.toml").expect_err("escape attempt must fail");
        assert!(error.contains("unknown docs path"), "{error}");
    }

    #[test]
    fn docs_tool_name_is_stable() {
        assert_eq!(FORK_ASK_DOCS_TOOL_NAME, "fabro_docs");
    }
}
