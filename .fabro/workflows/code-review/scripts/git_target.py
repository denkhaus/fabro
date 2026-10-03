#!/usr/bin/env python3
"""Git target resolution and reviewed-source access.

Turns the run inputs (mode, base, commit, range, scope) into the reviewed
revision, diff stats and records, and the file inventories, and reads source
lines at the reviewed revision for exact anchors and report code frames.

Python 3.9-compatible. Standard library only."""

from __future__ import annotations

import functools
import hashlib
import os
import re
import sys
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath
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

from review_common import (  # noqa: E402
    CODE_FRAME_CONTEXT,
    CODE_FRAME_LANGUAGES,
    CODE_FRAME_MAX_BYTES,
    CODE_FRAME_MAX_LINE_LENGTH,
    MAX_CHANGED_FILES_LISTED,
    SAFE_REV_RE,
    SUGGESTION_CODE_MAX_LENGTH,
    WorkflowDataError,
    git,
    git_text,
    inside_git_worktree,
    root,
)


# --- Git target resolution ---------------------------------------------------


def validate_revision(value: str, field: str) -> str:
    text = value.strip()
    if not SAFE_REV_RE.fullmatch(text):
        raise WorkflowDataError(
            f"{field} must be one conservative Git revision token, got {value!r}"
        )
    return text


def resolve_commit(value: str, field: str) -> str:
    revision = validate_revision(value, field)
    resolved = git_text(
        "rev-parse",
        "--verify",
        "--quiet",
        revision + "^{commit}",
    )
    if not resolved:
        raise WorkflowDataError(
            f"{field} {value!r} does not resolve to a commit in this checkout; "
            "the workflow does not fetch missing refs"
        )
    return resolved


def parse_two_sided_range(raw: str) -> Tuple[str, str, str]:
    text = raw.strip()
    separator = "..." if "..." in text else ".."
    if separator not in text:
        raise WorkflowDataError(
            "range must be explicit and two-sided, such as base..HEAD"
        )
    left, right = text.split(separator, 1)
    if not left or not right or ".." in left or ".." in right:
        raise WorkflowDataError(
            "range must contain exactly two Git revision tokens"
        )
    return (
        validate_revision(left, "range start"),
        separator,
        validate_revision(right, "range end"),
    )


def default_base_ref() -> str:
    candidates: List[str] = []
    upstream = git_text(
        "rev-parse",
        "--abbrev-ref",
        "--symbolic-full-name",
        "@{upstream}",
    )
    if upstream and SAFE_REV_RE.fullmatch(upstream):
        candidates.append(upstream)
    candidates.extend(
        ["origin/HEAD", "origin/main", "origin/master", "main", "master"]
    )
    seen = set()
    for candidate in candidates:
        if candidate in seen:
            continue
        seen.add(candidate)
        if git_text(
            "rev-parse",
            "--verify",
            "--quiet",
            candidate + "^{commit}",
        ):
            return candidate
    raise WorkflowDataError(
        "changes mode could not resolve a base ref. Supply --base or an "
        "explicit two-sided --range; the workflow does not fetch"
    )


def empty_tree_hash() -> str:
    result = git(
        "hash-object", "-t", "tree", "--stdin", check=True, input_bytes=b""
    )
    value = result.stdout.decode("ascii", "replace").strip()
    if not value:
        raise WorkflowDataError("Git did not return the empty-tree object id")
    return value


def parse_scope(raw: str) -> List[str]:
    entries = [entry.strip().replace("\\", "/") for entry in raw.split(",")]
    entries = [entry for entry in entries if entry]
    if entries and all(entry in (".", "./") for entry in entries):
        return []
    normalized: List[str] = []
    for entry in entries:
        candidate = normalize_repo_path(entry)
        if candidate is None:
            raise WorkflowDataError(f"scope path is unsafe: {entry!r}")
        if candidate != "." and candidate not in normalized:
            normalized.append(candidate)
    return normalized


def normalize_repo_path(value: Any) -> Optional[str]:
    text = str("" if value is None else value).strip().replace("\\", "/")
    repository = root().as_posix().rstrip("/")
    if text == repository:
        return "."
    if text.startswith(repository + "/"):
        text = text[len(repository) + 1 :]
    while text.startswith("./"):
        text = text[2:]
    text = re.sub(r"/+$", "", text)
    if not text:
        return "."
    path = PurePosixPath(text)
    if path.is_absolute() or ".." in path.parts:
        return None
    return path.as_posix()


def decode_z_paths(raw: bytes) -> List[str]:
    return [
        item.decode("utf-8", "surrogateescape").replace("\\", "/")
        for item in raw.split(b"\0")
        if item
    ]


def tracked_files(scopes: Sequence[str] = ()) -> List[str]:
    if not inside_git_worktree():
        return []
    arguments = ["ls-files", "-z"]
    if scopes:
        arguments.extend(["--", *scopes])
    result = git(*arguments, check=True)
    return sorted(decode_z_paths(result.stdout))


def is_generated_path(path: str) -> bool:
    normalized = path.replace("\\", "/")
    while normalized.startswith("./"):
        normalized = normalized[2:]
    top = normalized.split("/", 1)[0]
    if top.startswith("CODE-REVIEW-"):
        return True
    generated = (
        ".fabro/blobs",
        ".fabro/workflows/code-review/runtime",
    )
    return any(
        normalized == prefix or normalized.startswith(prefix + "/")
        for prefix in generated
    )


def repo_files() -> List[str]:
    listing = git("ls-files", "--cached", "--others", "--exclude-standard", "-z")
    if listing.returncode == 0:
        return sorted(
            path
            for path in decode_z_paths(listing.stdout)
            if not is_generated_path(path)
        )

    paths: List[str] = []
    skipped_directories = {
        ".git",
        ".cache",
        ".venv",
        "dist",
        "node_modules",
        "target",
    }
    for current, directories, files in os.walk(root()):
        directories[:] = [
            name
            for name in directories
            if name not in skipped_directories
            and not name.startswith("CODE-REVIEW-")
        ]
        for name in files:
            relative = (Path(current) / name).relative_to(root()).as_posix()
            if not is_generated_path(relative):
                paths.append(relative)
    return sorted(paths)


def diff_stats(
    revision_range: str,
    scopes: Sequence[str],
) -> Tuple[List[str], Optional[int]]:
    suffix = ["--", *scopes] if scopes else ["--"]
    names = git(
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--name-only",
        "-z",
        revision_range,
        *suffix,
        check=True,
    )
    files = decode_z_paths(names.stdout)
    numstat = git(
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--numstat",
        revision_range,
        *suffix,
        check=True,
    )
    total = 0
    for raw_line in numstat.stdout.decode("utf-8", "replace").splitlines():
        columns = raw_line.split("\t", 2)
        if (
            len(columns) < 3
            or not columns[0].isdigit()
            or not columns[1].isdigit()
        ):
            return files, None
        total += int(columns[0]) + int(columns[1])
    return files, total


def diff_file_records(
    revision_range: str,
    scopes: Sequence[str],
) -> Dict[str, Dict[str, Any]]:
    """Per-file status and churn for the range, keyed by new-side path."""
    suffix = ["--", *scopes] if scopes else ["--"]
    records: Dict[str, Dict[str, Any]] = {}
    status_listing = git(
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--name-status",
        "-z",
        revision_range,
        *suffix,
        check=True,
    )
    items = [
        item.decode("utf-8", "surrogateescape").replace("\\", "/")
        for item in status_listing.stdout.split(b"\0")
    ]
    index = 0
    while index < len(items):
        status = items[index]
        if not status:
            index += 1
            continue
        letter = status[0]
        if letter in {"R", "C"} and index + 2 < len(items):
            old_path, new_path = items[index + 1], items[index + 2]
            records[new_path] = {"status": letter, "old_path": old_path}
            index += 3
        elif index + 1 < len(items):
            records[items[index + 1]] = {"status": letter}
            index += 2
        else:
            break
    numstat = git(
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--numstat",
        "-z",
        revision_range,
        *suffix,
        check=True,
    )
    entries = [
        item.decode("utf-8", "surrogateescape").replace("\\", "/")
        for item in numstat.stdout.split(b"\0")
    ]
    index = 0
    while index < len(entries):
        entry = entries[index]
        if not entry:
            index += 1
            continue
        columns = entry.split("\t")
        if len(columns) < 3:
            index += 1
            continue
        added, deleted, path = columns[0], columns[1], columns[2]
        if not path:
            # -z rename form: "added\tdeleted\t\0old\0new\0"
            if index + 2 >= len(entries):
                break
            path = entries[index + 2]
            index += 3
        else:
            index += 1
        record = records.setdefault(path, {"status": "M"})
        if added.isdigit() and deleted.isdigit():
            record["added"] = int(added)
            record["deleted"] = int(deleted)
    return records


def diff_record_summary(
    records: Mapping[str, Mapping[str, Any]],
) -> Tuple[List[str], Optional[int]]:
    """Return sorted paths and total text churn from one diff scan."""
    churn = [
        record.get("added", 0) + record.get("deleted", 0)
        for record in records.values()
        if isinstance(record.get("added"), int)
        and isinstance(record.get("deleted"), int)
    ]
    total = sum(churn) if len(churn) == len(records) else None
    return sorted(records), total


def read_file_at_revision(revision: str, path: str) -> Optional[bytes]:
    result = git("show", f"{revision}:{path}")
    if result.returncode != 0:
        return None
    return result.stdout


def workspace_digest() -> str:
    digest = hashlib.sha256()
    for relative in repo_files():
        digest.update(relative.encode("utf-8", "surrogateescape"))
        digest.update(b"\0")
        path = root() / relative
        try:
            stat_result = path.lstat()
        except FileNotFoundError:
            digest.update(b"MISSING\0")
            continue
        digest.update(str(stat_result.st_mode).encode("ascii"))
        digest.update(b"\0")
        if path.is_symlink():
            digest.update(os.readlink(path).encode("utf-8", "surrogateescape"))
        elif path.is_file():
            with path.open("rb") as handle:
                while True:
                    chunk = handle.read(1024 * 1024)
                    if not chunk:
                        break
                    digest.update(chunk)
        digest.update(b"\0")
    return digest.hexdigest()


def assert_workspace_unchanged(state: Mapping[str, Any]) -> None:
    """Refuse to publish results derived from a tampered source tree.

    Tamper evidence behind the read-only tool guard: an agent that finds a way
    to write could shape what the verifiers and the report see. Checked at
    final-tally after the last agent and again at publish-pr immediately before
    anything leaves for GitHub.
    """
    expected = state.get("workspace_digest")
    actual = workspace_digest()
    if not isinstance(expected, str) or actual != expected:
        raise WorkflowDataError(
            "the reviewed source tree changed during the review; refusing "
            "to publish results derived from it"
        )


def worktree_dirty() -> Optional[bool]:
    status = git("status", "--porcelain=v1", "-z", "--untracked-files=all")
    if status.returncode != 0:
        return None
    entries = status.stdout.split(b"\0")
    index = 0
    while index < len(entries):
        raw_entry = entries[index]
        index += 1
        if not raw_entry:
            continue
        entry = raw_entry.decode("utf-8", "surrogateescape")
        if len(entry) < 4:
            return True
        status_code = entry[:2]
        paths = [entry[3:]]
        if status_code[0] in {"R", "C"} or status_code[1] in {"R", "C"}:
            if index >= len(entries) or not entries[index]:
                return True
            paths.append(
                entries[index].decode("utf-8", "surrogateescape")
            )
            index += 1
        if any(not is_generated_path(path) for path in paths):
            return True
    return False


def revision_record(
    mode: str,
    target_commit: Optional[str],
    base: Optional[str],
    merge_base: Optional[str],
    parent: Optional[str],
    revision_range: Optional[str],
) -> Dict[str, Any]:
    if not inside_git_worktree():
        return {"versioned": False}
    head = git_text("rev-parse", "HEAD")
    branch = git_text("symbolic-ref", "--short", "-q", "HEAD")
    if mode == "commit":
        return {
            "versioned": True,
            "commit": target_commit,
            "parent": parent,
            "branch": branch,
            "dirty": worktree_dirty(),
            "range": revision_range,
        }
    revision: Dict[str, Any] = {
        "versioned": True,
        "commit": target_commit or head,
        "branch": branch,
        "dirty": worktree_dirty(),
    }
    if mode == "changes":
        revision.update(
            {
                "base": base,
                "merge_base": merge_base,
                "range": revision_range,
            }
        )
    return revision


def unique_report_dir() -> Tuple[Path, str]:
    stem = datetime.now(timezone.utc).strftime("%Y%m%d-%H%M%S")
    name = f"CODE-REVIEW-{stem}"
    candidate = root() / name
    suffix = 1
    while candidate.exists():
        name = f"CODE-REVIEW-{stem}-{suffix}"
        candidate = root() / name
        suffix += 1
    candidate.mkdir(parents=False)
    return candidate, name


def common_target(state: Mapping[str, Any]) -> Dict[str, Any]:
    changed = list(state.get("changed_files") or [])
    return {
        "mode": state.get("mode"),
        "scope": state.get("scope") or [],
        "range": state.get("range"),
        "changedFileCount": state.get("diff_files"),
        "changedLineCount": state.get("diff_lines"),
        "changedFiles": changed[:MAX_CHANGED_FILES_LISTED],
        "changedFilesTruncated": max(0, len(changed) - MAX_CHANGED_FILES_LISTED),
        "reviewRoot": str(root()),
        "gitWrapper": (
            "python3 -I .fabro/workflows/code-review/scripts/git_readonly.py"
        ),
    }


# --- Source access at the reviewed revision -------------------------------
def code_frame_language(file_path: str) -> str:
    suffix = PurePosixPath(file_path).suffix.lstrip(".").lower()
    return CODE_FRAME_LANGUAGES.get(suffix, "Source")


def safe_code_text(value: str) -> str:
    text = "".join(
        character
        if character == "\t" or ord(character) >= 0x20
        else " "
        for character in value
    )
    if len(text) > CODE_FRAME_MAX_LINE_LENGTH:
        return text[:CODE_FRAME_MAX_LINE_LENGTH] + "..."
    return text


def reviewed_revision(state: Optional[Mapping[str, Any]]) -> Optional[str]:
    if not state or state.get("mode") not in {"changes", "commit"}:
        return None
    revision = state.get("commit")
    return revision if isinstance(revision, str) and revision else None


@functools.lru_cache(maxsize=16)
def reviewed_source_lines(
    file_path: str, revision: Optional[str] = None
) -> Optional[List[str]]:
    """Read one UTF-8 source file from the exact reviewed revision."""
    if revision is not None:
        tree_entry = git("ls-tree", "-z", revision, "--", file_path)
        if tree_entry.returncode != 0 or not tree_entry.stdout:
            return None
        mode = tree_entry.stdout.split(None, 1)[0]
        if mode == b"120000":
            return None
        raw = read_file_at_revision(revision, file_path)
        if raw is None or len(raw) > CODE_FRAME_MAX_BYTES:
            return None
    else:
        target = root() / file_path
        try:
            if target.is_symlink() or not target.is_file():
                return None
            if target.stat().st_size > CODE_FRAME_MAX_BYTES:
                return None
            raw = target.read_bytes()
        except OSError:
            return None
    if b"\0" in raw:
        return None
    try:
        return raw.decode("utf-8").splitlines()
    except UnicodeError:
        return None


def resolved_location(
    file_path: str,
    start_line: int,
    end_line: int,
    state: Optional[Mapping[str, Any]] = None,
) -> Dict[str, Any]:
    """Build an engine-derived exact anchor for a finding."""
    existing_code = ""
    source_lines = reviewed_source_lines(file_path, reviewed_revision(state))
    if (
        source_lines is not None
        and 1 <= start_line <= end_line <= len(source_lines)
    ):
        candidate = "\n".join(source_lines[start_line - 1:end_line])
        if (
            len(candidate) <= SUGGESTION_CODE_MAX_LENGTH
            and not any(
                character not in "\n\t" and ord(character) < 0x20
                for character in candidate
            )
        ):
            existing_code = candidate
    return {
        "start_line": start_line,
        "end_line": end_line,
        "existing_code": existing_code,
    }


def code_frame(
    file_path: str,
    start_line: int,
    end_line: Optional[int] = None,
    state: Optional[Mapping[str, Any]] = None,
) -> Dict[str, Any]:
    """Read the lines around a finding's anchor range from the reviewed tree.

    The excerpt shown in the report is read here, so its line numbers are the
    tree's own and no agent transcribes them. An unreadable, binary,
    oversized, or out-of-range target yields an empty excerpt.
    """
    end_line = start_line if end_line is None else end_line
    language = code_frame_language(file_path)
    empty: Dict[str, Any] = {
        "language": language,
        "label": f"{file_path}:{start_line}-{end_line}",
        "lines": [],
    }
    source_lines = reviewed_source_lines(file_path, reviewed_revision(state))
    if (
        source_lines is None
        or start_line < 1
        or end_line < start_line
        or end_line > len(source_lines)
    ):
        return empty
    start = max(1, start_line - CODE_FRAME_CONTEXT)
    end = min(len(source_lines), end_line + CODE_FRAME_CONTEXT)
    lines: List[Dict[str, Any]] = []
    for number in range(start, end + 1):
        entry: Dict[str, Any] = {
            "number": number,
            "text": safe_code_text(source_lines[number - 1]),
        }
        if start_line <= number <= end_line:
            entry["highlight"] = True
        lines.append(entry)
    return {
        "language": language,
        "label": f"{file_path}:{start}-{end}",
        "lines": lines,
    }
