use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::common::{FabroToolBackend, ToolError, ToolResult};

/// Log severity filter, ordered ascending by severity: `Trace` < `Debug`
/// < `Info` < `Warn` < `Error`. A filter keeps every line whose own
/// severity is at least the requested one, so `Warn` (the default) covers
/// the warnings-and-errors view an analyst usually needs.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RunLogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl RunLogLevel {
    fn from_token(token: &str) -> Option<Self> {
        match token.to_ascii_uppercase().as_str() {
            "ERROR" => Some(Self::Error),
            "WARN" => Some(Self::Warn),
            "INFO" => Some(Self::Info),
            "DEBUG" => Some(Self::Debug),
            "TRACE" => Some(Self::Trace),
            _ => None,
        }
    }
}

impl std::fmt::Display for RunLogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        };
        f.write_str(name)
    }
}

const DEFAULT_TAIL: usize = 200;
const MAX_TAIL: usize = 1000;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FabroRunLogsParams {
    /// Run id (or unique prefix) whose persisted worker log to read.
    pub run_id: String,
    /// Minimum severity to keep; defaults to `warn`.
    pub level:  Option<RunLogLevel>,
    /// How many of the NEWEST matching lines to return (default 200, max
    /// 1000). Older matching lines are summarized in `total_matching`.
    pub tail:   Option<usize>,
}

#[derive(Debug)]
pub struct ValidatedRunLogs {
    pub raw:   FabroRunLogsParams,
    pub level: RunLogLevel,
    pub tail:  usize,
}

impl TryFrom<FabroRunLogsParams> for ValidatedRunLogs {
    type Error = ToolError;

    fn try_from(params: FabroRunLogsParams) -> Result<Self, Self::Error> {
        if params.run_id.trim().is_empty() {
            return Err(ToolError::message("run_id is required"));
        }
        let tail = params.tail.unwrap_or(DEFAULT_TAIL);
        if tail == 0 || tail > MAX_TAIL {
            return Err(ToolError::message(format!(
                "tail must be between 1 and {MAX_TAIL}"
            )));
        }
        Ok(Self {
            level: params.level.unwrap_or(RunLogLevel::Warn),
            tail,
            raw: params,
        })
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RunLogsResult {
    pub run_id:         String,
    pub level:          RunLogLevel,
    pub returned:       usize,
    pub total_matching: usize,
    pub truncated:      bool,
    pub lines:          Vec<String>,
}

/// Read the persisted worker tracing log of a run, filtered by severity.
///
/// Lines are redacted before they leave the tool: tracing output is
/// developer diagnostics and may carry credentials a structured event
/// would never persist.
pub async fn run_logs(
    backend: Arc<dyn FabroToolBackend>,
    params: ValidatedRunLogs,
) -> ToolResult<RunLogsResult> {
    let level = params.level;
    let tail = params.tail;
    let run_id = backend
        .resolve_run(&params.raw.run_id)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))?
        .id;
    let bytes = backend
        .get_run_logs(&run_id)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))?
        .ok_or_else(|| ToolError::message("no persisted log available for this run"))?;
    let text = String::from_utf8_lossy(&bytes);

    let matched = filter_lines(&text, level)
        .into_iter()
        .map(|line| fabro_redact::redact_string(&line))
        .collect::<Vec<_>>();
    let total_matching = matched.len();
    let lines = if matched.len() > tail {
        matched[matched.len() - tail..].to_vec()
    } else {
        matched
    };
    let returned = lines.len();
    Ok(RunLogsResult {
        run_id: run_id.to_string(),
        level,
        truncated: total_matching > returned,
        total_matching,
        returned,
        lines,
    })
}

#[must_use]
pub fn run_logs_text(result: &RunLogsResult) -> String {
    format!(
        "returned {} of {} matching log line(s) at level {} or above",
        result.returned, result.total_matching, result.level
    )
}

/// Keep lines whose severity is at least `level`. Continuation lines (no
/// tracing header) inherit the previous header line's severity so
/// multi-line panics and stack traces stay attached to their error.
fn filter_lines(text: &str, level: RunLogLevel) -> Vec<String> {
    let mut kept = Vec::new();
    let mut current = RunLogLevel::Trace;
    for line in text.lines() {
        current = severity_of_header(line).unwrap_or(current);
        if current >= level {
            kept.push(line.to_string());
        }
    }
    kept
}

/// Extract the severity from a tracing header line
/// (`2026-08-27T13:33:01.461981Z  WARN run{...}: target: message`);
/// `None` for lines without a header. The timestamp may carry fractional
/// seconds, so the level is the first whitespace token AFTER a
/// timestamp-shaped first token.
fn severity_of_header(line: &str) -> Option<RunLogLevel> {
    let mut tokens = line.split_whitespace();
    let stamp = tokens.next()?.as_bytes();
    if stamp.len() < 20 || stamp[10] != b'T' {
        return None;
    }
    RunLogLevel::from_token(tokens.next().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "2026-08-27T13:33:01.461981Z  INFO run{id=1}: setup: sandbox ready
2026-08-27T13:33:02.100000Z  WARN run{id=1}: write_locks: concurrent write; serializing
2026-08-27T13:33:03.200000Z ERROR run{id=1}: pipeline: PR content generation failed
panic:
stack frame one
2026-08-27T13:33:04.300000Z DEBUG run{id=1}: cache: hit
";

    #[test]
    fn validation_rejects_empty_run_id_and_tail_bounds() {
        let error = ValidatedRunLogs::try_from(FabroRunLogsParams {
            run_id: "  ".to_string(),
            level:  None,
            tail:   None,
        })
        .unwrap_err();
        assert_eq!(error.as_str(), "run_id is required");

        let error = ValidatedRunLogs::try_from(FabroRunLogsParams {
            run_id: "01M".to_string(),
            level:  None,
            tail:   Some(MAX_TAIL + 1),
        })
        .unwrap_err();
        assert!(error.as_str().contains("tail"));
    }

    #[test]
    fn defaults_are_warn_level_and_200_tail() {
        let validated = ValidatedRunLogs::try_from(FabroRunLogsParams {
            run_id: "01M".to_string(),
            level:  None,
            tail:   None,
        })
        .unwrap();
        assert_eq!(validated.level, RunLogLevel::Warn);
        assert_eq!(validated.tail, DEFAULT_TAIL);
    }

    #[test]
    fn warn_filter_keeps_warn_error_and_error_continuations() {
        let kept = filter_lines(LOG, RunLogLevel::Warn);
        assert_eq!(kept.len(), 4);
        assert!(kept.iter().any(|line| line.contains("concurrent write")));
        assert!(
            kept.iter()
                .any(|line| line.contains("PR content generation"))
        );
        assert!(kept.iter().any(|line| line == "panic:"));
        assert!(kept.iter().any(|line| line == "stack frame one"));
        assert!(!kept.iter().any(|line| line.contains("sandbox ready")));
    }

    #[test]
    fn error_filter_drops_warn_but_keeps_panic_continuations() {
        let kept = filter_lines(LOG, RunLogLevel::Error);
        assert_eq!(kept.len(), 3);
        assert!(!kept.iter().any(|line| line.contains("concurrent write")));
        assert!(
            kept.iter()
                .any(|line| line.contains("PR content generation"))
        );
        assert!(kept.iter().any(|line| line == "panic:"));
        assert!(kept.iter().any(|line| line == "stack frame one"));
    }

    #[test]
    fn info_filter_adds_info_but_not_debug() {
        let kept = filter_lines(LOG, RunLogLevel::Info);
        assert_eq!(kept.len(), 5);
        assert!(kept.iter().any(|line| line.contains("sandbox ready")));
        assert!(!kept.iter().any(|line| line.contains("cache: hit")));
    }

    #[test]
    fn tail_window_summarizes_older_lines() {
        let text = (0..10)
            .map(|i| format!("2026-08-27T13:33:0{i}.000000Z ERROR run{{id=1}}: failure {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let kept = filter_lines(&text, RunLogLevel::Error);
        assert_eq!(kept.len(), 10);
        let truncated: Vec<_> = kept[kept.len() - 3..].to_vec();
        assert_eq!(truncated.len(), 3);
        assert!(truncated[0].contains("failure 7"));
    }

    #[test]
    fn redaction_scrubs_secret_patterns_from_lines() {
        let dirty = "2026-08-27T13:33:01.000000Z ERROR auth: key=AKIAYRWQG5EJLPZLBYNP rejected";
        let kept = filter_lines(dirty, RunLogLevel::Error);
        let redacted = fabro_redact::redact_string(&kept[0]);
        assert!(redacted.contains("REDACTED"));
        assert!(!redacted.contains("AKIAYRWQG5EJLPZLBYNP"));
    }
}
