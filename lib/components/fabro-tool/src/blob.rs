//! `fabro_blob`: read a demoted value back with deterministic paging
//! (fabro-d774, the fabro-owned half). Petri offloads oversized stage
//! values to the run's blob table and leaves a bare `blob://sha256/<hex>`
//! reference with no size, line count, or materialized path; this tool
//! answers both from the server-side blob read: the info call sizes the
//! paging, offset/limit pages the content by lines.

use std::sync::Arc;

use fabro_types::{BlobHash, RunId, parse_blob_ref};
use schemars::JsonSchema;
use serde::Serialize;

use super::common::{FabroToolBackend, ToolError, ToolResult};

/// How many lines one page returns when the caller does not say.
const DEFAULT_PAGE_LINES: u32 = 200;
/// The page ceiling regardless of the line budget, so one call cannot
/// drag a whole multi-megabyte blob into a tool result.
const PAGE_MAX_BYTES: usize = 64 * 1024;

#[derive(Debug, serde::Deserialize, JsonSchema)]
pub struct FabroBlobParams {
    /// The blob reference (`blob://sha256/<hex>` or the bare hex digest).
    pub r#ref:  String,
    /// First line to return, 0-based (default 0).
    pub offset: Option<u32>,
    /// How many lines to return (default 200, at most 1000).
    pub limit:  Option<u32>,
}

#[derive(Debug)]
pub struct ValidatedBlob {
    pub hash:   BlobHash,
    pub offset: u32,
    pub limit:  u32,
}

impl TryFrom<FabroBlobParams> for ValidatedBlob {
    type Error = ToolError;

    fn try_from(params: FabroBlobParams) -> Result<Self, Self::Error> {
        let hash = parse_blob_ref(params.r#ref.trim()).ok_or_else(|| {
            ToolError::message(
                "ref must be a blob reference (blob://sha256/<hex> or the bare digest)",
            )
        })?;
        let offset = params.offset.unwrap_or(0);
        let limit = params.limit.unwrap_or(DEFAULT_PAGE_LINES).clamp(1, 1000);
        Ok(Self {
            hash,
            offset,
            limit,
        })
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct BlobResult {
    pub sha256:         String,
    /// Total size of the blob in bytes.
    pub bytes:          u64,
    /// Total lines of the decoded text.
    pub lines:          u64,
    /// The page's first line, 0-based.
    pub offset:         u32,
    /// How many lines this page carries.
    pub returned_lines: u32,
    /// The page's text.
    pub page:           String,
}

/// The blob's line count: newline occurrences, plus one when the text
/// does not end on a newline (the trailing partial line).
#[must_use]
pub fn count_lines(text: &str) -> u64 {
    if text.is_empty() {
        return 0;
    }
    let newlines = text.bytes().filter(|byte| *byte == b'\n').count();
    if text.ends_with('\n') {
        newlines as u64
    } else {
        newlines as u64 + 1
    }
}

/// Read one page of the run's blob. The backend returns the whole blob;
/// paging happens here, deterministically, by lines.
pub async fn blob_page(
    backend: &Arc<dyn FabroToolBackend>,
    run_id: &RunId,
    validated: &ValidatedBlob,
) -> ToolResult<BlobResult> {
    let bytes = backend
        .read_run_blob(run_id, &validated.hash)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))?
        .ok_or_else(|| ToolError::message("blob not found for this run"))?;
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let total = u64::try_from(lines.len()).unwrap_or(u64::MAX);
    let start = usize::try_from(u64::from(validated.offset).min(total)).unwrap_or(usize::MAX);
    let limit = usize::try_from(u64::from(validated.limit).min(total)).unwrap_or(usize::MAX);
    let end = start.saturating_add(limit).min(lines.len());
    let mut page: Vec<&str> = lines[start..end].to_vec();
    let mut page_bytes: usize = page.iter().map(|line| line.len() + 1).sum();
    while page_bytes > PAGE_MAX_BYTES && page.len() > 1 {
        page_bytes -= page.pop().map_or(0, |line| line.len() + 1);
    }
    let returned = u32::try_from(page.len()).unwrap_or(u32::MAX);
    Ok(BlobResult {
        sha256:         validated.hash.to_string(),
        bytes:          u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        lines:          total,
        offset:         u32::try_from(start).unwrap_or(u32::MAX),
        returned_lines: returned,
        page:           page.join("\n"),
    })
}

/// The compact text form the agent sees.
#[must_use]
pub fn blob_page_text(result: &BlobResult) -> String {
    let short = &result.sha256[..result.sha256.len().min(12)];
    format!(
        "blob {short}: {} bytes, {} lines (page {}..{} of {})\n{}",
        result.bytes,
        result.lines,
        result.offset,
        result.offset + result.returned_lines,
        result.lines,
        result.page,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_counting_matches_text_and_trailing_partial_lines() {
        assert_eq!(count_lines(""), 0);
        assert_eq!(count_lines("one"), 1);
        assert_eq!(count_lines("one\ntwo\n"), 2);
        assert_eq!(count_lines("one\ntwo"), 2);
    }

    #[test]
    fn validation_rejects_non_references() {
        let error = FabroBlobParams {
            r#ref:  "not-a-ref".to_owned(),
            offset: None,
            limit:  None,
        };
        let error = ValidatedBlob::try_from(error).expect_err("malformed refs refuse");
        assert!(
            error.to_string().contains("blob reference"),
            "the error names the shape: {error}"
        );
    }
}
