#!/usr/bin/env python3
"""Deterministic report renderer for the Fabro code-review workflow.

Validates the canonical bundle written by `code_review.py final-tally` and
derives every presentation artifact from it: the Markdown, HTML, and JSONL
reports plus `metadata/revision.json`. No model output reaches a report
without passing this program's checks, and no finding text is ever placed
into markup -- the HTML report receives one escaped JSON payload and renders
it with `textContent`.

Python 3.9-compatible. Standard library only.
"""

from __future__ import annotations

import json
import os
import re
import sys
from pathlib import Path, PurePosixPath
from typing import Any, Dict, List, Mapping, NoReturn, Sequence, Tuple


CANONICAL_SCHEMA_VERSION = 3
TEMPLATE_RELATIVE_PATH = ("..", "templates", "report.html")
PAYLOAD_PLACEHOLDER = "__CODE_REVIEW_PAYLOAD__"

CATEGORIES = (
    "correctness",
    "reuse",
    "simplification",
    "efficiency",
    "altitude",
    "conventions",
    "test-coverage",
)
SEVERITIES = ("HIGH", "MEDIUM", "LOW")
FINDING_VERDICTS = ("CONFIRMED", "PLAUSIBLE", "UNVERIFIED")
VOTE_VERDICTS = ("CONFIRMED", "PLAUSIBLE", "REFUTED")
DISPOSITIONS = (
    "reportable",
    "refuted",
    "verification-incomplete",
    "deferred-by-cap",
    "duplicate",
)
VERIFICATION_STATUSES = ("complete", "partial", "skipped-low-effort")
COMPLETION_STATUSES = ("complete", "partial")
EFFORT_TIERS = ("low", "medium", "high", "xhigh", "max")
REVIEW_MODES = ("changes", "commit", "files")
FINDING_ID_RE = re.compile(r"^R[1-9][0-9]*$")
COMPILED_RULE_ID_RE = re.compile(
    r"^(builtin|repo):"
    r"[a-z0-9](?:[a-z0-9.-]{0,62}[a-z0-9])?"
    r"/"
    r"[a-z0-9](?:[a-z0-9.-]{0,62}[a-z0-9])?$"
)
MAX_TEXT = 8000
MAX_RULE_IDS_PER_FINDING = 50


class RenderError(RuntimeError):
    """A canonical-bundle or rendering failure."""


def die(message: str) -> NoReturn:
    raise RenderError(message)


# --- Validation helpers ------------------------------------------------------


def as_map(value: object) -> Dict[str, Any]:
    if not isinstance(value, dict):
        die("expected a JSON object")
    return value


def safe_text(value: object, field: str, allow_empty: bool = True) -> str:
    if not isinstance(value, str):
        die(f"{field} must be a string")
    if len(value) > MAX_TEXT:
        die(f"{field} exceeds the {MAX_TEXT}-character limit")
    if any(
        character not in "\n\t" and ord(character) < 0x20
        for character in value
    ):
        die(f"{field} contains control characters")
    if not allow_empty and not value.strip():
        die(f"{field} is empty")
    return value


def non_negative_int(value: object, field: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        die(f"{field} must be a non-negative integer")
    return value


def positive_int(value: object, field: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 1:
        die(f"{field} must be a positive integer")
    return value


def safe_repo_path(value: object, field: str) -> str:
    text = safe_text(value, field, allow_empty=False)
    if "\n" in text or "\t" in text:
        die(f"{field} contains control characters")
    path = PurePosixPath(text.replace("\\", "/"))
    if path.is_absolute() or ".." in path.parts:
        die(f"{field} is not a safe repository path")
    return path.as_posix()


def read_json(directory: str, name: str) -> Any:
    path = Path(directory) / name
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        die(f"canonical file is missing: {path}")
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        die(f"could not read {path}: {error}")


def read_jsonl(directory: str, name: str) -> List[Dict[str, Any]]:
    path = Path(directory) / name
    try:
        raw = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        die(f"canonical file is missing: {path}")
    except (OSError, UnicodeError) as error:
        die(f"could not read {path}: {error}")
    records: List[Dict[str, Any]] = []
    for number, line in enumerate(raw.splitlines(), 1):
        if not line.strip():
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError as error:
            die(f"{path}:{number} is not valid JSON: {error}")
        if not isinstance(value, dict):
            die(f"{path}:{number} must be a JSON object")
        records.append(value)
    return records


# --- Canonical-bundle validation ---------------------------------------------


def validate_manifest(value: object) -> Dict[str, Any]:
    manifest = as_map(value)
    if manifest.get("schema_version") != CANONICAL_SCHEMA_VERSION:
        die("review-manifest.json has an unsupported schema_version")
    safe_text(manifest.get("review_id"), "manifest review_id", allow_empty=False)
    if manifest.get("mode") not in REVIEW_MODES:
        die("manifest mode is not a known review mode")
    if manifest.get("effort") not in EFFORT_TIERS:
        die("manifest effort is not a known tier")
    if manifest.get("guidance") is not None:
        safe_text(manifest.get("guidance"), "manifest guidance")
    counts = as_map(manifest.get("counts"))
    for field in ("raw", "deduplicated", "sweep", "kept", "reported"):
        non_negative_int(counts.get(field), f"manifest counts.{field}")
    if "duplicates" in counts:
        non_negative_int(counts.get("duplicates"), "manifest counts.duplicates")
    completion = as_map(manifest.get("completion"))
    if completion.get("status") not in COMPLETION_STATUSES:
        die("manifest completion.status is invalid")
    verification = as_map(manifest.get("verification"))
    if verification.get("status") not in VERIFICATION_STATUSES:
        die("manifest verification.status is invalid")
    rules = manifest.get("rules")
    if rules is not None:
        rules = as_map(rules)
        for field in ("configSha256", "builtinManifestSha256"):
            value = rules.get(field)
            if not isinstance(value, str) or not re.fullmatch(
                r"[0-9a-f]{64}", value
            ):
                die(f"manifest rules.{field} must be a SHA-256 hex digest")
        counts = as_map(rules.get("counts"))
        for field in (
            "builtin_packs",
            "repo_packs",
            "builtin_checks",
            "repo_checks",
        ):
            non_negative_int(counts.get(field), f"manifest rules.counts.{field}")
    return manifest


def validate_code(value: object, field: str) -> Dict[str, Any]:
    code = as_map(value)
    safe_text(code.get("language"), f"{field}.language", allow_empty=False)
    safe_text(code.get("label"), f"{field}.label", allow_empty=False)
    lines = code.get("lines")
    if not isinstance(lines, list):
        die(f"{field}.lines must be an array")
    highlighted = 0
    normalized: List[Dict[str, Any]] = []
    for index, entry in enumerate(lines):
        record = as_map(entry)
        number = positive_int(record.get("number"), f"{field}.lines[{index}]")
        text = safe_text(record.get("text"), f"{field}.lines[{index}].text")
        if "\n" in text:
            die(f"{field}.lines[{index}].text spans lines")
        line: Dict[str, Any] = {"number": number, "text": text}
        if record.get("highlight"):
            line["highlight"] = True
            highlighted += 1
        normalized.append(line)
    if normalized and highlighted != 1:
        die(f"{field} must highlight exactly one line")
    return {
        "language": code["language"],
        "label": code["label"],
        "lines": normalized,
    }


def validate_finding(value: object, index: int) -> Dict[str, Any]:
    field = f"findings[{index}]"
    finding = as_map(value)
    display_id = safe_text(finding.get("id"), f"{field}.id", allow_empty=False)
    if not FINDING_ID_RE.fullmatch(display_id):
        die(f"{field}.id is not a valid display ID")
    path = safe_repo_path(finding.get("file"), f"{field}.file")
    line = positive_int(finding.get("line"), f"{field}.line")
    if finding.get("category") not in CATEGORIES:
        die(f"{field}.category is not in the closed list")
    if finding.get("severity") not in SEVERITIES:
        die(f"{field}.severity is invalid")
    if finding.get("confidence") not in SEVERITIES:
        die(f"{field}.confidence is invalid")
    if finding.get("verdict") not in FINDING_VERDICTS:
        die(f"{field}.verdict is invalid")
    reporters = finding.get("reporters")
    if not isinstance(reporters, list) or not all(
        isinstance(item, str) for item in reporters
    ):
        die(f"{field}.reporters must be an array of strings")
    rule_ids = finding.get("rule_ids", [])
    if not isinstance(rule_ids, list) or len(rule_ids) > (
        MAX_RULE_IDS_PER_FINDING
    ):
        die(f"{field}.rule_ids must be a bounded array")
    for rule_id in rule_ids:
        if not isinstance(rule_id, str) or not COMPILED_RULE_ID_RE.fullmatch(
            rule_id
        ):
            die(f"{field}.rule_ids contains an invalid compiled check ID")
    if len(set(rule_ids)) != len(rule_ids):
        die(f"{field}.rule_ids repeats a check ID")
    anchors = finding.get("anchors", [])
    if not isinstance(anchors, list) or len(anchors) > MAX_RULE_IDS_PER_FINDING:
        die(f"{field}.anchors must be a bounded array")
    normalized_anchors: List[Dict[str, Any]] = []
    for index, anchor in enumerate(anchors):
        record = as_map(anchor)
        anchor_field = f"{field}.anchors[{index}]"
        if record.get("category") not in CATEGORIES:
            die(f"{anchor_field}.category is not in the closed list")
        normalized_anchors.append(
            {
                "id": safe_text(record.get("id"), f"{anchor_field}.id", allow_empty=False),
                "file": safe_repo_path(record.get("file"), f"{anchor_field}.file"),
                "line": positive_int(record.get("line"), f"{anchor_field}.line"),
                "category": record["category"],
            }
        )
    return {
        "id": display_id,
        "file": path,
        "line": line,
        "summary": safe_text(
            finding.get("summary"), f"{field}.summary", allow_empty=False
        ),
        "short_summary": safe_text(
            finding.get("short_summary"),
            f"{field}.short_summary",
            allow_empty=False,
        ),
        "failure_scenario": safe_text(
            finding.get("failure_scenario"),
            f"{field}.failure_scenario",
            allow_empty=False,
        ),
        "category": finding["category"],
        "severity": finding["severity"],
        "confidence": finding["confidence"],
        "reports": positive_int(finding.get("reports"), f"{field}.reports"),
        "reporters": [safe_text(item, f"{field}.reporters") for item in reporters],
        "rule_ids": list(rule_ids),
        "anchors": normalized_anchors,
        "source": safe_text(finding.get("source"), f"{field}.source"),
        "verdict": finding["verdict"],
        "verdict_reasoning": safe_text(
            finding.get("verdict_reasoning"), f"{field}.verdict_reasoning"
        ),
        "code": validate_code(finding.get("code"), f"{field}.code"),
    }


def validate_findings(value: object) -> List[Dict[str, Any]]:
    if not isinstance(value, list):
        die("findings.json must contain a JSON array")
    findings = [
        validate_finding(entry, index) for index, entry in enumerate(value)
    ]
    seen_ids = {finding["id"] for finding in findings}
    if len(seen_ids) != len(findings):
        die("findings.json repeats a display ID")
    return findings


def finding_key(record: Mapping[str, Any]) -> Tuple[str, int, str]:
    return (
        str(record.get("file")),
        int(record.get("line") or 0),
        str(record.get("category")),
    )


def validate_ledger(records: Sequence[Mapping[str, Any]]) -> List[Dict[str, Any]]:
    validated: List[Dict[str, Any]] = []
    for index, value in enumerate(records):
        field = f"ledger[{index}]"
        record = as_map(dict(value))
        safe_text(record.get("id"), f"{field}.id", allow_empty=False)
        safe_repo_path(record.get("file"), f"{field}.file")
        positive_int(record.get("line"), f"{field}.line")
        if record.get("category") not in CATEGORIES:
            die(f"{field}.category is not in the closed list")
        if record.get("disposition") not in DISPOSITIONS:
            die(f"{field}.disposition is invalid")
        validated.append(dict(record))
    return validated


def validate_votes(records: Sequence[Mapping[str, Any]]) -> List[Dict[str, Any]]:
    validated: List[Dict[str, Any]] = []
    for index, value in enumerate(records):
        field = f"votes[{index}]"
        record = as_map(dict(value))
        completed = record.get("completed")
        if not isinstance(completed, bool):
            die(f"{field}.completed must be a boolean")
        if completed:
            if record.get("verdict") not in VOTE_VERDICTS:
                die(f"{field}.verdict is invalid")
            safe_text(record.get("reasoning"), f"{field}.reasoning")
        validated.append(dict(record))
    return validated


def validate_coverage(value: object) -> Dict[str, Any]:
    coverage = as_map(value)
    finders = as_map(coverage.get("finders"))
    non_negative_int(finders.get("dispatched"), "coverage.finders.dispatched")
    non_negative_int(finders.get("returned"), "coverage.finders.returned")
    verification = as_map(coverage.get("verification"))
    if verification.get("status") not in VERIFICATION_STATUSES:
        die("coverage.verification.status is invalid")
    non_negative_int(
        verification.get("votesDispatched"),
        "coverage.verification.votesDispatched",
    )
    non_negative_int(
        verification.get("votesCompleted"),
        "coverage.verification.votesCompleted",
    )
    rejected = coverage.get("rejectedFindingReports")
    if not isinstance(rejected, list) or not all(
        isinstance(item, str) for item in rejected
    ):
        die("coverage.rejectedFindingReports must be an array of strings")
    filtered = coverage.get("filteredFindingReports", [])
    if not isinstance(filtered, list) or not all(
        isinstance(item, str) for item in filtered
    ):
        die("coverage.filteredFindingReports must be an array of strings")
    rules = coverage.get("rules")
    if rules is not None:
        rules = as_map(rules)
        effective = as_map(rules.get("effectiveChecksByFile"))
        for path, check_ids in effective.items():
            safe_repo_path(path, "coverage.rules.effectiveChecksByFile key")
            if not isinstance(check_ids, list) or not all(
                isinstance(check_id, str)
                and COMPILED_RULE_ID_RE.fullmatch(check_id)
                for check_id in check_ids
            ):
                die(
                    "coverage.rules.effectiveChecksByFile values must be "
                    "arrays of compiled check IDs"
                )
    return coverage


def validate_relationships(
    manifest: Mapping[str, Any],
    findings: Sequence[Mapping[str, Any]],
    ledger: Sequence[Mapping[str, Any]],
    votes: Sequence[Mapping[str, Any]],
    coverage: Mapping[str, Any],
) -> None:
    counts = manifest["counts"]
    if counts["reported"] != len(findings):
        die("manifest counts.reported does not match findings.json")
    reportable = [
        record for record in ledger if record["disposition"] == "reportable"
    ]
    if len(reportable) != len(findings):
        die("reportable ledger records do not match findings.json")
    ledger_keys = {finding_key(record) for record in reportable}
    finding_keys = {finding_key(finding) for finding in findings}
    if ledger_keys != finding_keys:
        die("reportable ledger records and findings.json disagree")
    verification = as_map(coverage.get("verification"))
    if verification["status"] != manifest["verification"]["status"]:
        die("coverage and manifest verification statuses disagree")
    if verification["votesDispatched"] != len(votes):
        die("coverage vote counts do not match votes.jsonl")
    completed = sum(1 for vote in votes if vote.get("completed"))
    if verification["votesCompleted"] != completed:
        die("coverage completed-vote count does not match votes.jsonl")
    if manifest["effort"] == "low":
        if any(finding["verdict"] != "UNVERIFIED" for finding in findings):
            die("a low-effort review cannot carry verified verdicts")
    else:
        if any(finding["verdict"] == "UNVERIFIED" for finding in findings):
            die("a verified tier cannot report an UNVERIFIED finding")
    coverage_rules = coverage.get("rules")
    manifest_rules = manifest.get("rules")
    if (coverage_rules is None) != (manifest_rules is None):
        die("manifest and coverage disagree about rule compilation")
    if coverage_rules is not None:
        if as_map(coverage_rules).get("configSha256") != as_map(
            manifest_rules
        ).get("configSha256"):
            die("manifest and coverage rule configuration hashes disagree")
        effective = as_map(as_map(coverage_rules).get("effectiveChecksByFile"))
        for finding in findings:
            allowed = effective.get(finding["file"]) or []
            for rule_id in finding.get("rule_ids") or []:
                if rule_id not in allowed:
                    die(
                        "a reported finding names a rule check that is not "
                        "effective for its file"
                    )
    else:
        for finding in findings:
            if finding.get("rule_ids"):
                die(
                    "a reported finding names rule checks but the review "
                    "compiled no rules"
                )


def partial_reasons(
    manifest: Mapping[str, Any],
    coverage: Mapping[str, Any],
) -> List[str]:
    # A completed review carries no partial banner. Report-cap deferral at
    # the rule-mapped tiers is a completed policy selection: it stays
    # visible in the coverage section and the ledger, not here.
    if manifest["completion"]["status"] == "complete":
        return []
    reasons: List[str] = []
    finders = coverage["finders"]
    missing = finders["dispatched"] - finders["returned"]
    if missing > 0:
        reasons.append(
            f"{missing} finder angle(s) returned no usable result"
        )
    verification = coverage["verification"]
    incomplete = verification.get("incomplete")
    if isinstance(incomplete, int) and incomplete > 0:
        reasons.append(
            f"{incomplete} candidate(s) have no verdict and were not reported"
        )
    rejected = coverage.get("rejectedFindingReports") or []
    if rejected:
        reasons.append(
            f"{len(rejected)} reported finding(s) failed the finding contract"
        )
    sweep = coverage.get("sweep")
    if isinstance(sweep, dict) and sweep.get("planned") and not sweep.get(
        "returned"
    ):
        reasons.append("the planned gap-fill sweep returned no usable result")
    caps = coverage.get("caps")
    if isinstance(caps, dict):
        deferred = caps.get("verificationDeferred")
        if isinstance(deferred, int) and deferred > 0:
            reasons.append(
                f"{deferred} candidate(s) were deferred by the verification cap"
            )
        report_deferred = caps.get("reportDeferred")
        if isinstance(report_deferred, int) and report_deferred > 0:
            reasons.append(
                f"{report_deferred} kept finding(s) were cut by the report cap"
            )
    if manifest["completion"]["status"] == "partial" and not reasons:
        reasons.append("the review recorded a partial completion")
    return reasons


# --- Markdown rendering ------------------------------------------------------


MARKDOWN_ESCAPES = str.maketrans(
    {
        "\\": "\\\\",
        "`": "\\`",
        "*": "\\*",
        "_": "\\_",
        "[": "\\[",
        "]": "\\]",
        "<": "&lt;",
        ">": "&gt;",
        "|": "\\|",
        "#": "\\#",
        "~": "\\~",
    }
)


def escape_markdown(value: object) -> str:
    return str("" if value is None else value).translate(MARKDOWN_ESCAPES)


def code_span(value: object) -> str:
    text = str("" if value is None else value).replace("`", "'")
    return f"`{text}`"


def code_block(code: Mapping[str, Any]) -> List[str]:
    lines = code.get("lines") or []
    if not lines:
        return []
    fence = "````"
    body: List[str] = [fence + "text"]
    width = max(len(str(entry["number"])) for entry in lines)
    for entry in lines:
        marker = ">" if entry.get("highlight") else " "
        body.append(
            f"{marker} {str(entry['number']).rjust(width)} | {entry['text']}"
        )
    body.append(fence)
    return body


def describe_target(manifest: Mapping[str, Any]) -> str:
    mode = str(manifest.get("mode"))
    if mode == "files":
        scope = manifest.get("scope") or []
        return f"files mode, {len(scope)} scope path(s)"
    range_text = manifest.get("range") or "(unknown range)"
    return f"{mode} mode, range {range_text}"


def revision_summary(manifest: Mapping[str, Any]) -> str:
    revision = manifest.get("revision")
    if not isinstance(revision, dict) or not revision.get("versioned"):
        return "unversioned tree"
    commit = str(revision.get("commit") or "")[:12] or "(unknown)"
    branch = revision.get("branch")
    return f"commit {commit}" + (f" on {branch}" if branch else "")


def finding_markdown(finding: Mapping[str, Any]) -> List[str]:
    location = f"{finding['file']}:{finding['line']}"
    rule_ids = finding.get("rule_ids") or []
    lines = [
        f"### {finding['id']} · {finding['severity']} "
        f"{finding['category']} — {escape_markdown(finding['short_summary'])}",
        "",
        f"{code_span(location)} · verdict {finding['verdict']} · "
        f"confidence {finding['confidence']} · reported by "
        f"{finding['reports']} pass(es) "
        f"({escape_markdown(', '.join(finding['reporters']))})"
        + (
            " · rule " + ", ".join(code_span(item) for item in rule_ids)
            if rule_ids
            else ""
        ),
    ]
    anchors = finding.get("anchors") or []
    if anchors:
        lines.append(
            "Also reported at "
            + ", ".join(
                f"{code_span(anchor['file'] + ':' + str(anchor['line']))} "
                f"({anchor['category']}, {escape_markdown(anchor['id'])})"
                for anchor in anchors
            )
            + " -- judged the same defect and folded in."
        )
    if finding["summary"].strip() != finding["short_summary"].strip():
        lines.extend(["", escape_markdown(finding["summary"])])
    lines.extend(
        ["", f"**Failure scenario.** {escape_markdown(finding['failure_scenario'])}"]
    )
    reasoning = str(finding.get("verdict_reasoning") or "").strip()
    if reasoning:
        lines.extend(["", f"**Verifier.** {escape_markdown(reasoning)}"])
    excerpt = code_block(finding["code"])
    if excerpt:
        lines.extend(["", *excerpt])
    lines.append("")
    return lines


def render_markdown(
    manifest: Mapping[str, Any],
    findings: Sequence[Mapping[str, Any]],
    coverage: Mapping[str, Any],
    reasons: Sequence[str],
) -> str:
    correctness = sum(
        1 for finding in findings if finding["category"] == "correctness"
    )
    cleanup = len(findings) - correctness
    lines: List[str] = [
        "# Code review results",
        "",
        f"- **Target:** {escape_markdown(describe_target(manifest))}",
        f"- **Revision:** {escape_markdown(revision_summary(manifest))}",
        f"- **Effort:** {manifest['effort']}"
        + (
            f" · **Model:** {escape_markdown(manifest.get('model'))}"
            if manifest.get("model")
            else ""
        ),
        *(
            [f"- **Guidance:** {escape_markdown(manifest['guidance'])}"]
            if manifest.get("guidance")
            else []
        ),
        f"- **Completed:** {escape_markdown(manifest.get('completed_at'))}",
        f"- **Verification:** {manifest['verification']['status']} · "
        f"**Completion:** {manifest['completion']['status']}",
        "",
        f"**{len(findings)} finding(s) reported** "
        f"({correctness} correctness, {cleanup} cleanup).",
        "",
    ]
    if manifest["effort"] == "low":
        lines.extend(
            [
                "This was a low-effort single-pass review: findings were not "
                "independently verified.",
                "",
            ]
        )
    if reasons:
        lines.append("> **Partial review.** " + " ".join(
            escape_markdown(reason) + "." for reason in reasons
        ))
        lines.append("")
    if findings:
        lines.append("## Findings")
        lines.append("")
        for finding in findings:
            lines.extend(finding_markdown(finding))
    else:
        lines.extend(["No findings survived review.", ""])
    finders = coverage["finders"]
    verification = coverage["verification"]
    lines.extend(
        [
            "## Coverage",
            "",
            f"- Finder jobs: {finders['returned']} of "
            f"{finders['dispatched']} returned a usable result.",
            f"- Verification: {verification['votesCompleted']} of "
            f"{verification['votesDispatched']} verdict(s) returned "
            f"({verification['status']}).",
        ]
    )
    rules = coverage.get("rules")
    if isinstance(rules, dict):
        counts = rules.get("counts") or {}
        lines.append(
            "- Rules: "
            f"{counts.get('builtin_packs', 0)} built-in and "
            f"{counts.get('repo_packs', 0)} repository pack(s) "
            f"({counts.get('builtin_checks', 0)} + "
            f"{counts.get('repo_checks', 0)} check(s)) applied by path."
        )
        failed_cells = rules.get("failedAuditCells") or []
        if failed_cells:
            lines.append(
                f"- Rule audits: {len(failed_cells)} cell(s) returned no "
                "usable result; their files and checks are recorded as "
                "uncovered."
            )
    caps = coverage.get("caps") or {}
    report_deferred = caps.get("reportDeferred")
    if isinstance(report_deferred, int) and report_deferred > 0:
        lines.append(
            f"- Report cap: {report_deferred} additional kept finding(s) "
            "are recorded in the candidate ledger."
        )
    rejected = coverage.get("rejectedFindingReports") or []
    if rejected:
        lines.append(
            f"- Rejected finding reports ({len(rejected)}):"
        )
        lines.extend(
            f"  - {escape_markdown(entry)}" for entry in rejected
        )
    filtered = coverage.get("filteredFindingReports") or []
    if filtered:
        lines.append(
            f"- Filtered by review policy ({len(filtered)}):"
        )
        lines.extend(
            f"  - {escape_markdown(entry)}" for entry in filtered
        )
    lines.append("")
    return "\n".join(lines)


# --- HTML and JSONL rendering ------------------------------------------------


def embed_json(value: object) -> str:
    text = json.dumps(value, ensure_ascii=True, separators=(",", ":"))
    return (
        text.replace("&", "\\u0026")
        .replace("<", "\\u003c")
        .replace(">", "\\u003e")
    )


def read_template() -> str:
    path = Path(__file__).resolve().parent.joinpath(*TEMPLATE_RELATIVE_PATH)
    try:
        template = path.read_text(encoding="utf-8")
    except OSError as error:
        die(f"could not read the HTML template: {error}")
    if template.count(PAYLOAD_PLACEHOLDER) != 1:
        die("the HTML template must contain the payload placeholder once")
    return template


def render_html(
    manifest: Mapping[str, Any],
    findings: Sequence[Mapping[str, Any]],
    coverage: Mapping[str, Any],
    reasons: Sequence[str],
) -> str:
    payload = {
        "meta": {
            "target": describe_target(manifest),
            "revision": revision_summary(manifest),
            "mode": manifest.get("mode"),
            "effort": manifest.get("effort"),
            "model": manifest.get("model"),
            "guidance": manifest.get("guidance") or "",
            "completed_at": manifest.get("completed_at"),
            "verification": manifest["verification"]["status"],
            "completion": manifest["completion"]["status"],
            "counts": manifest.get("counts"),
        },
        "partialReasons": list(reasons),
        "findings": [dict(finding) for finding in findings],
        "coverage": dict(coverage),
    }
    return read_template().replace(PAYLOAD_PLACEHOLDER, embed_json(payload))


def jsonl_line(finding: Mapping[str, Any]) -> str:
    record = {
        key: finding[key]
        for key in (
            "id",
            "file",
            "line",
            "category",
            "severity",
            "confidence",
            "verdict",
            "short_summary",
            "summary",
            "failure_scenario",
            "reports",
            "rule_ids",
            "anchors",
            "source",
        )
    }
    return json.dumps(record, ensure_ascii=False, separators=(",", ":"))


# --- Entry point -------------------------------------------------------------


def atomic_write(path: Path, text: str) -> None:
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(text, encoding="utf-8")
    os.replace(temporary, path)


def render(
    evidence_dir: str,
    products_dir: str,
    metadata_dir: str,
) -> Tuple[List[Dict[str, Any]], Dict[str, Any]]:
    manifest = validate_manifest(read_json(evidence_dir, "review-manifest.json"))
    findings = validate_findings(read_json(evidence_dir, "findings.json"))
    ledger = validate_ledger(read_jsonl(evidence_dir, "candidate-ledger.jsonl"))
    votes = validate_votes(read_jsonl(evidence_dir, "votes.jsonl"))
    coverage = validate_coverage(read_json(evidence_dir, "coverage.json"))
    validate_relationships(manifest, findings, ledger, votes, coverage)
    reasons = partial_reasons(manifest, coverage)

    products = Path(products_dir)
    atomic_write(
        products / "CODE-REVIEW-RESULTS.md",
        render_markdown(manifest, findings, coverage, reasons),
    )
    atomic_write(
        products / "CODE-REVIEW-RESULTS.html",
        render_html(manifest, findings, coverage, reasons),
    )
    atomic_write(
        products / "CODE-REVIEW-RESULTS.jsonl",
        "".join(jsonl_line(finding) + "\n" for finding in findings),
    )
    revision = {
        "schema_version": CANONICAL_SCHEMA_VERSION,
        "review_id": manifest.get("review_id"),
        "completed_at": manifest.get("completed_at"),
        "mode": manifest.get("mode"),
        "effort": manifest.get("effort"),
        "model": manifest.get("model"),
        "guidance": manifest.get("guidance"),
        "scope": manifest.get("scope"),
        "range": manifest.get("range"),
        "revision": manifest.get("revision"),
        "counts": manifest.get("counts"),
        "verification": manifest.get("verification"),
        "completion": manifest.get("completion"),
        "evidence_dir": evidence_dir,
    }
    atomic_write(
        Path(metadata_dir) / "revision.json",
        json.dumps(revision, ensure_ascii=False, indent=2, sort_keys=True)
        + "\n",
    )
    return findings, dict(manifest["verification"])


def main(argv: Sequence[str]) -> int:
    if len(argv) != 3:
        print(
            "usage: render_report.py <evidence_dir> <products_dir> "
            "<metadata_dir>",
            file=sys.stderr,
        )
        return 2
    try:
        findings, verification = render(argv[0], argv[1], argv[2])
    except RenderError as error:
        print(f"render_report.py: {error}", file=sys.stderr)
        return 2
    print(
        f"Rendered {len(findings)} finding(s); verification "
        f"{verification.get('status')}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
