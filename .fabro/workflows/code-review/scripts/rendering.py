#!/usr/bin/env python3
"""Rendering, publishing, and expectations.

Loads the renderer and publisher siblings through the workflow-script
bootstrap, runs render-report, publish-pr, lint-rules, and the
verify-expectations gate. Python 3.9-compatible. Standard library only."""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import importlib.util
from pathlib import Path
from typing import (
    Any,
    Dict,
    List,
    Optional,
)

sys.path.insert(0, str(Path(__file__).resolve().parent))

from git_target import (  # noqa: E402
    assert_workspace_unchanged,
    normalize_repo_path,
    repo_files,
)
from review_common import (  # noqa: E402
    PUBLISHER_PATH,
    RENDERER_PATH,
    WORKFLOW_ROOT,
    WorkflowDataError,
    emit,
    load_state,
    one_line,
    read_json,
    root,
    save_state,
)
from rule_compile import (  # noqa: E402
    import_rule_loader,
    read_repo_rule_files,
)


# --- Rendering and expectations ----------------------------------------------


def resolve_workflow_script(rel_path: Path, description: str) -> Path:
    path = (root() / rel_path).resolve()
    if not path.is_file():
        raise WorkflowDataError(f"the {description} is missing: {rel_path}")
    return path


def load_renderer() -> Any:
    path = resolve_workflow_script(RENDERER_PATH, "deterministic renderer")
    spec = importlib.util.spec_from_file_location(
        "code_review_render_report",
        path,
    )
    if spec is None or spec.loader is None:
        raise WorkflowDataError("could not load the report renderer")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def render_report() -> None:
    state = load_state()
    products_rel = str(state["products_rel"])
    evidence_rel = str(state["evidence_rel"])
    metadata_rel = str(state["metadata_rel"])
    renderer = load_renderer()
    try:
        findings, verification = renderer.render(
            evidence_rel,
            products_rel,
            metadata_rel,
        )
    except Exception as error:
        raise WorkflowDataError(
            f"the report renderer refused the report: {error}"
        ) from error
    state["revision_path"] = f"{metadata_rel}/revision.json"
    state["verification_status"] = verification.get("status")
    state["finding_count"] = len(findings)
    save_state(state)
    print(
        f"Wrote {products_rel}/CODE-REVIEW-RESULTS.md, "
        "CODE-REVIEW-RESULTS.html, CODE-REVIEW-RESULTS.jsonl, and "
        "metadata/revision.json; canonical evidence retained with the reports"
    )
    emit(
        report_path=f"{products_rel}/CODE-REVIEW-RESULTS.md",
        revision_path=state["revision_path"],
        verification_status=verification.get("status"),
        finding_count=len(findings),
    )


def publish_pr_command(args: argparse.Namespace) -> None:
    """Post the completed review to its GitHub PR (the P1 publisher).

    The graph runs this node unconditionally after render-report; the
    post_pr input decides whether anything happens. The plan step is pure
    and runs with credentials scrubbed (R18); only apply sees
    GITHUB_TOKEN. The plan and outcome files land in the products
    directory as replayable evidence, peers of the canonical bundle.
    """
    requested = args.post_pr.strip().lower() in ("true", "1", "yes", "on")
    if not requested:
        print("PR publishing not requested (post_pr is off)")
        emit(publish_pr={"requested": False})
        return
    state = load_state()
    if not isinstance(state.get("review_manifest"), dict):
        raise WorkflowDataError(
            "publish-pr requires a completed review bundle; it runs after "
            "final-tally and render-report"
        )
    assert_workspace_unchanged(state)
    repo = args.pr_repo.strip()
    pr_text = args.pr_number.strip()
    if not repo or not pr_text:
        raise WorkflowDataError(
            "post_pr is enabled but pr_repo/pr_number do not name the "
            "target pull request"
        )
    if not os.environ.get("GITHUB_TOKEN"):
        raise WorkflowDataError(
            "publish-pr needs GITHUB_TOKEN in the environment; the "
            "workflow's [run.integrations.github.permissions] makes Fabro "
            "inject one when its GitHub integration is configured"
        )
    publisher = resolve_workflow_script(PUBLISHER_PATH, "PR publisher")
    products_rel = str(state["products_rel"])
    evidence_rel = str(state["evidence_rel"])
    plan_rel = f"{products_rel}/pr-publish-plan.json"
    outcome_rel = f"{products_rel}/pr-publish-outcome.json"

    def run_publisher(
        arguments: List[str],
        environment: Optional[Dict[str, str]] = None,
    ) -> subprocess.CompletedProcess:
        return subprocess.run(
            [sys.executable, str(publisher), *arguments],
            cwd=root(),
            env=environment,
            capture_output=True,
        )

    def publisher_error(
        prefix: str, failed: subprocess.CompletedProcess
    ) -> WorkflowDataError:
        detail = failed.stderr.decode("utf-8", "replace").strip()
        return WorkflowDataError(prefix + one_line(detail, 2000))

    # The plan step needs no credentials and runs with none (R18). Its
    # environment is rebuilt from a benign allowlist, so a credential
    # injected under any name -- not just the ones Fabro uses today --
    # never reaches the plan.
    plan_environment = {
        key: value
        for key, value in os.environ.items()
        if key in ("PATH", "HOME", "TZ", "USER", "LOGNAME", "SHELL")
        or key.startswith(("LANG", "LC_", "PYTHON", "TMP", "TEMP"))
    }
    result = run_publisher(
        [
            "plan",
            "--evidence-dir", evidence_rel,
            "--repo", repo,
            "--pr", pr_text,
            "--route-severity-below", args.route_severity_below,
            "--route-categories", args.route_categories,
            "--run-url", one_line(args.run_url, 2000),
            "--output", plan_rel,
        ],
        plan_environment,
    )
    if result.returncode != 0:
        raise publisher_error("the publication plan failed: ", result)
    print(result.stdout.decode("utf-8", "replace").strip())

    result = run_publisher(
        [
            "apply",
            "--plan", plan_rel,
            "--repo", repo,
            "--pr", pr_text,
            "--api-base", args.api_base,
            "--outcome", outcome_rel,
        ]
    )
    value = read_json(root() / outcome_rel, required=False)
    outcome: Dict[str, Any] = value if isinstance(value, dict) else {}
    updates: Dict[str, Any] = {"requested": True}
    if outcome:
        updates["counts"] = outcome.get("counts")
        updates["summary_url"] = outcome.get("summary_url") or ""
        updates["outcome_path"] = outcome_rel
    emit(publish_pr=updates)
    stdout_text = result.stdout.decode("utf-8", "replace").strip()
    if stdout_text:
        print(stdout_text)
    if result.returncode != 0:
        raise publisher_error("posting to the PR failed: ", result)


def lint_rules() -> None:
    """Validate the rule configuration from the working tree, for authors.

    A review reads repository rules from its base revision, so an invalid
    rule file otherwise surfaces only after it lands and a rule-mapped run
    starts. This command runs the same loader against the working
    filesystem so a rule change can be checked before it is committed. It
    reads no workflow state and writes nothing.
    """
    loader = import_rule_loader()
    workflow_root = root() / WORKFLOW_ROOT
    manifest = read_json(workflow_root / loader.BUILTIN_MANIFEST)
    try:
        builtin_files = loader.load_builtin_files(workflow_root, manifest)
        builtin_packs = loader.load_rule_layer(builtin_files, "builtin")
        repo_files = read_repo_rule_files(loader, None)
        repo_packs = loader.load_rule_layer(repo_files, "repo")
    except loader.RuleLoaderError as error:
        raise WorkflowDataError(f"rule configuration is invalid: {error}")

    for path, _content in repo_files:
        pack_summaries = [
            f"{pack['pack_id']} ({len(pack['checks'])} check(s)"
            + (", override" if pack["mode"] == "override" else "")
            + ")"
            for pack in repo_packs
            if pack["source_path"] == path
        ]
        print(f"{path}: " + (", ".join(pack_summaries) or "no rules"))
    if not repo_files:
        print(
            "no repository rule files found (.fabro/rules.yaml, "
            ".fabro/rules/**/*.yaml)"
        )
    print(
        f"rule configuration OK: {len(builtin_packs)} built-in pack(s) "
        f"({sum(len(pack['checks']) for pack in builtin_packs)} check(s)), "
        f"{len(repo_packs)} repository pack(s) "
        f"({sum(len(pack['checks']) for pack in repo_packs)} check(s)); "
        f"config sha256 "
        f"{loader.rule_config_sha256(builtin_packs, repo_packs)[:12]}"
    )
    print(
        "note: reviews read repository rules from their base revision, so "
        "a change takes effect after it lands"
    )


def verify_expectations(
    expected_min_text: str,
    expected_file: str,
    expected_min_rule_text: str = "",
) -> None:
    expected_min_text = expected_min_text.strip()
    expected_file = expected_file.strip()
    expected_min_rule_text = expected_min_rule_text.strip()
    if not expected_min_text and not expected_file and not (
        expected_min_rule_text
    ):
        print("No report expectations configured")
        emit(report_expectations_checked=False)
        return
    for text, label in (
        (expected_min_text, "finding"),
        (expected_min_rule_text, "rule-derived finding"),
    ):
        if text and not re.fullmatch(r"0|[1-9][0-9]*", text):
            raise WorkflowDataError(
                f"expected minimum {label} count must be a non-negative "
                "integer"
            )
    expected_min = int(expected_min_text) if expected_min_text else 0
    expected_min_rule = (
        int(expected_min_rule_text) if expected_min_rule_text else 0
    )
    normalized_expected_file = (
        normalize_repo_path(expected_file) if expected_file else None
    )
    if expected_file and normalized_expected_file in (None, "."):
        raise WorkflowDataError("expected file is not a safe repository path")

    state = load_state()
    evidence_dir = state.get("evidence_dir")
    if not isinstance(evidence_dir, str) or not evidence_dir:
        raise WorkflowDataError("state has no evidence directory")
    findings = read_json(Path(evidence_dir) / "findings.json")
    if not isinstance(findings, list):
        raise WorkflowDataError("findings.json must contain a JSON array")
    if len(findings) < expected_min:
        raise WorkflowDataError(
            f"expected at least {expected_min} reported finding(s), found "
            f"{len(findings)}"
        )
    rule_findings = sum(
        1
        for finding in findings
        if isinstance(finding, dict) and finding.get("rule_ids")
    )
    if rule_findings < expected_min_rule:
        raise WorkflowDataError(
            f"expected at least {expected_min_rule} rule-derived "
            f"finding(s), found {rule_findings}"
        )
    if normalized_expected_file:
        files = {
            finding.get("file")
            for finding in findings
            if isinstance(finding, dict)
        }
        if normalized_expected_file not in files:
            raise WorkflowDataError(
                f"expected a finding in {normalized_expected_file!r}, found "
                f"findings in {sorted(str(name) for name in files)!r}"
            )

    print(
        "Verified report expectations: "
        f">={expected_min} finding(s)"
        + (
            f", >={expected_min_rule} rule-derived"
            if expected_min_rule_text
            else ""
        )
        + (
            f" including {normalized_expected_file}"
            if normalized_expected_file
            else ""
        )
    )
    emit(
        report_expectations_checked=True,
        expected_min_findings=expected_min,
        expected_min_rule_findings=expected_min_rule,
        expected_file=normalized_expected_file or "",
    )
