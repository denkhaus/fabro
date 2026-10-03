#!/usr/bin/env python3
"""Parallel-phase merge commands.

Merges the grouping proposal, finder/verifier/sweep outputs, and the
plan-finders command that turns the merged grouping into discovery jobs.

Python 3.9-compatible. Standard library only."""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import (
    Any,
    Dict,
    List,
    Optional,
)

sys.path.insert(0, str(Path(__file__).resolve().parent))

from git_target import normalize_repo_path  # noqa: E402
from job_planning import (  # noqa: E402
    build_discovery_jobs,
    chunk_paths,
    finalize_groups,
    phase_jobs_context,
    set_phase_jobs,
)
from normalize import (  # noqa: E402
    findings_and_rejections,
    normalize_verdict,
    state_rule_context,
)
from review_common import (  # noqa: E402
    DISCOVERY_JOB_CEILING,
    MAX_STDIN_BYTES,
    PHASE_OUTPUT_KEYS,
    SWEEP_CANDIDATE_CAP,
    WorkflowDataError,
    emit,
    load_state,
    one_line,
    save_state,
)
from verification import (  # noqa: E402
    build_verify_jobs,
    candidate_key,
    rank_key,
)


# --- Parallel merges ---------------------------------------------------------


def read_merge_input() -> Any:
    raw = sys.stdin.buffer.read(MAX_STDIN_BYTES + 1)
    if len(raw) > MAX_STDIN_BYTES:
        raise WorkflowDataError(
            f"merge input exceeds the {MAX_STDIN_BYTES}-byte limit"
        )
    try:
        return json.loads(raw.decode("utf-8"))
    except (UnicodeError, json.JSONDecodeError) as error:
        raise WorkflowDataError(
            f"merge stdin is not valid JSON: {error}"
        ) from error


def merge_phase(
    state: Dict[str, Any],
    phase: str,
    raw_results: Any,
) -> Dict[str, Any]:
    if phase not in PHASE_OUTPUT_KEYS:
        raise WorkflowDataError(f"unknown parallel merge phase: {phase}")
    if not isinstance(raw_results, list):
        raise WorkflowDataError("parallel merge input must be a JSON array")
    jobs = (
        state.get("phase_jobs", {}).get(phase)
        if isinstance(state.get("phase_jobs"), dict)
        else None
    )
    if not isinstance(jobs, list):
        raise WorkflowDataError(f"{phase} merge jobs are missing from state")
    result_map = state.setdefault("phase_results", {}).setdefault(phase, {})
    if not isinstance(result_map, dict):
        raise WorkflowDataError(f"{phase} result accumulator is invalid")
    for position, branch in enumerate(raw_results):
        if position >= len(jobs) or not isinstance(branch, dict):
            continue
        branch_index = branch.get("index")
        if (
            branch_index is not None
            and (
                isinstance(branch_index, bool)
                or not isinstance(branch_index, int)
                or branch_index != position
            )
        ):
            continue
        updates = branch.get("context_updates")
        if not isinstance(updates, dict):
            continue
        value = updates.get(PHASE_OUTPUT_KEYS[phase])
        job = jobs[position]
        if not isinstance(job, dict):
            continue
        rejections: List[str] = []
        filtered: List[str] = []
        if phase == "finders":
            # Rejections are recorded here, where the agent's raw output is
            # first seen. Later steps re-normalize an already-clean result and
            # would find nothing to report.
            normalized, rejections, filtered = findings_and_rejections(
                value,
                state_rule_context(
                    state, require=job.get("kind") == "rule-audit"
                ),
            )
        else:
            normalized = normalize_verdict(value)
        if normalized is None:
            continue
        job_id = job.get("job_id")
        if isinstance(job_id, str) and job_id:
            if job_id not in result_map:
                result_map[job_id] = normalized
                for key, reasons in (
                    ("rejected_findings", rejections),
                    ("filtered_findings", filtered),
                ):
                    if reasons:
                        state.setdefault(key, {})[job_id] = [
                            f"{one_line(job.get('name'), 200)}: {reason}"
                            for reason in reasons
                        ]
    return {f"{phase}_results_merged": len(result_map)}


def merge_grouping(state: Dict[str, Any], raw: Any) -> Dict[str, Any]:
    """Record the grouping agent's proposal after minimal normalization.

    Unusable output is not an error: plan-finders falls back to
    deterministic lexical chunks and coverage records the degradation.
    """
    groups_raw: Optional[List[List[str]]] = None
    if isinstance(raw, dict) and isinstance(raw.get("groups"), list):
        collected: List[List[str]] = []
        for entry in raw["groups"]:
            files = entry.get("files") if isinstance(entry, dict) else None
            if not isinstance(files, list):
                continue
            paths: List[str] = []
            for item in files:
                normalized = normalize_repo_path(item)
                if normalized is not None and normalized != ".":
                    paths.append(normalized)
            if paths:
                collected.append(paths)
        if collected:
            groups_raw = collected
    grouping = state.setdefault("grouping", {})
    if isinstance(grouping, dict):
        grouping["agent_returned"] = groups_raw is not None
    state["grouping_raw"] = groups_raw
    return {"grouping_merged": groups_raw is not None}


def plan_finders() -> None:
    """Finalize file groups and build the rule-mapped discovery jobs."""
    state = load_state()
    if not state.get("rule_mapped"):
        raise WorkflowDataError(
            "plan-finders only runs for the rule-mapped tiers"
        )
    grouping = state.get("grouping")
    if not isinstance(grouping, dict):
        grouping = {}
    raw = state.get("grouping_raw")
    groups, fallback, corrections = finalize_groups(state, raw)
    if not grouping.get("planned"):
        # A single-file target never dispatched the grouping agent; its
        # lexical group is the plan, not a degradation.
        fallback = None
        corrections = []
    jobs = build_discovery_jobs(state, groups)
    if len(jobs) > DISCOVERY_JOB_CEILING and fallback is None:
        # Grouping degradation alone never trips the ceiling: lexical
        # chunking is the densest exact-coverage packing.
        lexical = chunk_paths(sorted(state.get("changed_files") or []))
        lexical_jobs = build_discovery_jobs(state, lexical)
        if len(lexical_jobs) <= DISCOVERY_JOB_CEILING:
            groups, jobs = lexical, lexical_jobs
            fallback = "lexical"
            corrections.append("regrouped lexically to fit the job ceiling")
    if len(jobs) > DISCOVERY_JOB_CEILING:
        raise WorkflowDataError(
            f"exact coverage needs {len(jobs)} discovery jobs, over the "
            f"{DISCOVERY_JOB_CEILING}-job discovery ceiling. Narrow the review "
            "scope and run again; files and rule checks are never silently "
            "omitted"
        )
    grouping.update(
        {"fallback": fallback, "corrections": corrections, "groups": groups}
    )
    state["grouping"] = grouping
    state["finder_jobs"] = jobs
    set_phase_jobs(state, "finders", jobs)
    save_state(state)
    by_kind: Dict[str, int] = {}
    for job in jobs:
        by_kind[str(job.get("kind"))] = by_kind.get(str(job.get("kind")), 0) + 1
    print(
        f"Planned {len(jobs)} discovery job(s): "
        f"{by_kind.get('local-correctness', 0)} local-correctness, "
        f"{by_kind.get('angle', 0)} whole-change angle(s), "
        f"{by_kind.get('rule-audit', 0)} rule-audit cell(s)"
        + (f"; grouping fallback: {fallback}" if fallback else "")
    )
    emit(**phase_jobs_context(state, "finders", jobs))


def merge_sweep(state: Dict[str, Any], raw: Any) -> Dict[str, Any]:
    """Record the single sweeper's result and plan verification of what's new.

    Sweep candidates are deduplicated against every candidate already seen --
    kept or not -- so a candidate the panel already refuted cannot reappear
    through the sweep.
    """
    normalized, rejections, filtered = findings_and_rejections(
        raw, state_rule_context(state, require=False)
    )
    if normalized is None:
        state["sweep_returned"] = False
        state["sweep_candidates"] = []
        state["sweep_verify_jobs"] = []
        state["run_sweep_verify"] = False
        set_phase_jobs(state, "sweep_verify", [])
        return {"run_sweep_verify": False}
    for key, reasons in (
        ("rejected_findings", rejections),
        ("filtered_findings", filtered),
    ):
        if reasons:
            state.setdefault(key, {})["sweep"] = [
                f"sweep: {reason}" for reason in reasons
            ]
    seen = {
        candidate_key(candidate)
        for candidate in state.get("candidates") or []
    }
    fresh: List[Dict[str, Any]] = []
    for finding in normalized["findings"]:
        key = candidate_key(finding)
        if key in seen:
            continue
        seen.add(key)
        copy = dict(finding)
        copy["reports"] = 1
        copy["source"] = "sweep"
        fresh.append(copy)
    fresh.sort(key=rank_key)
    fresh = fresh[:SWEEP_CANDIDATE_CAP]
    for index, candidate in enumerate(fresh, 1):
        candidate["id"] = f"S{index}"

    use_verify = bool(state.get("use_verify"))
    # A sweep candidate's siblings include the kept finder findings, so a
    # re-found defect can be folded into the finding already on the list.
    kept_finder = [
        record["candidate"]
        for record in state.get("reviewed") or []
        if isinstance(record, dict) and record.get("kept")
    ]
    jobs = (
        build_verify_jobs(state, fresh, "sweep_verify", pool=fresh + kept_finder)
        if use_verify
        else []
    )
    state["sweep_returned"] = True
    state["sweep_candidates"] = fresh
    state["sweep_verify_jobs"] = jobs
    state["run_sweep_verify"] = bool(jobs)
    set_phase_jobs(state, "sweep_verify", jobs)
    updates: Dict[str, Any] = {"run_sweep_verify": bool(jobs)}
    if jobs:
        updates.update(phase_jobs_context(state, "sweep_verify", jobs))
    return updates


def merge(phase: str) -> None:
    state = load_state()
    raw = read_merge_input()
    if phase == "grouping":
        updates = merge_grouping(state, raw)
        print(
            "Merged grouping: "
            + (
                f"{len(state.get('grouping_raw') or [])} proposed group(s)"
                if state.get("grouping_raw")
                else "no usable grouping; plan-finders will fall back to "
                "lexical chunks"
            )
        )
    elif phase == "sweep":
        updates = merge_sweep(state, raw)
        print(
            f"Merged sweep: {len(state.get('sweep_candidates') or [])} fresh "
            f"candidate(s), {len(state.get('sweep_verify_jobs') or [])} "
            "verification job(s)"
        )
    else:
        updates = merge_phase(state, phase, raw)
        print(
            f"Merged {phase}: "
            f"{updates[f'{phase}_results_merged']} result(s) recorded"
        )
    if phase in PHASE_OUTPUT_KEYS:
        # The merge has copied every usable branch output into canonical
        # state. Do not let the next fan-out inherit this fan-in payload.
        # Fabro includes inherited context changes in each branch result, so
        # retaining an earlier parallel.results array multiplies it by the
        # next branch count and can exceed the checkpoint request limit.
        updates["parallel.results"] = []
    save_state(state)
    emit(**updates)
