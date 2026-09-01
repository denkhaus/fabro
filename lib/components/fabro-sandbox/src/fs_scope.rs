//! Per-stage filesystem scope (fabro-ba96, ADR-0009 stage envelope).
//!
//! [`FsScope`] compiles a node's `fs_hide`/`fs_write` glob lists into one
//! policy. [`ScopedSandbox`] wraps a stage session's sandbox and enforces
//! that policy on every filesystem operation that flows through the
//! [`Sandbox`] trait, so all builtin file tools — in every provider profile
//! vocabulary — plus `read_many_files` and `apply_patch` are covered by a
//! single seam. Spawned subagent sessions share the same wrapper and inherit
//! the scope.
//!
//! The trust model is drift protection, not adversarial containment
//! (ADR-0009): `shell` and process execution delegate to the inner sandbox
//! untouched and remain the documented escape hatch. Sandbox-level
//! per-stage materialization is deliberately out of scope for v1.

use std::collections::HashMap;
use std::sync::Arc;

use fabro_util::workspace_glob::{WorkspaceGlob, WorkspaceGlobError};
use tokio_util::sync::CancellationToken;

use crate::sandbox::{
    DirEntry, ExecStreamingRequest, ExecStreamingResult, GrepOptions, Sandbox, SandboxActivation,
    SandboxFile, StdioProcess, WalkOptions, grep_result_path,
};
use crate::{Error, GitRunInfo, GitSetupIntent, PushError, PushReport, RefreshOutcome, RetryPlan};

/// A compiled per-node filesystem scope.
///
/// Constructed from the `fs_hide`/`fs_write` node attributes by the workflow
/// layer (fail-closed on invalid globs) and shared between the
/// [`ScopedSandbox`] wrapper and the tool layer's `apply_patch` pre-check.
///
/// Semantics (grilling 2026-09-03, ADR-0009 amendment):
///
/// - `fs_hide` entries are workspace-relative globs. A hidden path behaves as
///   if it did not exist for the stage: reads fail with a scope error,
///   `file_exists` reports `false`, discovery results are filtered, and
///   writes/deletes are denied. A `dir/**` glob also hides the `dir` entry
///   itself, so the directory's existence does not leak through listings.
/// - `fs_write` (when set) is an allow-list: only matching workspace paths are
///   writable. Paths outside the workspace are denied while a list is set. An
///   empty list admits no write at all (a read-only stage).
/// - Reads outside the workspace pass through — the default-open posture; the
///   hide list only governs workspace-relative paths.
#[derive(Clone, Debug)]
pub struct FsScope {
    hidden:      Vec<WorkspaceGlob>,
    write_allow: Option<Vec<WorkspaceGlob>>,
}

/// Compilation failure for one glob entry of a scope attribute.
#[derive(Debug, thiserror::Error)]
#[error("invalid glob in {attribute} '{pattern}'")]
pub struct FsScopeError {
    attribute: &'static str,
    pattern:   String,
    #[source]
    source:    WorkspaceGlobError,
}

impl FsScopeError {
    /// The attribute (`fs_hide`/`fs_write`) the failing entry came from.
    #[must_use]
    pub fn attribute(&self) -> &'static str {
        self.attribute
    }

    /// The raw glob entry that failed to compile.
    #[must_use]
    pub fn pattern(&self) -> &str {
        &self.pattern
    }
}

/// Why a scope check denied an operation. Rendered into the model-facing
/// tool error; the variants Display as sentence fragments about the path
/// (`"path 'x' <fragment>"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
pub enum ScopeDenial {
    /// The path matches a `fs_hide` glob: it behaves as if it did not exist.
    #[strum(to_string = "is hidden from this stage by fs_hide and behaves as if it did not exist")]
    HiddenByFsHide,
    /// `fs_write` is set and the path is outside its globs (including paths
    /// outside the workspace).
    #[strum(
        to_string = "is not writable by this stage: fs_write is set and the path is outside its globs"
    )]
    OutsideFsWrite,
}

fn compile_globs(
    attribute: &'static str,
    entries: &[&str],
) -> Result<Vec<WorkspaceGlob>, FsScopeError> {
    let mut globs = Vec::new();
    for pattern in entries {
        let compiled = |source: &str| {
            WorkspaceGlob::try_new(source).map_err(|error| FsScopeError {
                attribute,
                pattern: source.to_string(),
                source: error,
            })
        };
        globs.push(compiled(pattern)?);
        // A `dir/**` glob also hides the directory entry itself: the
        // intent of `.seeds/**` is "the whole tree is invisible", and a
        // listing that still shows an (empty) `.seeds` directory would
        // leak its existence.
        if let Some(prefix) = pattern.strip_suffix("/**") {
            if !prefix.is_empty() {
                globs.push(compiled(prefix)?);
            }
        }
    }
    Ok(globs)
}

impl FsScope {
    /// Compile a scope from raw attribute entries. `write` is `None` when
    /// `fs_write` is unset (every write admitted) and `Some` for an
    /// explicit list, which may be empty (no write admitted).
    ///
    /// # Errors
    ///
    /// Returns the first non-compiling entry as an [`FsScopeError`].
    pub fn try_new(hide: &[&str], write: Option<&[&str]>) -> Result<Self, FsScopeError> {
        let hidden = compile_globs("fs_hide", hide)?;
        let write_allow = match write {
            None => None,
            Some(entries) => Some(compile_globs("fs_write", entries)?),
        };
        Ok(Self {
            hidden,
            write_allow,
        })
    }

    /// Whether this scope restricts anything. A scope with no hide entries
    /// and unset `fs_write` is inert.
    #[must_use]
    pub fn is_active(&self) -> bool {
        !self.hidden.is_empty() || self.write_allow.is_some()
    }

    /// Whether the workspace-relative `relative` matches a hide glob.
    #[must_use]
    pub fn is_hidden(&self, relative: &str) -> bool {
        self.hidden.iter().any(|glob| glob.is_match(relative))
    }

    /// Whether `path` (resolved against `working_dir`) is hidden. Paths
    /// outside the workspace are never hidden.
    #[must_use]
    pub fn is_path_hidden(&self, working_dir: &str, path: &str) -> bool {
        workspace_relative(working_dir, path).is_some_and(|relative| self.is_hidden(&relative))
    }

    /// Read-side check: denies hidden workspace paths; outside-workspace
    /// reads pass through (default-open).
    ///
    /// # Errors
    ///
    /// Returns [`Error::StageFsScope`] when the path is hidden.
    pub fn check_read(&self, working_dir: &str, path: &str) -> crate::Result<()> {
        if self.is_path_hidden(working_dir, path) {
            return Err(Error::StageFsScope {
                path:   path.to_string(),
                reason: ScopeDenial::HiddenByFsHide,
            });
        }
        Ok(())
    }

    /// Write-side check: denies hidden paths always (nonexistent
    /// semantics), and denies everything outside the `fs_write` globs
    /// while a list is set — including paths outside the workspace.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StageFsScope`] when the path is hidden or outside
    /// the write allow-list.
    pub fn check_write(&self, working_dir: &str, path: &str) -> crate::Result<()> {
        if let Some(relative) = workspace_relative(working_dir, path) {
            if self.is_hidden(&relative) {
                return Err(Error::StageFsScope {
                    path:   path.to_string(),
                    reason: ScopeDenial::HiddenByFsHide,
                });
            }
            if let Some(allow) = &self.write_allow {
                if !allow.iter().any(|glob| glob.is_match(&relative)) {
                    return Err(Error::StageFsScope {
                        path:   path.to_string(),
                        reason: ScopeDenial::OutsideFsWrite,
                    });
                }
            }
            return Ok(());
        }
        if self.write_allow.is_some() {
            return Err(Error::StageFsScope {
                path:   path.to_string(),
                reason: ScopeDenial::OutsideFsWrite,
            });
        }
        Ok(())
    }
}

/// Map `path` onto the workspace-relative form used by scope globs.
///
/// Absolute paths under `working_dir` are stripped of the prefix (the
/// workspace root itself maps to the empty string). Absolute paths
/// elsewhere map to `None` (outside the workspace). Relative paths pass
/// through; [`WorkspaceGlob::is_match`] normalizes candidates.
fn workspace_relative(working_dir: &str, path: &str) -> Option<String> {
    if !path.starts_with('/') {
        return Some(path.to_string());
    }
    let dir = working_dir.trim_end_matches('/');
    if path == dir {
        return Some(String::new());
    }
    path.strip_prefix(&format!("{dir}/")).map(str::to_string)
}

/// A [`Sandbox`] decorator enforcing an [`FsScope`] on every filesystem
/// operation (fabro-ba96).
///
/// Hand-written instead of [`crate::delegate_sandbox!`] because the macro
/// emits unconditional delegations and cannot skip methods this decorator
/// overrides; revisit the macro when a second decorator appears.
///
/// Shell and process execution (`exec_command`,
/// `exec_command_streaming`, `spawn_stdio_process`) delegate to the inner
/// sandbox untouched: the scope is drift protection, and shell is the
/// documented escape hatch (ADR-0009 trust model). Run-level plumbing
/// (git setup/push, upload/download) also delegates — the wrapper is
/// handed only to stage sessions, never to the run's own sandbox handle.
pub struct ScopedSandbox {
    inner: Arc<dyn Sandbox>,
    scope: Arc<FsScope>,
}

impl ScopedSandbox {
    /// Wrap `inner` so its filesystem surface follows `scope`.
    #[must_use]
    pub fn new(inner: Arc<dyn Sandbox>, scope: Arc<FsScope>) -> Self {
        Self { inner, scope }
    }

    fn working_dir(&self) -> &str {
        self.inner.working_directory()
    }
}

/// Compose a listed directory and an entry name into one path for scope
/// matching. `DirEntry::name` is already relative to the listed directory
/// with `/` separators, including recursive depth listings.
fn join_entry_path(dir: &str, name: &str) -> String {
    let trimmed = dir.trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "." {
        name.to_string()
    } else {
        format!("{trimmed}/{name}")
    }
}

#[async_trait::async_trait]
impl Sandbox for ScopedSandbox {
    async fn read_file_bytes(&self, path: &str) -> crate::Result<Vec<u8>> {
        self.scope.check_read(self.working_dir(), path)?;
        self.inner.read_file_bytes(path).await
    }

    async fn write_file(&self, path: &str, content: &str) -> crate::Result<()> {
        self.scope.check_write(self.working_dir(), path)?;
        self.inner.write_file(path, content).await
    }

    async fn delete_file(&self, path: &str) -> crate::Result<()> {
        self.scope.check_write(self.working_dir(), path)?;
        self.inner.delete_file(path).await
    }

    async fn file_exists(&self, path: &str) -> crate::Result<bool> {
        if self.scope.is_path_hidden(self.working_dir(), path) {
            return Ok(false);
        }
        self.inner.file_exists(path).await
    }

    async fn list_directory(
        &self,
        path: &str,
        depth: Option<usize>,
    ) -> crate::Result<Vec<DirEntry>> {
        let entries = self.inner.list_directory(path, depth).await?;
        Ok(entries
            .into_iter()
            .filter(|entry| {
                let candidate = join_entry_path(path, &entry.name);
                !self.scope.is_path_hidden(self.working_dir(), &candidate)
            })
            .collect())
    }

    async fn grep(
        &self,
        pattern: &str,
        path: &str,
        options: &GrepOptions,
    ) -> crate::Result<Vec<String>> {
        self.scope.check_read(self.working_dir(), path)?;
        let lines = self.inner.grep(pattern, path, options).await?;
        Ok(lines
            .into_iter()
            .filter(|line| {
                let file = grep_result_path(line, path);
                !self.scope.is_path_hidden(self.working_dir(), file)
            })
            .collect())
    }

    async fn glob(&self, pattern: &str, path: Option<&str>) -> crate::Result<Vec<String>> {
        let results = self.inner.glob(pattern, path).await?;
        Ok(results
            .into_iter()
            .filter(|result| !self.scope.is_path_hidden(self.working_dir(), result))
            .collect())
    }

    async fn walk_files(
        &self,
        base: &str,
        relative_start: &str,
        options: &WalkOptions,
    ) -> crate::Result<Vec<SandboxFile>> {
        let files = self.inner.walk_files(base, relative_start, options).await?;
        Ok(files
            .into_iter()
            .filter(|file| !self.scope.is_path_hidden(self.working_dir(), &file.path))
            .collect())
    }

    // Everything below delegates to the inner sandbox.

    async fn exec_command(
        &self,
        command: &str,
        timeout_ms: u64,
        working_dir: Option<&str>,
        env_vars: Option<&HashMap<String, String>>,
        cancel_token: Option<CancellationToken>,
    ) -> crate::Result<crate::ExecResult> {
        self.inner
            .exec_command(command, timeout_ms, working_dir, env_vars, cancel_token)
            .await
    }

    async fn exec_command_streaming(
        &self,
        request: ExecStreamingRequest<'_>,
    ) -> crate::Result<ExecStreamingResult> {
        self.inner.exec_command_streaming(request).await
    }

    async fn spawn_stdio_process(
        &self,
        command: &str,
        working_dir: Option<&str>,
        env_vars: Option<&HashMap<String, String>>,
        cancel_token: Option<CancellationToken>,
    ) -> crate::Result<StdioProcess> {
        self.inner
            .spawn_stdio_process(command, working_dir, env_vars, cancel_token)
            .await
    }

    async fn download_file_to_local(
        &self,
        remote_path: &str,
        local_path: &std::path::Path,
    ) -> crate::Result<()> {
        self.inner
            .download_file_to_local(remote_path, local_path)
            .await
    }

    async fn upload_file_from_local(
        &self,
        local_path: &std::path::Path,
        remote_path: &str,
    ) -> crate::Result<()> {
        self.inner
            .upload_file_from_local(local_path, remote_path)
            .await
    }

    async fn initialize(&self) -> crate::Result<()> {
        self.inner.initialize().await
    }

    async fn activate(&self) -> crate::Result<SandboxActivation> {
        self.inner.activate().await
    }

    async fn start(&self) -> crate::Result<()> {
        self.inner.start().await
    }

    async fn stop(&self) -> crate::Result<()> {
        self.inner.stop().await
    }

    async fn delete(&self) -> crate::Result<()> {
        self.inner.delete().await
    }

    async fn cleanup(&self) -> crate::Result<()> {
        self.inner.cleanup().await
    }

    fn working_directory(&self) -> &str {
        self.inner.working_directory()
    }

    fn platform(&self) -> &str {
        self.inner.platform()
    }

    fn os_version(&self) -> String {
        self.inner.os_version()
    }

    fn sandbox_info(&self) -> String {
        self.inner.sandbox_info()
    }

    fn snapshot_info(&self) -> Option<String> {
        self.inner.snapshot_info()
    }

    async fn refresh_push_credentials(&self) -> crate::Result<RefreshOutcome> {
        self.inner.refresh_push_credentials().await
    }

    async fn set_autostop_interval(&self, minutes: i32) -> crate::Result<()> {
        self.inner.set_autostop_interval(minutes).await
    }

    async fn setup_git(&self, intent: &GitSetupIntent) -> crate::Result<Option<GitRunInfo>> {
        self.inner.setup_git(intent).await
    }

    async fn git_push_ref(&self, refspec: &str, plan: &RetryPlan) -> Result<PushReport, PushError> {
        self.inner.git_push_ref(refspec, plan).await
    }

    async fn ssh_access_command(&self) -> crate::Result<Option<String>> {
        self.inner.ssh_access_command().await
    }

    async fn get_preview_url(
        &self,
        port: u16,
    ) -> crate::Result<Option<(String, HashMap<String, String>)>> {
        self.inner.get_preview_url(port).await
    }

    fn origin_url(&self) -> Option<&str> {
        self.inner.origin_url()
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "scope tests stage fixtures with sync std::fs writes/reads"
)]
#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use super::*;
    use crate::local::LocalSandbox;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fs_scope_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn scoped(dir: PathBuf, hide: &[&str], write: Option<&[&str]>) -> ScopedSandbox {
        let scope = Arc::new(FsScope::try_new(hide, write).expect("globs compile"));
        ScopedSandbox::new(Arc::new(LocalSandbox::new(dir)) as Arc<dyn Sandbox>, scope)
    }

    #[test]
    fn try_new_rejects_invalid_globs_with_the_attribute() {
        let error = FsScope::try_new(&["../*"], None).expect_err("parent traversal rejected");
        assert_eq!(error.attribute(), "fs_hide");
        assert_eq!(error.pattern(), "../*");

        let error = FsScope::try_new(&[], Some(&["/abs"])).expect_err("absolute rejected");
        assert_eq!(error.attribute(), "fs_write");
        assert_eq!(error.pattern(), "/abs");
    }

    #[test]
    fn inactive_when_nothing_is_declared() {
        assert!(!FsScope::try_new(&[], None).unwrap().is_active());
        assert!(FsScope::try_new(&[".seeds/**"], None).unwrap().is_active());
        assert!(FsScope::try_new(&[], Some(&[])).unwrap().is_active());
    }

    #[test]
    fn is_hidden_matches_workspace_relative_globs() {
        let scope = FsScope::try_new(&[".fabro/**", "*.env"], None).unwrap();
        assert!(scope.is_hidden(".fabro/workflow.toml"));
        assert!(scope.is_hidden(".env"));
        assert!(!scope.is_hidden("src/main.rs"));
        // A dir/** glob also hides the directory entry itself.
        assert!(scope.is_hidden(".fabro"));
        // Lone ** hides every path, with no extra prefix entry.
        let scope = FsScope::try_new(&["**"], None).unwrap();
        assert!(scope.is_hidden("anything/at/all"));
    }

    #[test]
    fn workspace_relative_strips_the_working_dir_prefix() {
        assert_eq!(
            workspace_relative("/workspace", "/workspace/src/main.rs"),
            Some("src/main.rs".to_string())
        );
        assert_eq!(
            workspace_relative("/workspace", "/workspace"),
            Some(String::new())
        );
        assert_eq!(workspace_relative("/workspace", "/etc/passwd"), None);
        assert_eq!(
            workspace_relative("/workspace", "src/main.rs"),
            Some("src/main.rs".to_string())
        );
    }

    #[tokio::test]
    async fn hidden_files_behave_as_if_nonexistent() {
        let dir = temp_dir();
        std::fs::write(dir.join("visible.txt"), "keep").unwrap();
        std::fs::create_dir_all(dir.join(".seeds")).unwrap();
        std::fs::write(dir.join(".seeds/issues.jsonl"), "{}").unwrap();
        let sandbox = scoped(dir.clone(), &[".seeds/**"], None);

        // Reads fail with the scope error.
        let error = sandbox
            .read_file_text(".seeds/issues.jsonl")
            .await
            .expect_err("hidden read denied");
        assert!(
            error.to_string().contains("fs_hide"),
            "unexpected error: {error}"
        );

        // file_exists reports false — the path does not exist for the stage.
        assert!(!sandbox.file_exists(".seeds/issues.jsonl").await.unwrap());
        assert!(sandbox.file_exists("visible.txt").await.unwrap());

        // Discovery results are filtered, so hiding does not leak.
        let entries = sandbox.list_directory(".", None).await.unwrap();
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].name, "visible.txt");

        let globbed = sandbox.glob("**", None).await.unwrap();
        assert_eq!(globbed.len(), 1, "{globbed:?}");
        assert!(globbed[0].ends_with("visible.txt"));

        let grepped = sandbox
            .grep("{}", ".", &GrepOptions::default())
            .await
            .unwrap();
        assert!(
            grepped.is_empty(),
            "hidden matches must not leak: {grepped:?}"
        );

        // Writes to hidden paths are denied too.
        let error = sandbox
            .write_file(".seeds/new.jsonl", "x")
            .await
            .expect_err("hidden write denied");
        assert!(
            error.to_string().contains("fs_hide"),
            "unexpected error: {error}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn grep_filters_hidden_matches_but_keeps_visible_ones() {
        let dir = temp_dir();
        std::fs::write(dir.join("open.rs"), "needle\n").unwrap();
        std::fs::create_dir_all(dir.join("hidden")).unwrap();
        std::fs::write(dir.join("hidden/secret.rs"), "needle\n").unwrap();
        let sandbox = scoped(dir.clone(), &["hidden/**"], None);

        let lines = sandbox
            .grep("needle", ".", &GrepOptions::default())
            .await
            .unwrap();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("open.rs"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn write_allowlist_governs_writes_and_deletes() {
        let dir = temp_dir();
        std::fs::write(dir.join("keep.go"), "package main").unwrap();
        std::fs::write(dir.join("notes.md"), "hi").unwrap();
        let sandbox = scoped(dir.clone(), &[], Some(&["*.go"]));

        sandbox
            .write_file("new.go", "package x")
            .await
            .expect("listed glob writable");
        sandbox
            .write_existing_file("keep.go", "package main\n")
            .await
            .expect("listed glob writable via write_existing_file");
        let error = sandbox
            .write_file("notes.md", "changed")
            .await
            .expect_err("unlisted write denied");
        assert!(
            error.to_string().contains("fs_write"),
            "unexpected error: {error}"
        );
        let error = sandbox
            .delete_file("notes.md")
            .await
            .expect_err("delete counts as a write");
        assert!(
            error.to_string().contains("fs_write"),
            "unexpected error: {error}"
        );

        // Reads stay open regardless of the write list.
        sandbox
            .read_file_text("notes.md")
            .await
            .expect("reads stay open");
        assert_eq!(std::fs::read_to_string(dir.join("notes.md")).unwrap(), "hi");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn empty_write_list_makes_the_stage_read_only() {
        let dir = temp_dir();
        std::fs::write(dir.join("any.txt"), "x").unwrap();
        let sandbox = scoped(dir.clone(), &[], Some(&[]));

        let error = sandbox
            .write_file("any.txt", "y")
            .await
            .expect_err("empty list admits no writes");
        assert!(
            error.to_string().contains("fs_write"),
            "unexpected error: {error}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn outside_workspace_writes_are_denied_only_when_a_list_is_set() {
        let dir = temp_dir();
        let outside = std::env::temp_dir().join(format!("fs_scope_out_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&outside).unwrap();

        // Unset write list: outside paths pass through (default-open).
        let open = scoped(dir.clone(), &[".x/**"], None);
        open.write_file(&outside.join("scratch.txt").to_string_lossy(), "ok")
            .await
            .expect("outside writes pass through without fs_write");

        // Set write list: outside paths never match workspace-relative globs.
        let restricted = scoped(dir, &[], Some(&["*.go"]));
        let error = restricted
            .write_file(&outside.join("scratch2.txt").to_string_lossy(), "no")
            .await
            .expect_err("outside write denied under fs_write");
        assert!(
            error.to_string().contains("fs_write"),
            "unexpected error: {error}"
        );
        std::fs::remove_dir_all(&outside).unwrap();
    }

    #[tokio::test]
    async fn absolute_paths_inside_the_workspace_are_scoped() {
        let dir = temp_dir();
        std::fs::create_dir_all(dir.join(".fabro")).unwrap();
        std::fs::write(dir.join(".fabro/secret"), "s").unwrap();
        let sandbox = scoped(dir.clone(), &[".fabro/**"], None);

        let absolute = dir.join(".fabro/secret").to_string_lossy().into_owned();
        let error = sandbox
            .read_file_text(&absolute)
            .await
            .expect_err("absolute hidden path denied");
        assert!(
            error.to_string().contains("fs_hide"),
            "unexpected error: {error}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn exec_command_delegates_untouched() {
        let dir = temp_dir();
        let sandbox = scoped(dir.clone(), &["**"], Some(&[]));
        let result = sandbox
            .exec_command("printf scoped", 5_000, None, None, None)
            .await
            .expect("shell is the documented escape hatch");
        assert!(result.is_success());
        assert_eq!(result.stdout.trim(), "scoped");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
