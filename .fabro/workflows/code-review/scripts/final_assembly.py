#!/usr/bin/env python3
"""Final assembly: the canonical review bundle.

Reportable findings with engine-derived locations and code frames (the
source reads live in git_target), vote records, the calibration summary,
duplicate folding, and the final-tally command. Python 3.9-compatible.
Standard library only."""

from __future__ import annotations

import sys
from datetime import datetime, timezone
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

from review_contract import MAX_RULE_IDS_PER_FINDING  # noqa: E402
from git_target import (  # noqa: E402
    assert_workspace_unchanged,
    code_frame,
    resolved_location,
)
from normalize import bounded_rule_ids  # noqa: E402
from review_common import (  # noqa: E402
    CANONICAL_FILES,
    CANONICAL_SCHEMA_VERSION,
    emit,
    load_state,
    root,
    save_state,
    write_json,
    write_jsonl,
)
from verification import (  # noqa: E402
    apply_verdicts,
    candidate_key,
    rank_key,
    stored_claim,
    sweep_coverage_summary,
    tally,
)


# --- Final assembly ----------------------------------------------------------


def reportable_finding(
    record: Mapping[str, Any],
    display_id: str,
    state: Optional[Mapping[str, Any]] = None,
) -> Dict[str, Any]:
    candidate = record["candidate"]
    verdict = record.get("verdict")
    start_line = int(candidate.get("start_line") or candidate["line"])
    end_line = int(candidate.get("end_line") or candidate["line"])
    location = resolved_location(
        candidate["file"], start_line, end_line, state
    )
    finding = {
        "id": display_id,
        "file": candidate["file"],
        "line": end_line,
        "location": location,
        "summary": candidate["summary"],
        "short_summary": candidate["short_summary"],
        "failure_scenario": candidate["failure_scenario"],
        "category": candidate["category"],
        "issue_type": candidate["issue_type"],
        "severity": candidate["severity"],
        "confidence": candidate["confidence"],
        "reports": int(candidate.get("reports") or 1),
        "reporters": list(candidate.get("reporters") or [])
        or [str(candidate.get("angle") or candidate.get("source") or "")],
        "rule_ids": list(candidate.get("rule_ids") or []),
        "anchors": list(candidate.get("anchors") or []),
        "source": candidate.get("source", "finder"),
        "verdict": verdict["verdict"] if verdict else "UNVERIFIED",
        "verdict_reasoning": verdict["reasoning"] if verdict else "",
        "code": code_frame(
            candidate["file"], start_line, end_line, state
        ),
    }
    suggestion_code = str(candidate.get("suggestion_code") or "")
    if (
        suggestion_code
        and location["existing_code"]
        and verdict is not None
        and verdict.get("suggestion_valid") is True
        and suggestion_code != location["existing_code"]
    ):
        finding["suggestion"] = {"replacement_code": suggestion_code}
    return finding


def finding_reports(state: Mapping[str, Any], key: str) -> List[str]:
    """Flatten per-job rejection or filter reports in job-ID order."""
    by_job = state.get(key)
    if not isinstance(by_job, dict):
        return []
    reports: List[str] = []
    for job_id in sorted(by_job):
        entries = by_job[job_id]
        if isinstance(entries, list):
            reports.extend(str(entry) for entry in entries)
    return reports


def vote_records(
    state: Mapping[str, Any],
    reviewed: Sequence[Mapping[str, Any]],
    phase: str,
) -> List[Dict[str, Any]]:
    records: List[Dict[str, Any]] = []
    for record in reviewed:
        candidate = record["candidate"]
        verdict = record.get("verdict")
        entry: Dict[str, Any] = {
            "phase": phase,
            "candidate_id": candidate.get("id"),
            "claim": stored_claim(
                state, "verify" if phase == "verify" else "sweep_verify", candidate
            ),
            "bias": str(state.get("verify_bias") or "standard"),
            "completed": verdict is not None,
        }
        if verdict is not None:
            entry["verdict"] = verdict["verdict"]
            entry["reasoning"] = verdict["reasoning"]
            if verdict.get("duplicate_of"):
                entry["duplicate_of"] = verdict["duplicate_of"]
            if "suggestion_valid" in verdict:
                entry["suggestion_valid"] = verdict["suggestion_valid"]
        records.append(entry)
    return records


def reporter_kind(reporter: str) -> str:
    if reporter.startswith("finder:local:"):
        return "local-correctness"
    if reporter.startswith("finder:rule:"):
        return "rule-audit"
    if reporter == "sweep":
        return "sweep"
    return "angle"


def calibration_summary(
    state: Mapping[str, Any],
    candidates: Sequence[Mapping[str, Any]],
    ledger: Sequence[Mapping[str, Any]],
    votes: Sequence[Mapping[str, Any]],
    coverage: Mapping[str, Any],
) -> Dict[str, Any]:
    """A compact, aggregatable account of how the run's candidates fared.

    Emitted into the workflow context so calibration across many runs can
    read it from the event log without downloading bundles: dispositions
    and verdicts overall, then per reporter kind, per reporter, per rule
    check, and per category, plus rejection reasons and cap drops. Reasons
    and IDs are engine strings; no model text is included.
    """
    disposition_by_id = {
        str(entry.get("id")): str(entry.get("disposition"))
        for entry in ledger
    }

    def tally(bucket: Dict[str, int], disposition: str) -> None:
        bucket["candidates"] += 1
        if disposition in {"reportable", "deferred-by-cap", "duplicate"}:
            bucket["kept"] += 1
        elif disposition == "refuted":
            bucket["refuted"] += 1
        elif disposition == "verification-incomplete":
            bucket["incomplete"] += 1

    def fresh_bucket() -> Dict[str, int]:
        return {"candidates": 0, "kept": 0, "refuted": 0, "incomplete": 0}

    by_kind: Dict[str, Dict[str, int]] = {}
    by_reporter: Dict[str, Dict[str, int]] = {}
    by_rule: Dict[str, Dict[str, int]] = {}
    by_category: Dict[str, Dict[str, int]] = {}
    for candidate in candidates:
        disposition = disposition_by_id.get(str(candidate.get("id")), "")
        reporters = list(candidate.get("reporters") or []) or [
            str(candidate.get("source") or "finder")
        ]
        for reporter in reporters:
            tally(by_reporter.setdefault(reporter, fresh_bucket()), disposition)
            tally(
                by_kind.setdefault(reporter_kind(reporter), fresh_bucket()),
                disposition,
            )
        for rule_id in candidate.get("rule_ids") or []:
            tally(by_rule.setdefault(str(rule_id), fresh_bucket()), disposition)
        tally(
            by_category.setdefault(
                str(candidate.get("category")), fresh_bucket()
            ),
            disposition,
        )

    dispositions: Dict[str, int] = {}
    for entry in ledger:
        key = str(entry.get("disposition"))
        dispositions[key] = dispositions.get(key, 0) + 1
    verdicts: Dict[str, int] = {}
    for vote in votes:
        if vote.get("completed"):
            key = str(vote.get("verdict"))
            verdicts[key] = verdicts.get(key, 0) + 1
    rejections: Dict[str, int] = {}
    for report in coverage.get("rejectedFindingReports") or []:
        reason = str(report).rsplit(": ", 1)[-1]
        rejections[reason] = rejections.get(reason, 0) + 1
    filtered: Dict[str, int] = {}
    for report in coverage.get("filteredFindingReports") or []:
        reason = str(report).rsplit(": ", 1)[-1]
        filtered[reason] = filtered.get(reason, 0) + 1

    grouping = coverage.get("grouping") or {}
    rules = coverage.get("rules") or {}
    finders = coverage.get("finders") or {}
    caps = coverage.get("caps") or {}
    return {
        "effort": state.get("effort"),
        "mode": state.get("mode"),
        "model": state.get("model"),
        "targetFiles": len(state.get("changed_files") or []),
        "changedLines": state.get("diff_lines"),
        "collapsed": coverage.get("collapsed"),
        "grouping": {
            "mode": grouping.get("mode"),
            "fallback": grouping.get("fallback"),
            "groups": len(grouping.get("groups") or []),
        },
        "ruleLayers": rules.get("layers"),
        "jobs": {
            "dispatched": finders.get("dispatched", 0),
            "returned": finders.get("returned", 0),
            "byKind": {
                kind: {
                    "dispatched": stats.get("dispatched", 0),
                    "returned": stats.get("returned", 0),
                }
                for kind, stats in (finders.get("byKind") or {}).items()
            },
        },
        "candidates": {
            "raw": int(state.get("raw_candidate_count") or 0),
            "deduplicated": len(state.get("candidates") or []),
            "sweep": len(state.get("sweep_candidates") or []),
        },
        "dispositions": dispositions,
        "verdicts": verdicts,
        "byKind": by_kind,
        "byReporter": by_reporter,
        "byRule": by_rule,
        "byCategory": by_category,
        "rejections": rejections,
        "filtered": filtered,
        "caps": {
            "jobDrops": sum(
                int(value) for value in (caps.get("perJobDrops") or {}).values()
            ),
            "verificationDeferred": caps.get("verificationDeferred", 0),
            "reportDeferred": caps.get("reportDeferred", 0),
        },
    }


def fold_duplicates(
    state: Mapping[str, Any],
    kept_records: Sequence[Dict[str, Any]],
) -> Tuple[List[Dict[str, Any]], Dict[str, str]]:
    """Fold verified duplicates into the finding they duplicate.

    A verifier may name a sibling as ``duplicate_of``. The fold is
    deterministic: the named sibling must have been shown to that verifier
    and must itself have survived; the lower-ranked finding folds into the
    higher-ranked one (a mutual claim resolves the same way), chains follow
    to their surviving root, and the primary gains the secondary's anchor,
    reporters, rule IDs, and report count. Returns the surviving primaries,
    re-ranked, and the secondary-to-primary map.
    """
    allowed: Dict[str, set] = {}
    for phase in ("verify", "sweep_verify"):
        for job in (state.get("phase_jobs") or {}).get(phase) or []:
            if not isinstance(job, dict):
                continue
            siblings = (job.get("claim") or {}).get("siblings") or []
            allowed[str(job.get("candidate_id"))] = {
                str(sibling.get("id"))
                for sibling in siblings
                if isinstance(sibling, dict)
            }
    ordered = sorted(kept_records, key=lambda record: rank_key(record["candidate"]))
    by_id = {str(record["candidate"].get("id")): record for record in ordered}
    rank_index = {
        str(record["candidate"].get("id")): index
        for index, record in enumerate(ordered)
    }
    folded: Dict[str, str] = {}

    def root(candidate_id: str) -> str:
        seen = set()
        while candidate_id in folded and candidate_id not in seen:
            seen.add(candidate_id)
            candidate_id = folded[candidate_id]
        return candidate_id

    for record in ordered:
        candidate_id = str(record["candidate"].get("id"))
        target = (record.get("verdict") or {}).get("duplicate_of")
        if (
            not target
            or target == candidate_id
            or target not in allowed.get(candidate_id, set())
            or target not in by_id
            or rank_index[target] > rank_index[candidate_id]
        ):
            continue
        primary_id = root(target)
        if primary_id != candidate_id:
            folded[candidate_id] = primary_id

    for secondary_id, primary_id in folded.items():
        primary = by_id[primary_id]["candidate"]
        secondary = by_id[secondary_id]["candidate"]
        primary["reports"] = int(primary.get("reports") or 1) + int(
            secondary.get("reports") or 1
        )
        reporters = list(primary.get("reporters") or [])
        for reporter in secondary.get("reporters") or [
            str(secondary.get("source") or "")
        ]:
            if reporter and reporter not in reporters:
                reporters.append(reporter)
        primary["reporters"] = reporters
        primary["rule_ids"] = bounded_rule_ids(
            list(primary.get("rule_ids") or [])
            + list(secondary.get("rule_ids") or [])
        )
        anchors = primary.setdefault("anchors", [])
        if len(anchors) < MAX_RULE_IDS_PER_FINDING:
            anchors.append(
                {
                    "id": secondary_id,
                    "file": secondary.get("file"),
                    "line": secondary.get("line"),
                    "category": secondary.get("category"),
                }
            )
    primaries = [
        record
        for record in ordered
        if str(record["candidate"].get("id")) not in folded
    ]
    for record in primaries:
        anchors = record["candidate"].get("anchors")
        if anchors:
            anchors.sort(key=lambda anchor: (str(anchor["file"]), int(anchor["line"])))
    primaries.sort(key=lambda record: rank_key(record["candidate"]))
    return primaries, folded


def final_tally() -> None:
    state = load_state()
    assert_workspace_unchanged(state)
    use_verify = bool(state.get("use_verify"))
    reviewed = list(state.get("reviewed") or [])
    sweep_candidates = list(state.get("sweep_candidates") or [])
    sweep_reviewed = (
        apply_verdicts(state, "sweep_verify", sweep_candidates)
        if state.get("run_sweep_verify")
        else [
            {"candidate": dict(candidate), "verdict": None, "kept": not use_verify}
            for candidate in sweep_candidates
        ]
    )
    state["sweep_reviewed"] = sweep_reviewed

    kept_records = [record for record in reviewed if record["kept"]]
    kept_records.extend(
        record for record in sweep_reviewed if record["kept"]
    )
    kept_records.sort(key=lambda record: rank_key(record["candidate"]))
    kept_records, folded = fold_duplicates(state, kept_records)
    report_cap = int(state.get("report_cap") or 8)
    reported_records = kept_records[:report_cap]
    deferred_by_report_cap = max(0, len(kept_records) - len(reported_records))

    findings = [
        reportable_finding(record, f"R{index}", state)
        for index, record in enumerate(reported_records, 1)
    ]

    reported_keys = {
        candidate_key(record["candidate"]) for record in reported_records
    }
    ledger: List[Dict[str, Any]] = []
    verification_incomplete = 0

    def ledger_entry(
        record: Mapping[str, Any],
        disposition: str,
    ) -> Dict[str, Any]:
        candidate = record["candidate"]
        verdict = record.get("verdict")
        entry = {
            "id": candidate.get("id"),
            "file": candidate.get("file"),
            "line": candidate.get("line"),
            "start_line": candidate.get("start_line"),
            "end_line": candidate.get("end_line"),
            "category": candidate.get("category"),
            "issue_type": candidate.get("issue_type"),
            "severity": candidate.get("severity"),
            "confidence": candidate.get("confidence"),
            "reports": int(candidate.get("reports") or 1),
            "rule_ids": list(candidate.get("rule_ids") or []),
            "source": candidate.get("source", "finder"),
            "short_summary": candidate.get("short_summary"),
            "summary": candidate.get("summary"),
            "failure_scenario": candidate.get("failure_scenario"),
            "disposition": disposition,
        }
        if candidate.get("suggestion_code"):
            entry["suggestion_code"] = candidate["suggestion_code"]
        if verdict is not None:
            entry["verdict"] = verdict["verdict"]
        if disposition == "duplicate":
            entry["duplicate_of"] = folded.get(str(candidate.get("id")))
        if candidate.get("anchors"):
            entry["anchors"] = list(candidate["anchors"])
        return entry

    verified_ids = {
        record["candidate"].get("id") for record in reviewed
    }

    def disposition_for(record: Mapping[str, Any]) -> str:
        candidate = record["candidate"]
        if str(candidate.get("id")) in folded:
            return "duplicate"
        if candidate_key(candidate) in reported_keys and record["kept"]:
            return "reportable"
        if record["kept"]:
            return "deferred-by-cap"
        if record.get("verdict") is None and use_verify:
            return "verification-incomplete"
        return "refuted"

    for candidate in state.get("candidates") or []:
        record = next(
            (
                entry
                for entry in reviewed
                if entry["candidate"].get("id") == candidate.get("id")
            ),
            None,
        )
        if record is None:
            if use_verify and candidate.get("id") not in verified_ids:
                ledger.append(
                    ledger_entry(
                        {"candidate": candidate, "verdict": None},
                        "deferred-by-cap",
                    )
                )
            continue
        disposition = disposition_for(record)
        if disposition == "verification-incomplete":
            verification_incomplete += 1
        ledger.append(ledger_entry(record, disposition))
    for record in sweep_reviewed:
        disposition = disposition_for(record)
        if disposition == "verification-incomplete":
            verification_incomplete += 1
        ledger.append(ledger_entry(record, disposition))

    votes = (
        vote_records(state, reviewed, "verify")
        + vote_records(state, sweep_reviewed, "sweep-verify")
        if use_verify
        else []
    )
    completed_votes = sum(1 for vote in votes if vote.get("completed"))

    if not use_verify:
        verification_status = "skipped-low-effort"
    elif verification_incomplete:
        verification_status = "partial"
    else:
        verification_status = "complete"

    finders_dispatched = int(state.get("finders_dispatched") or 0)
    finders_returned = int(state.get("finders_returned") or 0)
    rule_mapped = bool(state.get("rule_mapped"))
    coverage = {
        "finders": {
            "dispatched": finders_dispatched,
            "returned": finders_returned,
            "invalid": list(state.get("invalid_finder_results") or []),
        },
        "sweep": {
            "planned": bool(state.get("run_sweep")),
            "returned": bool(state.get("sweep_returned")),
        },
        "verification": {
            "status": verification_status,
            "bias": state.get("verify_bias"),
            "votesDispatched": len(votes),
            "votesCompleted": completed_votes,
            "incomplete": verification_incomplete,
        },
        "caps": {
            "perJobDrops": dict(state.get("job_cap_drops") or {}),
            "verificationDeferred": int(
                state.get("verification_deferred_by_cap") or 0
            ),
            "reportDeferred": deferred_by_report_cap,
        },
        "rejectedFindingReports": finding_reports(state, "rejected_findings"),
        "filteredFindingReports": finding_reports(state, "filtered_findings"),
    }
    if rule_mapped:
        coverage["finders"]["byKind"] = dict(
            state.get("finder_kind_stats") or {}
        )
        grouping = (
            state.get("grouping")
            if isinstance(state.get("grouping"), dict)
            else {}
        )
        rules_state = (
            state.get("rules") if isinstance(state.get("rules"), dict) else {}
        )
        discovery = sweep_coverage_summary(state)
        coverage["targetFiles"] = list(state.get("changed_files") or [])
        coverage["collapsed"] = state.get("collapsed")
        coverage["grouping"] = {
            "mode": grouping.get("mode"),
            "planned": bool(grouping.get("planned")),
            "agentReturned": bool(grouping.get("agent_returned")),
            "fallback": grouping.get("fallback"),
            "corrections": list(grouping.get("corrections") or []),
            "groups": list(grouping.get("groups") or []),
        }
        coverage["rules"] = {
            "layers": rules_state.get("layers"),
            "configSha256": rules_state.get("config_sha256"),
            "repoRuleRevision": rules_state.get("repo_rule_revision"),
            "repoRuleFiles": list(rules_state.get("repo_rule_files") or []),
            "counts": dict(rules_state.get("counts") or {}),
            "effectiveChecksByFile": dict(rules_state.get("effective") or {}),
            # The compiled text of every effective check, so the renderer
            # can attach guidance to rule-derived findings (SARIF rule help).
            "checkCatalog": {
                check_id: {
                    "category": check.get("category"),
                    "guidance": check.get("guidance"),
                }
                for check_id, check in sorted(
                    (rules_state.get("catalog") or {}).items()
                )
            },
            "overriddenBuiltinChecksByFile": dict(
                rules_state.get("overridden") or {}
            ),
            "mFileClassification": dict(rules_state.get("sniff") or {}),
            "failedAuditCells": discovery["ruleAuditCells"]["failed"],
            "uncoveredFiles": discovery["uncoveredFiles"],
            "uncoveredCheckIds": discovery["uncoveredCheckIds"],
        }
    completion_partial = (
        finders_returned < finders_dispatched
        or verification_status == "partial"
        or bool(coverage["rejectedFindingReports"])
        or bool(state.get("run_sweep")) and not state.get("sweep_returned")
        # Report-cap deferral is a completed policy selection at the
        # rule-mapped tiers: recorded in coverage and the ledger, but not a
        # completion failure. The low tier keeps its original meaning.
        or (not rule_mapped and deferred_by_report_cap > 0)
        or int(state.get("verification_deferred_by_cap") or 0) > 0
    )
    completed_at = (
        datetime.now(timezone.utc).replace(microsecond=0).isoformat()
    )
    manifest = {
        "schema_version": CANONICAL_SCHEMA_VERSION,
        "review_id": state.get("review_id"),
        "started_at": state.get("started_at"),
        "completed_at": completed_at,
        "mode": state.get("mode"),
        "effort": state.get("effort"),
        "model": state.get("model"),
        "guidance": state.get("guidance") or "",
        "scope": state.get("scope") or [],
        "range": state.get("range"),
        "revision": state.get("revision"),
        "counts": {
            "raw": int(state.get("raw_candidate_count") or 0),
            "deduplicated": len(state.get("candidates") or []),
            "sweep": len(sweep_candidates),
            "kept": len(kept_records),
            "duplicates": len(folded),
            "reported": len(findings),
        },
        "completion": {
            "status": "partial" if completion_partial else "complete",
        },
        "verification": {"status": verification_status},
        "canonical_files": list(CANONICAL_FILES),
    }
    if rule_mapped:
        rules_state = (
            state.get("rules") if isinstance(state.get("rules"), dict) else {}
        )
        manifest["rules"] = {
            "layers": rules_state.get("layers"),
            "configSha256": rules_state.get("config_sha256"),
            "builtinManifestSha256": rules_state.get(
                "builtin_manifest_sha256"
            ),
            "repoRuleRevision": rules_state.get("repo_rule_revision"),
            "counts": dict(rules_state.get("counts") or {}),
        }

    coverage["calibration"] = calibration_summary(
        state,
        list(state.get("candidates") or []) + sweep_candidates,
        ledger,
        votes,
        coverage,
    )

    state["completed_at"] = completed_at
    state["review_manifest"] = manifest
    state["final_findings"] = findings
    state["final_coverage"] = coverage
    save_state(state)

    evidence_dir = Path(str(state["evidence_dir"]))
    write_json(evidence_dir / "review-manifest.json", manifest)
    write_jsonl(evidence_dir / "candidate-ledger.jsonl", ledger)
    write_json(evidence_dir / "findings.json", findings)
    write_json(evidence_dir / "coverage.json", coverage)
    write_jsonl(evidence_dir / "votes.jsonl", votes)

    print(
        f"Final result: {len(findings)} finding(s) reported of "
        f"{len(kept_records)} kept ({deferred_by_report_cap} beyond the "
        f"report cap); verification {verification_status}, completion "
        f"{manifest['completion']['status']}"
    )
    emit(
        reported_count=len(findings),
        verification_status=verification_status,
        products_dir=state["products_rel"],
        evidence_dir=state["evidence_rel"],
        metadata_dir=state["metadata_rel"],
        canonical_bundle_written=True,
        calibration=coverage["calibration"],
    )
