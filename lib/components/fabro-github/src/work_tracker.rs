//! The GitHub adapter for the forge-agnostic tracker port (ADR-0032).
//!
//! One central private issue repository per deployment is the default
//! topology, but the adapter itself only knows "the repository the project
//! names". It implements [`fabro_work::TrackerProvider`] the way the ADR
//! spells it: items are issues, the claim projection is the run-id label
//! `fabro:run:<run_id>` (the label object must exist before it can be
//! attached — GitHub answers 422 otherwise), and `clear_claim` removes the
//! label from the issue and deletes the label object so only live claims
//! stay visible. A Forgejo adapter can implement the same port beside this
//! one; the port and the work store change nothing.

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fabro_http::{HeaderMap, HttpClient, HttpError, Method, RequestBuilder, StatusCode, header};
use fabro_work::{
    TrackerProvider, WorkItem, WorkItemDraft, WorkItemFilter, WorkItemStateFilter, WorkProject,
};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::Value;
use tokio::time::sleep;

/// Total attempts one API call makes while GitHub answers rate-limit
/// 403/429 (denkhaus/work#1364): the initial try plus two backed-off
/// retries, then the call fails as [`TrackerApiError::RateLimitExhausted`].
const RATE_LIMIT_MAX_ATTEMPTS: u32 = 3;

/// The longest single wait between rate-limit retries. A `Retry-After`
/// above this cap fails fast instead of parking the caller for the
/// quota window — the feed prefers an explicit stale state over a
/// multi-minute hang.
const RATE_LIMIT_MAX_WAIT: Duration = Duration::from_secs(20);

/// Typed GitHub API failure so callers can tell an exhausted rate-limit
/// budget from any other GitHub answer (denkhaus/work#1364): the
/// observation feed marks such reads stale instead of silently null.
#[derive(Debug, thiserror::Error)]
pub enum TrackerApiError {
    /// GitHub refused the call for rate-limit reasons and the retry
    /// budget is spent. The installation token's API budget is
    /// exhausted; `retry_after_secs` is the last `Retry-After` hint.
    #[error(
        "GitHub API rate limit exhausted (status {status}); last Retry-After hint: {retry_after_secs:?}"
    )]
    RateLimitExhausted {
        status:           u16,
        retry_after_secs: Option<u64>,
    },

    /// GitHub answered a non-success status that is not a rate limit.
    #[error("GitHub API {status}: {body}")]
    Status { status: u16, body: String },

    /// The request never reached a GitHub answer.
    #[error("GitHub API transport failure")]
    Transport(#[source] HttpError),

    /// GitHub answered a success status with unparseable JSON.
    #[error("GitHub API returned invalid JSON")]
    InvalidJson(#[source] serde_json::Error),
}

/// GitHub Issues in one repository.
pub struct GitHubTracker {
    client:   HttpClient,
    base_url: String,
    token:    String,
    repo:     String,
}

impl std::fmt::Debug for GitHubTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubTracker")
            .field("base_url", &self.base_url)
            .field("repo", &self.repo)
            .finish_non_exhaustive()
    }
}

impl GitHubTracker {
    #[must_use]
    pub fn new(client: HttpClient, base_url: &str, token: String, repo: String) -> Self {
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
            repo,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/repos/{}/{}", self.base_url, self.repo, path)
    }

    fn request(&self, method: Method, url: &str) -> RequestBuilder {
        self.client
            .request(method, url)
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            // GitHub refuses every API call without a User-Agent (403
            // "request forbidden by administrative rules"); httpmock never
            // notices, only the live API enforces it.
            .header("User-Agent", "fabro")
    }

    /// Whether a GitHub answer is a rate-limit refusal (denkhaus/work#1364):
    /// a 429, or a 403 whose body names the rate limit, whose headers
    /// carry a `Retry-After`, or whose `x-ratelimit-remaining` hit zero —
    /// the same classification the pull-request door uses
    /// (denkhaus/work#1332).
    fn is_rate_limited(status: StatusCode, headers: &HeaderMap, body_lower: &str) -> bool {
        if status == StatusCode::TOO_MANY_REQUESTS {
            return true;
        }
        if status != StatusCode::FORBIDDEN {
            return false;
        }
        body_lower.contains("rate limit")
            || headers.contains_key(header::RETRY_AFTER)
            || headers
                .get("x-ratelimit-remaining")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|remaining| remaining == "0")
    }

    fn retry_after_hint(headers: &HeaderMap) -> Option<u64> {
        headers
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok())
    }

    /// One GitHub API call with rate-limit hygiene (denkhaus/work#1364):
    /// a 403/429 rate-limit answer is retried with a backoff that
    /// honors `Retry-After` (capped at [`RATE_LIMIT_MAX_WAIT`]), and a
    /// budget that stays exhausted fails as
    /// [`TrackerApiError::RateLimitExhausted`] instead of a generic
    /// 403 — callers can distinguish "cannot ask" from "asked and got
    /// an answer".
    async fn issue_json(
        &self,
        method: Method,
        url: &str,
        body: Option<Value>,
    ) -> Result<Value, TrackerApiError> {
        let mut attempt = 1;
        loop {
            let mut request = self.request(method.clone(), url);
            if let Some(body) = &body {
                request = request.json(body);
            }
            let response = request.send().await.map_err(TrackerApiError::Transport)?;
            let status = response.status();
            let headers = response.headers().clone();
            let text = response.text().await.map_err(TrackerApiError::Transport)?;
            if status.is_success() {
                return if text.trim().is_empty() {
                    Ok(Value::Null)
                } else {
                    serde_json::from_str(&text).map_err(TrackerApiError::InvalidJson)
                };
            }
            if !Self::is_rate_limited(status, &headers, &text.to_ascii_lowercase()) {
                return Err(TrackerApiError::Status {
                    status: status.as_u16(),
                    body:   text,
                });
            }
            let retry_after_secs = Self::retry_after_hint(&headers);
            if attempt >= RATE_LIMIT_MAX_ATTEMPTS {
                return Err(TrackerApiError::RateLimitExhausted {
                    status: status.as_u16(),
                    retry_after_secs,
                });
            }
            let backoff = Duration::from_secs(1 << (attempt - 1));
            let wait = retry_after_secs
                .map_or(backoff, Duration::from_secs)
                .min(RATE_LIMIT_MAX_WAIT);
            tracing::warn!(
                attempt,
                wait_secs = wait.as_secs(),
                retry_after_secs = ?retry_after_secs,
                "github tracker rate limited, backing off"
            );
            sleep(wait).await;
            attempt += 1;
        }
    }

    /// The failure of an `issue_json` call whose caller tolerates one
    /// specific status (422 for an already-existing label, 404 for an
    /// already-gone one; `None` tolerates nothing): the tolerated
    /// status is success, every other failure keeps the
    /// operation-specific message.
    fn tolerate_status(
        err: TrackerApiError,
        tolerated: Option<u16>,
        what: &str,
    ) -> anyhow::Result<()> {
        match err {
            TrackerApiError::Status { status, .. } if Some(status) == tolerated => Ok(()),
            TrackerApiError::Status { status, body } => {
                anyhow::bail!("GitHub {what} {status}: {body}")
            }
            other => Err(other.into()),
        }
    }

    async fn list_issue_json(
        &self,
        _project: &WorkProject,
        issue_number: u64,
    ) -> anyhow::Result<Value> {
        let url = self.url(&format!("issues/{issue_number}"));
        Ok(self.issue_json(Method::GET, &url, None).await?)
    }

    async fn ensure_label(&self, name: &str) -> anyhow::Result<()> {
        let url = self.url("labels");
        let body = serde_json::json!({"name": name, "color": "ededed"});
        match self.issue_json(Method::POST, &url, Some(body)).await {
            Ok(_) => Ok(()),
            // 422: the label already exists — the ensure is done.
            Err(err) => Self::tolerate_status(err, Some(422), "label create"),
        }
    }

    async fn add_label(&self, issue_number: u64, name: &str) -> anyhow::Result<()> {
        let url = self.url(&format!("issues/{issue_number}/labels"));
        let body = serde_json::json!({"labels": [name]});
        match self.issue_json(Method::POST, &url, Some(body)).await {
            Ok(_) => Ok(()),
            Err(err) => Self::tolerate_status(err, None, "label attach"),
        }
    }

    async fn remove_label_from_issue(&self, issue_number: u64, name: &str) -> anyhow::Result<()> {
        let encoded = utf8_percent_encode(name, NON_ALPHANUMERIC).to_string();
        let url = self.url(&format!("issues/{issue_number}/labels/{encoded}"));
        match self.issue_json(Method::DELETE, &url, None).await {
            Ok(_) => Ok(()),
            // 404: the label is already off the issue — the detach is done.
            Err(err) => Self::tolerate_status(err, Some(404), "label detach"),
        }
    }

    async fn delete_label(&self, name: &str) -> anyhow::Result<()> {
        let encoded = utf8_percent_encode(name, NON_ALPHANUMERIC).to_string();
        let url = self.url(&format!("labels/{encoded}"));
        match self.issue_json(Method::DELETE, &url, None).await {
            Ok(_) => Ok(()),
            // 404: the label is already gone — the delete is done.
            Err(err) => Self::tolerate_status(err, Some(404), "label delete"),
        }
    }

    fn item_key(&self, issue_number: u64) -> String {
        format!("{}#{issue_number}", self.repo)
    }

    fn issue_number(item_key: &str) -> anyhow::Result<u64> {
        item_key
            .rsplit_once('#')
            .and_then(|(_, number)| number.parse().ok())
            .ok_or_else(|| anyhow::anyhow!("work item key must be owner/repo#number: {item_key}"))
    }

    /// Resolve a legacy `.seeds` id through its migration label: exactly
    /// one issue carrying `seeds:<id>` (the importer's parity report
    /// guarantees zero duplicates), any state — closed migrated items stay
    /// resolvable for the dedupe and basis lookups that cite them.
    async fn resolve_legacy_id(
        &self,
        project: &WorkProject,
        legacy_id: &str,
    ) -> anyhow::Result<Option<WorkItem>> {
        let label = if legacy_id.starts_with("seeds:") {
            legacy_id.to_string()
        } else {
            format!("seeds:{legacy_id}")
        };
        let (hits, _total) = self
            .search_items(
                project,
                &WorkItemFilter {
                    lane:  None,
                    limit: 5,
                    query: None,
                    label: None,
                    page:  1,
                    // Legacy ids resolve in ANY state (closed migrated
                    // items stay resolvable); the search arm ignores
                    // the state filter — `All` states the intent.
                    state: WorkItemStateFilter::All,
                },
                &format!("label:\"{label}\""),
            )
            .await?;
        match hits.len() {
            0 => Ok(None),
            1 => Ok(hits.into_iter().next()),
            n => anyhow::bail!(
                "legacy id {legacy_id} resolves to {n} issues labeled {label} — duplicate seeds labels (the importer's parity report guarantees zero)"
            ),
        }
    }

    /// The labels every list/search carries: the project label, the
    /// lane's label when a lane is filtered, and the raw label filter
    /// (denkhaus/work#1001).
    fn filter_labels(project: &WorkProject, filter: &WorkItemFilter) -> Vec<String> {
        let mut labels = vec![project.project_label.clone()];
        if let Some(lane) = filter.lane.as_deref() {
            let label = project
                .lane_labels
                .get(lane)
                .cloned()
                .unwrap_or_else(|| format!("lane:{lane}"));
            labels.push(label);
        }
        if let Some(label) = filter.label.as_deref() {
            labels.push(label.to_string());
        }
        labels
    }

    fn priority(labels: &[String]) -> i64 {
        labels
            .iter()
            .find_map(|label| {
                label
                    .strip_prefix("prio:p")
                    .and_then(|value| value.parse::<i64>().ok())
                    .or_else(|| {
                        label
                            .strip_prefix("priority:")
                            .and_then(|value| value.parse::<i64>().ok())
                    })
            })
            .unwrap_or(2)
    }

    fn json_item(&self, value: &Value) -> Option<WorkItem> {
        if value.get("pull_request").is_some() {
            return None;
        }
        let number = value.get("number").and_then(Value::as_u64)?;
        let labels = value
            .get("labels")
            .and_then(Value::as_array)
            .map(|labels| {
                labels
                    .iter()
                    .filter_map(|label| label.get("name").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Some(WorkItem {
            item_key: self.item_key(number),
            title: value
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            body: value
                .get("body")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            state: value
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or("open")
                .to_string(),
            priority: Self::priority(&labels),
            labels,
            url: value
                .get("html_url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            updated_at: value
                .get("updated_at")
                .and_then(Value::as_str)
                .and_then(|raw| raw.parse::<DateTime<Utc>>().ok()),
            created_at: value
                .get("created_at")
                .and_then(Value::as_str)
                .and_then(|raw| raw.parse::<DateTime<Utc>>().ok()),
            closed_at: value
                .get("closed_at")
                .and_then(Value::as_str)
                .and_then(|raw| raw.parse::<DateTime<Utc>>().ok()),
        })
    }

    async fn list_open_items(
        &self,
        project: &WorkProject,
        filter: &WorkItemFilter,
    ) -> anyhow::Result<Vec<WorkItem>> {
        let limit = filter.limit.clamp(1, 100);
        let page = filter.page.max(1);
        let encoded_labels = Self::filter_labels(project, filter)
            .iter()
            .map(|label| utf8_percent_encode(label, NON_ALPHANUMERIC).to_string())
            .collect::<Vec<_>>()
            .join(",");
        // The state filter rides GitHub's own `state` query param
        // (work#1476): open stays every established caller's answer;
        // `all`/`closed` extend the listing to the census reads (the
        // friction-score swap) without touching the default.
        let state = filter.state.to_string();
        let url = format!(
            "{}?state={state}&labels={encoded_labels}&per_page={limit}&page={page}",
            self.url("issues")
        );
        let value = self.issue_json(Method::GET, &url, None).await?;
        let values = value
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("GitHub issue list was not an array"))?;
        let mut items: Vec<WorkItem> = values
            .iter()
            .filter_map(|value| self.json_item(value))
            .collect();
        items.sort_by_key(|item| (item.priority, item.updated_at));
        Ok(items)
    }

    /// Title+body text search INCLUDING closed items (ADR-0032 phase 3:
    /// the pre-create duplicate check must see what already closed, not
    /// just what is open). Search ranks by relevance; the caller's
    /// judgment decides whether a hit covers the planned filing.
    /// Returns the page beside GitHub search's `total_count` — the
    /// exact number the qualifiers match (denkhaus/work#1296).
    async fn search_items(
        &self,
        project: &WorkProject,
        filter: &WorkItemFilter,
        query: &str,
    ) -> anyhow::Result<(Vec<WorkItem>, u64)> {
        let limit = filter.limit.clamp(1, 100);
        let page = filter.page.max(1);
        let trimmed = query.trim();
        if trimmed.is_empty() {
            anyhow::bail!("work item search query must not be blank");
        }
        let mut terms = vec![format!("repo:{}", self.repo), "is:issue".to_string()];
        terms.extend(Self::label_qualifiers(project, filter));
        terms.push(trimmed.to_string());
        let (value, total) = self.search_issues_json(&terms, limit, page).await?;
        let values = value
            .get("items")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("GitHub issue search returned no items array"))?;
        Ok((
            values
                .iter()
                .filter_map(|value| self.json_item(value))
                .collect(),
            total,
        ))
    }

    /// The search qualifiers every label-scoped search carries: the
    /// project label, the lane's label when a lane is filtered, and the
    /// raw label filter (denkhaus/work#1001) — one definition shared by
    /// the item search and the count search.
    fn label_qualifiers(project: &WorkProject, filter: &WorkItemFilter) -> Vec<String> {
        Self::filter_labels(project, filter)
            .into_iter()
            .map(|label| format!("label:\"{label}\""))
            .collect()
    }

    /// One GitHub search/issues call: the percent-encoded qualifier
    /// terms, `per_page`/`page`, answered as the response JSON beside
    /// its `total_count` (missing `total_count` reads as zero).
    async fn search_issues_json(
        &self,
        terms: &[String],
        limit: usize,
        page: usize,
    ) -> anyhow::Result<(Value, u64)> {
        let encoded = utf8_percent_encode(&terms.join(" "), NON_ALPHANUMERIC).to_string();
        let url = format!(
            "{}/search/issues?q={encoded}&per_page={limit}&page={page}",
            self.base_url
        );
        let value = self.issue_json(Method::GET, &url, None).await?;
        let total = value
            .get("total_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        Ok((value, total))
    }

    /// The list arm's count (denkhaus/work#1296): GitHub's issue
    /// LIST answers a page, never a total — the search API's
    /// `total_count` over the same qualifiers (repo, issue-only, the
    /// state's `is:` qualifier, the filter's labels) names it, one
    /// cheap request beside the page fetch.
    async fn count_open_items(
        &self,
        project: &WorkProject,
        filter: &WorkItemFilter,
    ) -> anyhow::Result<u64> {
        let mut terms = vec![format!("repo:{}", self.repo), "is:issue".to_string()];
        // The state's `is:` qualifier mirrors the list URL's `state=`
        // param (work#1476): open (the default), closed, or none for
        // `all`.
        match filter.state {
            WorkItemStateFilter::Open => terms.push("is:open".to_string()),
            WorkItemStateFilter::Closed => terms.push("is:closed".to_string()),
            WorkItemStateFilter::All => {}
        }
        terms.extend(Self::label_qualifiers(project, filter));
        let (_, total) = self.search_issues_json(&terms, 1, 1).await?;
        Ok(total)
    }
}

#[async_trait]
impl TrackerProvider for GitHubTracker {
    async fn list(
        &self,
        project: &WorkProject,
        filter: WorkItemFilter,
    ) -> anyhow::Result<Vec<WorkItem>> {
        match filter.query.as_deref() {
            Some(query) => Ok(self.search_items(project, &filter, query).await?.0),
            None => self.list_open_items(project, &filter).await,
        }
    }

    /// The exact-total form (denkhaus/work#1296): both arms answer the
    /// page beside the number of items the filter matches — the search
    /// arm from the same response's `total_count`, the open-list arm
    /// from a count-only search with identical qualifiers.
    async fn list_with_total(
        &self,
        project: &WorkProject,
        filter: WorkItemFilter,
    ) -> anyhow::Result<fabro_work::WorkItemPage> {
        if let Some(query) = filter.query.as_deref() {
            let (items, total) = self.search_items(project, &filter, query).await?;
            Ok(fabro_work::WorkItemPage {
                items,
                total: Some(total),
            })
        } else {
            let items = self.list_open_items(project, &filter).await?;
            let total = self.count_open_items(project, &filter).await?;
            Ok(fabro_work::WorkItemPage {
                items,
                total: Some(total),
            })
        }
    }

    async fn get(&self, project: &WorkProject, item_key: &str) -> anyhow::Result<Option<WorkItem>> {
        let number = match Self::issue_number(item_key) {
            Ok(number) => number,
            Err(parse_error) => {
                // Legacy-id resolver (ADR-0032 phase 4, grill amendment
                // 2026-10-21): the importer stamps `seeds:fabro-xxxx` on
                // every migrated issue — the label replaces the mapping
                // file, and this seam keeps every `fabro-xxxx` reference
                // in migrated bodies resolvable. The bare form namespaces
                // itself; an explicit `seeds:` prefix passes through.
                // Anything else stays the parse error (never a silent
                // miss).
                if item_key.starts_with("fabro-") || item_key.starts_with("seeds:") {
                    return self.resolve_legacy_id(project, item_key).await;
                }
                return Err(parse_error);
            }
        };
        let value = self.list_issue_json(project, number).await?;
        Ok(self.json_item(&value).filter(|item| {
            item.labels
                .iter()
                .any(|label| label == &project.project_label)
        }))
    }

    async fn create(
        &self,
        _project: &WorkProject,
        draft: WorkItemDraft,
    ) -> anyhow::Result<WorkItem> {
        for label in &draft.labels {
            self.ensure_label(label).await?;
        }
        let body = serde_json::json!({
            "title": draft.title,
            "body": draft.body,
            "labels": draft.labels,
        });
        let value = self
            .issue_json(Method::POST, &self.url("issues"), Some(body))
            .await?;
        self.json_item(&value)
            .ok_or_else(|| anyhow::anyhow!("GitHub did not return an issue"))
    }

    async fn update(
        &self,
        _project: &WorkProject,
        item_key: &str,
        title: Option<&str>,
        body: Option<&str>,
    ) -> anyhow::Result<WorkItem> {
        let number = Self::issue_number(item_key)?;
        let mut patch = serde_json::Map::new();
        if let Some(title) = title {
            patch.insert("title".to_string(), Value::String(title.to_string()));
        }
        if let Some(body) = body {
            patch.insert("body".to_string(), Value::String(body.to_string()));
        }
        if patch.is_empty() {
            anyhow::bail!("work item update needs a title or a body");
        }
        let value = self
            .issue_json(
                Method::PATCH,
                &self.url(&format!("issues/{number}")),
                Some(Value::Object(patch)),
            )
            .await?;
        self.json_item(&value)
            .ok_or_else(|| anyhow::anyhow!("GitHub did not return an issue"))
    }

    async fn set_lane(
        &self,
        project: &WorkProject,
        item_key: &str,
        lane: Option<&str>,
    ) -> anyhow::Result<WorkItem> {
        let number = Self::issue_number(item_key)?;
        // Read the item's current labels so the move is a swap, not an
        // accumulate: every lane label the project declares comes off,
        // then the new one goes on. A lane move must never leave two
        // lanes on one item (the queues would double-count it).
        let value = self.list_issue_json(project, number).await?;
        let item = self
            .json_item(&value)
            .filter(|item| {
                item.labels
                    .iter()
                    .any(|label| label == &project.project_label)
            })
            .ok_or_else(|| anyhow::anyhow!("work item {item_key} not found"))?;
        for label in project.lane_labels.values() {
            if item.labels.iter().any(|current| current == label) {
                self.remove_label_from_issue(number, label).await?;
            }
        }
        if let Some(lane) = lane {
            let label = project.lane_labels.get(lane).ok_or_else(|| {
                anyhow::anyhow!(
                    "work project {} declares no lane {lane:?}",
                    project.project_id
                )
            })?;
            self.ensure_label(label).await?;
            self.add_label(number, label).await?;
        }
        let value = self.list_issue_json(project, number).await?;
        self.json_item(&value)
            .ok_or_else(|| anyhow::anyhow!("GitHub did not return an issue"))
    }

    async fn close(
        &self,
        _project: &WorkProject,
        item_key: &str,
        reason: &str,
    ) -> anyhow::Result<()> {
        let number = Self::issue_number(item_key)?;
        if !reason.trim().is_empty() {
            self.comment(_project, item_key, reason).await?;
        }
        let body = serde_json::json!({"state": "closed", "state_reason": "completed"});
        match self
            .issue_json(
                Method::PATCH,
                &self.url(&format!("issues/{number}")),
                Some(body),
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(err) => Self::tolerate_status(err, None, "issue close"),
        }
    }

    async fn comment(
        &self,
        _project: &WorkProject,
        item_key: &str,
        body: &str,
    ) -> anyhow::Result<()> {
        let number = Self::issue_number(item_key)?;
        let body = serde_json::json!({"body": body});
        match self
            .issue_json(
                Method::POST,
                &self.url(&format!("issues/{number}/comments")),
                Some(body),
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(err) => Self::tolerate_status(err, None, "issue comment"),
        }
    }

    /// The item's comments, oldest first (ADR-0039 stage D part 2): the
    /// READ half of the comment door. GitHub returns the issue's comments
    /// chronologically with `per_page=100`; a comment without an author
    /// login is skipped rather than reported blank.
    async fn list_comments(
        &self,
        _project: &WorkProject,
        item_key: &str,
    ) -> anyhow::Result<Vec<fabro_work::WorkItemComment>> {
        let number = Self::issue_number(item_key)?;
        let url = format!(
            "{}?per_page=100",
            self.url(&format!("issues/{number}/comments"))
        );
        let value = self.issue_json(Method::GET, &url, None).await?;
        let values = value
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("GitHub issue comments were not an array"))?;
        Ok(values
            .iter()
            .filter_map(|comment| {
                let author = comment.get("user")?.get("login")?.as_str()?.to_string();
                let created_at = comment
                    .get("created_at")
                    .and_then(Value::as_str)
                    .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
                    .map(|at| at.with_timezone(&Utc));
                Some(fabro_work::WorkItemComment {
                    author,
                    body: comment
                        .get("body")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    created_at,
                    url: comment
                        .get("html_url")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                })
            })
            .collect())
    }

    async fn project_claim(
        &self,
        project: &WorkProject,
        item_key: &str,
        run_id: &str,
    ) -> anyhow::Result<()> {
        let number = Self::issue_number(item_key)?;
        let label = format!("{}{run_id}", project.claim_label_prefix);
        // A stale projection from an earlier claim is removed before the new
        // one is attached. Only labels carrying this project's prefix are
        // touched.
        if let Ok(value) = self.list_issue_json(project, number).await {
            if let Some(labels) = value.get("labels").and_then(Value::as_array) {
                for old in labels {
                    if let Some(name) = old.get("name").and_then(Value::as_str) {
                        if name.starts_with(&project.claim_label_prefix) && name != label {
                            self.remove_label_from_issue(number, name).await?;
                        }
                    }
                }
            }
        }
        self.ensure_label(&label).await?;
        self.add_label(number, &label).await
    }

    async fn clear_claim(
        &self,
        project: &WorkProject,
        item_key: &str,
        run_id: &str,
    ) -> anyhow::Result<()> {
        let number = Self::issue_number(item_key)?;
        let label = format!("{}{run_id}", project.claim_label_prefix);
        self.remove_label_from_issue(number, &label).await?;
        self.delete_label(&label).await
    }

    /// Delete run-id labels no issue carries anymore (ADR-0032 label GC):
    /// only live claims stay visible in the tracker's label list.
    async fn gc_claim_labels(&self, project: &WorkProject) -> anyhow::Result<u64> {
        let mut removed = 0;
        for page in 1..=3 {
            let url = format!("{}?per_page=100&page={page}", self.url("labels"));
            let value = self.issue_json(Method::GET, &url, None).await?;
            let labels = value
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("GitHub label list was not an array"))?;
            if labels.is_empty() {
                break;
            }
            for label in labels {
                let Some(name) = label.get("name").and_then(Value::as_str) else {
                    continue;
                };
                if !name.starts_with(&project.claim_label_prefix) {
                    continue;
                }
                let encoded = utf8_percent_encode(name, NON_ALPHANUMERIC).to_string();
                let issues_url = format!(
                    "{}?state=all&per_page=1&labels={encoded}",
                    self.url("issues")
                );
                let issues = self.issue_json(Method::GET, &issues_url, None).await?;
                let empty = issues.as_array().is_some_and(Vec::is_empty);
                if empty && self.delete_label(name).await.is_ok() {
                    removed += 1;
                }
            }
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use fabro_http::HttpClientBuilder;
    use fabro_work::{TrackerProvider as _, WorkItemFilter, WorkItemStateFilter, WorkProject};
    use httpmock::HttpMockResponse;

    use super::{GitHubTracker, TrackerApiError};

    fn issue(number: u64) -> serde_json::Value {
        serde_json::json!({
            "number": number,
            "title": format!("Item {number}"),
            "body": "b",
            "state": "open",
            "labels": [{"name": "project:fabro"}],
            "html_url": "https://github.com/denkhaus/work/issues/1",
            "updated_at": "2026-11-15T15:14:00Z",
        })
    }

    /// A mock whose first `limited` calls answer the installation-budget
    /// 403 (with `Retry-After: 0` so the backoff never sleeps) and every
    /// later call the issue body — the retry path of denkhaus/work#1364.
    fn rate_limited_then_issue(limited: usize) -> (httpmock::Mock<'static>, String) {
        // Leaked on purpose: the mock outlives the helper, the server
        // lives exactly as long as the one test using it.
        let server = Box::leak(Box::new(httpmock::MockServer::start()));
        let seen = AtomicUsize::new(0);
        let base_url = server.base_url();
        let mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/repos/denkhaus/work/issues/1");
            then.respond_with(move |_req| {
                let calls = seen.fetch_add(1, Ordering::SeqCst);
                if calls < limited {
                    HttpMockResponse::builder()
                        .status(403)
                        .header("Retry-After", "0")
                        .body("API rate limit exceeded for installation ID 161242007")
                        .build()
                } else {
                    HttpMockResponse::builder()
                        .status(200)
                        .body(issue(1).to_string())
                        .build()
                }
            });
        });
        (mock, base_url)
    }

    #[tokio::test]
    async fn a_rate_limited_403_backs_off_and_retries_to_success() {
        let (mock, base_url) = rate_limited_then_issue(2);
        let item = tracker(&base_url)
            .get(&project(), "denkhaus/work#1")
            .await
            .expect("the retried call resolves")
            .expect("the issue parses");
        assert_eq!(item.item_key, "denkhaus/work#1");
        mock.assert_calls(3);
    }

    #[tokio::test]
    async fn an_exhausted_rate_limit_fails_as_rate_limit_exhausted() {
        let (mock, base_url) = rate_limited_then_issue(usize::MAX);
        let error = tracker(&base_url)
            .get(&project(), "denkhaus/work#1")
            .await
            .expect_err("the budget stays exhausted");
        mock.assert_calls(3);
        let typed = error
            .downcast_ref::<TrackerApiError>()
            .expect("the failure is the typed rate-limit marker");
        assert!(
            matches!(typed, TrackerApiError::RateLimitExhausted {
                status: 403,
                ..
            }),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn a_plain_403_without_a_rate_limit_signature_is_not_retried() {
        let server = httpmock::MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/repos/denkhaus/work/issues/1");
            then.status(403)
                .body("Resource not accessible by integration");
        });

        let error = tracker(&server.base_url())
            .get(&project(), "denkhaus/work#1")
            .await
            .expect_err("the scope 403 fails");
        mock.assert_calls(1);
        assert!(
            matches!(
                error.downcast_ref::<TrackerApiError>(),
                Some(TrackerApiError::Status { status: 403, .. })
            ),
            "unexpected error: {error}"
        );
    }

    fn project() -> WorkProject {
        WorkProject {
            project_id:               "fabro".to_string(),
            display_name:             "Fabro".to_string(),
            tracker_provider:         "github".to_string(),
            tracker_repo:             "denkhaus/work".to_string(),
            target_repo:              Some("denkhaus/fabro".to_string()),
            project_label:            "project:fabro".to_string(),
            lane_labels:              [("loop".to_string(), "lane:loop".to_string())]
                .into_iter()
                .collect(),
            claim_label_prefix:       "fabro:run:".to_string(),
            claim_comment_enabled:    false,
            backpressure_open_pr_cap: 0,
        }
    }

    fn tracker(base_url: &str) -> GitHubTracker {
        let client = HttpClientBuilder::new()
            .no_proxy()
            .build()
            .expect("test http client");
        GitHubTracker::new(
            client,
            base_url,
            "token".to_string(),
            "denkhaus/work".to_string(),
        )
    }

    #[tokio::test]
    async fn search_sends_repo_issue_and_label_qualifiers() {
        let server = httpmock::MockServer::start_async().await;
        // The q value is fully percent-encoded (NON_ALPHANUMERIC), so the
        // mock below pins the exact qualifiers on the wire: repo,
        // issue-only, both labels (project + lane), then the keyword.
        let searched = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/search/issues")
                    // GitHub answers 403 "forbidden by administrative rules"
                    // without a User-Agent — only the live API enforces it,
                    // so the wire pin carries it (2026-10-21 live incident).
                    .header("User-Agent", "fabro")
                    .query_param(
                        "q",
                        "repo:denkhaus/work is:issue label:\"project:fabro\" label:\"lane:loop\" watchdog",
                    )
                    .query_param("per_page", "25")
                    .query_param("page", "1");
                then.status(200).json_body(serde_json::json!({
                    "total_count": 2,
                    "items": [
                        {"number": 7, "title": "Stall watchdog", "body": "b", "state": "closed",
                         "labels": [{"name": "project:fabro"}], "html_url": "u7",
                         "updated_at": "2026-10-21T04:00:00Z"},
                        {"number": 9, "title": "Watchdog tuning", "body": "b", "state": "open",
                         "labels": [{"name": "project:fabro"}], "html_url": "u9",
                         "updated_at": "2026-10-21T05:00:00Z"}
                    ]
                }));
            })
            .await;
        let tracker = tracker(&server.base_url());
        let items = tracker
            .list(&project(), WorkItemFilter {
                lane:  Some("loop".to_string()),
                limit: 25,
                query: Some("watchdog".to_string()),
                label: None,
                page:  1,
                state: WorkItemStateFilter::Open,
            })
            .await
            .expect("the search arm resolves");
        searched.assert_calls_async(1).await;
        assert_eq!(items.len(), 2, "closed items are part of search results");
        assert_eq!(items[0].state, "closed");
        assert_eq!(items[0].item_key, "denkhaus/work#7");
        assert_eq!(items[1].state, "open");
    }

    /// The exact-total read (denkhaus/work#1296): the search arm's
    /// `total_count` rides the SAME response as the items — the pager's
    /// total without a second request.
    #[tokio::test]
    async fn list_with_total_surfaces_the_search_total_count() {
        let server = httpmock::MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/search/issues")
                    .header("User-Agent", "fabro")
                    .query_param("per_page", "10")
                    .query_param("page", "2");
                then.status(200).json_body(serde_json::json!({
                    "total_count": 17,
                    "items": [
                        {"number": 4, "title": "One hit on page 2", "body": "b", "state": "open",
                         "labels": [{"name": "project:fabro"}], "html_url": "u4",
                         "updated_at": "2026-11-13T09:00:00Z"}
                    ]
                }));
            })
            .await;
        let tracker = tracker(&server.base_url());
        let page = tracker
            .list_with_total(&project(), WorkItemFilter {
                lane:  None,
                limit: 10,
                query: Some("watchdog".to_string()),
                label: None,
                page:  2,
                state: WorkItemStateFilter::Open,
            })
            .await
            .expect("the counting search resolves");
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.total, Some(17), "the pager's total is total_count");
    }

    /// The open-list arm has no count on its list response — the exact
    /// total comes from a count-only search with the SAME qualifiers
    /// (repo, issue-only, open, the filter's labels; denkhaus/work#1296).
    #[tokio::test]
    async fn list_with_total_counts_the_open_arm_through_a_count_search() {
        let server = httpmock::MockServer::start_async().await;
        let listed = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/repos/denkhaus/work/issues")
                    .query_param("state", "open")
                    .query_param("labels", "project:fabro,lane:loop")
                    .query_param("per_page", "10")
                    .query_param("page", "1");
                then.status(200).json_body(serde_json::json!([
                    {"number": 5, "title": "An open item", "body": "b", "state": "open",
                     "labels": [{"name": "project:fabro"}], "html_url": "u5",
                     "updated_at": "2026-11-13T08:00:00Z"}
                ]));
            })
            .await;
        let counted = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/search/issues")
                    .header("User-Agent", "fabro")
                    .query_param(
                        "q",
                        "repo:denkhaus/work is:issue is:open label:\"project:fabro\" \
                         label:\"lane:loop\"",
                    )
                    .query_param("per_page", "1")
                    .query_param("page", "1");
                then.status(200).json_body(serde_json::json!({
                    "total_count": 23, "items": []
                }));
            })
            .await;
        let tracker = tracker(&server.base_url());
        let page = tracker
            .list_with_total(&project(), WorkItemFilter {
                lane:  Some("loop".to_string()),
                limit: 10,
                query: None,
                label: None,
                page:  1,
                state: WorkItemStateFilter::Open,
            })
            .await
            .expect("the counting list resolves");
        listed.assert_calls_async(1).await;
        counted.assert_calls_async(1).await;
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.total, Some(23), "the count search names the total");
    }

    #[tokio::test]
    async fn a_blank_query_is_rejected_before_any_request() {
        let server = httpmock::MockServer::start_async().await;
        let tracker = tracker(&server.base_url());
        let error = tracker
            .list(&project(), WorkItemFilter {
                lane:  None,
                limit: 50,
                query: Some("   ".to_string()),
                label: None,
                page:  1,
                state: WorkItemStateFilter::Open,
            })
            .await
            .expect_err("a blank query must fail");
        // No mock is registered: any request would have failed the call
        // with a connection-level mock error instead of the blank guard.
        assert!(error.to_string().contains("blank"), "{error}");
    }

    #[tokio::test]
    async fn list_mode_keeps_the_open_only_label_filter_path() {
        let server = httpmock::MockServer::start_async().await;
        let listed = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/repos/denkhaus/work/issues")
                    .query_param("state", "open")
                    .query_param("labels", "project:fabro,lane:loop")
                    .query_param("per_page", "50")
                    .query_param("page", "1");
                then.status(200).json_body(serde_json::json!([]));
            })
            .await;
        let tracker = tracker(&server.base_url());
        let items = tracker
            .list(&project(), WorkItemFilter {
                lane:  Some("loop".to_string()),
                limit: 50,
                query: None,
                label: None,
                page:  1,
                state: WorkItemStateFilter::Open,
            })
            .await
            .expect("the open-list arm resolves");
        listed.assert_calls_async(1).await;
        assert!(items.is_empty());
    }

    /// The label census seam (denkhaus/work#1001): a raw label filter
    /// rides beside the project/lane labels and `page` paginates the
    /// listing — `limit` stays the page size.
    #[tokio::test]
    async fn label_filter_and_page_reach_the_issue_list_call() {
        let server = httpmock::MockServer::start_async().await;
        let listed = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/repos/denkhaus/work/issues")
                    .query_param("state", "open")
                    .query_param("labels", "project:fabro,arch")
                    .query_param("per_page", "100")
                    .query_param("page", "3");
                then.status(200).json_body(serde_json::json!([]));
            })
            .await;
        let tracker = tracker(&server.base_url());
        tracker
            .list(&project(), WorkItemFilter {
                lane:  None,
                limit: 100,
                query: None,
                label: Some("arch".to_string()),
                page:  3,
                state: WorkItemStateFilter::Open,
            })
            .await
            .expect("the labeled paged list resolves");
        listed.assert_calls_async(1).await;
    }

    /// The state filter (work#1476): `All` rides GitHub's own `state=`
    /// query param — the census enumerates every state through the
    /// same list arm, page semantics unchanged.
    #[tokio::test]
    async fn the_all_state_filter_reaches_the_issue_list_call() {
        let server = httpmock::MockServer::start_async().await;
        let listed = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/repos/denkhaus/work/issues")
                    .query_param("state", "all")
                    .query_param("labels", "project:fabro")
                    .query_param("per_page", "100")
                    .query_param("page", "1");
                then.status(200).json_body(serde_json::json!([
                    {"number": 12, "title": "Closed long ago", "body": "b", "state": "closed",
                     "labels": [{"name": "project:fabro"}], "html_url": "u12",
                     "created_at": "2026-10-25T10:00:00Z",
                     "closed_at": "2026-10-26T10:00:00Z",
                     "updated_at": "2026-10-26T10:00:00Z"}
                ]));
            })
            .await;
        let tracker = tracker(&server.base_url());
        let items = tracker
            .list(&project(), WorkItemFilter {
                lane:  None,
                limit: 100,
                query: None,
                label: None,
                page:  1,
                state: WorkItemStateFilter::All,
            })
            .await
            .expect("the all-state list resolves");
        listed.assert_calls_async(1).await;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].state, "closed");
        assert!(items[0].created_at.is_some());
        assert!(items[0].closed_at.is_some());
    }

    /// The same seam on the search arm: the raw label becomes a label
    /// qualifier and `page` paginates the search results.
    #[tokio::test]
    async fn label_filter_and_page_reach_the_issue_search_call() {
        let server = httpmock::MockServer::start_async().await;
        let searched = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/search/issues")
                    .header("User-Agent", "fabro")
                    .query_param(
                        "q",
                        "repo:denkhaus/work is:issue label:\"project:fabro\" label:\"prio:p0\" census",
                    )
                    .query_param("per_page", "100")
                    .query_param("page", "2");
                then.status(200).json_body(serde_json::json!({
                    "total_count": 0, "items": []
                }));
            })
            .await;
        let tracker = tracker(&server.base_url());
        tracker
            .list(&project(), WorkItemFilter {
                lane:  None,
                limit: 100,
                query: Some("census".to_string()),
                label: Some("prio:p0".to_string()),
                page:  2,
                state: WorkItemStateFilter::Open,
            })
            .await
            .expect("the labeled paged search resolves");
        searched.assert_calls_async(1).await;
    }

    fn search_hit(number: u64, label: &str) -> serde_json::Value {
        serde_json::json!({
            "number": number, "title": format!("Migrated {label}"), "body": "b",
            "state": "closed",
            "labels": [{"name": "project:fabro"}, {"name": label}],
            "html_url": format!("u{number}"), "updated_at": "2026-10-25T10:00:00Z"
        })
    }

    #[tokio::test]
    async fn legacy_id_resolves_through_the_seeds_label() {
        let server = httpmock::MockServer::start_async().await;
        // The bare fabro-xxxx form namespaces itself into the
        // seeds:fabro-xxxx label qualifier (grill amendment 2026-10-21),
        // and a closed migrated issue stays resolvable — dedupe and
        // basis lookups cite closed items.
        let searched = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/search/issues")
                    .header("User-Agent", "fabro")
                    .query_param(
                        "q",
                        "repo:denkhaus/work is:issue label:\"project:fabro\" label:\"seeds:fabro-d5f0\"",
                    )
                    .query_param("per_page", "5")
                    .query_param("page", "1");
                then.status(200).json_body(serde_json::json!({
                    "total_count": 1,
                    "items": [search_hit(42, "seeds:fabro-d5f0")]
                }));
            })
            .await;
        let tracker = tracker(&server.base_url());
        let item = tracker
            .get(&project(), "fabro-d5f0")
            .await
            .expect("legacy resolution runs")
            .expect("exactly one labeled issue resolves");
        assert_eq!(item.item_key, "denkhaus/work#42");
        searched.assert_calls_async(1).await;
    }

    #[tokio::test]
    async fn duplicate_seeds_labels_are_an_error_never_a_guess() {
        let server = httpmock::MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET).path("/search/issues");
                then.status(200).json_body(serde_json::json!({
                    "total_count": 2,
                    "items": [
                        search_hit(41, "seeds:fabro-d5f0"),
                        search_hit(42, "seeds:fabro-d5f0")
                    ]
                }));
            })
            .await;
        let tracker = tracker(&server.base_url());
        let error = tracker
            .get(&project(), "fabro-d5f0")
            .await
            .expect_err("duplicate labels must error");
        assert!(
            error.to_string().contains("duplicate seeds labels"),
            "the error names the class: {error}"
        );
    }

    #[tokio::test]
    async fn an_unresolvable_legacy_id_is_not_found_and_junk_still_errors() {
        let server = httpmock::MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET).path("/search/issues");
                then.status(200)
                    .json_body(serde_json::json!({"total_count": 0, "items": []}));
            })
            .await;
        let tracker = tracker(&server.base_url());
        assert!(
            tracker
                .get(&project(), "fabro-zzzz")
                .await
                .expect("zero hits resolve cleanly")
                .is_none(),
            "an unresolvable legacy id is Not Found, never an error"
        );
        let junk = tracker
            .get(&project(), "junk")
            .await
            .expect_err("malformed keys keep the parse error");
        assert!(
            junk.to_string().contains("owner/repo#number"),
            "the teaching error survives: {junk}"
        );
    }

    /// The comment READ verb (ADR-0039 stage D part 2, denkhaus/work#1107):
    /// a work item's comments oldest first, each with its author, body and
    /// timestamp — the answer channel a parked run's fork travels back on
    /// (the operator answers on the work item; the next pass reads it).
    #[tokio::test]
    async fn list_comments_reads_the_issue_comments_oldest_first() {
        let server = httpmock::MockServer::start_async().await;
        let read = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/repos/denkhaus/work/issues/1107/comments")
                    .query_param("per_page", "100")
                    // Same User-Agent pin as the search arm: only the live
                    // API enforces it, the wire pin carries it.
                    .header("User-Agent", "fabro");
                then.status(200).json_body(serde_json::json!([
                    {"user": {"login": "adjudicator-bot"},
                     "body": "parked-fork: residual file — unfinished or exempt?",
                     "created_at": "2026-11-02T09:00:00Z",
                     "html_url": "https://github.com/denkhaus/work/issues/1107#issuecomment-1"},
                    {"user": {"login": "denkhaus"},
                     "body": "exempt — the residual ships as-is",
                     "created_at": "2026-11-02T10:00:00Z",
                     "html_url": "https://github.com/denkhaus/work/issues/1107#issuecomment-2"}
                ]));
            })
            .await;
        let tracker = tracker(&server.base_url());
        let comments = tracker
            .list_comments(&project(), "denkhaus/work#1107")
            .await
            .expect("the comment read resolves");
        read.assert_calls_async(1).await;
        assert_eq!(comments.len(), 2, "both comments come back in order");
        assert_eq!(comments[0].author, "adjudicator-bot");
        assert!(comments[0].body.contains("parked-fork"));
        assert_eq!(
            comments[0].created_at.map(|at| at.timestamp()),
            Some(1_793_610_000),
            "the forge timestamp parses"
        );
        assert_eq!(comments[1].author, "denkhaus");
        assert_eq!(
            comments[1].url.as_deref(),
            Some("https://github.com/denkhaus/work/issues/1107#issuecomment-2"),
            "the permalink survives for the operator's trace"
        );
    }
}
