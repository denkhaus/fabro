#!/usr/bin/env python3
"""Shared constants and helpers for the code-review engine.

Owns the workflow paths, transport caps, verdict/severity tables, review
angles, effort-tier cells, and the small state/IO/git helpers every engine
module uses; WorkflowDataError is the engine's deterministic failure type.

Python 3.9-compatible. Standard library only."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import uuid
from pathlib import Path
from typing import (
    Any,
    Dict,
    Iterable,
    Mapping,
    Optional,
)

sys.path.insert(0, str(Path(__file__).resolve().parent))

from review_contract import CATEGORIES  # noqa: E402


WORKFLOW_ROOT = Path(".fabro/workflows/code-review")
CONTROL_DIR = WORKFLOW_ROOT / "runtime"
STATE_PATH = CONTROL_DIR / "state.json"
RENDERER_PATH = WORKFLOW_ROOT / "scripts/render_report.py"
PUBLISHER_PATH = WORKFLOW_ROOT / "scripts/publish_pr.py"
FINDINGS_SCHEMA_PATH = WORKFLOW_ROOT / "schemas/findings.schema.json"
VERDICT_SCHEMA_PATH = WORKFLOW_ROOT / "schemas/verdict.schema.json"

# Fabro resolves stdin_source before starting a command and enforces this same
# ceiling. Keep the driver's direct-input guard aligned with that transport.
MAX_STDIN_BYTES = 30 * 1024 * 1024
MAX_REVIEW_ID_STDIN_BYTES = 256
MAX_CHANGED_FILES_LISTED = 200

# Rule-mapped review shape (every tier above low).
GROUP_MAX_FILES = 10
GROUP_CHAR_BUDGET = 2000  # estimated per-job path payload, in characters
# A cell audits at most this many checks; a larger effective set splits into
# evenly sized cells over the same files. Each cell reports at most
# candidate_cap findings, so unbounded checks would dilute every check's
# share of the cell's attention as repository rules stack up.
MAX_CHECKS_PER_CELL = 12
DISCOVERY_JOB_CEILING = 64  # discovery jobs (local + angle + rule-audit)
# A small target at medium collapses to local passes and rule audits only
# (no whole-change angles), mirroring the security-review workflow's
# small-diff collapse. Verification still runs.
SMALL_DIFF_MAX_FILES = 5
SMALL_DIFF_MAX_LINES = 300
SMALL_SCOPE_MAX_FILES = 5

# Correctness bugs always outrank cleanup findings when a cap forces a cut.
CLEANUP_CATEGORIES = frozenset(CATEGORIES) - {"correctness"}
# Policy filters drop well-formed findings the review does not want; unlike a
# contract rejection they are recorded in coverage without making the run
# partial. Conventions findings must cite a rule check: calibration showed
# generic angles' unbacked style observations were the noisiest class, while
# every rule-cited conventions finding survived verification.
CONVENTIONS_FILTER_REASON = "conventions finding names no applicable rule check"
POLICY_FILTER_REASONS = frozenset({CONVENTIONS_FILTER_REASON})
VERDICTS = ("CONFIRMED", "PLAUSIBLE", "REFUTED")
KEPT_VERDICTS = frozenset({"CONFIRMED", "PLAUSIBLE"})
SEVERITY_RANK = {"HIGH": 3, "MEDIUM": 2, "LOW": 1}
CONFIDENCE_RANK = SEVERITY_RANK

SAFE_REV_RE = re.compile(r"^[A-Za-z0-9@][A-Za-z0-9._/@{}^~:+-]{0,399}$")
REVIEW_ID_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$")
CANDIDATE_ID_RE = re.compile(r"^[FS][1-9][0-9]*$")
# Other candidates in the same file a verifier is shown, nearest first, so it
# can mark its claim a duplicate of one that describes the same defect.
SIBLING_CAP = 6

# Lines of context kept on each side of a finding's anchor line.
CODE_FRAME_CONTEXT = 4
CODE_FRAME_MAX_LINE_LENGTH = 400
CODE_FRAME_MAX_BYTES = 2 * 1024 * 1024
LOCATION_MAX_LINES = 50
SUGGESTION_CODE_MAX_LENGTH = 8000
CODE_FRAME_LANGUAGES = {
    "c": "C",
    "cc": "C++",
    "cpp": "C++",
    "cs": "C#",
    "css": "CSS",
    "ex": "Elixir",
    "exs": "Elixir",
    "go": "Go",
    "h": "C",
    "hpp": "C++",
    "html": "HTML",
    "java": "Java",
    "js": "JavaScript",
    "json": "JSON",
    "jsx": "JavaScript",
    "kt": "Kotlin",
    "lua": "Lua",
    "php": "PHP",
    "pl": "Perl",
    "py": "Python",
    "rb": "Ruby",
    "rs": "Rust",
    "scala": "Scala",
    "sh": "Shell",
    "sql": "SQL",
    "swift": "Swift",
    "toml": "TOML",
    "ts": "TypeScript",
    "tsx": "TypeScript",
    "yaml": "YAML",
    "yml": "YAML",
}

CANONICAL_SCHEMA_VERSION = 4
CANONICAL_FILES = (
    "review-manifest.json",
    "candidate-ledger.jsonl",
    "findings.json",
    "coverage.json",
    "votes.jsonl",
)
PHASE_OUTPUT_KEYS = {
    "finders": "output.finder",
    "verify": "output.verifier",
    "sweep_verify": "output.sweep_verifier",
}
PHASE_JOB_KEYS = {
    "finders": "finder_jobs",
    "verify": "verify_jobs",
    "sweep_verify": "sweep_verify_jobs",
}


# --- Review angles ------------------------------------------------------------

ANGLE_LOW_PASS = (
    "low-pass",
    "Single-pass diff scan",
    """Read the unified diff once. Skip test/fixture hunks (`test/`, `spec/`,
`__tests__/`, `*_test.*`, `*.test.*`, `fixtures/`, `testdata/`) -- test-file
changes are not reviewed at this level. Do not read whole files beyond the
hunks. Flag runtime-correctness bugs visible from the hunk alone:
inverted/wrong condition, off-by-one, null/undefined deref where adjacent
lines show the value can be absent, removed guard, falsy-zero check, missing
`await`, wrong-variable copy-paste, error swallowed in a catch that should
propagate. Also flag -- still from the hunk alone -- new code that duplicates
an existing helper visible in the diff context, and dead code the diff leaves
behind. Do NOT flag style, naming, perf, missing tests, or anything outside
the hunk.""",
)

# --- The rule-mapped shape (every tier above low) -----------------------------
#
# Generic angles describe how to investigate; path-matched rules describe what
# invariants apply. Language pitfalls and conventions live in the rule
# library, so the rule-mapped tiers run four whole-change angles plus one
# local-correctness pass per file group and one audit job per rule cell.

LOCAL_CORRECTNESS_INSTRUCTIONS = """Review the files listed in `files` one at
a time -- make an individual pass over every listed file; do not skim the set
as a whole. For each file, read every hunk of the diff that touches it, line
by line, then read the enclosing function of each hunk -- bugs in unchanged
lines of a touched function are in scope (the change re-exposes or fails to
fix them). For every line ask: what input, state, timing, or platform makes
this line wrong? Look for inverted/wrong conditions, off-by-one,
null/undefined deref, missing `await`, falsy-zero checks, wrong-variable
copy-paste, error swallowed in catch, unescaped regex metachars, and the
language's classic pitfalls (`==` coercion, closure-captured loop variables,
mutable default arguments, nil-map writes, float equality). When `mode` is
`files` there is no diff: read each listed file in full and treat every line
as under review."""

ANGLE_BEHAVIOR_PRESERVATION = (
    "behavior-preservation",
    "Behavior preservation",
    """For every line the diff DELETES or replaces, name the invariant or
behavior it enforced, then search the new code for where that invariant is
re-established. If you can't find it, that's a candidate: a removed guard, a
dropped error path, a narrowed validation, a lost compatibility shim, a
deleted test that was covering a real case. Also check that error paths and
validation the change touches still fire under the same conditions, and that
behavior contracts visible in tests survive the change.""",
)
ANGLE_CONTRACTS_DATA_FLOW = (
    "contracts-data-flow",
    "Contracts and data flow",
    """For each function or type the diff changes, find its callers (search
for the symbol) and check whether the change breaks any call site: a new
precondition, a changed return shape, a new exception, a timing/ordering
dependency. Also check callees: does a parallel change in the same change set
make a call unsafe? Trace cross-file contracts, data ownership, and ordering.
When the change adds or modifies a type that wraps another (cache, proxy,
decorator, adapter): check that every method routes to the wrapped instance
and not back through a registry/session/global -- e.g. a caching provider
holding a `delegate` field that resolves IDs via `session.get(...)` instead
of `delegate.get(...)` will re-enter the cache or recurse. Also check that
the wrapper forwards all the methods the callers actually use.""",
)
ANGLE_DESIGN_ECONOMY = (
    "design-economy",
    "Design economy",
    """This angle hunts for cleanup in the changed code, not bugs. Flag new
code that re-implements something the codebase already has -- search
shared/utility modules and files adjacent to the change, and name the
existing helper to call instead. Flag unnecessary complexity the diff adds:
redundant or derivable state, copy-paste with slight variation, deep nesting,
dead code left behind -- name the simpler form that does the same job. Check
that each change is implemented at the right depth, not as a fragile bandaid:
special cases layered on shared infrastructure are a sign the fix isn't deep
enough -- prefer generalizing the underlying mechanism over adding special
cases.""",
)
ANGLE_PERFORMANCE_LIFETIME = (
    "performance-lifetime",
    "Performance and lifetime",
    """This angle hunts for wasted work and lifetime problems in the changed
code, not logic bugs. Flag redundant computation or repeated I/O, independent
operations run sequentially, and blocking work added to startup or hot paths.
Check resource ownership: acquisitions without a release on every path, and
state retained longer than its use. Flag long-lived objects built from
closures or captured environments -- they keep the entire enclosing scope
alive for the object's lifetime (a memory leak when that scope holds large
values); prefer a class/struct that copies only the fields it needs. Name the
cheaper alternative.""",
)
WHOLE_CHANGE_ANGLES = (
    ANGLE_BEHAVIOR_PRESERVATION,
    ANGLE_CONTRACTS_DATA_FLOW,
    ANGLE_DESIGN_ECONOMY,
    ANGLE_PERFORMANCE_LIFETIME,
)

STANCE_PRECISION = (
    "precision: every finding you surface should be one a maintainer would "
    "act on."
)
STANCE_RECALL = (
    "recall: catch every real bug a careful reviewer would catch in one "
    "sitting. Catching real bugs matters more than avoiding false positives. "
    "Err on the side of surfacing."
)
STANCE_MAX_RECALL = (
    "recall: catch every real bug. A missed bug ships. Catching real bugs "
    "matters more than avoiding false positives. Err on the side of "
    "surfacing."
)
STANCE_LOW = (
    "precision, single pass: report only defects visible from the hunk alone."
)
STANCE_RULE_AUDIT = (
    "rule audit: report violations of the assigned checks that you can "
    "anchor to the changed code; each check's own guidance sets the "
    "precision bar."
)

SWEEP_FOCUS = (
    "moved/extracted code that dropped a guard or anchor; second-tier "
    "footguns (dataclass default evaluated once, `hash()` non-determinism, "
    "lock-scope shrink, predicate methods with side effects); setup/teardown "
    "asymmetry in tests; config defaults flipped."
)

# Every tier above low is a projection of the rule-mapped structure; the
# dials are grouping fidelity, rule layers, caps, bias, and the sweep. The
# xhigh and max cells are identical by design: the two tiers share one
# graph, job structure, caps, prompts, and verification policy, and differ
# only in the reasoning effort the Fabro model stylesheet selects.
_XHIGH_CELL = {
    "angles": (),
    "rule_mapped": True,
    "grouping": "semantic",
    "rule_layers": "full",
    "collapse": False,
    "stance": STANCE_MAX_RECALL,
    "per_angle_cap": 8,
    "verify": True,
    "bias": "standard",
    "sweep": True,
    "report_cap": 25,
    "verification_cap": 120,
}
EFFORT_CELLS = {
    "low": {
        "angles": (ANGLE_LOW_PASS,),
        "rule_mapped": False,
        "stance": STANCE_LOW,
        "per_angle_cap": 6,
        "verify": False,
        "bias": None,
        "sweep": False,
        "report_cap": 4,
        "verification_cap": 60,
    },
    "medium": {
        "angles": (),
        "rule_mapped": True,
        "grouping": "lexical",
        "rule_layers": "repo-instructions",
        "collapse": True,
        "stance": STANCE_PRECISION,
        "per_angle_cap": 6,
        "verify": True,
        "bias": "standard",
        "sweep": False,
        "report_cap": 8,
        "verification_cap": 60,
    },
    "high": {
        "angles": (),
        "rule_mapped": True,
        "grouping": "semantic",
        "rule_layers": "full",
        "collapse": False,
        "stance": STANCE_RECALL,
        "per_angle_cap": 6,
        "verify": True,
        "bias": "recall",
        "sweep": False,
        "report_cap": 10,
        "verification_cap": 60,
    },
    "xhigh": dict(_XHIGH_CELL),
    "max": dict(_XHIGH_CELL),
}
SWEEP_CANDIDATE_CAP = 8


class WorkflowDataError(RuntimeError):
    """A deterministic workflow-data failure."""


# --- Small shared helpers ----------------------------------------------------


def root() -> Path:
    return Path.cwd().resolve()


def clean_text(value: Any, cap: int = 4000) -> str:
    text = str("" if value is None else value)
    text = "".join(
        character
        if character in "\n\t" or ord(character) >= 0x20
        else " "
        for character in text
    )
    if len(text) > cap:
        return text[:cap] + f"...[+{len(text) - cap} chars]"
    return text


def one_line(value: Any, cap: int = 500) -> str:
    return (
        clean_text(value, cap)
        .replace("\r", " ")
        .replace("\n", " ")
        .replace("\t", " ")
    )


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(
        json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    os.replace(temporary, path)


def write_jsonl(path: Path, values: Iterable[Mapping[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    with temporary.open("w", encoding="utf-8", newline="\n") as handle:
        for value in values:
            handle.write(
                json.dumps(
                    value,
                    ensure_ascii=False,
                    separators=(",", ":"),
                )
                + "\n"
            )
        handle.flush()
        os.fsync(handle.fileno())
    os.replace(temporary, path)


def read_json(path: Path, required: bool = True) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        if required:
            raise WorkflowDataError(f"required file is missing: {path}")
        return None
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        if required:
            raise WorkflowDataError(
                f"could not read JSON from {path}: {error}"
            ) from error
        return None


def load_state() -> Dict[str, Any]:
    value = read_json(STATE_PATH)
    if not isinstance(value, dict):
        raise WorkflowDataError(f"{STATE_PATH} must contain a JSON object")
    return value


def point_state_locator_at(path: Path) -> None:
    """Make the fixed runtime path locate the published canonical state."""
    CONTROL_DIR.mkdir(parents=True, exist_ok=True)
    temporary = STATE_PATH.with_name(STATE_PATH.name + ".link.tmp")
    try:
        temporary.unlink(missing_ok=True)
        target = os.path.relpath(path, start=STATE_PATH.parent)
        temporary.symlink_to(target)
        os.replace(temporary, STATE_PATH)
    finally:
        temporary.unlink(missing_ok=True)


def save_state(state: Mapping[str, Any]) -> None:
    copy = dict(state)
    state_path = copy.get("state_path")
    if isinstance(state_path, str) and state_path:
        canonical_path = Path(state_path)
        write_json(canonical_path, copy)
        point_state_locator_at(canonical_path)
    else:
        write_json(STATE_PATH, copy)


def emit(**updates: Any) -> None:
    print(
        json.dumps(
            {"context_updates": updates},
            ensure_ascii=False,
            separators=(",", ":"),
        )
    )


def git(
    *arguments: str,
    check: bool = False,
    input_bytes: Optional[bytes] = None,
) -> subprocess.CompletedProcess:
    environment = os.environ.copy()
    environment.update(
        {
            "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_TERMINAL_PROMPT": "0",
            "GIT_PAGER": "cat",
            "PAGER": "cat",
        }
    )
    try:
        result = subprocess.run(
            ["git", "-C", str(root()), *arguments],
            input=input_bytes,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
            env=environment,
        )
    except OSError as error:
        raise WorkflowDataError(f"could not run Git: {error}") from error
    if check and result.returncode != 0:
        detail = result.stderr.decode("utf-8", "replace").strip()
        raise WorkflowDataError(
            f"git {' '.join(arguments)} failed"
            + (f": {one_line(detail, 2000)}" if detail else "")
        )
    return result


def git_text(*arguments: str, check: bool = False) -> Optional[str]:
    result = git(*arguments, check=check)
    if result.returncode != 0:
        return None
    return result.stdout.decode("utf-8", "replace").rstrip("\r\n")


def inside_git_worktree() -> bool:
    return git_text("rev-parse", "--is-inside-work-tree") == "true"


def sha256_text(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def review_id_from_args(args: argparse.Namespace) -> str:
    """Resolve the run-scoped review ID, using Fabro's run ID when supplied."""
    explicit = getattr(args, "review_id", "")
    from_stdin = bool(getattr(args, "review_id_stdin", False))
    if from_stdin:
        raw = sys.stdin.buffer.read(MAX_REVIEW_ID_STDIN_BYTES + 1)
        if len(raw) > MAX_REVIEW_ID_STDIN_BYTES:
            raise WorkflowDataError(
                f"review ID input exceeds {MAX_REVIEW_ID_STDIN_BYTES} bytes"
            )
        try:
            explicit = raw.decode("utf-8").strip()
        except UnicodeError as error:
            raise WorkflowDataError(
                "review ID input is not valid UTF-8"
            ) from error
        if not explicit:
            raise WorkflowDataError("Fabro did not supply a review ID")
    review_id = str(explicit or f"local_{uuid.uuid4().hex}").strip()
    if not REVIEW_ID_RE.fullmatch(review_id):
        raise WorkflowDataError("review ID has an invalid format")
    return review_id
