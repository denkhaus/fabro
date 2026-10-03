#!/usr/bin/env python3
"""Deterministic engine entry for the Fabro code-review workflow.

Review agents return validated JSON in their final messages. Fabro passes those
results to deterministic merge commands over standard input. This program owns
state transitions after Fabro's native agent retries, plus normalization, caps,
deduplication, verdict arithmetic, coverage records, and the canonical result
bundle.

Every tier above low projects one rule-mapped structure: a grouping pass
assigns every target file to exactly one local-correctness job, whole-change
angles cover cross-cutting concerns, path-matched YAML rules produce their
own audit jobs, and (at xhigh/max) a coverage-aware gap-fill sweep runs
last; fresh sweep candidates are verified the same way. The tiers differ
only in deterministic dials: grouping fidelity (semantic agent vs lexical
chunks), rule layers (full built-in library vs repository rules plus the
repository-instructions pack), caps, verification bias, the sweep, and the
model reasoning effort the graph's stylesheet selects. The low tier keeps
the original single-pass shape from the Claude Code local /code-review
workflow: one hunk-only finder, no verification, no rules. Deterministic
code decides what merges, what survives, and what the report can claim.

The engine lives in sibling modules, split along the original file's section
banners; this file is only the command-line entry, and the graph's script
nodes invoke it by name, so the CLI surface is load-bearing:

- review_common    constants, review angles, tier cells, shared helpers
- git_target       git target resolution and reviewed-source access
- rule_compile     rule compilation for the rule-mapped tiers
- job_planning     grouping, discovery-job planning, phase bookkeeping
- prepare          the prepare command
- normalize        finding and verdict normalization
- parallel_merges  parallel-phase merge commands
- verification     candidate planning, verification, and tally
- final_assembly   the canonical review bundle
- rendering        render-report, publish-pr, lint-rules, verify-expectations

Python 3.9-compatible. Standard library only, except that rule compilation
(every tier above low) imports rule_loader, which requires the pinned
PyYAML dependency; the low tier never imports it.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path
from typing import Sequence

sys.path.insert(0, str(Path(__file__).resolve().parent))

from final_assembly import final_tally  # noqa: E402
from parallel_merges import (  # noqa: E402
    merge,
    plan_finders,
)
from prepare import prepare  # noqa: E402
from rendering import (  # noqa: E402
    lint_rules,
    publish_pr_command,
    render_report,
    verify_expectations,
)
from review_common import (  # noqa: E402
    PHASE_OUTPUT_KEYS,
    WorkflowDataError,
)
from verification import (  # noqa: E402
    plan_verify,
    tally,
)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)

    prepare_parser = subparsers.add_parser("prepare")
    prepare_parser.add_argument("--mode", default="changes")
    prepare_parser.add_argument("--effort", default="medium")
    prepare_parser.add_argument("--scope", default="")
    prepare_parser.add_argument("--base", default="")
    prepare_parser.add_argument("--commit", default="")
    prepare_parser.add_argument("--range", default="")
    prepare_parser.add_argument("--model", default="")
    prepare_parser.add_argument("--guidance", default="")
    prepare_parser.add_argument("--review-id-stdin", action="store_true")

    merge_parser = subparsers.add_parser("merge")
    merge_parser.add_argument(
        "phase",
        choices=("grouping", "sweep", *PHASE_OUTPUT_KEYS.keys()),
    )

    publish_parser = subparsers.add_parser("publish-pr")
    publish_parser.add_argument("--post-pr", default="")
    publish_parser.add_argument("--pr-repo", default="")
    publish_parser.add_argument("--pr-number", default="")
    publish_parser.add_argument("--route-severity-below", default="")
    publish_parser.add_argument("--route-categories", default="")
    publish_parser.add_argument("--run-url", default="")
    publish_parser.add_argument("--api-base", default="https://api.github.com")

    expectations_parser = subparsers.add_parser("verify-expectations")
    expectations_parser.add_argument("--expected-min-findings", default="")
    expectations_parser.add_argument("--expected-file", default="")
    expectations_parser.add_argument(
        "--expected-min-rule-findings", default=""
    )

    for name in (
        "plan-finders",
        "plan-verify",
        "tally",
        "final-tally",
        "render-report",
        "lint-rules",
    ):
        subparsers.add_parser(name)
    return parser


def main(argv: Sequence[str]) -> int:
    args = build_parser().parse_args(argv)
    commands = {
        "prepare": lambda: prepare(args),
        "merge": lambda: merge(args.phase),
        "plan-finders": plan_finders,
        "plan-verify": plan_verify,
        "tally": tally,
        "final-tally": final_tally,
        "render-report": render_report,
        "publish-pr": lambda: publish_pr_command(args),
        "lint-rules": lint_rules,
        "verify-expectations": lambda: verify_expectations(
            args.expected_min_findings,
            args.expected_file,
            args.expected_min_rule_findings,
        ),
    }
    try:
        commands[args.command]()
    except WorkflowDataError as error:
        print(f"code_review.py: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
