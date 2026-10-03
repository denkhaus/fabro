#!/usr/bin/env python3
"""Grouping and rule-mapped job planning.

Lexical chunking and semantic group finalization, rule-audit cells, the
discovery-job assembly (local-correctness, whole-change angles, rule
audits), the low tier's finder jobs, and phase-job bookkeeping on the state.

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

from git_target import common_target  # noqa: E402
from review_common import (  # noqa: E402
    EFFORT_CELLS,
    GROUP_CHAR_BUDGET,
    GROUP_MAX_FILES,
    LOCAL_CORRECTNESS_INSTRUCTIONS,
    MAX_CHECKS_PER_CELL,
    PHASE_JOB_KEYS,
    STANCE_RULE_AUDIT,
    WHOLE_CHANGE_ANGLES,
    WorkflowDataError,
)


# --- Grouping and rule-mapped job planning -------------------------------------


def path_cost(path: str) -> int:
    return len(path) + 16


def chunk_paths(paths: Sequence[str]) -> List[List[str]]:
    """Split an ordered path list at the group size and size-estimate caps."""
    chunks: List[List[str]] = []
    current: List[str] = []
    cost = 0
    for path in paths:
        item_cost = path_cost(path)
        if current and (
            len(current) >= GROUP_MAX_FILES
            or cost + item_cost > GROUP_CHAR_BUDGET
        ):
            chunks.append(current)
            current = []
            cost = 0
        current.append(path)
        cost += item_cost
    if current:
        chunks.append(current)
    return chunks


def finalize_groups(
    state: Mapping[str, Any],
    raw_groups: Optional[Sequence[Sequence[str]]],
) -> Tuple[List[List[str]], Optional[str], List[str]]:
    """Turn the grouping agent's proposal into exact target-file coverage.

    The semantic choice can be model-authored; file coverage cannot be.
    Returns (groups, fallback, corrections); corrections are fixed engine
    strings that never quote model text.
    """
    target = list(state.get("changed_files") or [])
    target_set = set(target)
    corrections: List[str] = []
    fallback: Optional[str] = None
    if raw_groups:
        seen = set()
        unknown = duplicates = 0
        groups: List[List[str]] = []
        for raw in raw_groups:
            cleaned: List[str] = []
            for path in raw:
                if path not in target_set:
                    unknown += 1
                    continue
                if path in seen:
                    duplicates += 1
                    continue
                seen.add(path)
                cleaned.append(path)
            if cleaned:
                groups.append(sorted(cleaned))
        if unknown:
            corrections.append(f"ignored {unknown} unknown path(s)")
        if duplicates:
            corrections.append(
                f"kept the first assignment for {duplicates} duplicate "
                "path(s)"
            )
        omitted = [path for path in target if path not in seen]
        if omitted:
            corrections.append(
                f"added {len(omitted)} omitted file(s) in lexical chunks"
            )
            groups.extend(chunk_paths(sorted(omitted)))
        split_groups: List[List[str]] = []
        oversized = 0
        for group in groups:
            chunks = chunk_paths(group)
            if len(chunks) > 1:
                oversized += 1
            split_groups.extend(chunks)
        if oversized:
            corrections.append(f"split {oversized} oversized group(s)")
        groups = split_groups
        if not groups:
            fallback = "lexical"
            corrections.append(
                "the grouping result assigned no target files; fell back to "
                "lexical chunks"
            )
            groups = chunk_paths(sorted(target))
    else:
        fallback = "lexical"
        groups = chunk_paths(sorted(target))

    flat = [path for group in groups for path in group]
    if sorted(flat) != sorted(target) or len(flat) != len(set(flat)):
        raise WorkflowDataError(
            "file grouping lost exact target coverage; refusing to plan"
        )
    groups.sort(key=lambda group: group[0])
    return groups, fallback, corrections


def build_rule_audit_cells(
    state: Mapping[str, Any],
    groups: Sequence[Sequence[str]],
) -> List[Dict[str, Any]]:
    """Pack files sharing one effective check set into audit cells.

    Packing is across the whole target, not within semantic groups: a rule
    audit checks each file against the same guidance regardless of its
    neighbors, so grouping only multiplied cells (calibration measured
    rule cells as 43% of finder agents for 11% of candidates). Cells are
    deterministic -- lexical files per check set, the ten-file and size
    caps applied -- and ordered by check set, then first file.
    """
    rules_state = state.get("rules") or {}
    effective: Mapping[str, Sequence[str]] = rules_state.get("effective") or {}
    by_check_set: Dict[Tuple[str, ...], List[str]] = {}
    for path in sorted(path for group in groups for path in group):
        check_ids = tuple(effective.get(path) or ())
        if check_ids:
            by_check_set.setdefault(check_ids, []).append(path)
    cells: List[Dict[str, Any]] = []
    for check_ids in sorted(by_check_set):
        slice_count = -(-len(check_ids) // MAX_CHECKS_PER_CELL)
        slice_size = -(-len(check_ids) // slice_count)
        for start in range(0, len(check_ids), slice_size):
            check_slice = check_ids[start : start + slice_size]
            for chunk in chunk_paths(by_check_set[check_ids]):
                cells.append({"files": chunk, "check_ids": list(check_slice)})
    return cells


def build_discovery_jobs(
    state: Mapping[str, Any],
    groups: Sequence[Sequence[str]],
) -> List[Dict[str, Any]]:
    cell = EFFORT_CELLS[str(state["effort"])]
    target = common_target(state)
    candidate_cap = int(cell["per_angle_cap"])
    rules_state = state.get("rules") or {}
    catalog: Mapping[str, Mapping[str, Any]] = rules_state.get("catalog") or {}
    overridden: Mapping[str, Sequence[str]] = (
        rules_state.get("overridden") or {}
    )
    jobs: List[Dict[str, Any]] = []
    for index, group in enumerate(groups, 1):
        job_id = f"finder:local:{index:02d}"
        jobs.append(
            {
                "name": job_id,
                "job_id": job_id,
                "kind": "local-correctness",
                "files": list(group),
                "instructions": LOCAL_CORRECTNESS_INSTRUCTIONS,
                "stance": cell["stance"],
                "candidate_cap": candidate_cap,
                "target": target,
            }
        )
    # A collapsed small target keeps its local passes and rule audits (exact
    # coverage) and skips the whole-change fan-out.
    whole_change_angles = (
        () if state.get("collapsed") else WHOLE_CHANGE_ANGLES
    )
    for key, title, instructions in whole_change_angles:
        jobs.append(
            {
                "name": f"finder:angle:{key}",
                "job_id": f"finder:angle:{key}",
                "kind": "angle",
                "angle": {
                    "key": key,
                    "title": title,
                    "instructions": instructions,
                },
                "stance": cell["stance"],
                "candidate_cap": candidate_cap,
                "target": target,
            }
        )
    for index, audit_cell in enumerate(
        build_rule_audit_cells(state, groups), 1
    ):
        job_id = f"finder:rule:{index:02d}"
        checks = [
            dict(catalog[check_id])
            for check_id in audit_cell["check_ids"]
            if check_id in catalog
        ]
        resolution = {
            path: list(overridden[path])
            for path in audit_cell["files"]
            if path in overridden
        }
        jobs.append(
            {
                "name": job_id,
                "job_id": job_id,
                "kind": "rule-audit",
                "files": audit_cell["files"],
                "checks": checks,
                "overridden_builtin_checks": resolution,
                "stance": STANCE_RULE_AUDIT,
                "candidate_cap": candidate_cap,
                "target": target,
            }
        )
    return jobs


# --- Low-tier finder jobs (the prepare command consumes these) ----------------


def build_finder_jobs(state: Mapping[str, Any]) -> List[Dict[str, Any]]:
    cell = EFFORT_CELLS[str(state["effort"])]
    target = common_target(state)
    jobs: List[Dict[str, Any]] = []
    for key, title, instructions in cell["angles"]:
        jobs.append(
            {
                "name": f"finder:{key}",
                "job_id": f"finder:{key}",
                "kind": "angle",
                "angle": {
                    "key": key,
                    "title": title,
                    "instructions": instructions,
                },
                "stance": cell["stance"],
                "candidate_cap": cell["per_angle_cap"],
                "target": target,
            }
        )
    return jobs


# --- Phase job bookkeeping ---------------------------------------------------


def set_phase_jobs(
    state: Dict[str, Any],
    phase: str,
    jobs: Sequence[Mapping[str, Any]],
) -> None:
    values = [dict(job) for job in jobs]
    state.setdefault("phase_jobs", {})[phase] = values
    state.setdefault("phase_results", {}).setdefault(phase, {})


def phase_jobs_context(
    state: Mapping[str, Any],
    phase: str,
    jobs: Sequence[Mapping[str, Any]],
) -> Dict[str, Any]:
    del state
    key = PHASE_JOB_KEYS[phase]
    # Fabro's parallel handler requires the context value itself to be an
    # array. Fabro offloads large values and hydrates them before `for_each`.
    return {key: [dict(job) for job in jobs]}
