#!/usr/bin/env python3
"""The prepare command: resolve and size the review target.

Verifies the schema sources against the shared contract (drift check),
compiles the rule state for rule-mapped tiers, plans phase one, and writes
the initial run state. Python 3.9-compatible. Standard library only."""

from __future__ import annotations

import argparse
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import (
    Any,
    Dict,
    List,
    Mapping,
    Optional,
)

sys.path.insert(0, str(Path(__file__).resolve().parent))

from review_contract import (  # noqa: E402
    CATEGORIES,
    EFFORT_TIERS,
    ISSUE_TYPES,
    REVIEW_MODES,
)
from git_target import (  # noqa: E402
    common_target,
    default_base_ref,
    diff_file_records,
    diff_record_summary,
    diff_stats,
    empty_tree_hash,
    parse_scope,
    parse_two_sided_range,
    repo_files,
    resolve_commit,
    revision_record,
    tracked_files,
    unique_report_dir,
    validate_revision,
    workspace_digest,
)
from job_planning import (  # noqa: E402
    build_finder_jobs,
    phase_jobs_context,
    set_phase_jobs,
)
from review_common import (  # noqa: E402
    CANONICAL_SCHEMA_VERSION,
    CONTROL_DIR,
    EFFORT_CELLS,
    FINDINGS_SCHEMA_PATH,
    GROUP_MAX_FILES,
    SMALL_DIFF_MAX_FILES,
    SMALL_DIFF_MAX_LINES,
    SMALL_SCOPE_MAX_FILES,
    VERDICTS,
    VERDICT_SCHEMA_PATH,
    WorkflowDataError,
    clean_text,
    emit,
    git_text,
    inside_git_worktree,
    one_line,
    read_json,
    review_id_from_args,
    root,
    save_state,
    write_json,
)
from rule_compile import compile_rule_state  # noqa: E402


# --- Contract drift checks ---------------------------------------------------


def verify_schema_sources() -> None:
    """Refuse to start when the static schemas disagree with this engine.

    Fabro reads the schema files directly, and this engine restates their
    closed enums. Neither is generated from the other, so drift is caught here
    rather than in a finished report.
    """
    findings_schema = read_json(root() / FINDINGS_SCHEMA_PATH)
    try:
        schema_categories = findings_schema["properties"]["findings"]["items"][
            "properties"
        ]["category"]["enum"]
    except (KeyError, TypeError) as error:
        raise WorkflowDataError(
            f"{FINDINGS_SCHEMA_PATH} has no category enum"
        ) from error
    if list(schema_categories) != list(CATEGORIES):
        raise WorkflowDataError(
            f"{FINDINGS_SCHEMA_PATH} category enum does not match this "
            "engine's category list"
        )
    try:
        schema_issue_types = findings_schema["properties"]["findings"]["items"][
            "properties"
        ]["issue_type"]["enum"]
    except (KeyError, TypeError) as error:
        raise WorkflowDataError(
            f"{FINDINGS_SCHEMA_PATH} has no issue_type enum"
        ) from error
    if list(schema_issue_types) != list(ISSUE_TYPES):
        raise WorkflowDataError(
            f"{FINDINGS_SCHEMA_PATH} issue_type enum does not match this "
            "engine's issue type list"
        )
    verdict_schema = read_json(root() / VERDICT_SCHEMA_PATH)
    try:
        schema_verdicts = verdict_schema["properties"]["verdict"]["enum"]
    except (KeyError, TypeError) as error:
        raise WorkflowDataError(
            f"{VERDICT_SCHEMA_PATH} has no verdict enum"
        ) from error
    if list(schema_verdicts) != list(VERDICTS):
        raise WorkflowDataError(
            f"{VERDICT_SCHEMA_PATH} verdict enum does not match this "
            "engine's verdict list"
        )


# --- Prepare -----------------------------------------------------------------


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


def prepare(args: argparse.Namespace) -> None:
    verify_schema_sources()
    CONTROL_DIR.mkdir(parents=True, exist_ok=True)
    started_at = (
        datetime.now(timezone.utc).replace(microsecond=0).isoformat()
    )
    review_id = review_id_from_args(args)
    mode = args.mode.strip().lower()
    if mode not in REVIEW_MODES:
        raise WorkflowDataError(
            f"mode must be one of {', '.join(REVIEW_MODES)}, got {args.mode!r}"
        )
    effort = args.effort if args.effort in EFFORT_TIERS else "medium"
    if args.effort and args.effort not in EFFORT_TIERS:
        print(
            f'unknown effort "{one_line(args.effort, 60)}" -- using medium '
            f"(tiers: {', '.join(EFFORT_TIERS)})"
        )
    scope = parse_scope(args.scope)
    base_input = args.base.strip()
    commit_input = args.commit.strip()
    range_input = args.range.strip()
    model = one_line(args.model, 120)
    # Guidance reaches the finder and sweep prompts through their MiniJinja
    # templates; the engine only records it so the report says what steering
    # was applied.
    guidance = clean_text(args.guidance, 2000).strip()
    cell = EFFORT_CELLS[effort]

    revision_range: Optional[str] = None
    base: Optional[str] = None
    merge_base: Optional[str] = None
    target_commit: Optional[str] = None
    parent: Optional[str] = None
    changed_files: List[str] = []
    diff_lines: Optional[int] = None
    scope_file_count: Optional[int] = None
    file_records: Optional[Dict[str, Dict[str, Any]]] = None

    if mode == "changes":
        if not inside_git_worktree():
            raise WorkflowDataError("changes mode requires a Git worktree")
        if commit_input:
            raise WorkflowDataError(
                "changes mode does not accept commit; use mode=commit"
            )
        if range_input and base_input:
            raise WorkflowDataError(
                "changes mode accepts either base or an explicit range, not both"
            )
        if range_input:
            left, separator, right = parse_two_sided_range(range_input)
            left_commit = resolve_commit(left, "range start")
            target_commit = resolve_commit(right, "range end")
            revision_range = f"{left}{separator}{right}"
            base = left
            if separator == "...":
                merge_base = git_text("merge-base", left_commit, target_commit)
                if not merge_base:
                    raise WorkflowDataError(
                        "the explicit range endpoints have no merge base"
                    )
            else:
                merge_base = left_commit
        else:
            base = validate_revision(
                base_input or default_base_ref(),
                "base",
            )
            base_commit = resolve_commit(base, "base")
            target_commit = resolve_commit("HEAD", "HEAD")
            merge_base = git_text("merge-base", base_commit, target_commit)
            if not merge_base:
                raise WorkflowDataError(
                    f"base {base!r} and HEAD have no merge base"
                )
            revision_range = f"{merge_base}..HEAD"
        if cell["rule_mapped"]:
            file_records = diff_file_records(revision_range, scope)
            changed_files, diff_lines = diff_record_summary(file_records)
        else:
            changed_files, diff_lines = diff_stats(revision_range, scope)
    elif mode == "commit":
        if not inside_git_worktree():
            raise WorkflowDataError("commit mode requires a Git worktree")
        if not commit_input:
            raise WorkflowDataError("commit mode requires a commit input")
        if base_input or range_input:
            raise WorkflowDataError(
                "commit mode accepts commit only; base and range are not used"
            )
        target_commit = resolve_commit(commit_input, "commit")
        parent = git_text(
            "rev-parse",
            "--verify",
            "--quiet",
            target_commit + "^",
        )
        revision_range = f"{parent or empty_tree_hash()}..{target_commit}"
        if cell["rule_mapped"]:
            file_records = diff_file_records(revision_range, scope)
            changed_files, diff_lines = diff_record_summary(file_records)
        else:
            changed_files, diff_lines = diff_stats(revision_range, scope)
    else:
        if base_input or commit_input or range_input:
            raise WorkflowDataError(
                "files mode does not accept base, commit, or range inputs"
            )
        if not scope:
            raise WorkflowDataError(
                "files mode requires a scope naming the files to review"
            )
        if inside_git_worktree():
            changed_files = tracked_files(scope)
        else:
            changed_files = [
                path
                for path in repo_files()
                if any(
                    path == item
                    or path.startswith(item.rstrip("/") + "/")
                    for item in scope
                )
            ]
        scope_file_count = len(changed_files)

    diff_file_count = (
        len(changed_files) if mode in {"changes", "commit"} else None
    )
    empty_diff = mode in {"changes", "commit"} and diff_file_count == 0
    empty_scope = mode == "files" and scope_file_count == 0
    empty_target = empty_diff or empty_scope

    state: Dict[str, Any] = {
        "version": 1,
        "root": str(root()),
        "started_at": started_at,
        "review_id": review_id,
        "products_dir": None,
        "products_rel": None,
        "evidence_dir": None,
        "evidence_rel": None,
        "metadata_dir": None,
        "metadata_rel": None,
        "state_path": None,
        "mode": mode,
        "effort": effort,
        "model": model,
        "guidance": guidance,
        "scope": scope,
        "range": revision_range,
        "base": base,
        "merge_base": merge_base,
        "commit": target_commit,
        "parent": parent,
        "changed_files": changed_files,
        "diff_files": diff_file_count,
        "diff_lines": diff_lines,
        "scope_files": scope_file_count,
        "empty_diff": empty_diff,
        "empty_scope": empty_scope,
        "use_verify": bool(cell["verify"]),
        "verify_bias": cell["bias"],
        "use_sweep": bool(cell["sweep"]),
        "per_angle_cap": int(cell["per_angle_cap"]),
        "report_cap": int(cell["report_cap"]),
        "verification_cap": int(cell["verification_cap"]),
        "rule_mapped": bool(cell["rule_mapped"]),
        "rule_layers": cell.get("rule_layers"),
        "collapsed": None,
        "phase_results": {},
        "phase_jobs": {},
    }

    if empty_target:
        save_state(state)
        reason = (
            "the committed range has no changed files"
            if empty_diff
            else "the scope resolves to no files"
        )
        print(f"Nothing to review: {reason}")
        emit(
            empty_target=True,
            empty_reason=reason,
            mode=mode,
            effort=effort,
        )
        return

    products_dir, products_rel = unique_report_dir()
    evidence_dir = products_dir / "evidence"
    metadata_dir = products_dir / "metadata"
    evidence_dir.mkdir()
    metadata_dir.mkdir()
    (products_dir / ".gitignore").write_text("*\n", encoding="utf-8")
    state["products_dir"] = products_dir.as_posix()
    state["products_rel"] = products_rel
    state["evidence_dir"] = evidence_dir.as_posix()
    state["evidence_rel"] = f"{products_rel}/evidence"
    state["metadata_dir"] = metadata_dir.as_posix()
    state["metadata_rel"] = f"{products_rel}/metadata"
    state["state_path"] = (metadata_dir / "state.json").as_posix()

    revision = revision_record(
        mode,
        target_commit,
        base,
        merge_base,
        parent,
        revision_range,
    )
    state["revision"] = revision
    review_meta = {
        "schema_version": CANONICAL_SCHEMA_VERSION,
        "started_at": started_at,
        "review_id": review_id,
        "review_root": str(root()),
        "metadata_dir": metadata_dir.as_posix(),
        "agent": "fabro:code-review",
        "mode": mode,
        "scope": scope,
        "effort": effort,
        "model": model,
        "guidance": guidance,
        "revision": revision,
        "revision_source": "self-reported",
        "range": revision_range,
    }
    write_json(metadata_dir / "review-meta.json", review_meta)
    state["workspace_digest"] = workspace_digest()

    if mode == "files":
        described = (
            f"{scope_file_count} file(s) in scope"
        )
    else:
        described = (
            f"{diff_file_count} changed file(s)"
            + (f", {diff_lines} line(s)" if diff_lines is not None else "")
        )

    if cell["rule_mapped"]:
        # Compile the rule configuration now (an invalid rule file must fail
        # here, before any review agent runs), then hand the target list to
        # the grouping pass. Finder jobs are planned by plan-finders after
        # grouping.
        if mode in {"changes", "commit"}:
            if file_records is None:
                raise WorkflowDataError(
                    "rule-mapped diff metadata was not prepared"
                )
        else:
            file_records = {path: {"status": "full"} for path in changed_files}
        compile_rule_state(state, file_records)
        if cell.get("collapse"):
            small_diff = (
                mode in {"changes", "commit"}
                and diff_file_count is not None
                and 0 < diff_file_count <= SMALL_DIFF_MAX_FILES
                and diff_lines is not None
                and diff_lines <= SMALL_DIFF_MAX_LINES
            )
            small_scope = (
                mode == "files"
                and scope_file_count is not None
                and 0 < scope_file_count <= SMALL_SCOPE_MAX_FILES
            )
            state["collapsed"] = (
                "small-diff"
                if small_diff
                else ("small-scope" if small_scope else None)
            )
        grouping_mode = str(cell.get("grouping") or "lexical")
        use_grouping = grouping_mode == "semantic" and len(changed_files) > 1
        state["use_grouping"] = use_grouping
        state["use_planner"] = True
        state["grouping"] = {
            "mode": grouping_mode,
            "planned": use_grouping,
            "agent_returned": False,
            "fallback": None,
            "corrections": [],
            "groups": [],
        }
        assignment = {
            "files": [
                {
                    "path": path,
                    "status": str(
                        (file_records.get(path) or {}).get("status") or "M"
                    ),
                    "added": (file_records.get(path) or {}).get("added"),
                    "deleted": (file_records.get(path) or {}).get("deleted"),
                }
                for path in sorted(changed_files)
            ],
            "max_files_per_group": GROUP_MAX_FILES,
            "mode": mode,
        }
        state["grouping_assignment"] = assignment
        state["finder_jobs"] = []
        set_phase_jobs(state, "finders", [])
        save_state(state)
        rule_counts = state["rules"]["counts"]
        print(
            f"Prepared {effort} {mode} code review: {described}; "
            f"{rule_counts['builtin_packs']} built-in and "
            f"{rule_counts['repo_packs']} repository rule pack(s) compiled; "
            "grouping "
            + ("dispatched" if use_grouping else grouping_mode)
            + (
                f"; collapsed ({state['collapsed']})"
                if state.get("collapsed")
                else ""
            )
        )
        emit(
            empty_target=False,
            mode=mode,
            effort=effort,
            products_dir=products_rel,
            use_grouping=use_grouping,
            use_planner=True,
            grouping_assignment=assignment,
        )
        return

    state["use_grouping"] = False
    state["use_planner"] = False
    finder_jobs = build_finder_jobs(state)
    state["finder_jobs"] = finder_jobs
    set_phase_jobs(state, "finders", finder_jobs)
    save_state(state)

    print(
        f"Prepared {effort} {mode} code review: {described}; "
        f"{len(finder_jobs)} finder angle(s), verify="
        + ("on" if state["use_verify"] else "off")
        + ", sweep="
        + ("on" if state["use_sweep"] else "off")
    )
    emit(
        empty_target=False,
        mode=mode,
        effort=effort,
        products_dir=products_rel,
        use_grouping=False,
        use_planner=False,
        **phase_jobs_context(state, "finders", finder_jobs),
    )
