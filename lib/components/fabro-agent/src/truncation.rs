use crate::config::SessionOptions;
use crate::sandbox::OutputCaptureStats;
use crate::tool_permissions::canonical_tool_name;

pub(crate) const MAX_RETAINED_TOOL_OUTPUT_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_SERIALIZED_TOOL_OUTPUT_BYTES: usize = 3 * 1024 * 1024 / 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RetainedToolOutput {
    pub output: String,
    pub stats:  OutputCaptureStats,
    head_bytes: usize,
}

/// Keep an equal-sized UTF-8 prefix and suffix within a byte budget.
///
/// `previously_omitted_bytes` accounts for output a streaming provider
/// discarded before the rendered result was assembled.
#[must_use]
pub(crate) fn retain_tool_output(
    output: &str,
    max_bytes: usize,
    previously_omitted_bytes: usize,
) -> RetainedToolOutput {
    let observed_bytes = output.len().saturating_add(previously_omitted_bytes);
    if output.len() <= max_bytes {
        return RetainedToolOutput {
            output:     output.to_string(),
            stats:      OutputCaptureStats {
                observed_bytes,
                retained_bytes: output.len(),
                omitted_bytes: previously_omitted_bytes,
            },
            head_bytes: if previously_omitted_bytes == 0 {
                output.len()
            } else {
                output.floor_char_boundary(output.len() / 2)
            },
        };
    }

    let head_budget = max_bytes / 2;
    let tail_budget = max_bytes.saturating_sub(head_budget);
    let head_end = output.floor_char_boundary(head_budget);
    let tail_start = ceil_char_boundary(output, output.len().saturating_sub(tail_budget));
    let retained_bytes = head_end.saturating_add(output.len().saturating_sub(tail_start));
    let mut retained = String::with_capacity(retained_bytes);
    retained.push_str(&output[..head_end]);
    retained.push_str(&output[tail_start..]);

    RetainedToolOutput {
        output:     retained,
        stats:      OutputCaptureStats {
            observed_bytes,
            retained_bytes,
            omitted_bytes: observed_bytes.saturating_sub(retained_bytes),
        },
        head_bytes: head_end,
    }
}

/// Build the final model-facing preview, including truncation notices inside
/// the total byte budget and JSON serialization limit.
#[must_use]
pub(crate) fn preview_tool_output(
    output: &str,
    max_bytes: usize,
    previously_omitted_bytes: usize,
) -> RetainedToolOutput {
    let mut content_budget = max_bytes;
    loop {
        let retained = retain_tool_output(output, content_budget, previously_omitted_bytes);
        let rendered = if retained.stats.omitted_bytes == 0 {
            retained.output.clone()
        } else {
            render_retained_output(&retained)
        };
        let serialized_bytes = serialized_json_string_bytes(&rendered);
        if rendered.len() <= max_bytes && serialized_bytes <= MAX_SERIALIZED_TOOL_OUTPUT_BYTES {
            if retained.stats.omitted_bytes == 0 {
                return retained;
            }
            return RetainedToolOutput {
                output:     rendered,
                stats:      retained.stats,
                head_bytes: 0,
            };
        }

        let mut next_budget = content_budget;
        if rendered.len() > max_bytes {
            let excess = rendered.len().saturating_sub(max_bytes);
            next_budget = next_budget.min(content_budget.saturating_sub(excess));
        }
        if serialized_bytes > MAX_SERIALIZED_TOOL_OUTPUT_BYTES {
            let scaled_budget = (content_budget as u128)
                .saturating_mul(MAX_SERIALIZED_TOOL_OUTPUT_BYTES as u128)
                .checked_div(serialized_bytes as u128)
                .and_then(|budget| usize::try_from(budget).ok())
                .unwrap_or(0);
            next_budget = next_budget.min(scaled_budget);
        }
        next_budget = next_budget.min(content_budget.saturating_sub(1));
        if next_budget == content_budget {
            return RetainedToolOutput {
                output:     truncate_plain_output(&rendered, max_bytes, TruncationMode::HeadTail),
                stats:      retained.stats,
                head_bytes: 0,
            };
        }
        content_budget = next_budget;
    }
}

fn serialized_json_string_bytes(output: &str) -> usize {
    serde_json::to_vec(output)
        .expect("strings always serialize as JSON")
        .len()
}

fn render_retained_output(retained: &RetainedToolOutput) -> String {
    let head = &retained.output[..retained.head_bytes];
    let tail = &retained.output[retained.head_bytes..];
    render_truncated_segments(head, tail, retained.stats, None)
}

fn render_truncated_segments(
    head: &str,
    tail: &str,
    stats: OutputCaptureStats,
    line_count_omitted: Option<usize>,
) -> String {
    let original_tokens = approximate_tokens(stats.observed_bytes);
    let omitted_tokens = approximate_tokens(stats.omitted_bytes);
    let middle_marker = line_count_omitted.map_or_else(
        || format!("... approximately {omitted_tokens} tokens truncated ..."),
        |lines| {
            format!(
                "... {lines} lines omitted (approximately {omitted_tokens} tokens truncated) ..."
            )
        },
    );
    format!(
        "Warning: truncated output (original token count: {original_tokens})\n... {} bytes omitted ...\n\n{head}\n\n{middle_marker}\n\n{tail}",
        stats.omitted_bytes
    )
}

fn approximate_tokens(bytes: usize) -> usize {
    bytes.div_ceil(4)
}

fn ceil_char_boundary(output: &str, index: usize) -> usize {
    let mut index = index.min(output.len());
    while index < output.len() && !output.is_char_boundary(index) {
        index += 1;
    }
    index
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TruncationMode {
    HeadTail,
    Tail,
}

fn default_char_limit(tool_name: &str) -> Option<usize> {
    match tool_name {
        "read_file" => Some(50_000),
        "shell" => Some(30_000),
        "grep" | "glob" | "spawn_agent" => Some(20_000),
        "edit_file" | "apply_patch" => Some(10_000),
        "write_file" => Some(1_000),
        _ => None,
    }
}

fn default_line_limit(tool_name: &str) -> Option<usize> {
    match tool_name {
        "shell" => Some(256),
        "grep" => Some(200),
        "glob" => Some(500),
        _ => None,
    }
}

fn default_truncation_mode(tool_name: &str) -> TruncationMode {
    match tool_name {
        "grep" | "glob" | "edit_file" | "apply_patch" | "write_file" => TruncationMode::Tail,
        _ => TruncationMode::HeadTail,
    }
}

#[must_use]
pub fn truncate_output(output: &str, max_chars: usize, mode: TruncationMode) -> String {
    if output.len() <= max_chars {
        return output.to_string();
    }

    match mode {
        TruncationMode::HeadTail => {
            let half = max_chars / 2;
            let head_end = output.floor_char_boundary(half);
            let tail_start = ceil_char_boundary(output, output.len().saturating_sub(half));
            let head = &output[..head_end];
            let tail = &output[tail_start..];
            let retained_bytes = head.len().saturating_add(tail.len());
            render_truncated_segments(
                head,
                tail,
                OutputCaptureStats {
                    observed_bytes: output.len(),
                    retained_bytes,
                    omitted_bytes: output.len().saturating_sub(retained_bytes),
                },
                None,
            )
        }
        TruncationMode::Tail => {
            let tail_start = ceil_char_boundary(output, output.len().saturating_sub(max_chars));
            let tail = &output[tail_start..];
            render_truncated_segments(
                "",
                tail,
                OutputCaptureStats {
                    observed_bytes: output.len(),
                    retained_bytes: tail.len(),
                    omitted_bytes:  output.len().saturating_sub(tail.len()),
                },
                None,
            )
        }
    }
}

fn truncate_plain_output(output: &str, max_bytes: usize, mode: TruncationMode) -> String {
    if output.len() <= max_bytes {
        return output.to_string();
    }

    match mode {
        TruncationMode::HeadTail => {
            let half = max_bytes / 2;
            let head_end = output.floor_char_boundary(half);
            let tail_start = ceil_char_boundary(output, output.len().saturating_sub(half));
            format!("{}{}", &output[..head_end], &output[tail_start..])
        }
        TruncationMode::Tail => {
            let tail_start = ceil_char_boundary(output, output.len().saturating_sub(max_bytes));
            output[tail_start..].to_string()
        }
    }
}

#[must_use]
pub fn truncate_lines(output: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = output.lines().collect();
    if lines.len() <= max_lines {
        return output.to_string();
    }

    let head_count = max_lines / 2;
    let tail_count = max_lines.saturating_sub(head_count);
    let head = lines[..head_count].join("\n");
    let tail = lines[lines.len() - tail_count..].join("\n");
    let omitted = lines.len() - max_lines;
    let retained_bytes = head.len().saturating_add(tail.len());

    render_truncated_segments(
        &head,
        &tail,
        OutputCaptureStats {
            observed_bytes: output.len(),
            retained_bytes,
            omitted_bytes: output.len().saturating_sub(retained_bytes),
        },
        Some(omitted),
    )
}

#[must_use]
pub fn truncate_tool_output(output: &str, tool_name: &str, config: &SessionOptions) -> String {
    let canonical_name = canonical_tool_name(tool_name);
    let mode = default_truncation_mode(canonical_name);

    // Char truncation first
    let char_limit = config
        .tool_output_limits
        .get(tool_name)
        .copied()
        .or_else(|| config.tool_output_limits.get(canonical_name).copied())
        .or_else(|| default_char_limit(canonical_name));

    let after_chars = match char_limit {
        Some(limit) => truncate_output(output, limit, mode),
        None => output.to_string(),
    };

    // Then line truncation
    let line_limit = config
        .tool_line_limits
        .get(tool_name)
        .copied()
        .or_else(|| config.tool_line_limits.get(canonical_name).copied())
        .or_else(|| default_line_limit(canonical_name));

    match line_limit {
        Some(limit) => truncate_lines(&after_chars, limit),
        None => after_chars,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_tool_output_keeps_equal_head_and_tail() {
        let retained = retain_tool_output("abcdefghijkl", 8, 0);

        assert_eq!(retained.output, "abcdijkl");
        assert_eq!(retained.stats.observed_bytes, 12);
        assert_eq!(retained.stats.retained_bytes, 8);
        assert_eq!(retained.stats.omitted_bytes, 4);
    }

    #[test]
    fn retained_tool_output_stays_within_budget_at_utf8_boundaries() {
        let retained = retain_tool_output("aa😀😀zz", 7, 3);

        assert!(retained.output.len() <= 7, "{}", retained.output.len());
        assert!(retained.output.starts_with("aa"));
        assert!(retained.output.ends_with("zz"));
        assert_eq!(retained.stats.observed_bytes, "aa😀😀zz".len() + 3);
        assert_eq!(
            retained.stats.omitted_bytes,
            retained.stats.observed_bytes - retained.output.len()
        );
    }

    #[test]
    fn model_preview_includes_codex_style_notice_inside_budget() {
        let output = format!("HEAD{}TAIL", "x".repeat(1_000));
        let preview = preview_tool_output(&output, 512, 0);

        assert!(preview.output.len() <= 512, "{}", preview.output.len());
        assert!(
            preview
                .output
                .starts_with("Warning: truncated output (original token count: 252)")
        );
        assert!(preview.output.contains(&format!(
            "... {} bytes omitted ...",
            preview.stats.omitted_bytes
        )));
        assert!(preview.output.contains("approximately"));
        assert!(preview.output.contains("tokens truncated"));
        assert!(preview.output.contains("HEAD"));
        assert!(preview.output.ends_with("TAIL"));
        assert!(!preview.output.contains("re-run"));
        assert!(!preview.output.contains("targeted parameters"));
    }

    #[test]
    fn model_preview_reports_bytes_omitted_before_rendering() {
        let preview = preview_tool_output("abcdefgh", 512, 100);

        assert_eq!(preview.stats.observed_bytes, 108);
        assert_eq!(preview.stats.retained_bytes, 8);
        assert_eq!(preview.stats.omitted_bytes, 100);
        assert!(preview.output.contains("... 100 bytes omitted ..."));
        assert!(preview.output.contains("abcd"));
        assert!(preview.output.ends_with("efgh"));
    }

    #[test]
    fn model_preview_bounds_pathological_json_serialization() {
        let output = format!(
            "HEAD{}TAIL",
            "\0".repeat(MAX_RETAINED_TOOL_OUTPUT_BYTES - "HEADTAIL".len())
        );
        assert_eq!(output.len(), MAX_RETAINED_TOOL_OUTPUT_BYTES);
        assert!(serialized_json_string_bytes(&output) > MAX_SERIALIZED_TOOL_OUTPUT_BYTES);

        let preview = preview_tool_output(&output, MAX_RETAINED_TOOL_OUTPUT_BYTES, 0);
        let serialized_bytes = serialized_json_string_bytes(&preview.output);

        assert!(preview.output.len() <= MAX_RETAINED_TOOL_OUTPUT_BYTES);
        assert!(
            serialized_bytes <= MAX_SERIALIZED_TOOL_OUTPUT_BYTES,
            "serialized preview was {serialized_bytes} bytes"
        );
        assert!(preview.output.starts_with("Warning: truncated output"));
        assert!(preview.output.contains("HEAD"));
        assert!(preview.output.ends_with("TAIL"));
        assert_eq!(preview.stats.observed_bytes, MAX_RETAINED_TOOL_OUTPUT_BYTES);
        assert!(preview.stats.retained_bytes < MAX_RETAINED_TOOL_OUTPUT_BYTES);
        assert_eq!(
            preview.stats.omitted_bytes,
            preview.stats.observed_bytes - preview.stats.retained_bytes
        );
    }

    #[test]
    fn under_limit_passthrough_chars() {
        let output = "short output";
        let result = truncate_output(output, 100, TruncationMode::HeadTail);
        assert_eq!(result, output);
    }

    #[test]
    fn under_limit_passthrough_lines() {
        let output = "line1\nline2\nline3";
        let result = truncate_lines(output, 10);
        assert_eq!(result, output);
    }

    #[test]
    fn head_tail_split() {
        let output = "a".repeat(100);
        let result = truncate_output(&output, 40, TruncationMode::HeadTail);
        assert!(result.contains(&"a".repeat(20)));
        assert!(result.starts_with("Warning: truncated output (original token count: 25)"));
        assert!(result.contains("... 60 bytes omitted ..."));
        assert!(result.contains("approximately 15 tokens truncated"));
    }

    #[test]
    fn tail_mode() {
        let output = format!("{}BBB", "A".repeat(100));
        let result = truncate_output(&output, 10, TruncationMode::Tail);
        assert!(result.starts_with("Warning: truncated output"));
        assert!(result.contains("... 93 bytes omitted ..."));
        assert!(result.contains("approximately 24 tokens truncated"));
        assert!(result.ends_with("AAAAAAABBB"));
    }

    #[test]
    fn line_truncation_splits_head_tail() {
        let lines: Vec<String> = (1..=20).map(|i| format!("line {i}")).collect();
        let output = lines.join("\n");
        let result = truncate_lines(&output, 6);
        assert!(result.contains("line 1"));
        assert!(result.contains("line 3"));
        assert!(result.contains("line 18"));
        assert!(result.contains("line 20"));
        assert!(result.contains("14 lines omitted"));
        assert!(result.contains("tokens truncated"));
    }

    #[test]
    fn char_truncation_before_lines() {
        // Create an output that is large in chars and many lines
        let long_line = "x".repeat(50_000);
        let output = format!("{long_line}\n{long_line}");
        let config = SessionOptions::default();
        let result = truncate_tool_output(&output, "shell", &config);
        // Should have been char-truncated first (30k limit for shell)
        assert!(result.len() < output.len());
    }

    #[test]
    fn kimi_aliases_use_canonical_limits() {
        let config = SessionOptions::default();
        let shell_output = "x".repeat(40_000);
        let write_output = "x".repeat(2_000);

        assert!(truncate_tool_output(&shell_output, "Bash", &config).len() < shell_output.len());
        assert!(truncate_tool_output(&write_output, "Write", &config).len() < write_output.len());
    }

    #[test]
    fn canonical_config_override_applies_to_kimi_alias() {
        let mut config = SessionOptions::default();
        config.tool_output_limits.insert("shell".into(), 100);
        let result = truncate_tool_output(&"x".repeat(1_000), "Bash", &config);
        assert!(result.contains("Warning: truncated output"));
    }

    #[test]
    fn config_override_char_limit() {
        let output = "x".repeat(5000);
        let mut config = SessionOptions::default();
        config.tool_output_limits.insert("my_tool".into(), 100);
        let result = truncate_tool_output(&output, "my_tool", &config);
        assert!(result.len() < output.len());
        assert!(result.contains("Warning: truncated output"));
    }

    #[test]
    fn config_override_line_limit() {
        let lines: Vec<String> = (1..=100).map(|i| format!("line {i}")).collect();
        let output = lines.join("\n");
        let mut config = SessionOptions::default();
        config.tool_line_limits.insert("my_tool".into(), 10);
        let result = truncate_tool_output(&output, "my_tool", &config);
        assert!(result.contains("lines omitted"));
    }

    #[test]
    fn unknown_tool_no_truncation() {
        let output = "x".repeat(200);
        let config = SessionOptions::default();
        let result = truncate_tool_output(&output, "unknown_tool", &config);
        assert_eq!(result, output);
    }

    #[test]
    fn default_char_limits_match_spec() {
        assert_eq!(default_char_limit("read_file"), Some(50_000));
        assert_eq!(default_char_limit("shell"), Some(30_000));
        assert_eq!(default_char_limit("grep"), Some(20_000));
        assert_eq!(default_char_limit("glob"), Some(20_000));
        assert_eq!(default_char_limit("edit_file"), Some(10_000));
        assert_eq!(default_char_limit("write_file"), Some(1_000));
        assert_eq!(default_char_limit("apply_patch"), Some(10_000));
        assert_eq!(default_char_limit("spawn_agent"), Some(20_000));
        assert_eq!(default_char_limit("unknown"), None);
    }

    #[test]
    fn default_line_limits_match_spec() {
        assert_eq!(default_line_limit("shell"), Some(256));
        assert_eq!(default_line_limit("grep"), Some(200));
        assert_eq!(default_line_limit("glob"), Some(500));
        assert_eq!(default_line_limit("unknown"), None);
    }

    #[test]
    fn exact_limit_not_truncated() {
        let output = "x".repeat(100);
        let result = truncate_output(&output, 100, TruncationMode::HeadTail);
        assert_eq!(result, output);
    }

    #[test]
    fn exact_line_limit_not_truncated() {
        let lines: Vec<String> = (1..=10).map(|i| format!("line {i}")).collect();
        let output = lines.join("\n");
        let result = truncate_lines(&output, 10);
        assert_eq!(result, output);
    }

    #[test]
    fn truncate_output_multibyte_no_panic() {
        let output = "✅".repeat(100); // 300 bytes
        let result = truncate_output(&output, 10, TruncationMode::HeadTail);
        assert!(result.contains("Warning: truncated output"));
    }
}
