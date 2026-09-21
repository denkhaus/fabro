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
//!
//! FORK SURFACE (fabro-4ebd/PR #221, fabro-ec00): this module carries the
//! diff-based publish squash-revert protection — a stale-workspace publish
//! builds its merge tree from the run's own compare entries so concurrently
//! landed base commits survive. Presence pin:
//! `server/fork_publish_conflict_tests.rs` plus the touchpoints registry
//! row ("Diff-based publish squash-revert protection").

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

/// One parsed JSONL record: the raw line (canonical form preserved), whether
/// the record is closed on its side, and the record's `updatedAt` timestamp
/// (when present) for newest-wins merging (fabro-4ebd).
struct ParsedRecord {
    id:         String,
    line:       String,
    closed:     bool,
    updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Parsed tracker file: records in file order, addressable by id. A duplicate
/// id within one file keeps the last occurrence in first-appearance order.
struct ParsedTrackerFile {
    records: Vec<ParsedRecord>,
    index:   HashMap<String, usize>,
}

fn parse_updated_at(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    object
        .get("updatedAt")
        .and_then(serde_json::Value::as_str)
        .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
        .map(|parsed| parsed.with_timezone(&chrono::Utc))
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
        let updated_at = parse_updated_at(object);
        let record = ParsedRecord {
            id: id.to_string(),
            line: line.to_string(),
            closed,
            updated_at,
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

/// Closed-wins, newest-wins union of the base-side and head-side versions of
/// one tracker JSONL file (fabro-895d, hardened fabro-4ebd).
///
/// Records present on one side only are kept verbatim. For an id present on
/// both sides:
///
/// - a closed base record always wins — a run branch forked before the record
///   was closed must not resurrect it, whatever the timestamps say;
/// - otherwise the record with the newer `updatedAt` wins, on either side — the
///   branch's stale snapshot must not clobber newer base body updates or
///   assignments (the incident's lost tracker records);
/// - with `updatedAt` missing, unparseable, or tied on both sides, the head
///   record wins (the branch may carry updates).
///
/// Base order is preserved, head-only records are appended in head order.
/// Any unparseable line fails the merge.
pub(super) fn union_jsonl_closed_wins(base: &str, head: &str) -> anyhow::Result<String> {
    let base_file = parse_tracker_file("base", base)?;
    let head_file = parse_tracker_file("head", head)?;

    let mut merged: Vec<String> = Vec::new();
    for base_record in &base_file.records {
        let winner = match head_file.index.get(&base_record.id) {
            Some(&head_slot) if !base_record.closed => {
                let head_record = &head_file.records[head_slot];
                // The base record wins only when it is strictly newer: a
                // missing/unparseable timestamp counts as oldest, and ties
                // (including both missing) keep the head (branch) record —
                // the historical head-wins behavior.
                let base_is_strictly_newer = match (head_record.updated_at, base_record.updated_at)
                {
                    (None, Some(_)) => true,
                    (Some(head_at), Some(base_at)) => base_at > head_at,
                    _ => false,
                };
                if base_is_strictly_newer {
                    base_record.line.clone()
                } else {
                    head_record.line.clone()
                }
            }
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

/// One changed path of a compare-API side, with GitHub's change status
/// (`added`, `removed`, `modified`, `renamed`, ...) and, for renames, the
/// path the file was moved away from (fabro-4ebd).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CompareEntry {
    pub filename:          String,
    pub status:            String,
    pub previous_filename: Option<String>,
}

/// One entry of the run-scoped merge tree (fabro-4ebd): set `path` to new
/// content, or remove `path` from the base tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TreeChange {
    Set(String),
    Remove(String),
}

/// A tree entry staged against the base tree: a blob to write, or a path to
/// delete (`sha: null` with a base tree set).
#[derive(Debug, Clone, PartialEq, Eq)]
enum StagedTreeEntry {
    SetBlob { path: String, sha: String },
    Remove { path: String },
}

/// Compute the tree changes that carry EXACTLY the run's own committed
/// changes onto the current base (fabro-4ebd (b), diff-based publish).
///
/// Input is the head side of `compare {base}...{head}` — the run branch's
/// own changes since the merge base. Paths the base gained after the run
/// forked never appear here, so they keep their base-tree content: what the
/// run never touched, its merge commit cannot delete. This is what makes
/// the stale-workspace squash revert (incident PR #216) structurally
/// impossible: the merge commit's tree is the base tree overlaid with these
/// entries, never a copy of the run's stale workspace tree.
pub(super) fn run_scoped_tree_changes(head_entries: &[CompareEntry]) -> Vec<TreeChange> {
    let mut changes = Vec::new();
    for entry in head_entries {
        match entry.status.as_str() {
            "removed" => changes.push(TreeChange::Remove(entry.filename.clone())),
            "renamed" => {
                if let Some(previous) = entry.previous_filename.as_deref() {
                    changes.push(TreeChange::Remove(previous.to_string()));
                }
                changes.push(TreeChange::Set(entry.filename.clone()));
            }
            _ => changes.push(TreeChange::Set(entry.filename.clone())),
        }
    }
    changes
}

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
        Ok(self
            .compare_entries(basehead)
            .await?
            .into_iter()
            .map(|entry| entry.filename)
            .collect())
    }

    /// Changed entries (path plus change status) of one
    /// `compare/{base}...{head}` side (fabro-4ebd).
    pub(super) async fn compare_entries(
        &self,
        basehead: &str,
    ) -> anyhow::Result<Vec<CompareEntry>> {
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
                let filename = file
                    .get("filename")
                    .and_then(serde_json::Value::as_str)?
                    .to_string();
                let status = file
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("modified")
                    .to_string();
                let previous_filename = file
                    .get("previous_filename")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
                Some(CompareEntry {
                    filename,
                    status,
                    previous_filename,
                })
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
        entries: &[StagedTreeEntry],
    ) -> anyhow::Result<String> {
        let url = format!(
            "{}/repos/{}/{}/git/trees",
            self.base_url, self.owner, self.repo
        );
        let tree: Vec<serde_json::Value> = entries
            .iter()
            .map(|entry| match entry {
                StagedTreeEntry::SetBlob { path, sha } => serde_json::json!({
                    "path": path,
                    "mode": "100644",
                    "type": "blob",
                    "sha": sha,
                }),
                // With a base tree set, a `sha: null` entry deletes the path
                // (the run itself removed or renamed the file).
                StagedTreeEntry::Remove { path } => serde_json::json!({
                    "path": path,
                    "mode": "100644",
                    "type": "blob",
                    "sha": serde_json::Value::Null,
                }),
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
    /// Diff-based publish (fabro-4ebd (b)): the merge commit's tree is the
    /// CURRENT base tree overlaid with exactly the run branch's own changes
    /// (`head_entries`, the head side of `compare {base}...{head}`) plus the
    /// unioned tracker files — never a copy of the run's (possibly stale)
    /// head tree. Paths the base gained after the run forked survive
    /// untouched, so the pull request's diff carries only the run's own
    /// committed changes and cannot revert concurrent landed work.
    ///
    /// `Err` carries the human-readable reason for parking: unparseable
    /// JSONL, an unreadable side, a failed git data API call, or a rejected
    /// push.
    pub(super) async fn resolve_tracker_conflict(
        &self,
        base_ref: &str,
        head_ref: &str,
        paths: &[String],
        head_entries: &[CompareEntry],
    ) -> Result<(), String> {
        self.resolve_tracker_conflict_inner(base_ref, head_ref, paths, head_entries)
            .await
            .map_err(|error| format!("paths [{}]: {error:#}", paths.join(", ")))
    }

    async fn resolve_tracker_conflict_inner(
        &self,
        base_ref: &str,
        head_ref: &str,
        paths: &[String],
        head_entries: &[CompareEntry],
    ) -> anyhow::Result<()> {
        let tracker_path_set: std::collections::HashSet<&str> =
            paths.iter().map(String::as_str).collect();

        // The unioned tracker files: for conflicting tracker paths the union
        // replaces whatever the run branch's own change would carry.
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

        let (head_sha, _) = self
            .commit_info(head_ref)
            .await
            .context("reading head commit")?;
        // The merge tree grows from the CURRENT base tree, not the run's
        // head tree: base-only paths survive verbatim (fabro-4ebd (b)).
        let (base_sha, base_tree) = self
            .commit_info(base_ref)
            .await
            .context("reading base commit")?;

        let mut staged: Vec<StagedTreeEntry> = Vec::new();
        for change in run_scoped_tree_changes(head_entries) {
            match change {
                TreeChange::Set(path) => {
                    if tracker_path_set.contains(path.as_str()) {
                        // Replaced by the unioned tracker content below.
                        continue;
                    }
                    let content = self
                        .file_content(&path, head_ref)
                        .await
                        .with_context(|| format!("reading run-authored {path} at {head_ref}"))?;
                    let blob = self
                        .create_blob(&content)
                        .await
                        .with_context(|| format!("staging run-authored {path}"))?;
                    staged.push(StagedTreeEntry::SetBlob { path, sha: blob });
                }
                // A tracker file the run itself removed keeps the removal
                // (the union would resurrect a deliberately deleted file);
                // any other removal is the run's own committed change.
                TreeChange::Remove(path) => {
                    staged.push(StagedTreeEntry::Remove { path });
                }
            }
        }
        for (path, content) in &merged_files {
            let blob = self
                .create_blob(content)
                .await
                .with_context(|| format!("storing merged {path}"))?;
            staged.push(StagedTreeEntry::SetBlob {
                path: path.clone(),
                sha:  blob,
            });
        }
        let tree = self.create_tree(&base_tree, &staged).await?;
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

    /// Newest `updatedAt` wins on BOTH sides (fabro-4ebd): the branch's
    /// stale snapshot must not clobber newer base body updates or
    /// assignments — the exact loss seen in the incident.
    #[test]
    fn union_prefers_newer_updated_at_on_both_sides() {
        let base = concat!(
            "{\"id\":\"fabro-1\",\"status\":\"open\",\"updatedAt\":\"2026-09-17T16:00:00Z\"}\n",
            "{\"id\":\"fabro-2\",\"status\":\"open\",\"updatedAt\":\"2026-09-17T16:00:00Z\"}\n",
        );
        let head = concat!(
            "{\"id\":\"fabro-1\",\"status\":\"open\",\"updatedAt\":\"2026-09-17T11:00:00Z\"}\n",
            "{\"id\":\"fabro-2\",\"status\":\"in_progress\",\"updatedAt\":\"2026-09-17T17:00:00Z\"}\n",
        );

        let merged = union_jsonl_closed_wins(base, head).expect("union should succeed");
        let lines: Vec<&str> = merged.lines().collect();
        assert_eq!(lines, vec![
            // Newer base record survives the branch's stale snapshot.
            "{\"id\":\"fabro-1\",\"status\":\"open\",\"updatedAt\":\"2026-09-17T16:00:00Z\"}",
            // Newer head (branch) record survives an older base copy.
            "{\"id\":\"fabro-2\",\"status\":\"in_progress\",\"updatedAt\":\"2026-09-17T17:00:00Z\"}",
        ]);
    }

    /// A side with a timestamp beats a side without one; a closed base record
    /// still always wins — a run branch must not resurrect a closed record,
    /// whatever the timestamps say.
    #[test]
    fn union_closed_base_wins_even_against_newer_head_and_timestamps_beat_none() {
        let base = concat!(
            "{\"id\":\"fabro-1\",\"status\":\"closed\",\"updatedAt\":\"2026-09-17T10:00:00Z\"}\n",
            "{\"id\":\"fabro-2\",\"status\":\"open\"}\n",
        );
        let head = concat!(
            "{\"id\":\"fabro-1\",\"status\":\"open\",\"updatedAt\":\"2026-09-17T18:00:00Z\"}\n",
            "{\"id\":\"fabro-2\",\"status\":\"open\",\"updatedAt\":\"2026-09-17T12:00:00Z\"}\n",
        );

        let merged = union_jsonl_closed_wins(base, head).expect("union should succeed");
        let lines: Vec<&str> = merged.lines().collect();
        assert_eq!(lines, vec![
            "{\"id\":\"fabro-1\",\"status\":\"closed\",\"updatedAt\":\"2026-09-17T10:00:00Z\"}",
            "{\"id\":\"fabro-2\",\"status\":\"open\",\"updatedAt\":\"2026-09-17T12:00:00Z\"}",
        ]);
    }

    /// Diff-based publish (fabro-4ebd (b)): the merge tree changes cover
    /// EXACTLY the run's own changed paths. A file the base gained after the
    /// run forked is not in the head-side entries, so it gets no change and
    /// keeps its base-tree content — the run cannot delete what it never
    /// touched.
    #[test]
    fn run_scoped_tree_changes_cover_only_the_runs_own_paths() {
        let head_entries = vec![
            CompareEntry {
                filename:          ".seeds/issues.jsonl".to_string(),
                status:            "modified".to_string(),
                previous_filename: None,
            },
            CompareEntry {
                filename:          "lib/foo.rs".to_string(),
                status:            "added".to_string(),
                previous_filename: None,
            },
        ];

        let changes = run_scoped_tree_changes(&head_entries);
        assert_eq!(changes, vec![
            TreeChange::Set(".seeds/issues.jsonl".to_string()),
            TreeChange::Set("lib/foo.rs".to_string()),
        ]);
        // The base's newer `lib/bar.rs` (absent from the run's own changes)
        // has no entry here — it survives via the base tree in create_tree.
        assert!(!changes.iter().any(|change| match change {
            TreeChange::Set(path) | TreeChange::Remove(path) => path == "lib/bar.rs",
        }));
    }

    /// Removals and renames the run itself committed carry through as tree
    /// deletions; a rename also removes its previous path.
    #[test]
    fn run_scoped_tree_changes_map_removals_and_renames() {
        let head_entries = vec![
            CompareEntry {
                filename:          "docs/old.md".to_string(),
                status:            "removed".to_string(),
                previous_filename: None,
            },
            CompareEntry {
                filename:          "docs/new.md".to_string(),
                status:            "renamed".to_string(),
                previous_filename: Some("docs/previous.md".to_string()),
            },
        ];

        assert_eq!(run_scoped_tree_changes(&head_entries), vec![
            TreeChange::Remove("docs/old.md".to_string()),
            TreeChange::Remove("docs/previous.md".to_string()),
            TreeChange::Set("docs/new.md".to_string()),
        ]);
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
