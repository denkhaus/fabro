//! The read-only seeds tracker search (`fabro_seeds_search`, fabro-d6de).
//!
//! The Ask-Fabro analyst recommends filings but could not see the live
//! tracker: the seeds read API exists, yet no agent-facing tool exposed
//! it, so recommendations duplicated open seeds. This tool searches the
//! tracker read-only — the backend lists the rows the server-side filters
//! select (status, assignee, label; the API has no keyword parameter), and
//! the tool narrows by space-separated AND-strict terms over id, title,
//! and labels before limiting.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::common::{FabroToolBackend, ToolError, ToolResult};

pub(crate) const DEFAULT_SEEDS_LIMIT: usize = 25;
pub(crate) const MAX_SEEDS_LIMIT: usize = 100;
/// The API caps a page at 2000; the tool always asks for the cap so the
/// keyword narrowing sees the tracker, not a page.
pub(crate) const SEEDS_PAGE_LIMIT: u64 = 2000;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FabroSeedsSearchParams {
    /// Space-separated terms; a row matches when EVERY term appears in its
    /// id, title, or labels (case-insensitive). Optional.
    pub query:    Option<String>,
    /// Filter by status: `open`, `in_progress`, or `closed`. Optional.
    pub status:   Option<String>,
    /// Filter by assignee; `none` selects unassigned. Optional.
    pub assignee: Option<String>,
    /// Filter by label. Optional.
    pub label:    Option<String>,
    /// How many rows to return at most (default 25, at most 100). The
    /// `matched` count reports the full hit set before this limit.
    pub limit:    Option<usize>,
}

/// A validated tracker search: the server-side filters plus the local
/// narrowing configuration.
#[derive(Debug)]
pub struct ValidatedSeedsSearchParams {
    pub terms:    Vec<String>,
    pub status:   Option<String>,
    pub assignee: Option<String>,
    pub label:    Option<String>,
    pub limit:    usize,
}

impl TryFrom<FabroSeedsSearchParams> for ValidatedSeedsSearchParams {
    type Error = ToolError;

    fn try_from(params: FabroSeedsSearchParams) -> Result<Self, Self::Error> {
        let terms = params
            .query
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_lowercase)
            .collect::<Vec<_>>();
        for filter in [&params.status, &params.assignee, &params.label] {
            if let Some(value) = filter.as_deref() {
                if value.trim().is_empty() {
                    return Err(ToolError::message(
                        "seeds filters must not be blank; omit the filter instead",
                    ));
                }
            }
        }
        let status = match params.status.as_deref() {
            None | Some("open" | "in_progress" | "closed") => params.status,
            Some(other) => {
                return Err(ToolError::message(format!(
                    "status must be one of open, in_progress, closed (got `{other}`)"
                )));
            }
        };
        let limit = params
            .limit
            .unwrap_or(DEFAULT_SEEDS_LIMIT)
            .clamp(1, MAX_SEEDS_LIMIT);
        Ok(Self {
            terms,
            status,
            assignee: params.assignee.map(|value| value.trim().to_string()),
            label: params.label.map(|value| value.trim().to_string()),
            limit,
        })
    }
}

/// One tracker row as the tool reports it — the API summary's stable core.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SeedsRow {
    pub id:        String,
    pub title:     String,
    pub status:    Option<String>,
    pub seed_type: Option<String>,
    pub assignee:  Option<String>,
    pub labels:    Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SeedsSearchResult {
    /// Rows matching every filter and term, before the limit.
    pub matched:   usize,
    /// The rows returned, at most the requested limit.
    pub rows:      Vec<SeedsRow>,
    /// Whether `matched` exceeds the returned rows.
    pub truncated: bool,
}

/// Narrow tracker rows by the validated terms and limit. The server-side
/// filters (status, assignee, label) are the backend's business; this is
/// the shared, testable narrowing.
#[must_use]
pub fn filter_seed_rows(
    rows: Vec<SeedsRow>,
    params: &ValidatedSeedsSearchParams,
) -> SeedsSearchResult {
    let mut hit: Vec<SeedsRow> = rows
        .into_iter()
        .filter(|row| {
            let hay = {
                let mut text = format!("{} {} {}", row.id, row.title, row.labels.join(" "));
                if let Some(status) = &row.status {
                    text.push(' ');
                    text.push_str(status);
                }
                text.to_lowercase()
            };
            params.terms.iter().all(|term| hay.contains(term))
        })
        .collect();
    let matched = hit.len();
    hit.truncate(params.limit);
    let truncated = matched > hit.len();
    SeedsSearchResult {
        matched,
        rows: hit,
        truncated,
    }
}

/// Apply the tracker-level filters (status, assignee with `none` =
/// unassigned, label) over raw run-context rows — the same semantics the
/// removed server-side listing served (fabro-d6de rework).
pub fn apply_tracker_filters(
    rows: Vec<SeedsRow>,
    status: Option<&str>,
    assignee: Option<&str>,
    label: Option<&str>,
) -> Vec<SeedsRow> {
    rows.into_iter()
        .filter(|row| {
            let status_ok = status
                .map(|wanted| row.status.as_deref().is_some_and(|have| have.eq_ignore_ascii_case(wanted)))
                .unwrap_or(true);
            let assignee_ok = assignee
                .map(|wanted| {
                    if wanted.eq_ignore_ascii_case("none") {
                        row.assignee.as_deref().is_none_or(str::is_empty)
                    } else {
                        row.assignee.as_deref().is_some_and(|have| have == wanted)
                    }
                })
                .unwrap_or(true);
            let label_ok = label
                .map(|wanted| row.labels.iter().any(|have| have == wanted))
                .unwrap_or(true);
            status_ok && assignee_ok && label_ok
        })
        .collect()
}

pub async fn seeds_search(
    backend: std::sync::Arc<dyn FabroToolBackend>,
    params: ValidatedSeedsSearchParams,
) -> ToolResult<SeedsSearchResult> {
    let rows = backend
        .list_seed_rows(
            params.status.as_deref(),
            params.assignee.as_deref(),
            params.label.as_deref(),
        )
        .await
        .map_err(|err| ToolError::from_anyhow(&err))?;
    Ok(filter_seed_rows(rows, &params))
}

pub fn seeds_search_text(result: &SeedsSearchResult) -> String {
    format!(
        "matched {} tracker seed(s), returning {}",
        result.matched,
        result.rows.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, title: &str, status: Option<&str>, labels: &[&str]) -> SeedsRow {
        SeedsRow {
            id:        id.to_string(),
            title:     title.to_string(),
            status:    status.map(str::to_string),
            seed_type: Some("task".to_string()),
            assignee:  Some("loop".to_string()),
            labels:    labels.iter().map(ToString::to_string).collect(),
        }
    }

    fn params(query: Option<&str>, limit: usize) -> ValidatedSeedsSearchParams {
        ValidatedSeedsSearchParams {
            terms: query
                .unwrap_or_default()
                .split_whitespace()
                .map(str::to_lowercase)
                .collect(),
            status: None,
            assignee: None,
            label: None,
            limit,
        }
    }

    #[test]
    fn terms_match_and_strict_over_id_title_and_labels() {
        let rows = vec![
            row("fabro-1", "Lint duplication in the gate", Some("open"), &[
                "loop-assets",
            ]),
            row("fabro-2", "Planner schema parity", Some("open"), &[
                "parity",
            ]),
            row("fabro-3", "lint AND schema both", Some("closed"), &[]),
        ];
        let result = filter_seed_rows(rows, &params(Some("lint schema"), 25));
        assert_eq!(result.matched, 1);
        assert_eq!(result.rows[0].id, "fabro-3");
    }

    #[test]
    fn no_terms_return_every_row_and_limit_truncates() {
        let rows: Vec<_> = (0..40)
            .map(|i| row(&format!("fabro-{i}"), "title", Some("open"), &[]))
            .collect();
        let result = filter_seed_rows(rows, &params(None, 25));
        assert_eq!(result.matched, 40);
        assert_eq!(result.rows.len(), 25);
        assert!(result.truncated);
    }

    #[test]
    fn status_terms_match_the_row_status_too() {
        let rows = vec![
            row("fabro-1", "first thing", Some("open"), &[]),
            row("fabro-2", "other", Some("closed"), &[]),
        ];
        let result = filter_seed_rows(rows, &params(Some("closed"), 25));
        assert_eq!(result.rows[0].id, "fabro-2");
    }

    #[test]
    fn validate_rejects_unknown_status_and_blank_filters() {
        assert!(
            ValidatedSeedsSearchParams::try_from(FabroSeedsSearchParams {
                query:    None,
                status:   Some("bogus".into()),
                assignee: None,
                label:    None,
                limit:    None,
            })
            .is_err()
        );
        assert!(
            ValidatedSeedsSearchParams::try_from(FabroSeedsSearchParams {
                query:    None,
                status:   None,
                assignee: Some("  ".into()),
                label:    None,
                limit:    None,
            })
            .is_err()
        );
    }

    #[test]
    fn validate_clamps_the_limit() {
        let validated = ValidatedSeedsSearchParams::try_from(FabroSeedsSearchParams {
            query:    None,
            status:   None,
            assignee: None,
            label:    None,
            limit:    Some(500),
        })
        .unwrap();
        assert_eq!(validated.limit, MAX_SEEDS_LIMIT);
    }
}
