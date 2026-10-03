#!/usr/bin/env python3
"""Finding and verdict normalization.

Validates agent JSON against the finding contract into findings or recorded
rejections, derives the per-phase rule context, and normalizes verdicts.

Python 3.9-compatible. Standard library only."""

from __future__ import annotations

import sys
from pathlib import Path
from typing import (
    Any,
    Dict,
    Iterable,
    List,
    Mapping,
    Optional,
    Tuple,
)

sys.path.insert(0, str(Path(__file__).resolve().parent))

from review_contract import (  # noqa: E402
    CATEGORIES,
    COMPILED_RULE_ID_RE,
    ISSUE_TYPES,
    MAX_RULE_IDS_PER_FINDING,
)
from git_target import normalize_repo_path  # noqa: E402
from review_common import (  # noqa: E402
    CANDIDATE_ID_RE,
    CONFIDENCE_RANK,
    CONVENTIONS_FILTER_REASON,
    LOCATION_MAX_LINES,
    POLICY_FILTER_REASONS,
    SEVERITY_RANK,
    SUGGESTION_CODE_MAX_LENGTH,
    VERDICTS,
    clean_text,
    one_line,
)


# --- Finding and verdict normalization ---------------------------------------


def bounded_rule_ids(values: Iterable[Any]) -> List[str]:
    """Return the renderer-safe, deterministic union of compiled rule IDs."""
    return sorted({value for value in values if isinstance(value, str)})[
        :MAX_RULE_IDS_PER_FINDING
    ]


def finding_or_rejection(
    value: Any,
    rule_context: Optional[Mapping[str, Any]] = None,
) -> Tuple[Optional[Dict[str, Any]], Optional[str]]:
    """Normalize one reported finding, or say which part of the contract failed.

    A rejected finding is dropped from the review, so the reason travels with
    the rejection into coverage. Reasons are fixed strings: they name the
    field, and never quote the model's own text back into a report.

    ``rule_context`` (rule-mapped tiers) carries ``require`` -- whether this
    result came from a rule-audit job, which must name a violated check -- and
    ``effective``, the engine's file-to-check-ID map. The engine is
    authoritative about applicability: a named check that does not apply to
    the finding's file is rejected. This also enforces the anchor rule: a
    finding must anchor in a changed file its check applies to.
    """
    if not isinstance(value, dict):
        return None, "the finding is not a JSON object"
    path = normalize_repo_path(value.get("file"))
    legacy_line = value.get("line")
    start_line = value.get("start_line", legacy_line)
    end_line = value.get("end_line", legacy_line)
    summary = one_line(value.get("summary"), 600).strip()
    short_summary = one_line(value.get("short_summary"), 200).strip()[:60]
    failure_scenario = clean_text(value.get("failure_scenario"), 4000).strip()
    category = one_line(value.get("category"), 40).strip().lower()
    issue_type = one_line(value.get("issue_type"), 40).strip().lower()
    severity = one_line(value.get("severity"), 20).upper()
    confidence = one_line(value.get("confidence"), 20).upper()
    raw_suggestion = value.get("suggestion_code", "")
    for failed, reason in (
        (path is None or path == ".", "file does not name a repository file"),
        (
            isinstance(start_line, bool)
            or not isinstance(start_line, int)
            or start_line < 1,
            "start_line is not a positive integer",
        ),
        (
            isinstance(end_line, bool)
            or not isinstance(end_line, int)
            or end_line < 1,
            "end_line is not a positive integer",
        ),
        (
            isinstance(start_line, int)
            and isinstance(end_line, int)
            and start_line > end_line,
            "start_line is after end_line",
        ),
        (
            isinstance(start_line, int)
            and isinstance(end_line, int)
            and end_line - start_line + 1 > LOCATION_MAX_LINES,
            f"location spans more than {LOCATION_MAX_LINES} lines",
        ),
        (not summary, "summary is empty"),
        (not failure_scenario, "failure_scenario is empty"),
        (category not in CATEGORIES, "category is not in the closed list"),
        (issue_type not in ISSUE_TYPES, "issue_type is not in the closed list"),
        (severity not in SEVERITY_RANK, "severity is not HIGH, MEDIUM, or LOW"),
        (
            confidence not in CONFIDENCE_RANK,
            "confidence is not HIGH, MEDIUM, or LOW",
        ),
        (
            not isinstance(raw_suggestion, str),
            "suggestion_code is not a string",
        ),
        (
            isinstance(raw_suggestion, str)
            and len(raw_suggestion) > SUGGESTION_CODE_MAX_LENGTH,
            f"suggestion_code exceeds {SUGGESTION_CODE_MAX_LENGTH} characters",
        ),
        (
            isinstance(raw_suggestion, str)
            and any(
                character not in "\n\t" and ord(character) < 0x20
                for character in raw_suggestion
            ),
            "suggestion_code contains control characters",
        ),
    ):
        if failed:
            return None, reason
    if not short_summary:
        short_summary = summary[:60]

    # Agents report one "rule_id"; normalized findings carry "rule_ids".
    # Accepting both lets stored findings re-normalize without loss.
    raw_rule_ids: List[Any] = []
    if isinstance(value.get("rule_ids"), list):
        raw_rule_ids = list(value["rule_ids"])
    elif value.get("rule_id") not in (None, ""):
        raw_rule_ids = [value.get("rule_id")]
    rule_ids: List[str] = []
    if rule_context is not None:
        if not raw_rule_ids and rule_context.get("require"):
            return None, "a rule-audit finding names no rule check"
        effective = rule_context.get("effective") or {}
        for raw_rule_id in raw_rule_ids:
            if not isinstance(raw_rule_id, str) or not (
                COMPILED_RULE_ID_RE.fullmatch(raw_rule_id)
            ):
                return None, "rule_id is not a compiled check ID"
            if raw_rule_id not in (effective.get(path) or ()):
                return None, "the named rule check does not apply to the file"
        rule_ids = bounded_rule_ids(raw_rule_ids)
        if len(set(raw_rule_ids)) > MAX_RULE_IDS_PER_FINDING:
            return None, (
                "rule_ids names more than "
                f"{MAX_RULE_IDS_PER_FINDING} distinct checks"
            )
    if category == "conventions" and not rule_ids:
        return None, CONVENTIONS_FILTER_REASON

    return {
        "file": path,
        # ``line`` remains the stable end-line alias used by ranking,
        # deduplication, and older consumers. The canonical finding also
        # carries the complete range.
        "line": end_line,
        "start_line": start_line,
        "end_line": end_line,
        "summary": summary,
        "short_summary": short_summary,
        "failure_scenario": failure_scenario,
        "category": category,
        "issue_type": issue_type,
        "severity": severity,
        "confidence": confidence,
        "suggestion_code": raw_suggestion if raw_suggestion.strip() else "",
        "rule_ids": rule_ids,
    }, None


def findings_and_rejections(
    value: Any,
    rule_context: Optional[Mapping[str, Any]] = None,
) -> Tuple[Optional[Dict[str, Any]], List[str], List[str]]:
    """Split a finder result into findings, contract rejections, and filters."""
    if not isinstance(value, dict) or not isinstance(value.get("findings"), list):
        return None, [], []
    findings: List[Dict[str, Any]] = []
    rejections: List[str] = []
    filtered: List[str] = []
    for position, raw in enumerate(value["findings"], 1):
        finding, reason = finding_or_rejection(raw, rule_context)
        if finding is not None:
            findings.append(finding)
        elif reason in POLICY_FILTER_REASONS:
            filtered.append(f"finding {position}: {reason}")
        else:
            rejections.append(f"finding {position}: {reason}")
    return {"findings": findings}, rejections, filtered


def normalize_findings_result(
    value: Any,
    rule_context: Optional[Mapping[str, Any]] = None,
) -> Optional[Dict[str, Any]]:
    result, _rejections, _filtered = findings_and_rejections(value, rule_context)
    return result


def state_rule_context(
    state: Mapping[str, Any],
    require: bool,
) -> Optional[Dict[str, Any]]:
    rules_state = state.get("rules")
    if not isinstance(rules_state, dict) or not rules_state.get("enabled"):
        return None
    return {
        "require": require,
        "effective": rules_state.get("effective") or {},
    }


def normalize_verdict(value: Any) -> Optional[Dict[str, Any]]:
    if not isinstance(value, dict):
        return None
    verdict = value.get("verdict")
    if verdict not in VERDICTS:
        return None
    if not isinstance(value.get("reasoning"), str):
        return None
    result = {
        "verdict": verdict,
        "reasoning": clean_text(value.get("reasoning"), 4000),
    }
    duplicate_of = value.get("duplicate_of")
    if isinstance(duplicate_of, str) and CANDIDATE_ID_RE.fullmatch(
        duplicate_of.strip()
    ):
        result["duplicate_of"] = duplicate_of.strip()
    suggestion_valid = value.get("suggestion_valid")
    if isinstance(suggestion_valid, bool):
        result["suggestion_valid"] = suggestion_valid
    return result
