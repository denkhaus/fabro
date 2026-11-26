//! `fabro_run_files` and `fabro_run_file` (ADR-0046 phase 2,
//! denkhaus/work#1680): the supervisor's read doors into a NAMED run's
//! sandbox files. The tool always takes an explicit run selector — a
//! runless session has no ambient anchor, and a foreign read must name
//! its target. The server mediates every read through `sandbox_access`
//! with the sensitive-path denylist and size/UTF-8 bounds; the tool
//! renders what the bounded read returned.

use std::fmt::Write as _;
use std::sync::Arc;

use fabro_types::RunId;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::common::{FabroToolBackend, ToolError, ToolResult};

/// The deepest directory walk one `fabro_run_files` call may ask for.
/// The server clamps the same bound; the tool clamps first so the
/// model's call shape stays honest.
pub const RUN_FILES_MAX_DEPTH: u32 = 4;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FabroRunFilesParams {
    /// The run whose sandbox to list — a run id. Required: this tool is
    /// the runless supervisor's door and never guesses an ambient run.
    pub run_id: String,
    /// The directory to list, workspace-relative (default: the run's
    /// working directory root).
    pub path:   Option<String>,
    /// How deep to walk (1-4; default 1 — the immediate children).
    pub depth:  Option<u32>,
}

impl FabroRunFilesParams {
    /// Validate and normalize: a blank run selector or a blank path is
    /// refused with the teaching error; the depth clamps to 1-4.
    pub fn validated(self) -> ToolResult<ValidatedRunFiles> {
        let run_id = parse_run_selector(&self.run_id)?;
        let path = match self.path.as_deref().map(str::trim) {
            None | Some("") => ".".to_string(),
            Some(path) => path.to_string(),
        };
        let depth = self.depth.unwrap_or(1).clamp(1, RUN_FILES_MAX_DEPTH);
        Ok(ValidatedRunFiles {
            run_id,
            path,
            depth,
        })
    }
}

#[derive(Debug)]
pub struct ValidatedRunFiles {
    pub run_id: RunId,
    pub path:   String,
    pub depth:  u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FabroRunFileParams {
    /// The run whose sandbox to read — a run id. Required: this tool is
    /// the runless supervisor's door and never guesses an ambient run.
    pub run_id: String,
    /// The file to read, workspace-relative or sandbox-absolute.
    pub path:   String,
}

impl FabroRunFileParams {
    /// Validate and normalize: blank selectors and blank paths are
    /// refused with the teaching error.
    pub fn validated(self) -> ToolResult<ValidatedRunFile> {
        let run_id = parse_run_selector(&self.run_id)?;
        let path = self.path.trim().to_string();
        if path.is_empty() {
            return Err(ToolError::message(
                "path must not be blank — a workspace-relative or sandbox-absolute file path",
            ));
        }
        Ok(ValidatedRunFile { run_id, path })
    }
}

#[derive(Debug)]
pub struct ValidatedRunFile {
    pub run_id: RunId,
    pub path:   String,
}

/// Parse the run selector into a [`RunId`] with the teaching refusal.
fn parse_run_selector(selector: &str) -> ToolResult<RunId> {
    let trimmed = selector.trim();
    if trimmed.is_empty() {
        return Err(ToolError::message(
            "run_id must not be blank — the id of the run whose sandbox to read",
        ));
    }
    trimmed
        .parse::<RunId>()
        .map_err(|err| ToolError::message(format!("run_id must be a run id: {err}")))
}

/// One directory entry of a sandbox listing, as the server reported it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunFileEntry {
    /// The entry's path, relative to the listed directory.
    pub name:   String,
    /// Whether the entry is a directory.
    pub is_dir: bool,
    /// The file's size in bytes; `None` for directories.
    pub size:   Option<i64>,
}

/// One bounded text read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunFileContent {
    /// The sandbox-absolute path the content was read from.
    pub path:    String,
    /// The file's UTF-8 text.
    pub content: String,
    /// The file's size in bytes.
    pub bytes:   i64,
}

/// One page of a sandbox listing.
#[derive(Debug, Serialize)]
pub struct RunFilesResult {
    pub run_id:  RunId,
    pub path:    String,
    pub entries: Vec<RunFileEntry>,
}

pub async fn list_run_files(
    backend: Arc<dyn FabroToolBackend>,
    params: ValidatedRunFiles,
) -> ToolResult<RunFilesResult> {
    let entries = backend
        .list_run_files(&params.run_id, &params.path, params.depth)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))?;
    Ok(RunFilesResult {
        run_id: params.run_id,
        path: params.path,
        entries,
    })
}

pub async fn read_run_file(
    backend: Arc<dyn FabroToolBackend>,
    params: ValidatedRunFile,
) -> ToolResult<RunFileContent> {
    backend
        .read_run_file(&params.run_id, &params.path)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))
}

/// The listing rendering: the count, then one line per entry with its
/// kind and size; the read of one file rides `fabro_run_file`.
pub fn run_files_text(result: &RunFilesResult) -> String {
    if result.entries.is_empty() {
        return format!("no entries under {} in run {}", result.path, result.run_id);
    }
    let mut listed = format!(
        "{} entr{} under {} in run {}:",
        result.entries.len(),
        if result.entries.len() == 1 {
            "y"
        } else {
            "ies"
        },
        result.path,
        result.run_id
    );
    for entry in &result.entries {
        let kind = if entry.is_dir {
            "dir".to_string()
        } else {
            match entry.size {
                Some(size) => format!("{size} bytes"),
                None => "file".to_string(),
            }
        };
        let _ = write!(listed, "\n{} — {}", entry.name, kind);
    }
    listed
}

/// The read rendering: the content is the payload; the header names the
/// path and size so the model can cite what it read.
pub fn run_file_text(content: &RunFileContent) -> String {
    format!(
        "{} ({} bytes):\n{}",
        content.path, content.bytes, content.content
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files_params(run_id: &str, path: Option<&str>, depth: Option<u32>) -> FabroRunFilesParams {
        FabroRunFilesParams {
            run_id: run_id.to_string(),
            path: path.map(str::to_string),
            depth,
        }
    }

    fn a_run_id() -> String {
        RunId::new().to_string()
    }

    #[test]
    fn a_normal_listing_validates_with_defaults() {
        let validated = files_params(&a_run_id(), None, None)
            .validated()
            .expect("a normal listing validates");
        assert_eq!(validated.path, ".");
        assert_eq!(validated.depth, 1);
    }

    #[test]
    fn the_depth_clamps_to_the_tool_bound() {
        let validated = files_params(&a_run_id(), Some("src"), Some(50))
            .validated()
            .expect("a deep listing clamps");
        assert_eq!(validated.depth, RUN_FILES_MAX_DEPTH);
    }

    #[test]
    fn a_blank_run_selector_is_refused_with_the_teaching_error() {
        let error = files_params("   ", None, None)
            .validated()
            .expect_err("a blank run selector must fail");
        assert!(
            error.as_str().contains("run_id must not be blank"),
            "the refusal teaches the selector: {}",
            error.as_str()
        );
    }

    #[test]
    fn a_non_run_id_selector_is_refused() {
        let error = files_params("not-a-run-id", None, None)
            .validated()
            .expect_err("a non-id selector must fail");
        assert!(
            error.as_str().contains("run_id must be a run id"),
            "the refusal names the shape: {}",
            error.as_str()
        );
    }

    #[test]
    fn a_blank_read_path_is_refused() {
        let error = FabroRunFileParams {
            run_id: a_run_id(),
            path:   "   ".to_string(),
        }
        .validated()
        .expect_err("a blank read path must fail");
        assert!(
            error.as_str().contains("path must not be blank"),
            "the refusal teaches the path: {}",
            error.as_str()
        );
    }

    #[test]
    fn the_listing_text_renders_kinds_and_sizes() {
        let run_id: RunId = a_run_id().parse().expect("a generated run id");
        let result = RunFilesResult {
            run_id,
            path: "src".to_string(),
            entries: vec![
                RunFileEntry {
                    name:   "lib".to_string(),
                    is_dir: true,
                    size:   None,
                },
                RunFileEntry {
                    name:   "main.rs".to_string(),
                    is_dir: false,
                    size:   Some(2048),
                },
            ],
        };
        let text = run_files_text(&result);
        assert!(text.contains("2 entries under src"), "{text}");
        assert!(text.contains("lib — dir"), "{text}");
        assert!(text.contains("main.rs — 2048 bytes"), "{text}");
    }

    #[test]
    fn the_empty_listing_text_names_the_directory() {
        let run_id: RunId = a_run_id().parse().expect("a generated run id");
        let result = RunFilesResult {
            run_id,
            path: "empty".to_string(),
            entries: Vec::new(),
        };
        assert_eq!(
            run_files_text(&result),
            format!("no entries under empty in run {run_id}")
        );
    }

    #[test]
    fn the_read_text_carries_path_size_and_content() {
        let content = RunFileContent {
            path:    "/workspace/src/main.rs".to_string(),
            content: "fn main() {}".to_string(),
            bytes:   12,
        };
        let text = run_file_text(&content);
        assert!(
            text.starts_with("/workspace/src/main.rs (12 bytes):"),
            "{text}"
        );
        assert!(text.contains("fn main() {}"), "{text}");
    }
}
