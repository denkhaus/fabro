#!/usr/bin/env python3
"""Candidate planning and verification.

Deduplicates and ranks candidates, builds verification claims and jobs,
applies verdicts, summarizes sweep coverage, and runs the tally command.

Python 3.9-compatible. Standard library only."""

from __future__ import annotations

import sys
from pathlib import Path
from typing import (
    Any,
    Dict,
    List,
    Mapping,
    Optional,
    Sequence,
    Tuple,
)

sys.path.insert(0, str(Path(__file__).resolve().parent))

from git_target import (  # noqa: E402
    common_target,
    resolved_location,
)
from job_planning import (  # noqa: E402
    phase_jobs_context,
    set_phase_jobs,
)
from normalize import (  # noqa: E402
    bounded_rule_ids,
    normalize_findings_result,
    normalize_verdict,
    state_rule_context,
)
from review_common import (  # noqa: E402
    CONFIDENCE_RANK,
    KEPT_VERDICTS,
    SEVERITY_RANK,
    SIBLING_CAP,
    SWEEP_CANDIDATE_CAP,
    SWEEP_FOCUS,
    emit,
    load_state,
    one_line,
    save_state,
)


# --- Candidate planning and verification -------------------------------------


def candidate_key(finding: Mapping[str, Any]) -> str:
    """The deduplication identity: normalized file, line, and category."""
    return "\0".join(
        [
            str(finding.get("file")),
            str(finding.get("line")),
            str(finding.get("category")),
        ]
    )


def category_class(finding: Mapping[str, Any]) -> int:
    return 0 if finding.get("category") == "correctness" else 1


def rank_key(finding: Mapping[str, Any]) -> Tuple[Any, ...]:
    return (
        category_class(finding),
        -SEVERITY_RANK.get(str(finding.get("severity")), 0),
        -int(finding.get("reports") or 1),
        -CONFIDENCE_RANK.get(str(finding.get("confidence")), 0),
        str(finding.get("file")),
        int(finding.get("line") or 0),
        str(finding.get("category")),
    )


def sibling_claims(
    candidate: Mapping[str, Any],
    pool: Sequence[Mapping[str, Any]],
) -> List[Dict[str, Any]]:
    """Other candidates in the same file, nearest by line first."""
    line = int(candidate.get("line") or 0)
    same_file = [
        other
        for other in pool
        if other.get("file") == candidate.get("file")
        and other.get("id") != candidate.get("id")
    ]
    same_file.sort(
        key=lambda other: (
            abs(int(other.get("line") or 0) - line),
            str(other.get("id")),
        )
    )
    return [
        {
            "id": other.get("id"),
            "line": other.get("line"),
            "category": other.get("category"),
            "short_summary": other.get("short_summary"),
        }
        for other in same_file[:SIBLING_CAP]
    ]


def verification_claim(
    candidate: Mapping[str, Any],
    state: Optional[Mapping[str, Any]] = None,
    pool: Optional[Sequence[Mapping[str, Any]]] = None,
) -> Dict[str, Any]:
    """The subset of a candidate a verifier is shown.

    The reporter's claim only. The reporter's confidence is withheld -- it
    could anchor a verifier that must judge the claim on the code. At the
    rule-mapped tiers the claim also carries the claimed rule IDs and every
    effective check for the candidate's file; the engine stays authoritative
    about applicability, and the verifier judges only violation. ``pool``
    supplies the same-file siblings the verifier may name as duplicates.
    """
    start_line = int(candidate.get("start_line") or candidate.get("line") or 0)
    end_line = int(candidate.get("end_line") or candidate.get("line") or 0)
    location = resolved_location(
        str(candidate.get("file") or ""), start_line, end_line, state
    )
    claim: Dict[str, Any] = {
        "file": candidate.get("file"),
        "line": candidate.get("line"),
        "location": location,
        "category": candidate.get("category"),
        "issue_type": candidate.get("issue_type"),
        "severityAsReported": candidate.get("severity"),
        "summary": candidate.get("summary"),
        "failure_scenario": candidate.get("failure_scenario"),
        "reports": int(candidate.get("reports") or 1),
        "siblings": sibling_claims(candidate, pool or []),
    }
    suggestion_code = str(candidate.get("suggestion_code") or "")
    if suggestion_code and location["existing_code"]:
        claim["suggestion"] = {"replacement_code": suggestion_code}
    rules_state = (state or {}).get("rules")
    if isinstance(rules_state, dict) and rules_state.get("enabled"):
        catalog = rules_state.get("catalog") or {}
        effective_ids = (rules_state.get("effective") or {}).get(
            str(candidate.get("file")), []
        )
        claim["rule_ids"] = list(candidate.get("rule_ids") or [])
        claim["effective_checks"] = [
            {
                "id": check["id"],
                "category": check["category"],
                "guidance": check["guidance"],
                "source": check["source"],
                "pack": check["pack"],
                "pattern": check["pattern"],
            }
            for check in (
                catalog.get(check_id) for check_id in effective_ids
            )
            if isinstance(check, dict)
        ]
    return claim


def build_verify_jobs(
    state: Mapping[str, Any],
    candidates: Sequence[Mapping[str, Any]],
    phase: str,
    pool: Optional[Sequence[Mapping[str, Any]]] = None,
) -> List[Dict[str, Any]]:
    bias = str(state.get("verify_bias") or "standard")
    target = common_target(state)
    prefix = "verify" if phase == "verify" else "sweep-verify"
    siblings_pool = list(pool if pool is not None else candidates)
    jobs: List[Dict[str, Any]] = []
    for candidate in candidates:
        jobs.append(
            {
                "name": f"{prefix}:{candidate['id']}",
                "job_id": f"{prefix}:{candidate['id']}",
                "candidate_id": candidate["id"],
                "claim": verification_claim(candidate, state, siblings_pool),
                "bias": bias,
                "target": target,
            }
        )
    return jobs


def stored_claim(
    state: Mapping[str, Any],
    phase: str,
    candidate: Mapping[str, Any],
) -> Dict[str, Any]:
    """The exact claim a verifier was shown, from the dispatched job."""
    for job in (state.get("phase_jobs") or {}).get(phase) or []:
        if isinstance(job, dict) and job.get("candidate_id") == candidate.get(
            "id"
        ) and isinstance(job.get("claim"), dict):
            return dict(job["claim"])
    return verification_claim(candidate, state)


def plan_verify() -> None:
    state = load_state()
    finder_jobs = list(state.get("finder_jobs") or [])
    finder_results = (
        state.get("phase_results", {}).get("finders", {})
        if isinstance(state.get("phase_results"), dict)
        else {}
    )
    rule_context = state_rule_context(state, require=False)
    raw_candidates: List[Dict[str, Any]] = []
    returned = 0
    invalid_results: List[str] = []
    kind_stats: Dict[str, Dict[str, Any]] = {}
    job_outcomes: Dict[str, str] = {}
    for job in finder_jobs:
        if not isinstance(job, dict):
            continue
        kind = str(job.get("kind") or "angle")
        stats = kind_stats.setdefault(
            kind, {"dispatched": 0, "returned": 0, "invalid": []}
        )
        stats["dispatched"] += 1
        job_id = str(job.get("job_id") or "")
        raw = (
            finder_results.get(job.get("job_id"))
            if isinstance(finder_results, dict)
            else None
        )
        normalized = normalize_findings_result(raw, rule_context)
        if normalized is None:
            invalid_results.append(one_line(job.get("name"), 200))
            stats["invalid"].append(one_line(job.get("name"), 200))
            job_outcomes[job_id] = "failed"
            continue
        returned += 1
        stats["returned"] += 1
        job_outcomes[job_id] = "returned"
        # A whole-change angle reports under its angle key; local and
        # rule-audit jobs report under their stable job IDs.
        if kind == "angle" and isinstance(job.get("angle"), dict):
            reporter = str(job["angle"].get("key") or "finder")
        else:
            reporter = job_id or "finder"
        job_cap = int(job.get("candidate_cap") or state.get("per_angle_cap") or 6)
        job_findings = sorted(normalized["findings"], key=rank_key)
        dropped_by_job_cap = max(0, len(job_findings) - job_cap)
        if dropped_by_job_cap:
            state.setdefault("job_cap_drops", {})[reporter] = (
                dropped_by_job_cap
            )
        for finding in job_findings[:job_cap]:
            copy = dict(finding)
            copy["angle"] = one_line(reporter, 60)
            raw_candidates.append(copy)

    # Two angles flagging the same line for different reasons stay separate
    # findings; the same defect reported twice under one category merges,
    # keeping the union of reporter job IDs and applicable rule IDs.
    by_key: Dict[str, Dict[str, Any]] = {}
    for report in sorted(raw_candidates, key=rank_key):
        key = candidate_key(report)
        existing = by_key.get(key)
        if existing is None:
            merged = dict(report)
            merged["reports"] = 1
            merged["reporters"] = [report["angle"]]
            merged["rule_ids"] = bounded_rule_ids(report.get("rule_ids") or [])
            by_key[key] = merged
            continue
        existing["reports"] += 1
        if report["angle"] not in existing["reporters"]:
            existing["reporters"].append(report["angle"])
        existing["rule_ids"] = bounded_rule_ids(
            list(existing.get("rule_ids") or [])
            + list(report.get("rule_ids") or [])
        )
        # A fix is publishable only when reporting passes agree on its exact
        # range and replacement. A pass that offers no fix does not veto an
        # otherwise consistent proposal.
        existing_suggestion = str(existing.get("suggestion_code") or "")
        report_suggestion = str(report.get("suggestion_code") or "")
        if not existing_suggestion and report_suggestion:
            existing["start_line"] = report["start_line"]
            existing["end_line"] = report["end_line"]
            existing["line"] = report["end_line"]
            existing["suggestion_code"] = report_suggestion
        elif existing_suggestion and report_suggestion and (
            existing_suggestion != report_suggestion
            or existing.get("start_line") != report.get("start_line")
            or existing.get("end_line") != report.get("end_line")
        ):
            existing["suggestion_code"] = ""
        if report.get("issue_type") == "security":
            existing["issue_type"] = "security"
        if (
            SEVERITY_RANK[report["severity"]]
            > SEVERITY_RANK[existing["severity"]]
        ):
            existing["severity"] = report["severity"]
        if (
            CONFIDENCE_RANK[report["confidence"]]
            > CONFIDENCE_RANK[existing["confidence"]]
        ):
            existing["confidence"] = report["confidence"]

    deduplicated = sorted(by_key.values(), key=rank_key)
    for index, candidate in enumerate(deduplicated, 1):
        candidate["id"] = f"F{index}"
        candidate["source"] = "finder"

    use_verify = bool(state.get("use_verify"))
    verification_cap = int(state.get("verification_cap") or 60)
    for_verification = deduplicated[:verification_cap]
    deferred_by_cap = max(0, len(deduplicated) - len(for_verification))
    state["finder_kind_stats"] = kind_stats
    state["finder_job_outcomes"] = job_outcomes
    verify_jobs = (
        build_verify_jobs(state, for_verification, "verify")
        if use_verify
        else []
    )

    state["raw_candidate_count"] = len(raw_candidates)
    state["candidates"] = deduplicated
    state["verification_candidates"] = for_verification if use_verify else []
    state["verification_deferred_by_cap"] = (
        deferred_by_cap if use_verify else 0
    )
    state["verify_jobs"] = verify_jobs
    state["run_verify"] = bool(verify_jobs)
    state["finders_dispatched"] = len(finder_jobs)
    state["finders_returned"] = returned
    state["invalid_finder_results"] = invalid_results
    set_phase_jobs(state, "verify", verify_jobs)
    save_state(state)
    updates: Dict[str, Any] = {"run_verify": bool(verify_jobs)}
    if verify_jobs:
        updates.update(phase_jobs_context(state, "verify", verify_jobs))
    if invalid_results:
        print(
            f"finders: {len(invalid_results)} of {len(finder_jobs)} finder "
            "agent(s) did not return a usable result"
        )
    if use_verify and deferred_by_cap:
        print(
            f"verification cap: {deferred_by_cap} lower-ranked candidate(s) "
            "will not be verified or reported -- the ledger records them as "
            "deferred"
        )
    print(
        f"Candidates: {len(raw_candidates)} raw -> "
        f"{len(deduplicated)} deduplicated; "
        f"{len(verify_jobs)} verification job(s)"
    )
    emit(**updates)


def candidate_verdict(
    state: Mapping[str, Any],
    phase: str,
    candidate: Mapping[str, Any],
) -> Optional[Dict[str, str]]:
    phase_results = state.get("phase_results")
    if not isinstance(phase_results, dict):
        return None
    results = phase_results.get(phase)
    if not isinstance(results, dict):
        return None
    prefix = "verify" if phase == "verify" else "sweep-verify"
    return normalize_verdict(results.get(f"{prefix}:{candidate.get('id')}"))


def apply_verdicts(
    state: Mapping[str, Any],
    phase: str,
    candidates: Sequence[Mapping[str, Any]],
) -> List[Dict[str, Any]]:
    """Attach each candidate's verdict and decide whether it is kept.

    The keep rule is the same at every verified tier: CONFIRMED and PLAUSIBLE
    survive, REFUTED drops, and a candidate whose verifier returned nothing is
    verification-incomplete and is not reported.
    """
    reviewed: List[Dict[str, Any]] = []
    for candidate in candidates:
        verdict = candidate_verdict(state, phase, candidate)
        record = {
            "candidate": dict(candidate),
            "verdict": verdict,
            "kept": verdict is not None
            and verdict["verdict"] in KEPT_VERDICTS,
        }
        reviewed.append(record)
    return reviewed


def sweep_coverage_summary(state: Mapping[str, Any]) -> Dict[str, Any]:
    """A compact map of what discovery covered, for the gap-fill sweep."""
    outcomes = state.get("finder_job_outcomes") or {}
    jobs = state.get("finder_jobs") or []
    grouping = state.get("grouping") if isinstance(state.get("grouping"), dict) else {}
    angle_returned: List[str] = []
    angle_failed: List[str] = []
    cells_returned: List[str] = []
    cells_failed: List[str] = []
    uncovered_files: set = set()
    uncovered_checks: set = set()
    for job in jobs:
        if not isinstance(job, dict):
            continue
        kind = str(job.get("kind") or "angle")
        job_id = str(job.get("job_id") or "")
        returned = outcomes.get(job_id) == "returned"
        if kind == "rule-audit":
            (cells_returned if returned else cells_failed).append(job_id)
            if not returned:
                uncovered_files.update(job.get("files") or [])
                uncovered_checks.update(
                    check.get("id")
                    for check in job.get("checks") or []
                    if isinstance(check, dict)
                )
        else:
            (angle_returned if returned else angle_failed).append(job_id)
            if not returned and kind == "local-correctness":
                uncovered_files.update(job.get("files") or [])
    return {
        "fileGroups": list(grouping.get("groups") or []),
        "angleJobs": {"returned": angle_returned, "failed": angle_failed},
        "ruleAuditCells": {
            "returned": cells_returned,
            "failed": cells_failed,
        },
        "uncoveredFiles": sorted(uncovered_files),
        "uncoveredCheckIds": sorted(
            check_id for check_id in uncovered_checks if check_id
        ),
    }


def tally() -> None:
    state = load_state()
    use_verify = bool(state.get("use_verify"))
    candidates = list(state.get("candidates") or [])
    if use_verify:
        for_verification = list(state.get("verification_candidates") or [])
        reviewed = apply_verdicts(state, "verify", for_verification)
    else:
        # Low effort skips verification by design; every candidate carries
        # through unverified, and the report says so.
        reviewed = [
            {"candidate": dict(candidate), "verdict": None, "kept": True}
            for candidate in candidates
        ]
    state["reviewed"] = reviewed
    kept = [record for record in reviewed if record["kept"]]

    run_sweep = bool(state.get("use_sweep"))
    state["run_sweep"] = run_sweep
    updates: Dict[str, Any] = {"run_sweep": run_sweep}
    if run_sweep:
        verified_summary = [
            {
                "id": record["candidate"].get("id"),
                "file": record["candidate"].get("file"),
                "line": record["candidate"].get("line"),
                "category": record["candidate"].get("category"),
                "short_summary": record["candidate"].get("short_summary"),
            }
            for record in kept
        ]
        sweep_assignment = {
            "verified": verified_summary,
            "candidate_cap": SWEEP_CANDIDATE_CAP,
            "focus": SWEEP_FOCUS,
            "stance": str(state.get("verify_bias") or "standard"),
            "target": common_target(state),
        }
        if state.get("rule_mapped"):
            sweep_assignment["coverage"] = sweep_coverage_summary(state)
        state["sweep_assignment"] = sweep_assignment
        updates["sweep_assignment"] = sweep_assignment
    save_state(state)
    verdict_count = sum(
        1 for record in reviewed if record.get("verdict") is not None
    )
    if use_verify:
        print(
            f"Verification returned {verdict_count} verdict(s) for "
            f"{len(reviewed)} candidate(s); {len(kept)} kept"
        )
    else:
        print(
            f"Low effort: verification skipped; {len(kept)} candidate(s) "
            "carry through unverified"
        )
    emit(**updates)
