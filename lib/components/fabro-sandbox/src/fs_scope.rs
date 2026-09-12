//! Per-stage filesystem scope (fabro-ba96, ADR-0009 stage envelope).
//!
//! [`FsScope`] compiles a node's `fs_hide`/`fs_write` glob lists into one
//! policy. Enforcement lives in the agent's tool layer: the file, search,
//! and listing tools consult the session's scope before touching the
//! sandbox, and `apply_patch` pre-checks every target of a patch. All
//! builtin tool vocabularies share the same native executors, so one seam
//! covers every profile; spawned subagent sessions inherit the scope
//! through their session options.
//!
//! The trust model is drift protection, not adversarial containment
//! (ADR-0009): `shell` and process execution remain the documented escape
//! hatch. Sandbox-level per-stage materialization is deliberately out of
//! scope for v1.

use fabro_util::workspace_glob::{WorkspaceGlob, WorkspaceGlobError};
use sandbox_driver::{DirEntry, GrepMatch};

use crate::Error;

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

/// Compose a listed directory and an entry path into one candidate for
/// scope matching. Absolute entry paths pass through as they are.
fn join_entry_path(dir: &str, entry_path: &str) -> String {
    if entry_path.starts_with('/') {
        return entry_path.to_string();
    }
    let trimmed = dir.trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "." {
        entry_path.to_string()
    } else {
        format!("{trimmed}/{entry_path}")
    }
}

impl FsScope {
    /// Filter directory entries from [`RunSandbox::list_directory`] so
    /// hidden paths do not leak through listings. A `dir/**` hide glob
    /// also removes the `dir` entry itself (nonexistent semantics).
    #[must_use]
    pub fn filter_dir_entries(
        &self,
        working_dir: &str,
        dir: &str,
        entries: Vec<DirEntry>,
    ) -> Vec<DirEntry> {
        entries
            .into_iter()
            .filter(|entry| {
                let candidate = join_entry_path(dir, &entry.path);
                !self.is_path_hidden(working_dir, &candidate)
            })
            .collect()
    }

    /// Filter absolute sandbox paths (glob and walk results) down to the
    /// visible ones.
    #[must_use]
    pub fn filter_paths<I>(&self, working_dir: &str, paths: I) -> Vec<String>
    where
        I: IntoIterator<Item = String>,
    {
        paths
            .into_iter()
            .filter(|path| !self.is_path_hidden(working_dir, path))
            .collect()
    }

    /// Filter grep matches so hidden files do not leak matches. Match
    /// paths are absolute sandbox paths.
    #[must_use]
    pub fn filter_grep_matches(
        &self,
        working_dir: &str,
        matches: Vec<GrepMatch>,
    ) -> Vec<GrepMatch> {
        matches
            .into_iter()
            .filter(|m| !self.is_path_hidden(working_dir, &m.path))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use sandbox_driver::{DirEntry, FileKind, GrepMatch};

    use super::*;

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

    #[test]
    fn hidden_reads_fail_and_hidden_paths_report_absent() {
        let scope = FsScope::try_new(&[".seeds/**"], None).unwrap();
        let error = scope
            .check_read("/workspace", ".seeds/issues.jsonl")
            .expect_err("hidden read denied");
        assert!(
            error.to_string().contains("fs_hide"),
            "unexpected error: {error}"
        );
        assert!(scope.is_path_hidden("/workspace", ".seeds/issues.jsonl"));
        assert!(!scope.is_path_hidden("/workspace", "visible.txt"));

        let absolute = "/workspace/.seeds/edge";
        let error = scope
            .check_read("/workspace", absolute)
            .expect_err("absolute hidden path denied");
        assert!(
            error.to_string().contains("fs_hide"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn dir_glob_hides_the_directory_entry_in_listings() {
        let scope = FsScope::try_new(&[".seeds/**"], None).unwrap();
        let entries = vec![
            DirEntry::new(".seeds", FileKind::Directory),
            DirEntry::new("visible.txt", FileKind::File),
            DirEntry::new(".seeds/issues.jsonl", FileKind::File),
        ];
        let visible = scope.filter_dir_entries("/workspace", ".", entries);
        assert_eq!(visible.len(), 1, "{visible:?}");
        assert_eq!(visible[0].path, "visible.txt");
    }

    #[test]
    fn filter_paths_and_grep_matches_drop_hidden_files() {
        let scope = FsScope::try_new(&["hidden/**"], None).unwrap();
        let paths = vec![
            "/workspace/hidden/secret.rs".to_string(),
            "/workspace/open.rs".to_string(),
        ];
        assert_eq!(scope.filter_paths("/workspace", paths), vec![
            "/workspace/open.rs".to_string()
        ]);

        let matches = vec![
            GrepMatch::new("/workspace/open.rs", 1, "needle"),
            GrepMatch::new("/workspace/hidden/secret.rs", 3, "needle"),
        ];
        let visible = scope.filter_grep_matches("/workspace", matches);
        assert_eq!(visible.len(), 1, "{visible:?}");
        assert_eq!(visible[0].path, "/workspace/open.rs");
    }

    #[test]
    fn write_allowlist_governs_writes() {
        let scope = FsScope::try_new(&[], Some(&["*.go"])).unwrap();
        scope
            .check_write("/workspace", "new.go")
            .expect("listed glob writable");
        scope
            .check_write("/workspace", "keep.go")
            .expect("listed glob writable");
        let error = scope
            .check_write("/workspace", "notes.md")
            .expect_err("unlisted write denied");
        assert!(
            error.to_string().contains("fs_write"),
            "unexpected error: {error}"
        );

        // Reads stay open regardless of the write list.
        scope
            .check_read("/workspace", "notes.md")
            .expect("reads stay open");
    }

    #[test]
    fn empty_write_list_makes_the_stage_read_only() {
        let scope = FsScope::try_new(&[], Some(&[])).unwrap();
        let error = scope
            .check_write("/workspace", "any.txt")
            .expect_err("empty list admits no writes");
        assert!(
            error.to_string().contains("fs_write"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn outside_workspace_writes_are_denied_only_when_a_list_is_set() {
        // Unset write list: outside paths pass through (default-open).
        let open = FsScope::try_new(&[".x/**"], None).unwrap();
        open.check_write("/workspace", "/tmp/scratch.txt")
            .expect("outside writes pass through without fs_write");

        // Set write list: outside paths never match workspace-relative globs.
        let restricted = FsScope::try_new(&[], Some(&["*.go"])).unwrap();
        let error = restricted
            .check_write("/workspace", "/tmp/scratch.txt")
            .expect_err("outside write denied under fs_write");
        assert!(
            error.to_string().contains("fs_write"),
            "unexpected error: {error}"
        );
    }
}
