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

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fabro_http::{HttpClient, Method, RequestBuilder, Response, StatusCode};
use fabro_work::{TrackerProvider, WorkItem, WorkItemDraft, WorkItemFilter, WorkProject};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::Value;

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

    async fn send_json(&self, response: Response) -> anyhow::Result<Value> {
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            anyhow::bail!("GitHub API {status}: {text}");
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text)
            .map_err(|err| anyhow::anyhow!("GitHub API returned invalid JSON: {err}"))
    }

    async fn list_issue_json(
        &self,
        _project: &WorkProject,
        issue_number: u64,
    ) -> anyhow::Result<Value> {
        let response = self
            .request(Method::GET, &self.url(&format!("issues/{issue_number}")))
            .send()
            .await?;
        self.send_json(response).await
    }

    async fn ensure_label(&self, name: &str) -> anyhow::Result<()> {
        let response = self
            .request(Method::POST, &self.url("labels"))
            .json(&serde_json::json!({"name": name, "color": "ededed"}))
            .send()
            .await?;
        if response.status().is_success() || response.status() == StatusCode::UNPROCESSABLE_ENTITY {
            return Ok(());
        }
        let status = response.status();
        let text = response.text().await?;
        anyhow::bail!("GitHub label create {status}: {text}")
    }

    async fn add_label(&self, issue_number: u64, name: &str) -> anyhow::Result<()> {
        let response = self
            .request(
                Method::POST,
                &self.url(&format!("issues/{issue_number}/labels")),
            )
            .json(&serde_json::json!({"labels": [name]}))
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await?;
            anyhow::bail!("GitHub label attach {status}: {text}");
        }
        Ok(())
    }

    async fn remove_label_from_issue(&self, issue_number: u64, name: &str) -> anyhow::Result<()> {
        let encoded = utf8_percent_encode(name, NON_ALPHANUMERIC).to_string();
        let response = self
            .request(
                Method::DELETE,
                &self.url(&format!("issues/{issue_number}/labels/{encoded}")),
            )
            .send()
            .await?;
        if response.status().is_success() || response.status() == StatusCode::NOT_FOUND {
            return Ok(());
        }
        let status = response.status();
        let text = response.text().await?;
        anyhow::bail!("GitHub label detach {status}: {text}")
    }

    async fn delete_label(&self, name: &str) -> anyhow::Result<()> {
        let encoded = utf8_percent_encode(name, NON_ALPHANUMERIC).to_string();
        let response = self
            .request(Method::DELETE, &self.url(&format!("labels/{encoded}")))
            .send()
            .await?;
        if response.status().is_success() || response.status() == StatusCode::NOT_FOUND {
            return Ok(());
        }
        let status = response.status();
        let text = response.text().await?;
        anyhow::bail!("GitHub label delete {status}: {text}")
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
        let hits = self
            .search_items(
                project,
                &WorkItemFilter {
                    lane:  None,
                    limit: 5,
                    query: None,
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

    fn project_lane_labels(project: &WorkProject, filter: &WorkItemFilter) -> Vec<String> {
        let mut labels = vec![project.project_label.clone()];
        if let Some(lane) = filter.lane.as_deref() {
            let label = project
                .lane_labels
                .get(lane)
                .cloned()
                .unwrap_or_else(|| format!("lane:{lane}"));
            labels.push(label);
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
        })
    }

    async fn list_open_items(
        &self,
        project: &WorkProject,
        filter: &WorkItemFilter,
    ) -> anyhow::Result<Vec<WorkItem>> {
        let limit = filter.limit.clamp(1, 100);
        let encoded_labels = Self::project_lane_labels(project, filter)
            .iter()
            .map(|label| utf8_percent_encode(label, NON_ALPHANUMERIC).to_string())
            .collect::<Vec<_>>()
            .join(",");
        let url = format!(
            "{}?state=open&labels={encoded_labels}&per_page={limit}",
            self.url("issues")
        );
        let response = self.request(Method::GET, &url).send().await?;
        let value = self.send_json(response).await?;
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
    async fn search_items(
        &self,
        project: &WorkProject,
        filter: &WorkItemFilter,
        query: &str,
    ) -> anyhow::Result<Vec<WorkItem>> {
        let limit = filter.limit.clamp(1, 100);
        let trimmed = query.trim();
        if trimmed.is_empty() {
            anyhow::bail!("work item search query must not be blank");
        }
        let mut terms = vec![format!("repo:{}", self.repo), "is:issue".to_string()];
        terms.extend(
            Self::project_lane_labels(project, filter)
                .into_iter()
                .map(|label| format!("label:\"{label}\"")),
        );
        terms.push(trimmed.to_string());
        let encoded = utf8_percent_encode(&terms.join(" "), NON_ALPHANUMERIC).to_string();
        let url = format!(
            "{}/search/issues?q={encoded}&per_page={limit}",
            self.base_url
        );
        let response = self.request(Method::GET, &url).send().await?;
        let value = self.send_json(response).await?;
        let values = value
            .get("items")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("GitHub issue search returned no items array"))?;
        Ok(values
            .iter()
            .filter_map(|value| self.json_item(value))
            .collect())
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
            Some(query) => self.search_items(project, &filter, query).await,
            None => self.list_open_items(project, &filter).await,
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
        let response = self
            .request(Method::POST, &self.url("issues"))
            .json(&serde_json::json!({
                "title": draft.title,
                "body": draft.body,
                "labels": draft.labels,
            }))
            .send()
            .await?;
        let value = self.send_json(response).await?;
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
        let response = self
            .request(Method::PATCH, &self.url(&format!("issues/{number}")))
            .json(&Value::Object(patch))
            .send()
            .await?;
        let value = self.send_json(response).await?;
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
        let response = self
            .request(Method::PATCH, &self.url(&format!("issues/{number}")))
            .json(&serde_json::json!({"state": "closed", "state_reason": "completed"}))
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await?;
            anyhow::bail!("GitHub issue close {status}: {text}");
        }
        Ok(())
    }

    async fn comment(
        &self,
        _project: &WorkProject,
        item_key: &str,
        body: &str,
    ) -> anyhow::Result<()> {
        let number = Self::issue_number(item_key)?;
        let response = self
            .request(
                Method::POST,
                &self.url(&format!("issues/{number}/comments")),
            )
            .json(&serde_json::json!({"body": body}))
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await?;
            anyhow::bail!("GitHub issue comment {status}: {text}");
        }
        Ok(())
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
            let response = self.request(Method::GET, &url).send().await?;
            let value = self.send_json(response).await?;
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
                let issues = self.request(Method::GET, &issues_url).send().await?;
                let issues = self.send_json(issues).await?;
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
    use fabro_http::HttpClientBuilder;
    use fabro_work::{TrackerProvider as _, WorkItemFilter, WorkProject};

    use super::GitHubTracker;

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
                    .query_param("per_page", "25");
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
            })
            .await
            .expect("the search arm resolves");
        searched.assert_calls_async(1).await;
        assert_eq!(items.len(), 2, "closed items are part of search results");
        assert_eq!(items[0].state, "closed");
        assert_eq!(items[0].item_key, "denkhaus/work#7");
        assert_eq!(items[1].state, "open");
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
                    .query_param("per_page", "50");
                then.status(200).json_body(serde_json::json!([]));
            })
            .await;
        let tracker = tracker(&server.base_url());
        let items = tracker
            .list(&project(), WorkItemFilter {
                lane:  Some("loop".to_string()),
                limit: 50,
                query: None,
            })
            .await
            .expect("the open-list arm resolves");
        listed.assert_calls_async(1).await;
        assert!(items.is_empty());
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
                    .query_param("per_page", "5");
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
}
