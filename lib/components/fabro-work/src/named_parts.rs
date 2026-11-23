//! The named-parts vocabulary (denkhaus/work#1553): the machine-
//! checkable acceptance criteria that live in the item body itself.
//! GitHub-style task lists (`- [ ]` / `- [x]` checkboxes) are the
//! carrier every layer can read without prose judgment — the reviewer's
//! per-part verdicts, the adjudicator's acceptance accounting, and the
//! merge door's mechanical backstop all consume THIS parser, never a
//! per-surface re-derivation (the one-reader rule the acceptance fold
//! set, work#1555).
//!
//! A named part is one top-level task-list checkbox line of the body:
//! up to three leading spaces (the GitHub list-item limit), a `-` or
//! `*` marker, the `[ ]`/`[x]` box, and the part text (trimmed).
//! Indented checkboxes are sub-parts of their parent part — they ride
//! the parent's verdict and are NOT parsed as parts of their own.

/// The observation head that carries the reviewer's per-part verified
/// verdict (work#1553): one note observation per verified part, its
/// text exactly `part=verified: <the part text verbatim>`. The merge
/// door accounts an unchecked part against these records — the run's
/// own review is the accountability carrier for the parts its pull
/// request delivers.
pub const PART_VERIFIED_HEAD: &str = "part=verified:";

/// One task-list checkbox of an item body: the acceptance chain's
/// machine-checkable criterion. Plain data — the parser owns the
/// shape, every consumer reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedPart {
    /// Whether the item body carries the box checked (`- [x]`).
    pub checked: bool,
    /// The part text after the box, trimmed. The identity every
    /// per-part verdict names verbatim.
    pub text:    String,
}

/// The named parts of an item body, in document order. See the module
/// docs for the exact grammar; bodies without task lists read empty.
#[must_use]
pub fn named_parts(body: &str) -> Vec<NamedPart> {
    let mut parts = Vec::new();
    for line in body.lines() {
        let Some(part) = parse_part_line(line) else {
            continue;
        };
        parts.push(part);
    }
    parts
}

/// One line's part, `None` for everything that is not a top-level
/// task-list checkbox (including sub-item checkboxes at four-plus
/// spaces of indentation).
fn parse_part_line(line: &str) -> Option<NamedPart> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let unchecked = ["- [ ] ", "* [ ] "]
        .iter()
        .find_map(|marker| rest.strip_prefix(marker));
    let (checked, text) = if let Some(text) = unchecked {
        (false, text)
    } else {
        let text = ["- [x] ", "- [X] ", "* [x] ", "* [X] "]
            .iter()
            .find_map(|marker| rest.strip_prefix(marker))?;
        (true, text)
    };
    Some(NamedPart {
        checked,
        text: text.trim().to_string(),
    })
}

/// The part text a per-part observation record names, `None` when the
/// text is not a `part=verified:` record (not-in-diff and cannot-verify
/// records never account a part — they are the reviewer's refusal
/// carriers, read by the agent layers, not the door).
#[must_use]
pub fn verified_part(text: &str) -> Option<&str> {
    text.trim()
        .strip_prefix(PART_VERIFIED_HEAD)
        .map(str::trim)
        .filter(|part| !part.is_empty())
}

/// The parts the acceptance chain still holds unaccounted: every
/// unchecked part whose text no verified-part observation names.
/// Checked parts are accounted by the body itself; verified parts by
/// the run's review. Empty means the chain is clear to land.
#[must_use]
pub fn unaccounted_parts<'a>(
    parts: &'a [NamedPart],
    verified_texts: &dyn Fn(&str) -> bool,
) -> Vec<&'a NamedPart> {
    parts
        .iter()
        .filter(|part| !part.checked && !verified_texts(&part.text))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{NamedPart, named_parts, unaccounted_parts, verified_part};

    fn part(checked: bool, text: &str) -> NamedPart {
        NamedPart {
            checked,
            text: text.to_string(),
        }
    }

    #[test]
    fn parses_unchecked_and_checked_boxes() {
        let body = "Intro prose.\n\n## Parts\n\n- [ ] Reviewer: per-part verdicts\n- [x] \
                    Close door: filed as work#1552\n";
        assert_eq!(
            named_parts(body),
            vec![
                part(false, "Reviewer: per-part verdicts"),
                part(true, "Close door: filed as work#1552"),
            ],
            "one part per top-level checkbox, checked state carried"
        );
    }

    #[test]
    fn accepts_list_marker_indent_and_case_variants() {
        let body = "- [x] star checked\n* [ ] star unchecked\n  - [X] two-space upper\n   \
                    - [ ] three-space\n";
        assert_eq!(
            named_parts(body),
            vec![
                part(true, "star checked"),
                part(false, "star unchecked"),
                part(true, "two-space upper"),
                part(false, "three-space"),
            ],
            "both markers, up to three indent spaces, x/X boxes"
        );
    }

    #[test]
    fn sub_items_and_prose_are_not_parts() {
        let body = "## Parts\n\n- [ ] parent part\n    - [ ] sub part rides the parent\n1. \
                    not a checkbox\n- []empty box is prose\n- [ ]no-space-after-box stays prose\n";
        assert_eq!(
            named_parts(body),
            vec![part(false, "parent part")],
            "four-plus-space indents and non-checkbox lines never parse"
        );
    }

    #[test]
    fn a_body_without_task_lists_reads_empty() {
        assert!(
            named_parts("Plain finding prose.\n\n## Closes when\nA pin on each door.\n").is_empty(),
            "no checkboxes, no parts — the doors stay open for criteria-free bodies"
        );
    }

    #[test]
    fn verified_part_extracts_only_the_verified_head() {
        assert_eq!(
            verified_part("part=verified: Reviewer: per-part verdicts"),
            Some("Reviewer: per-part verdicts"),
            "the head strips and the part text survives verbatim"
        );
        assert_eq!(
            verified_part("  part=verified: padded  "),
            Some("padded"),
            "surrounding whitespace trims"
        );
        assert_eq!(
            verified_part("part=not-in-diff: missing"),
            None,
            "refusal carriers never account a part"
        );
        assert_eq!(
            verified_part("part=verified:"),
            None,
            "an empty part text accounts nothing"
        );
        assert_eq!(verified_part("verdict: run merge=enabled"), None);
    }

    #[test]
    fn unaccounted_parts_skip_checked_and_verified() {
        let parts = named_parts("- [x] landed\n- [ ] verified by review\n- [ ] missing\n");
        let unaccounted = unaccounted_parts(&parts, &|text| text == "verified by review");
        assert_eq!(
            unaccounted,
            vec![&part(false, "missing")],
            "checked parts account by the body, verified parts by the review, the rest refuse"
        );
    }
}
