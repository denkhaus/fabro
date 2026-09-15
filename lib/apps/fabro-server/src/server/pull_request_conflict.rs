//! Tracker-aware update-branch conflict resolution for the PR staleness
//! supervisor (fabro-895d).
//!
//! When GitHub's update-branch endpoint refuses with 422 and the conflicting
//! paths are confined to the dev loop's own tracker files (`.seeds/**`,
//! `.mulch/**`), the conflict is a JSONL-level bookkeeping divergence that
//! GitHub cannot auto-merge but fabro can: the supervisor unions both sides
//! with closed-wins semantics and pushes a merge commit through the git data
//! API. Resolution failures park the pull request instead of retiring it —
//! silent loss of filed tracker state must be impossible.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context as _, bail};
use fabro_github::{GitHubCredentials, HttpMethod, HttpResponse};
use tokio::time;

use super::AppState;

/// Per-call timeout for the git data API calls made during conflict
/// resolution, mirroring the staleness leg's GitHub call budget.
const GITHUB_CALL_TIMEOUT: Duration = Duration::from_secs(15);

fn is_tracker_path(path: &str) -> bool {
    path.starts_with(".seeds/") || path.starts_with(".mulch/")
}

/// Bookkeeping paths for the age-cap disposition (fabro-895d): tracker files
/// plus the stage journals under `.fabro/journal/`.
fn is_bookkeeping_path(path: &str) -> bool {
    is_tracker_path(path) || path.starts_with(".fabro/journal/")
}

/// All conflicting paths are loop-tracker files the supervisor can union.
pub(super) fn all_tracker_paths(paths: &[String]) -> bool {
    !paths.is_empty() && paths.iter().all(|path| is_tracker_path(path))
}

/// All changed paths of the pull request are dev-loop bookkeeping (tracker
/// files plus stage journals): an over-age bookkeeping-only PR parks instead
/// of closing (fabro-895d).
pub(super) fn all_bookkeeping_paths(paths: &[String]) -> bool {
    !paths.is_empty() && paths.iter().all(|path| is_bookkeeping_path(path))
}

// ---------------------------------------------------------------------------
// Closed-wins JSONL union
// ---------------------------------------------------------------------------

/// One parsed JSONL record: the raw line (canonical form preserved) and
/// whether the record is closed on its side.
struct ParsedRecord {
    id:     String,
    line:   String,
    closed: bool,
}

/// Parsed tracker file: records in file order, addressable by id. A duplicate
/// id within one file keeps the last occurrence in first-appearance order.
struct ParsedTrackerFile {
    records: Vec<ParsedRecord>,
    index:   HashMap<String, usize>,
}

fn parse_tracker_file(side: &str, text: &str) -> anyhow::Result<ParsedTrackerFile> {
    let mut records = Vec::new();
    let mut index = HashMap::new();
    for (position, raw) in text.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line).with_context(|| {
            format!(
                "{side} tracker JSONL is unparseable at line {}",
                position + 1
            )
        })?;
        let object = value.as_object().with_context(|| {
            format!(
                "{side} tracker JSONL line {} is not an object",
                position + 1
            )
        })?;
        let id = object
            .get("id")
            .and_then(serde_json::Value::as_str)
            .with_context(|| {
                format!(
                    "{side} tracker JSONL line {} has no string id",
                    position + 1
                )
            })?;
        let closed = object.get("status").and_then(serde_json::Value::as_str) == Some("closed");
        let record = ParsedRecord {
            id: id.to_string(),
            line: line.to_string(),
            closed,
        };
        if let Some(&slot) = index.get(id) {
            records[slot] = record;
        } else {
            index.insert(id.to_string(), records.len());
            records.push(record);
        }
    }
    Ok(ParsedTrackerFile { records, index })
}

/// Closed-wins union of the base-side and head-side versions of one tracker
/// JSONL file (fabro-895d).
///
/// Records present on one side only are kept verbatim. For an id present on
/// both sides, a closed base record wins (a run branch forked before the
/// record was closed must not resurrect it); otherwise the head record wins
/// (the branch may carry updates). Base order is preserved, head-only records
/// are appended in head order. Any unparseable line fails the merge.
pub(super) fn union_jsonl_closed_wins(base: &str, head: &str) -> anyhow::Result<String> {
    let base_file = parse_tracker_file("base", base)?;
    let head_file = parse_tracker_file("head", head)?;

    let mut merged: Vec<String> = Vec::new();
    for base_record in &base_file.records {
        let winner = match head_file.index.get(&base_record.id) {
            Some(&head_slot) if !base_record.closed => head_file.records[head_slot].line.clone(),
            _ => base_record.line.clone(),
        };
        merged.push(winner);
    }
    for head_record in &head_file.records {
        if !base_file.index.contains_key(&head_record.id) {
            merged.push(head_record.line.clone());
        }
    }

    if merged.is_empty() {
        return Ok(String::new());
    }
    let mut out = merged.join("\n");
    out.push('\n');
    Ok(out)
}

// ---------------------------------------------------------------------------
// GitHub git data API client
// ---------------------------------------------------------------------------

/// Minimal REST client for the endpoints conflict resolution needs. All calls
/// carry the same bearer token and share the staleness leg's call timeout.
pub(super) struct ConflictResolutionClient {
    http:     fabro_http::HttpClient,
    base_url: String,
    owner:    String,
    repo:     String,
    token:    String,
}

impl ConflictResolutionClient {
    pub(super) async fn new(
        state: &AppState,
        creds: &GitHubCredentials,
        owner: &str,
        repo: &str,
    ) -> anyhow::Result<Self> {
        let http = state.http_client()?;
        let token = creds
            .resolve_bearer_token(
                &http,
                owner,
                repo,
                state.github_api_base_url.as_str(),
                serde_json::json!({ "contents": "write", "pull_requests": "write" }),
            )
            .await?;
        Ok(Self {
            http,
            base_url: state.github_api_base_url.clone(),
            owner: owner.to_string(),
            repo: repo.to_string(),
            token,
        })
    }

    async fn send(
        &self,
        method: HttpMethod,
        url: &str,
        accept: Option<&str>,
        body: Option<&serde_json::Value>,
    ) -> anyhow::Result<HttpResponse> {
        let auth = format!("Bearer {}", self.token);
        let headers: [(&str, &str); 3] = [
            ("Authorization", auth.as_str()),
            ("Accept", accept.unwrap_or("application/vnd.github+json")),
            ("User-Agent", "fabro"),
        ];
        let request = fabro_github::HttpClient::request(&self.http, method, url, &headers, body);
        let response = time::timeout(GITHUB_CALL_TIMEOUT, request).await??;
        Ok(response)
    }

    /// Filenames changed on the `head` side of `compare/{base}...{head}`.
    pub(super) async fn compare_filenames(&self, basehead: &str) -> anyhow::Result<Vec<String>> {
        let url = format!(
            "{}/repos/{}/{}/compare/{}",
            self.base_url, self.owner, self.repo, basehead
        );
        let response = self.send(HttpMethod::Get, &url, None, None).await?;
        if response.status != 200 {
            bail!(
                "compare {basehead} answered {}: {}",
                response.status,
                response.text()
            );
        }
        let payload: serde_json::Value = response.json()?;
        let files = payload
            .get("files")
            .and_then(serde_json::Value::as_array)
            .context("compare response has no files array")?;
        Ok(files
            .iter()
            .filter_map(|file| {
                file.get("filename")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .collect())
    }

    /// Filenames changed by the pull request (for the age-cap disposition).
    pub(super) async fn list_pull_request_files(&self, number: u64) -> anyhow::Result<Vec<String>> {
        let url = format!(
            "{}/repos/{}/{}/pulls/{number}/files?per_page=100",
            self.base_url, self.owner, self.repo
        );
        let response = self.send(HttpMethod::Get, &url, None, None).await?;
        if response.status != 200 {
            bail!(
                "listing PR files answered {}: {}",
                response.status,
                response.text()
            );
        }
        let files: Vec<serde_json::Value> = response.json()?;
        Ok(files
            .iter()
            .filter_map(|file| {
                file.get("filename")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .collect())
    }

    /// Raw file content at `git_ref` (raw media type, no base64 round trip).
    async fn file_content(&self, path: &str, git_ref: &str) -> anyhow::Result<String> {
        let url = format!(
            "{}/repos/{}/{}/contents/{}?ref={}",
            self.base_url, self.owner, self.repo, path, git_ref
        );
        let response = self
            .send(
                HttpMethod::Get,
                &url,
                Some("application/vnd.github.raw"),
                None,
            )
            .await?;
        if response.status != 200 {
            bail!(
                "reading {path} at {git_ref} answered {}: {}",
                response.status,
                response.text()
            );
        }
        Ok(response.text().to_string())
    }

    /// `(commit sha, tree sha)` at `git_ref`.
    async fn commit_info(&self, git_ref: &str) -> anyhow::Result<(String, String)> {
        let url = format!(
            "{}/repos/{}/{}/commits/{git_ref}",
            self.base_url, self.owner, self.repo
        );
        let response = self.send(HttpMethod::Get, &url, None, None).await?;
        if response.status != 200 {
            bail!(
                "reading commit {git_ref} answered {}: {}",
                response.status,
                response.text()
            );
        }
        let payload: serde_json::Value = response.json()?;
        let sha = payload
            .get("sha")
            .and_then(serde_json::Value::as_str)
            .context("commit response has no sha")?
            .to_string();
        let tree = payload
            .pointer("/commit/tree/sha")
            .and_then(serde_json::Value::as_str)
            .context("commit response has no tree sha")?
            .to_string();
        Ok((sha, tree))
    }

    async fn create_blob(&self, content: &str) -> anyhow::Result<String> {
        let url = format!(
            "{}/repos/{}/{}/git/blobs",
            self.base_url, self.owner, self.repo
        );
        let body = serde_json::json!({ "content": content, "encoding": "utf-8" });
        let response = self.send(HttpMethod::Post, &url, None, Some(&body)).await?;
        if response.status != 201 {
            bail!(
                "creating blob answered {}: {}",
                response.status,
                response.text()
            );
        }
        let payload: serde_json::Value = response.json()?;
        Ok(payload
            .get("sha")
            .and_then(serde_json::Value::as_str)
            .context("blob response has no sha")?
            .to_string())
    }

    async fn create_tree(
        &self,
        base_tree: &str,
        entries: &[(String, String)],
    ) -> anyhow::Result<String> {
        let url = format!(
            "{}/repos/{}/{}/git/trees",
            self.base_url, self.owner, self.repo
        );
        let tree: Vec<serde_json::Value> = entries
            .iter()
            .map(|(path, sha)| {
                serde_json::json!({
                    "path": path,
                    "mode": "100644",
                    "type": "blob",
                    "sha": sha,
                })
            })
            .collect();
        let body = serde_json::json!({ "base_tree": base_tree, "tree": tree });
        let response = self.send(HttpMethod::Post, &url, None, Some(&body)).await?;
        if response.status != 201 {
            bail!(
                "creating tree answered {}: {}",
                response.status,
                response.text()
            );
        }
        let payload: serde_json::Value = response.json()?;
        Ok(payload
            .get("sha")
            .and_then(serde_json::Value::as_str)
            .context("tree response has no sha")?
            .to_string())
    }

    async fn create_merge_commit(
        &self,
        message: &str,
        tree: &str,
        parents: [&str; 2],
    ) -> anyhow::Result<String> {
        let url = format!(
            "{}/repos/{}/{}/git/commits",
            self.base_url, self.owner, self.repo
        );
        let body = serde_json::json!({
            "message": message,
            "tree": tree,
            "parents": parents,
        });
        let response = self.send(HttpMethod::Post, &url, None, Some(&body)).await?;
        if response.status != 201 {
            bail!(
                "creating merge commit answered {}: {}",
                response.status,
                response.text()
            );
        }
        let payload: serde_json::Value = response.json()?;
        Ok(payload
            .get("sha")
            .and_then(serde_json::Value::as_str)
            .context("commit response has no sha")?
            .to_string())
    }

    /// Fast-forward-only push of `branch` to `sha`. A 422 answer means the
    /// branch moved (or the push is otherwise rejected) and surfaces as
    /// [`PushOutcome::Rejected`] instead of a generic error.
    async fn push_branch_head(&self, branch: &str, sha: &str) -> anyhow::Result<PushOutcome> {
        let url = format!(
            "{}/repos/{}/{}/git/refs/heads/{branch}",
            self.base_url, self.owner, self.repo
        );
        let body = serde_json::json!({ "sha": sha, "force": false });
        let response = self
            .send(HttpMethod::Patch, &url, None, Some(&body))
            .await?;
        match response.status {
            200 => Ok(PushOutcome::Updated),
            422 => Ok(PushOutcome::Rejected),
            status => bail!(
                "updating ref {branch} answered {status}: {}",
                response.text()
            ),
        }
    }

    /// Union-merge the tracker JSONL files in `paths` between `base_ref` and
    /// `head_ref` and push the resulting merge commit onto `head_ref`.
    ///
    /// `Err` carries the human-readable reason for parking: unparseable
    /// JSONL, an unreadable side, a failed git data API call, or a rejected
    /// push.
    pub(super) async fn resolve_tracker_conflict(
        &self,
        base_ref: &str,
        head_ref: &str,
        paths: &[String],
    ) -> Result<(), String> {
        self.resolve_tracker_conflict_inner(base_ref, head_ref, paths)
            .await
            .map_err(|error| format!("paths [{}]: {error:#}", paths.join(", ")))
    }

    async fn resolve_tracker_conflict_inner(
        &self,
        base_ref: &str,
        head_ref: &str,
        paths: &[String],
    ) -> anyhow::Result<()> {
        let mut merged_files = Vec::new();
        for path in paths {
            let base_content = self
                .file_content(path, base_ref)
                .await
                .with_context(|| format!("reading base side of {path}"))?;
            let head_content = self
                .file_content(path, head_ref)
                .await
                .with_context(|| format!("reading head side of {path}"))?;
            let merged = union_jsonl_closed_wins(&base_content, &head_content)
                .with_context(|| format!("unioning {path}"))?;
            merged_files.push((path.clone(), merged));
        }

        let (head_sha, head_tree) = self
            .commit_info(head_ref)
            .await
            .context("reading head commit")?;
        let (base_sha, _) = self
            .commit_info(base_ref)
            .await
            .context("reading base commit")?;

        let mut entries = Vec::new();
        for (path, content) in &merged_files {
            let blob = self
                .create_blob(content)
                .await
                .with_context(|| format!("storing merged {path}"))?;
            entries.push((path.clone(), blob));
        }
        let tree = self.create_tree(&head_tree, &entries).await?;
        let commit = self
            .create_merge_commit(
                "Merge base into run branch: tracker JSONL closed-wins union (fabro supervisor)",
                &tree,
                [&head_sha, &base_sha],
            )
            .await?;

        match self.push_branch_head(head_ref, &commit).await? {
            PushOutcome::Updated => Ok(()),
            PushOutcome::Rejected => {
                bail!("push of the resolved branch was rejected (branch moved); refusing to force")
            }
        }
    }
}

enum PushOutcome {
    Updated,
    Rejected,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_keeps_single_sided_records_and_base_closed_wins() {
        let base = concat!(
            "{\"id\":\"fabro-1\",\"status\":\"closed\"}\n",
            "{\"id\":\"fabro-2\",\"status\":\"open\"}\n",
            "{\"id\":\"fabro-3\",\"status\":\"closed\"}\n",
        );
        let head = concat!(
            "{\"id\":\"fabro-1\",\"status\":\"open\"}\n",
            "{\"id\":\"fabro-2\",\"status\":\"in_progress\"}\n",
            "{\"id\":\"fabro-9\",\"status\":\"open\"}\n",
        );

        let merged = union_jsonl_closed_wins(base, head).expect("union should succeed");

        let lines: Vec<&str> = merged.lines().collect();
        assert_eq!(lines, vec![
            // base closed record survives the branch's stale open copy
            "{\"id\":\"fabro-1\",\"status\":\"closed\"}",
            // head wins for open/updated records
            "{\"id\":\"fabro-2\",\"status\":\"in_progress\"}",
            // base-only record kept
            "{\"id\":\"fabro-3\",\"status\":\"closed\"}",
            // head-only record appended
            "{\"id\":\"fabro-9\",\"status\":\"open\"}",
        ]);
        assert!(merged.ends_with('\n'), "trailing newline preserved");
    }

    #[test]
    fn union_dedupes_ids_with_base_closed_winning() {
        let base =
            "{\"id\":\"mx-1\",\"status\":\"closed\"}\n{\"id\":\"mx-2\",\"status\":\"open\"}\n";
        let head =
            "{\"id\":\"mx-1\",\"status\":\"closed\"}\n{\"id\":\"mx-2\",\"status\":\"closed\"}\n";

        let merged = union_jsonl_closed_wins(base, head).expect("union should succeed");
        let lines: Vec<&str> = merged.lines().collect();
        // Duplicate ids collapse to one record each; base closed stays closed,
        // and an open base record closed only on head keeps the head (branch)
        // version — branch updates win unless base already closed it.
        assert_eq!(lines, vec![
            "{\"id\":\"mx-1\",\"status\":\"closed\"}",
            "{\"id\":\"mx-2\",\"status\":\"closed\"}",
        ]);
    }

    #[test]
    fn union_fails_on_unparseable_jsonl() {
        let base = "{\"id\":\"fabro-1\",\"status\":\"open\"}\n";
        let head = "this is not json\n";

        let error = union_jsonl_closed_wins(base, head).expect_err("unparseable head must fail");
        assert!(
            error.to_string().contains("unparseable"),
            "error should name the unparseable side: {error:#}"
        );
    }

    #[test]
    fn path_classification_covers_seeds_mulch_and_journals() {
        assert!(all_tracker_paths(&[".seeds/issues.jsonl".to_string()]));
        assert!(all_tracker_paths(&[
            ".seeds/issues.jsonl".to_string(),
            ".mulch/mulch.config.yaml".to_string()
        ]));
        assert!(!all_tracker_paths(&[
            ".seeds/issues.jsonl".to_string(),
            "lib/foo.rs".to_string()
        ]));
        assert!(!all_tracker_paths(&[]));

        assert!(all_bookkeeping_paths(&[
            ".fabro/journal/run.jsonl".to_string()
        ]));
        assert!(all_bookkeeping_paths(&[".seeds/issues.jsonl".to_string()]));
        assert!(!all_bookkeeping_paths(&["docs/foo.md".to_string()]));
    }
}
