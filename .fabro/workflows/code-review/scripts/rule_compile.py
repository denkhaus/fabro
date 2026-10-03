#!/usr/bin/env python3
"""Rule compilation for the rule-mapped tiers (every tier above low).

Imports the rule_loader sibling (which needs the pinned PyYAML dependency),
reads repository rule files at the base revision, sniffs .m files, and
compiles the canonical rule state the planner and report hash against.

Python 3.9-compatible. Standard library only (rule_loader brings PyYAML)."""

from __future__ import annotations

import hashlib
import sys
from pathlib import Path
from typing import (
    Any,
    Dict,
    List,
    Mapping,
    Optional,
    Tuple,
)

sys.path.insert(0, str(Path(__file__).resolve().parent))

from git_target import (  # noqa: E402
    decode_z_paths,
    read_file_at_revision,
    repo_files,
)
from review_common import (  # noqa: E402
    WORKFLOW_ROOT,
    WorkflowDataError,
    git,
    git_text,
    inside_git_worktree,
    read_json,
    root,
)


# --- Rule compilation (rule-mapped tiers) -------------------------------------

def import_rule_loader() -> Any:
    try:
        import rule_loader
    except ImportError as error:
        raise WorkflowDataError(
            "the rule-mapped tiers need the rule loader and its pinned PyYAML "
            f"dependency (see README, Developing): {error}"
        ) from error
    return rule_loader


def repo_rule_revision(state: Mapping[str, Any]) -> Optional[str]:
    """The revision repository rules are read from.

    Rules come from the base side of the review -- the merge base (or the
    left endpoint of an explicit two-dot range) in changes mode, the reviewed
    commit's parent in commit mode -- so a change cannot weaken the rules
    used to review itself. Files mode reads the reviewed HEAD revision, or
    the working filesystem outside a Git worktree (returned as None).
    """
    mode = str(state.get("mode"))
    if mode == "changes":
        merge_base = state.get("merge_base")
        if not isinstance(merge_base, str) or not merge_base:
            raise WorkflowDataError("changes mode has no resolved merge base")
        return merge_base
    if mode == "commit":
        parent = state.get("parent")
        return parent if isinstance(parent, str) and parent else None
    if inside_git_worktree():
        return git_text("rev-parse", "HEAD", check=True)
    return None


def read_repo_rule_files(
    loader: Any,
    revision: Optional[str],
) -> List[Tuple[str, bytes]]:
    if revision is None:
        candidates: List[str] = []
        entry = root() / loader.REPO_ENTRYPOINT
        if entry.is_file():
            candidates.append(loader.REPO_ENTRYPOINT)
        rules_dir = root() / loader.REPO_RULES_PREFIX
        if rules_dir.is_dir():
            candidates.extend(
                path.relative_to(root()).as_posix()
                for path in rules_dir.rglob("*.yaml")
                if path.is_file()
            )
        ordered = loader.discover_repo_rule_paths(candidates)
        return [(path, (root() / path).read_bytes()) for path in ordered]
    listing = git(
        "ls-tree", "-r", "--name-only", "-z", revision, "--", ".fabro",
        check=True,
    )
    ordered = loader.discover_repo_rule_paths(decode_z_paths(listing.stdout))
    files: List[Tuple[str, bytes]] = []
    for path in ordered:
        content = read_file_at_revision(revision, path)
        if content is None:
            raise WorkflowDataError(
                f"could not read repository rule file {path} at the rule "
                "revision"
            )
        files.append((path, content))
    return files


def sniff_m_files(
    loader: Any,
    state: Mapping[str, Any],
    file_records: Mapping[str, Mapping[str, Any]],
) -> Dict[str, Dict[str, str]]:
    """Classify every ".m" target file as MATLAB or Objective-C.

    Added, modified, and renamed files are read at the reviewed target
    revision; deleted files at the base revision; never an unrelated
    checkout. Missing or inconclusive bytes keep the deterministic MATLAB
    default.
    """
    mode = str(state.get("mode"))
    target_revision = state.get("commit")
    base_revision = (
        state.get("merge_base") if mode == "changes" else state.get("parent")
    )
    results: Dict[str, Dict[str, str]] = {}
    for path in state.get("changed_files") or []:
        if not path.lower().endswith(".m"):
            continue
        record = file_records.get(path) or {}
        deleted = record.get("status") == "D"
        content: Optional[bytes] = None
        if mode in {"changes", "commit"}:
            revision = base_revision if deleted else target_revision
            if isinstance(revision, str) and revision:
                content = read_file_at_revision(revision, path)
        elif isinstance(target_revision, str) and target_revision:
            content = read_file_at_revision(target_revision, path)
        else:
            try:
                target_path = root() / path
                if target_path.is_file() and not target_path.is_symlink():
                    content = target_path.read_bytes()
            except OSError:
                content = None
        language, source = loader.sniff_m_language(content)
        results[path] = {"language": language, "source": source}
    return results


def compile_rule_state(
    state: Dict[str, Any],
    file_records: Mapping[str, Mapping[str, Any]],
) -> None:
    """Resolve the effective rule checks for every target file.

    Runs for every rule-mapped tier. An invalid built-in or base-revision
    repository rule file is a deterministic workflow failure: the run must
    not proceed while claiming rule coverage that was not applied.

    The tier's rule layers select what compiles: "full" uses the whole
    built-in library; "repo-instructions" keeps only the
    repository-instructions built-in pack alongside the repository rules.
    Manifest integrity is verified either way.
    """
    layers = str(state.get("rule_layers") or "full")
    loader = import_rule_loader()
    workflow_root = root() / WORKFLOW_ROOT
    manifest_path = workflow_root / loader.BUILTIN_MANIFEST
    manifest = read_json(manifest_path)
    builtin_manifest_sha = hashlib.sha256(
        manifest_path.read_bytes()
    ).hexdigest()
    try:
        builtin_files = loader.load_builtin_files(workflow_root, manifest)
        builtin_packs = loader.load_rule_layer(builtin_files, "builtin")
        revision = repo_rule_revision(state)
        repo_files = read_repo_rule_files(loader, revision)
        repo_packs = loader.load_rule_layer(repo_files, "repo")
    except loader.RuleLoaderError as error:
        raise WorkflowDataError(f"rule configuration is invalid: {error}")
    if layers == "repo-instructions":
        builtin_packs = [
            pack
            for pack in builtin_packs
            if pack["pack_id"] == loader.INSTRUCTIONS_PACK_ID
        ]

    # The ".m" sniff only selects between built-in language packs, which the
    # filtered layers do not compile.
    sniff = (
        sniff_m_files(loader, state, file_records)
        if layers == "full"
        else {}
    )
    catalog: Dict[str, Dict[str, Any]] = {}
    effective: Dict[str, List[str]] = {}
    overridden: Dict[str, List[str]] = {}
    for path in state.get("changed_files") or []:
        m_language = sniff.get(path, {}).get("language")
        resolved = loader.effective_checks_for_path(
            path, builtin_packs, repo_packs, m_language
        )
        ids: List[str] = []
        for check in resolved["checks"]:
            catalog[check["id"]] = check
            ids.append(check["id"])
        effective[path] = ids
        if resolved["overridden"]:
            overridden[path] = list(resolved["overridden"])

    state["rules"] = {
        "enabled": True,
        "layers": layers,
        "config_sha256": loader.rule_config_sha256(
            builtin_packs, repo_packs
        ),
        "builtin_manifest_sha256": builtin_manifest_sha,
        "repo_rule_revision": revision,
        "repo_rule_files": [path for path, _content in repo_files],
        "counts": {
            "builtin_packs": len(builtin_packs),
            "repo_packs": len(repo_packs),
            "builtin_checks": sum(
                len(pack["checks"]) for pack in builtin_packs
            ),
            "repo_checks": sum(len(pack["checks"]) for pack in repo_packs),
        },
        "catalog": catalog,
        "effective": effective,
        "overridden": overridden,
        "sniff": sniff,
    }
