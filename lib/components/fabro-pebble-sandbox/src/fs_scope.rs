//! Per-stage filesystem scope (fabro-aa5f, ADR-0009 rev; carried from the
//! legacy `fabro-sandbox` crate, fabro-ba96).
//!
//! [`FsScope`] compiles a node's `x.fs_hide`/`x.fs_write` glob lists into
//! one policy. On Petri the values ride the run's `graph_source` (the
//! lowering drops `x.*` attributes), and enforcement runs at the seams the
//! platform owns: the create-time fork lints validate the values, and the
//! checkpoint commit denies a stage whose staged files fall outside its
//! `fs_write` scope, so an escape never reaches the run branch. The
//! session-level tool enforcement the ADR's session-build hook describes
//! needs a host capability the pinned Petri revision does not offer; it is
//! filed as an upstream offer, and this type is the policy that hook will
//! parameterize.
//!
//! The trust model is drift protection, not adversarial containment
//! (ADR-0009): `shell` and process execution remain the documented escape
//! hatch. Sandbox-level per-stage materialization is deliberately out of
//! scope.

use fabro_util::workspace_glob::{WorkspaceGlob, WorkspaceGlobError};

/// A compiled per-node filesystem scope.
///
/// Constructed from the `x.fs_hide`/`x.fs_write` node attributes by the
/// workflow layer (fail-closed on invalid globs).
///
/// Semantics (grilling 2026-09-03, ADR-0009 amendment):
///
/// - `fs_hide` entries are workspace-relative globs. A hidden path behaves as
///   if it did not exist for the stage.
/// - `fs_write` (when set) is an allow-list: only matching workspace paths are
///   writable. Paths outside the workspace are denied while a list is set. An
///   empty list admits no write at all (a read-only stage).
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

/// Why a scope check denied an operation. Displays as a sentence fragment
/// about the path (`"path 'x' <fragment>"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
pub enum ScopeDenial {
    /// The path matches a `fs_hide` glob: it behaves as if it did not
    /// exist.
    #[strum(to_string = "is hidden from this stage by fs_hide and behaves as if it did not exist")]
    HiddenByFsHide,
    /// `fs_write` is set and the path is outside its globs (including
    /// paths outside the workspace).
    #[strum(
        to_string = "is not writable by this stage: fs_write is set and the path is outside its globs"
    )]
    OutsideFsWrite,
}

/// One denied operation: the path and why. The checkpoint guard renders
/// the paths; a future tool-boundary hook hands the same value to the
/// model as its tool error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("path '{path}' {denial}")]
pub struct ScopeViolation {
    pub path:   String,
    pub denial: ScopeDenial,
}

fn compile_globs(
    attribute: &'static str,
    entries: &[&str],
) -> Result<Vec<WorkspaceGlob>, FsScopeError> {
    let mut globs = Vec::new();
    for pattern in entries {
        let compiled = |source: &str| {
            WorkspaceGlob::try_new(source).map_err(|source_err| FsScopeError {
                attribute,
                pattern: source.to_string(),
                source: source_err,
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

    /// Write-side check: denies hidden paths always (nonexistent
    /// semantics), and denies everything outside the `fs_write` globs
    /// while a list is set — including paths outside the workspace.
    ///
    /// # Errors
    ///
    /// Returns the [`ScopeViolation`] when the path is hidden or outside
    /// the write allow-list.
    pub fn check_write(&self, working_dir: &str, path: &str) -> Result<(), ScopeViolation> {
        let deny = |denial| ScopeViolation {
            path: path.to_string(),
            denial,
        };
        if let Some(relative) = workspace_relative(working_dir, path) {
            if self.is_hidden(&relative) {
                return Err(deny(ScopeDenial::HiddenByFsHide));
            }
            if let Some(allow) = &self.write_allow {
                if !allow.iter().any(|glob| glob.is_match(&relative)) {
                    return Err(deny(ScopeDenial::OutsideFsWrite));
                }
            }
            return Ok(());
        }
        if self.write_allow.is_some() {
            return Err(deny(ScopeDenial::OutsideFsWrite));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_write_admits_every_write() {
        let scope = FsScope::try_new(&[], None).expect("empty scope compiles");
        assert!(!scope.is_active());
        assert!(scope.check_write("", "anywhere/x.txt").is_ok());
    }

    #[test]
    fn empty_write_list_is_read_only() {
        let scope = FsScope::try_new(&[], Some(&[])).expect("empty lists compile");
        assert!(scope.is_active());
        let error = scope
            .check_write("", "docs/a.md")
            .expect_err("no path is writable");
        assert_eq!(error.denial, ScopeDenial::OutsideFsWrite);
    }

    #[test]
    fn write_allow_list_admits_only_its_globs() {
        let scope = FsScope::try_new(&[], Some(&["docs/**", "README.md"])).expect("globs compile");
        assert!(scope.check_write("", "docs/lab/a.md").is_ok());
        assert!(scope.check_write("", "README.md").is_ok());
        assert!(scope.check_write("", "lib/main.rs").is_err());
    }

    #[test]
    fn hidden_paths_are_never_writable() {
        let scope =
            FsScope::try_new(&[".seeds/**", "lib/**"], Some(&["lib/**"])).expect("globs compile");
        let error = scope
            .check_write("", "lib/main.rs")
            .expect_err("hide wins over write");
        assert_eq!(error.denial, ScopeDenial::HiddenByFsHide);
        assert!(scope.is_hidden(".seeds"));
    }

    #[test]
    fn outside_workspace_writes_denied_only_while_a_list_is_set() {
        let scoped = FsScope::try_new(&[], Some(&["docs/**"])).expect("globs compile");
        assert!(scoped.check_write("", "/abs/outside.txt").is_err());
        let unscoped = FsScope::try_new(&[], None).expect("scope compiles");
        assert!(unscoped.check_write("", "/abs/outside.txt").is_ok());
    }

    #[test]
    fn working_dir_prefix_is_stripped() {
        let scope = FsScope::try_new(&["lib/**"], None).expect("globs compile");
        assert!(scope.is_path_hidden("/workspace/repo", "/workspace/repo/lib/a.rs"));
        assert!(!scope.is_path_hidden("/workspace/repo", "/other/lib/a.rs"));
    }

    #[test]
    fn invalid_glob_fails_compilation_with_its_attribute() {
        let error = FsScope::try_new(&["../escape"], None)
            .expect_err("parent traversal is not a workspace glob");
        assert_eq!(error.attribute(), "fs_hide");
        assert_eq!(error.pattern(), "../escape");
    }
}
