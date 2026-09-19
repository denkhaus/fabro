/// How much of each output stream a redacted tail keeps by default.
pub const DEFAULT_EXEC_OUTPUT_TAIL_BYTES: usize = 8 * 1024;

/// Where a sandbox's workspace lives, as persisted on the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxWorkspaceLayout {
    pub workspace_root:    String,
    pub repos_root:        String,
    /// The repository checkout and its link in the workspace, when a
    /// repository was cloned.
    pub primary_repo_path: Option<String>,
    pub primary_repo_link: Option<String>,
}

/// Build a redacted `ExecOutputTail` from stdout/stderr text without
/// fabricating a synthetic `ExecResult`. Each stream is redacted, then
/// capped to its newest `max_bytes_per_stream`. Terminal control sequences
/// are not stripped here: command output reaches fabro with them already
/// removed by the driver under [`crate::exec::SandboxExec`]'s output policy.
/// Pass `""` for either stream that isn't relevant. Returns `None` when both
/// streams are empty.
#[must_use]
pub fn redacted_output_tail(
    stdout: &str,
    stderr: &str,
    max_bytes_per_stream: usize,
) -> Option<fabro_types::ExecOutputTail> {
    let (stdout, stdout_truncated) = redacted_tail(stdout, max_bytes_per_stream);
    let (stderr, stderr_truncated) = redacted_tail(stderr, max_bytes_per_stream);
    let tail = fabro_types::ExecOutputTail {
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
    };
    (!tail.is_empty()).then_some(tail)
}

fn redacted_tail(text: &str, max_bytes: usize) -> (Option<String>, bool) {
    if text.is_empty() || max_bytes == 0 {
        return (None, !text.is_empty());
    }

    let redacted = fabro_redact::redact_string(text);
    let truncated = redacted.len() > max_bytes;
    let start = if truncated {
        redacted.floor_char_boundary(redacted.len() - max_bytes)
    } else {
        0
    };
    let tail = redacted[start..].to_string();
    ((!tail.is_empty()).then_some(tail), truncated)
}

/// A regular file discovered inside a sandbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxFile {
    /// Provider-resolved path accepted by sandbox filesystem operations.
    pub path:          String,
    /// `/`-separated path relative to the requested traversal base.
    pub relative_path: String,
    pub size:          u64,
}

pub(crate) fn resolve_path(path: &str, working_dir: &str) -> String {
    if std::path::Path::new(path).is_absolute() {
        path.to_string()
    } else {
        join_sandbox_path(working_dir, path)
    }
}

pub(crate) fn join_sandbox_path(base: &str, relative_path: &str) -> String {
    if relative_path.is_empty() {
        return base.to_string();
    }
    if base.is_empty() {
        return relative_path.to_string();
    }
    if base == "/" {
        return format!("/{relative_path}");
    }
    format!("{}/{relative_path}", base.trim_end_matches('/'))
}

/// Creates the run branch in the sandbox's checkout through the driver's
/// git facet: a new run branches from `HEAD`, a fork from the source run's
/// checkpoint. The branch is created at that base, or moved to it when an
/// earlier attempt already created it.

#[cfg(test)]
mod tests {
    #[test]
    fn sandbox_tracing_events_do_not_log_raw_command_or_stdin_fields() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut failures = Vec::new();
        scan_for_command_tracing(&root, &mut failures);
        assert!(
            failures.is_empty(),
            "raw command/cmd/stdin tracing fields found:\n{}",
            failures.join("\n")
        );
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "unit test performs a small synchronous source scan of local Rust files"
    )]
    fn scan_for_command_tracing(path: &std::path::Path, failures: &mut Vec<String>) {
        for entry in std::fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                scan_for_command_tracing(&path, failures);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            for macro_name in [
                "tracing::trace!",
                "tracing::debug!",
                "tracing::info!",
                "tracing::warn!",
                "tracing::error!",
                "trace!",
                "debug!",
                "info!",
                "warn!",
                "error!",
            ] {
                let mut rest = source.as_str();
                while let Some(idx) = rest.find(macro_name) {
                    let start = source.len() - rest.len() + idx;
                    if start > 0 && source.as_bytes()[start - 1] == b'"' {
                        rest = &source[start + macro_name.len()..];
                        continue;
                    }
                    let Some(call) = tracing_call(&source[start..]) else {
                        break;
                    };
                    if call.contains("command,")
                        || call.contains("command =")
                        || call.contains("cmd,")
                        || call.contains("cmd =")
                        || call.contains("stdin,")
                        || call.contains("stdin =")
                    {
                        failures.push(format!(
                            "{}: {}",
                            path.display(),
                            call.lines().next().unwrap_or(call)
                        ));
                    }
                    rest = &source[start + call.len()..];
                }
            }
        }
    }

    fn tracing_call(source: &str) -> Option<&str> {
        let open = source.find('(')?;
        let mut depth = 0usize;
        for (idx, ch) in source.char_indices().skip(open) {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some(&source[..=idx]);
                    }
                }
                _ => {}
            }
        }
        None
    }
}
