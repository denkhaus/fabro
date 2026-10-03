//! One comment-aware DOT edge scan for the fork's `graph_source`
//! readers (fabro-8615).
//!
//! Three modules read the run's original DOT text: the projection's
//! [`crate::projection::fork_exit_kinds`] reads the fork's `x.kind` exit
//! routes and [`crate::projection::edge_conditions`] reads `condition=`
//! presence (both edge statements), and [`crate::fork_stage_envelope`]
//! reads node blocks and their `x.*` envelopes (fabro-e901). Both walked the
//! raw text with `find("->")` and `find(']')`, and that walk loses real edges
//! in two shapes Fabro graphs carry:
//!
//! - a COMMENT mentioning an arrow (`// match): failed -> soft exit`) hands the
//!   walk a bracket far away, so every edge statement up to that bracket
//!   collapses into one bogus statement (fabro-8615: the revisor's plain
//!   `select -> exit` route vanished, and a green "Nothing to revise" pass
//!   classified as `Failed { SoftStop }` — the misleading status of the seeds
//!   and mulch revisor runs, fabro-79ba);
//! - a BARE edge (`start -> select`, no attribute list) let the walk inherit
//!   the attribute block of the NEXT edge.
//!
//! This module owns the one scan both readers share: comments are
//! stripped quote-aware (`//` and `/* */` outside string literals — the
//! compile steps carry `https://…`), and an edge statement owns the
//! attributes it opens itself, or none.

/// `graph_source` with DOT comments removed: `//` to the end of the line
/// and `/* … */` blocks. Quote-aware, so a `//` inside a quoted attribute
/// value is text. A removed comment leaves one line break behind, which
/// keeps node names on the line they were written on.
pub(crate) fn strip_comments(graph_source: &str) -> String {
    let mut cleaned = String::with_capacity(graph_source.len());
    let mut chars = graph_source.chars().peekable();
    let mut in_string = false;
    while let Some(ch) = chars.next() {
        if in_string {
            cleaned.push(ch);
            match ch {
                '\\' => {
                    if let Some(escaped) = chars.next() {
                        cleaned.push(escaped);
                    }
                }
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' => {
                in_string = true;
                cleaned.push(ch);
            }
            '/' if chars.peek() == Some(&'/') => {
                for comment in chars.by_ref() {
                    if comment == '\n' {
                        cleaned.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                // One space keeps the tokens around the comment apart (a
                // comment may sit inside an edge statement); the comment's
                // own line breaks survive, so a multi-line comment does
                // not glue the following line onto this one.
                cleaned.push(' ');
                let mut previous = ' ';
                for comment in chars.by_ref() {
                    if previous == '*' && comment == '/' {
                        break;
                    }
                    if comment == '\n' {
                        cleaned.push('\n');
                    }
                    previous = comment;
                }
            }
            _ => cleaned.push(ch),
        }
    }
    cleaned
}

/// Walk every edge statement of `graph_source` in source order, calling
/// `visit(from, to, attrs)`: `attrs` is the statement's attribute block
/// INCLUDING its brackets, or `""` for a bare edge. A chain
/// (`a -> b -> c [attrs]`) visits every leg with the statement's
/// attributes, as DOT applies them. A leg with an empty name (a malformed
/// statement) is skipped.
pub(crate) fn for_each_edge(graph_source: &str, visit: impl FnMut(&str, &str, &str)) {
    let cleaned = strip_comments(graph_source);
    scan_edges(&cleaned, visit);
}

/// The scan itself, over comment-free DOT text (the slices `visit` sees
/// borrow from it).
fn scan_edges(cleaned: &str, mut visit: impl FnMut(&str, &str, &str)) {
    let mut rest = cleaned;
    // The legs of the statement being read. They wait for the statement's
    // attribute block: `a -> b [x.kind="soft"]` and `a -> b -> c [attrs]`
    // both end with the block, and DOT applies it to every leg.
    let mut legs: Vec<(&str, &str)> = Vec::new();
    // The last target read: the `from` of the next leg of a chain, whose
    // head (`greet ->` in `start -> greet -> exit`) names no source.
    let mut previous: Option<&str> = None;
    while let Some(arrow) = find_arrow(rest) {
        let head = node_name(rest[..arrow].lines().last().unwrap_or_default());
        let from = if head.is_empty() {
            previous
        } else {
            Some(head)
        };
        let after = &rest[arrow + 2..];
        // The target runs to the statement's block, its line end, or the
        // end of the text; a chain keeps its further arrows in the same
        // region.
        let target_end = after.find(is_edge_break).unwrap_or(after.len());
        let target = after[..target_end].trim_start();
        let token = target.split_whitespace().next().unwrap_or_default();
        let to = node_name(token);
        let token_end = target_end - target.len() + token.len();
        if let (Some(from), false) = (from, to.is_empty()) {
            legs.push((from, to));
        }
        previous = Some(to).filter(|to| !to.is_empty());
        // A chain keeps going: this statement's further legs sit between
        // the target and the terminator, and its block follows the last.
        if after[token_end..target_end].contains("->") {
            rest = &after[token_end..];
            continue;
        }
        // The statement ends here — with its attribute block, or bare.
        if after[target_end..].starts_with('[') {
            let Some(close) = find_bracket_end(after, target_end) else {
                break;
            };
            let attrs = &after[target_end..=close];
            for (from, to) in legs.drain(..) {
                visit(from, to, attrs);
            }
            rest = &after[close + 1..];
        } else {
            for (from, to) in legs.drain(..) {
                visit(from, to, "");
            }
            rest = &after[token_end..];
        }
        previous = None;
    }
}

/// The character that ends an edge statement's target: the next line, or
/// the statement's own attribute block. `;` and `}` close a statement
/// without attributes.
fn is_edge_break(ch: char) -> bool {
    matches!(ch, '\n' | '[' | ';' | '}')
}

/// The byte offset of the next `->` outside a quoted string.
fn find_arrow(text: &str) -> Option<usize> {
    let mut index = 0;
    while let Some(dash) = find_unquoted(text, index, |byte| byte == b'-') {
        if text.as_bytes().get(dash + 1) == Some(&b'>') {
            return Some(dash);
        }
        index = dash + 1;
    }
    None
}

/// The byte offset of the `]` closing the `[` at `open`, skipping quoted
/// attribute values.
pub(crate) fn find_bracket_end(text: &str, open: usize) -> Option<usize> {
    find_unquoted(text, open + 1, |byte| byte == b']')
}

/// The byte offset of the next character at or after `from` that `wanted`
/// accepts, skipping quoted strings (inside one, a backslash escapes the
/// character after it).
fn find_unquoted(text: &str, from: usize, wanted: impl Fn(u8) -> bool) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut in_string = false;
    let mut index = from;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if in_string => index += 2,
            b'"' => {
                in_string = !in_string;
                index += 1;
            }
            byte if !in_string && wanted(byte) => return Some(index),
            _ => index += 1,
        }
    }
    None
}

/// A node name as the DOT statements write it: trimmed of surrounding
/// whitespace, quotes and separators. Fabro's graphs name nodes bare.
fn node_name(text: &str) -> &str {
    text.trim()
        .trim_matches(|c: char| c.is_whitespace() || c == '"' || c == ',' || c == ';')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The edge statements of `source` as `(from, to, attrs)` triples.
    fn edge_statements(source: &str) -> Vec<(String, String, String)> {
        let mut found = Vec::new();
        for_each_edge(source, |from, to, attrs| {
            found.push((from.to_string(), to.to_string(), attrs.to_string()));
        });
        found
    }

    #[test]
    fn a_comment_mentioning_an_arrow_does_not_swallow_the_next_edges() {
        // The revisor shape (fabro-8615): a comment with `->`, a bare
        // `start -> select` edge, then the plain and the kind exit.
        let source = "\
digraph Revisor {
    // match): failed -> soft exit; budget exhausted -> natural exit
    start -> select
    select -> exit [label=\"Nothing to revise\"]
    select -> exit [x.kind=\"soft\", label=\"Selector failed\"]
}
";
        assert_eq!(edge_statements(source), vec![
            ("start".to_string(), "select".to_string(), String::new()),
            (
                "select".to_string(),
                "exit".to_string(),
                "[label=\"Nothing to revise\"]".to_string()
            ),
            (
                "select".to_string(),
                "exit".to_string(),
                "[x.kind=\"soft\", label=\"Selector failed\"]".to_string()
            ),
        ]);
    }

    #[test]
    fn a_block_comment_disappears() {
        let source = "a -> b /* an arrow -> and a bracket ] */ [x.kind=\"soft\"]";
        assert_eq!(edge_statements(source), vec![(
            "a".to_string(),
            "b".to_string(),
            "[x.kind=\"soft\"]".to_string()
        )]);
    }

    #[test]
    fn a_bare_edge_owns_no_attributes() {
        let found = edge_statements("start -> work\nwork -> exit [x.kind=\"soft\"]");
        assert_eq!(found[0].2, "", "the bare edge inherits nothing");
        assert_eq!(found[1].2, "[x.kind=\"soft\"]");
    }

    #[test]
    fn arrows_and_comments_inside_quoted_values_stay_text() {
        let source = "\
    note [script=\"echo a -> b // https://example.test\"]
    start -> note
";
        assert_eq!(
            edge_statements(source),
            vec![("start".to_string(), "note".to_string(), String::new())],
            "the arrow inside the quoted script is not an edge"
        );
    }

    #[test]
    fn a_bracket_inside_a_quoted_value_does_not_close_the_block() {
        let found = edge_statements("a -> b [label=\"brackets ] inside\"]");
        assert_eq!(found[0].2, "[label=\"brackets ] inside\"]");
    }

    #[test]
    fn a_multiline_attribute_block_stays_one_statement() {
        let source = "a -> b [label=\"X\",\n  condition=\"outcome=failed\",\n  x.kind=\"soft\"]";
        let found = edge_statements(source);
        assert_eq!(found.len(), 1);
        assert!(found[0].2.contains("x.kind=\"soft\""));
    }

    #[test]
    fn every_leg_of_a_chain_takes_the_statements_attributes() {
        // `start -> say -> exit` is the shape 76 checked-in graphs write;
        // DOT applies a trailing block to every leg of the chain.
        let found = edge_statements("start -> say -> exit [x.kind=\"soft\"]");
        assert_eq!(found, vec![
            (
                "start".to_string(),
                "say".to_string(),
                "[x.kind=\"soft\"]".to_string()
            ),
            (
                "say".to_string(),
                "exit".to_string(),
                "[x.kind=\"soft\"]".to_string()
            ),
        ]);
    }

    #[test]
    fn chain_legs_without_a_block_are_plain_edges() {
        let found = edge_statements("start -> say -> exit");
        assert_eq!(found, vec![
            ("start".to_string(), "say".to_string(), String::new()),
            ("say".to_string(), "exit".to_string(), String::new()),
        ]);
    }

    #[test]
    fn a_semicolon_ends_a_statement_without_attributes() {
        let found = edge_statements("a -> b; c -> d [x.kind=\"soft\"]");
        assert_eq!(found, vec![
            ("a".to_string(), "b".to_string(), String::new()),
            (
                "c".to_string(),
                "d".to_string(),
                "[x.kind=\"soft\"]".to_string()
            ),
        ]);
    }

    #[test]
    fn strip_comments_keeps_quoted_urls_and_escaped_quotes() {
        let source = "a -> b [script=\"curl https://x.test | sh\", label=\"\\\"quoted\\\"\"]";
        assert_eq!(strip_comments(source), source);
    }
}
