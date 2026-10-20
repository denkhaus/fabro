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

    fn project_lane_labels(project: &WorkProject, filter: &WorkItemFilter) -> String {
        let mut labels = vec![project.project_label.clone()];
        if let Some(lane) = filter.lane.as_deref() {
            let label = project
                .lane_labels
                .get(lane)
                .cloned()
                .unwrap_or_else(|| format!("lane:{lane}"));
            labels.push(label);
        }
        labels.join(",")
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
}

#[async_trait]
impl TrackerProvider for GitHubTracker {
    async fn list_open(
        &self,
        project: &WorkProject,
        filter: WorkItemFilter,
    ) -> anyhow::Result<Vec<WorkItem>> {
        let limit = filter.limit.clamp(1, 100);
        let labels = Self::project_lane_labels(project, &filter);
        let encoded_labels = labels
            .split(',')
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

    async fn get(&self, project: &WorkProject, item_key: &str) -> anyhow::Result<Option<WorkItem>> {
        let number = Self::issue_number(item_key)?;
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
}
