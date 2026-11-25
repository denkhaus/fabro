//! `fabro_session_search` (ADR-0046 phase 2, denkhaus/work#1679): the
//! supervisor's search across every Ask Fabro session — run anchors,
//! titles, and recorded message text — the read door that finds "the
//! conversation someone had about run X" before
//! [`crate::FABRO_SESSION_READ_TOOL_NAME`] loads its transcript.

use std::fmt::Write as _;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::common::{FabroToolBackend, ToolError, ToolResult};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FabroSessionSearchParams {
    /// The keyword: a case-insensitive substring matched against the
    /// session's run anchor, its title, and the recorded user and
    /// assistant message text. A run id (or a fragment of one) finds the
    /// sessions anchored to that run.
    pub query:  String,
    /// Maximum sessions to return (1-100); also the page size.
    pub limit:  Option<u32>,
    /// 0-based results offset for paging past the first page.
    pub offset: Option<u32>,
}

impl FabroSessionSearchParams {
    /// Validate and normalize: a blank query matches nothing and is
    /// refused with the teaching error; the page bounds clamp.
    pub fn validated(self) -> ToolResult<ValidatedSessionSearch> {
        let query = self.query.trim().to_string();
        if query.is_empty() {
            return Err(ToolError::message(
                "query must not be blank — a run id, its fragment, a title word, or message text",
            ));
        }
        Ok(ValidatedSessionSearch {
            query,
            limit: self.limit.unwrap_or(20).clamp(1, 100),
            offset: self.offset.unwrap_or(0),
        })
    }
}

#[derive(Debug)]
pub struct ValidatedSessionSearch {
    pub query:  String,
    pub limit:  u32,
    pub offset: u32,
}

/// One page of session matches.
#[derive(Debug, Serialize)]
pub struct SessionSearchResult {
    pub sessions:    Vec<fabro_types::SessionSummary>,
    /// Another page exists past this one's offset.
    pub has_more:    bool,
    /// The offset the next page reads from, when `has_more`.
    pub next_offset: Option<u32>,
}

pub async fn search_sessions(
    backend: Arc<dyn FabroToolBackend>,
    params: ValidatedSessionSearch,
) -> ToolResult<SessionSearchResult> {
    let page = backend
        .search_sessions(&params.query, params.limit, params.offset)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))?;
    let next_offset = page.has_more.then(|| {
        let page_len = u32::try_from(page.data.len()).unwrap_or(u32::MAX);
        params.offset.saturating_add(page_len)
    });
    Ok(SessionSearchResult {
        sessions: page.data,
        has_more: page.has_more,
        next_offset,
    })
}

/// The search rendering: the count, then one line per session with its
/// run anchor, title, and last activity — the transcript read rides
/// `fabro_session_read`.
pub fn search_sessions_text(result: &SessionSearchResult) -> String {
    if result.sessions.is_empty() {
        return "no Ask Fabro session matches the query".to_string();
    }
    let mut listed = format!("{} matching Ask Fabro session(s):", result.sessions.len());
    for session in &result.sessions {
        let title = session.title.as_deref().unwrap_or("(untitled)");
        let anchor = session
            .run_id
            .map_or_else(|| "runless".to_string(), |run_id| run_id.to_string());
        let _ = write!(
            listed,
            "\n{} — run {} — \"{}\" — updated {}",
            session.id, anchor, title, session.updated_at
        );
    }
    if result.has_more {
        let _ = write!(
            listed,
            "\nMore matches exist; page with offset {}.",
            result.next_offset.unwrap_or_default()
        );
    }
    listed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(query: &str) -> FabroSessionSearchParams {
        FabroSessionSearchParams {
            query:  query.to_string(),
            limit:  None,
            offset: None,
        }
    }

    #[test]
    fn a_normal_query_validates_and_trims() {
        let validated = params("  deploy stall  ")
            .validated()
            .expect("a normal query validates");
        assert_eq!(validated.query, "deploy stall");
        assert_eq!(validated.limit, 20);
        assert_eq!(validated.offset, 0);
    }

    #[test]
    fn a_blank_query_is_refused_with_the_teaching_error() {
        let error = params("   ")
            .validated()
            .expect_err("a blank query must fail");
        let message = error.as_str();
        assert!(
            message.contains("run id"),
            "the refusal teaches what a query is: {message}"
        );
    }

    #[test]
    fn the_page_bounds_clamp() {
        let validated = FabroSessionSearchParams {
            query:  "run".to_string(),
            limit:  Some(5000),
            offset: Some(0),
        }
        .validated()
        .expect("bounds clamp instead of failing");
        assert_eq!(validated.limit, 100);
    }

    #[test]
    fn the_text_lists_matches_and_paging() {
        let result = SessionSearchResult {
            sessions:    Vec::new(),
            has_more:    false,
            next_offset: None,
        };
        assert_eq!(
            search_sessions_text(&result),
            "no Ask Fabro session matches the query"
        );
    }
}
