"""
reviewer-agent: static reviewer for Fabro workflow assets and repo wiring.

Scans a repository that hosts Fabro workflows (`.fabro/workflows/...`) and
reports weaknesses and gaps with exact file:line references:

- AGNOSTICITY: the workflow and all its assets (graph, prompts, scripts,
  workflow.toml) must be project-agnostic. No references into the current
  project (project name, seed-id prefix, repo files) and no project tooling
  except an explicit allowlist (default: just, ml, sd). `git` and `fabro`
  count as platform substrate (Fabro checkpoints are plain Git, see
  docs/fabro/checkpoints.md) and are allowed by default.
- GRAPH: workflow.fabro correctness checked against the local Fabro docs in
  docs/fabro/ (outcome values, label-routing contract between prompts and
  edge labels, missing safeguards like timeouts / max_node_visits / goal
  gates, dangling @prompt and script references, template variables).
- SCRIPTS: content heuristics for the deterministic bridge scripts
  (checkpoint-base fragility, untracked-artifact blind spot, just-recipe
  existence).
- PROMPTS: cross-node context-key flow, goal-injection guards, reviewer
  ground truth, role boundaries.
- REPO: project.toml prepare wiring, tool provisioning for the allowlist.

The entry point is `run()`; see its docstring for the API. Findings carry a
rule id, a severity, `path:line` references and a concrete suggestion.
"""

from __future__ import annotations

import json
import re
import subprocess
from dataclasses import dataclass, field, asdict
from pathlib import Path
from typing import Iterable

# --------------------------------------------------------------------------
# data model
# --------------------------------------------------------------------------

SEVERITY_ORDER = {"error": 0, "warn": 1, "info": 2, "pass": 3}


@dataclass
class Finding:
    rule: str
    severity: str  # error | warn | info | pass
    path: str      # repo-relative path
    lines: tuple[int, ...]
    title: str
    detail: str = ""
    suggestion: str = ""

    def ref(self) -> str:
        loc = self.path
        if self.lines:
            first, last = self.lines[0], self.lines[-1]
            loc += f":{first}" + (f"-{last}" if last != first else "")
        return loc

    def render_md(self) -> str:
        icon = {"error": "ERROR", "warn": "WARN ", "info": "INFO ", "pass": "PASS "}[self.severity]
        out = [f"### [{icon}] {self.rule} — {self.title}", f"`{self.ref()}`"]
        if self.detail:
            out.append(self.detail)
        if self.suggestion:
            out.append(f"**Suggestion:** {self.suggestion}")
        return "\n".join(out) + "\n"


@dataclass
class Fingerprint:
    """Project identity + toolchain, used to detect agnosticity leaks."""

    root: Path
    project_names: list[str] = field(default_factory=list)
    seed_prefix: str | None = None
    mise_tools: list[str] = field(default_factory=list)   # raw tool names from mise config
    tracked_files: list[str] = field(default_factory=list)
    just_recipes: dict[str, int] = field(default_factory=dict)  # name -> line in justfile


# tool name -> binary names it provides (for word-boundary matching)
TOOL_BINARIES = {
    # NOTE: binaries that are also common English words need command context;
    # see AMBIGUOUS_BINARY_PATTERNS below.
    "nushell": ("nu", "nushell"),
    "bun": ("bun", "bunx"),
    "go": ("go", "gofmt"),
    "node": ("node", "npm", "npx"),
    "python": ("python", "python3", "pip"),
}


def _read(p: Path) -> str:
    try:
        return p.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return ""


# --------------------------------------------------------------------------
# fingerprinting the host project
# --------------------------------------------------------------------------

def build_fingerprint(root: Path) -> Fingerprint:
    fp = Fingerprint(root=root)

    # seeds project name -> seed-id prefix ("fabro" -> "fabro-f487")
    m = re.search(r'^\s*project:\s*["\']?([\w.-]+)', _read(root / ".seeds/config.yaml"), re.M)
    if m:
        fp.project_names.append(m.group(1))
        fp.seed_prefix = m.group(1)

    # module/package names
    m = re.search(r"^module\s+(\S+)", _read(root / "go.mod"), re.M)
    if m:
        fp.project_names.append(m.group(1).split("/")[-1])
    m = re.search(r'"name"\s*:\s*"([^"]+)"', _read(root / "package.json"))
    if m:
        fp.project_names.append(m.group(1).split("/")[-1])
    m = re.search(r'^name\s*=\s*"([^"]+)"', _read(root / "Cargo.toml"), re.M)
    if m:
        fp.project_names.append(m.group(1))
    m = re.search(r'^name\s*=\s*"([^"]+)"', _read(root / "pyproject.toml"), re.M)
    if m:
        fp.project_names.append(m.group(1))

    # toolchain from mise config (the "current tooling" the workflow must not name)
    mise = root / ".mise.toml"
    if not mise.exists():
        mise = root / "mise.toml"
    txt = mise.read_text() if mise.exists() else ""
    in_tools = False
    for line in txt.splitlines():
        if re.match(r"^\s*\[tools\]", line):
            in_tools = True
            continue
        if in_tools:
            if line.strip().startswith("["):
                break
            m = re.match(r'^\s*"?([^"=\s]+)"?\s*=', line)
            if m:
                name = m.group(1).split(":")[-1].split("/")[-1]
                fp.mise_tools.append(name)

    # tracked files (repo-specific names the workflow must not reference)
    try:
        out = subprocess.run(["git", "ls-files"], cwd=root, capture_output=True, text=True, timeout=30)
        fp.tracked_files = [l for l in out.stdout.splitlines() if l.strip()]
    except Exception:
        fp.tracked_files = [
            str(p.relative_to(root)) for p in root.rglob("*")
            if p.is_file() and ".git/" not in str(p) and ".fabro/" not in str(p)
        ]

    # justfile recipes
    justfile = None
    for cand in ("justfile", "Justfile"):
        if (root / cand).exists():
            justfile = root / cand
            break
    if justfile:
        for i, line in enumerate(_read(justfile).splitlines(), 1):
            m = re.match(r"^([A-Za-z0-9_-]+)\s*:", line)
            if m and not line.startswith("#"):
                fp.just_recipes[m.group(1)] = i
    return fp


# --------------------------------------------------------------------------
# DOT (workflow.fabro) parsing
# --------------------------------------------------------------------------

@dataclass
class Node:
    name: str
    attrs: dict[str, str] = field(default_factory=dict)
    attr_lines: dict[str, int] = field(default_factory=dict)
    line: int = 0


@dataclass
class Edge:
    src: str
    dst: str
    attrs: dict[str, str] = field(default_factory=dict)
    attr_lines: dict[str, int] = field(default_factory=dict)
    label: str = ""
    line: int = 0
    line_end: int = 0


def parse_dot(text: str) -> tuple[dict[str, Node], list[Edge], dict[str, str]]:
    """Tiny Graphviz-subset parser: node stmts, `a -> b` edges, graph attrs."""
    nodes: dict[str, Node] = {}
    edges: list[Edge] = []
    graph_attrs: dict[str, str] = {}
    i, n = 0, len(text)
    line = 1

    def skip_ws() -> None:
        nonlocal i, line
        while i < n:
            c = text[i]
            if c == "\n":
                line += 1
                i += 1
            elif c in " \t\r":
                i += 1
            elif text.startswith("//", i):
                while i < n and text[i] != "\n":
                    i += 1
            elif c in "#;":
                while i < n and text[i] != "\n":
                    i += 1
            else:
                return

    def ident() -> str:
        nonlocal i
        start = i
        while i < n and (text[i].isalnum() or text[i] in "_."):
            i += 1
        return text[start:i]

    def bracket_block() -> tuple[str, int, int]:
        """Consume `[ ... ]` respecting double-quoted strings."""
        nonlocal i, line
        assert text[i] == "["
        start_line = line
        depth = 0
        in_str = False
        start = i
        while i < n:
            c = text[i]
            if c == '"' and text[i - 1] != "\\":
                in_str = not in_str
            elif not in_str:
                if c == "[":
                    depth += 1
                elif c == "]":
                    depth -= 1
                    if depth == 0:
                        block = text[start + 1:i]
                        i += 1
                        return block, start_line, line
            if c == "\n":
                line += 1
            i += 1
        return text[start + 1:], start_line, line

    def parse_attrs(block: str, block_line: int = 0) -> tuple[dict[str, str], dict[str, int]]:
        attrs: dict[str, str] = {}
        attr_lines: dict[str, int] = {}
        for m in re.finditer(r'([\w.]+)\s*=\s*"((?:[^"\\]|\\.)*)"', block):
            attrs[m.group(1)] = m.group(2)
            attr_lines[m.group(1)] = block_line + block[: m.start()].count("\n") + 1
        for m in re.finditer(r'([\w.]+)\s*=\s*([A-Za-z0-9_.:-]+)', block):
            if m.group(1) not in attrs:
                attrs[m.group(1)] = m.group(2)
                attr_lines[m.group(1)] = block_line + block[: m.start()].count("\n") + 1
        return attrs, attr_lines

    while True:
        skip_ws()
        if i >= n:
            break
        if text[i] in "{}":
            i += 1
            continue
        start_line = line
        word = ident()
        skip_ws()
        if not word:
            i += 1
            continue
        if word in ("graph", "node", "edge"):
            if i < n and text[i] == "[":
                block, sl, _ = bracket_block()
                attrs, _al = parse_attrs(block, sl)
                if word == "graph":
                    graph_attrs.update(attrs)
            continue
        if word in ("digraph", "subgraph", "strict"):
            if i < n and text[i] not in "{[":
                ident()  # swallow the graph name
            continue
        if text.startswith("->", i):
            srcs = [word]
            while text.startswith("->", i):
                i += 2
                skip_ws()
                srcs.append(ident())
                skip_ws()
            attrs: dict[str, str] = {}
            alines: dict[str, int] = {}
            end_line = line
            if i < n and text[i] == "[":
                block, sl, end_line = bracket_block()
                attrs, alines = parse_attrs(block, sl)
            for a, b in zip(srcs, srcs[1:]):
                edges.append(Edge(a, b, attrs, alines, attrs.get("label", ""), start_line, end_line))
            continue
        if i < n and text[i] == "[":
            block, sl, _ = bracket_block()
            attrs, alines = parse_attrs(block, sl)
            if word not in nodes:
                nodes[word] = Node(word, attrs, alines, start_line)
            else:
                nodes[word].attrs.update(attrs)
                nodes[word].attr_lines.update(alines)
        else:
            # bare statement (e.g. `start -> planner` without attrs handled above;
            # standalone id is a node)
            if word not in nodes:
                nodes[word] = Node(word, {}, start_line)
    return nodes, edges, graph_attrs


# Binaries whose bare name is a common English word: only flag them when they
# look like an invocation (subcommand or backticked), not as prose verbs.
AMBIGUOUS_BINARY_PATTERNS = {
    "go": re.compile(
        r"`go`|(?<![\w.\-])go\s+(?:build|test|vet|run|fmt|mod|install|generate|version|doc|tool|get|work)\b"
    ),
}


def _tool_ref_pattern(binary: str) -> re.Pattern[str]:
    if binary in AMBIGUOUS_BINARY_PATTERNS:
        return AMBIGUOUS_BINARY_PATTERNS[binary]
    return re.compile(rf"(?<![\w.-]){re.escape(binary)}(?![\w.-])")


VALID_OUTCOMES = {"succeeded", "failed", "partially_succeeded", "skipped"}
KNOWN_SHAPES = {
    "box", "tab", "parallelogram", "hexagon", "diamond", "component",
    "Mdiamond", "Msquare", "house", "circle", "doublecircle",
}
COMMAND_SHAPES = {"parallelogram"}       # script nodes (docs/fabro/tools.md)
PROMPT_ONLY_SHAPES = {"tab"}             # single LLM call, no tools (docs/fabro/prompts.md L297-309)


# --------------------------------------------------------------------------
# agnosticity rules
# --------------------------------------------------------------------------

# platform vocabulary: `fabro` the platform must not be confused with a
# project that happens to be called "fabro"
PLATFORM_FABRO_PATTERNS = (
    r"\.fabro\b", r"\.fabro/", r"docs\.fabro\.sh", r"Fabro-", r"fabro/run/",
    r"fabro/meta/", r"fabro\(", r"\bfabro\b(?=\s*(?:workflow|run|CLI|server|checkpoint))",
)


def _platform_line(line: str) -> bool:
    return any(re.search(p, line) for p in PLATFORM_FABRO_PATTERNS)


def check_agnosticity(
    wf_root: Path, rel_dir: str, files: list[Path], fp: Fingerprint,
    allowed_tools: set[str], platform_tools: set[str],
) -> list[Finding]:
    findings: list[Finding] = []
    allowed_bins = set(allowed_tools)
    for t in allowed_tools | platform_tools:
        allowed_bins.update(TOOL_BINARIES.get(t, (t,)))

    # forbidden tool binaries: mise tools minus allowlist
    forbidden_tools: dict[str, str] = {}   # binary -> mise tool name
    for t in fp.mise_tools:
        if t in allowed_tools or t in platform_tools:
            continue
        for b in TOOL_BINARIES.get(t, (t,)):
            forbidden_tools[b] = t

    # repo files outside the workflow dir (referencing them couples the workflow to this repo)
    own_prefix = f"{rel_dir}/"
    external_files = [f for f in fp.tracked_files if not f.startswith(own_prefix)]

    seed_prefix_re = (
        re.compile(rf"\b{re.escape(fp.seed_prefix)}-[a-z0-9]{{2,}}\b", re.I)
        if fp.seed_prefix else None
    )

    for f in files:
        rel = str(f.relative_to(fp.root))
        lines = _read(f).splitlines()
        for idx, line in enumerate(lines, 1):
            # AGNOS-01: project tooling referenced in workflow assets.
            # script= attrs in graph files are covered precisely by AGNOS-06.
            graph_script_line = f.name == "workflow.fabro" and 'script="' in line
            if not graph_script_line:
                for binary, tool in sorted(forbidden_tools.items()):
                    if _tool_ref_pattern(binary).search(line):
                        findings.append(Finding(
                            "AGNOS-01", "error", rel, (idx,),
                            f"workflow asset references project tooling '{binary}' (from .mise.toml tool '{tool}')",
                            "Only just, ml and sd may be referenced by the workflow; "
                            f"'{binary}' is toolchain of this specific project.",
                            f"Remove the '{binary}' reference or route it through a `just` recipe "
                            f"defined by the host project.",
                        ))
            # AGNOS-02: project name leak (excluding platform use of 'fabro')
            for name in fp.project_names:
                if name.lower() == "fabro":
                    if _platform_line(line):
                        continue
                for m in re.finditer(rf"(?<![\w.-]){re.escape(name)}(?![\w.-])", line, re.I):
                    ctx = line[max(0, m.start() - 20): m.end() + 20]
                    findings.append(Finding(
                        "AGNOS-02", "error", rel, (idx,),
                        f"workflow asset leaks the current project name '{name}'",
                        f"Context: ...{ctx.strip()}...",
                        "Replace with a generic placeholder (e.g. `<seed id>`, `the project`).",
                    ))
                    break
            # AGNOS-03: seed-id prefix of THIS tracker
            if seed_prefix_re and seed_prefix_re.search(line) and not _platform_line(line):
                findings.append(Finding(
                    "AGNOS-03", "error", rel, (idx,),
                    f"example seed id uses this repo's tracker prefix '{fp.seed_prefix}-…'",
                    f"Line: {line.strip()[:120]}",
                    "Use a neutral example id (e.g. `<project>-<hash>` or `abc-123`).",
                ))
            # AGNOS-04: references to files outside the workflow directory
            universal = {".gitignore", ".gitattributes", ".gitmodules", "readme.md",
                         "license", "licence", "changelog.md", "makefile",
                         "justfile", ".justfile",
                         # agent/docs convention roots shared across repos,
                         # not this project's layout
                         "agents.md", "claude.md", "context.md", "context-map.md",
                         "contributors.md", "lefthook.yml", "go.sum", "go.mod",
                         "package.json", "bun.lock", "cargo.lock", "rust-toolchain",
                         "rust-toolchain.toml", "mise.toml", ".mise.toml",
                         "pyproject.toml", "requirements.txt", "dockerfile",
                         "docker-compose.yaml", "docker-compose.yml", ".env.example"}
            for tf in external_files:
                base = Path(tf).name
                if len(base) < 5 or base.lower() in universal:
                    continue
                if (wf_root / base).exists():
                    continue  # self-reference to a file shipped in the workflow dir
                if re.search(rf"(?<![\w/]){re.escape(base)}(?![\w.])", line):
                    findings.append(Finding(
                        "AGNOS-04", "warn", rel, (idx,),
                        f"workflow asset references repo file '{tf}' outside the workflow directory",
                        "Couples the workflow to this repository's layout.",
                        "Reference only files that ship inside the workflow directory, or go "
                        "through a `just` recipe the host project defines.",
                    ))
                    break
            # AGNOS-05: hard-coded gate/toolchain semantics in prompts
            if f.suffix == ".md":
                for pat, what in (
                    (r"\b\d+\s?MB\b", "a hard size limit"),
                    (r"\bgofmt\b|\bgo vet\b|\bgo build\b|\bgo test\b", "Go toolchain commands"),
                    (r"\bcargo\b", "cargo"),
                    (r"\bnpm (test|run|install)\b", "npm commands"),
                ):
                    if re.search(pat, line):
                        findings.append(Finding(
                            "AGNOS-05", "warn", rel, (idx,),
                            f"prompt hard-codes project-specific gate semantics ({what})",
                            f"Line: {line.strip()[:120]}",
                            "The gate is defined by the host project's `just qualitygate`. "
                            "Prompts should treat it as opaque ('the gate') instead of naming "
                            "its checks.",
                        ))
                        break
    return findings


def check_script_attrs(
    wf_root: Path, rel_dir: str, nodes: dict[str, Node], fp: Fingerprint,
    allowed_tools: set[str], platform_tools: set[str],
) -> list[Finding]:
    """AGNOS-06/07: interpreter binaries in script= attrs + path stability."""
    findings: list[Finding] = []
    allowed_bins = set(allowed_tools)
    for t in allowed_tools | platform_tools:
        allowed_bins.update(TOOL_BINARIES.get(t, (t,)))
    forbidden_tools = {}
    for t in fp.mise_tools:
        if t in allowed_tools or t in platform_tools:
            continue
        for b in TOOL_BINARIES.get(t, (t,)):
            forbidden_tools[b] = t

    script_uses: dict[str, list[int]] = {}
    for name, node in sorted(nodes.items()):
        script = node.attrs.get("script")
        if not script:
            continue
        head = script.strip().splitlines()[0].strip()
        first = re.split(r"\s+", head)[0]
        script_uses.setdefault(first, []).append(node.line)
        script_line = node.attr_lines.get("script", node.line)

        # AGNOS-06: first token must be an allowed binary
        if first in forbidden_tools and first not in allowed_bins:
            findings.append(Finding(
                "AGNOS-06", "error", f"{rel_dir}/workflow.fabro", (script_line,),
                f"script= invokes '{first}' (project tooling via .mise.toml '{forbidden_tools[first]}')",
                f"Node '{name}': script=\"{script[:80]}...\"",
                f"Use an allowed command (`just …`, `sd …`, `ml …`) or a POSIX shell script; "
                f"'{first}' pins the workflow to this project's toolchain.",
            ))

        # AGNOS-07 / dangling path: referenced script file must exist
        m = re.match(r'^([A-Za-z0-9_./-]+\.(?:nu|sh|py|ts|js))', head)
        if m:
            cand_repo = fp.root / m.group(1)
            cand_wf = wf_root / m.group(1)
            if not cand_repo.exists() and not cand_wf.exists():
                findings.append(Finding(
                    "GRAPH-06", "error", f"{rel_dir}/workflow.fabro", (script_line,),
                    f"script= references missing file '{m.group(1)}'",
                    suggestion="Ship the script inside the workflow directory.",
                ))
            elif cand_repo.exists() and not m.group(1).lstrip("./").startswith(rel_dir):
                findings.append(Finding(
                    "AGNOS-07", "info", f"{rel_dir}/workflow.fabro", (script_line,),
                    f"script= uses repo-root-relative path '{m.group(1)}'",
                    "Works, but couples the workflow to the repo layout and duplicates the "
                    "path for every node. Prefer a path relative to the workflow directory "
                    "(like @prompts/… file refs, docs/fabro/prompts.md L39-49).",
                ))
    return findings


# --------------------------------------------------------------------------
# graph rules (checked against docs/fabro/)
# --------------------------------------------------------------------------

def prompt_contract_labels(prompt_text: str) -> list[tuple[str, int]]:
    """preferred_next_label values declared in a prompt's JSON contracts."""
    out = []
    for i, line in enumerate(prompt_text.splitlines(), 1):
        for m in re.finditer(r'"preferred_next_label"\s*:\s*"([^"]+)"', line):
            out.append((m.group(1), i))
    return out


def context_update_keys(prompt_text: str) -> list[str]:
    keys: list[str] = []
    for block in re.finditer(r'"context_updates"\s*:\s*\{(.*?)\}', prompt_text, re.S):
        for m in re.finditer(r'"([^"]+)"\s*:', block.group(1)):
            keys.append(m.group(1))
    return keys


KNOWN_CONTEXT_KEYS_PRODUCED = {"command.output"}  # engine-managed (docs/fabro/context.md)


def check_graph(
    wf_root: Path, rel_dir: str, nodes: dict[str, Node], edges: list[Edge],
    graph_attrs: dict[str, str], fp: Fingerprint,
) -> tuple[list[Finding], dict[str, str]]:
    findings: list[Finding] = []
    graph_rel = f"{rel_dir}/workflow.fabro"
    prompts: dict[str, str] = {}
    prompt_lines: dict[str, list[str]] = {}

    outgoing: dict[str, list[Edge]] = {}
    for e in edges:
        outgoing.setdefault(e.src, []).append(e)

    # GRAPH-01: known shapes
    for name, node in sorted(nodes.items()):
        shape = node.attrs.get("shape")
        if shape and shape not in KNOWN_SHAPES:
            findings.append(Finding(
                "GRAPH-01", "warn", graph_rel, (node.line,),
                f"node '{name}' uses unusual shape '{shape}'",
                suggestion="Stick to documented handler shapes (box/tab/parallelogram/hexagon, "
                           "docs/fabro/outcomes.md L28-35).",
            ))

    # GRAPH-02: outcome values in conditions (docs/fabro/outcomes.md L13-18, L121-134)
    for e in edges:
        cond = e.attrs.get("condition", "")
        for m in re.finditer(r"outcome\s*=\s*([\w.]+)", cond):
            if m.group(1) not in VALID_OUTCOMES:
                findings.append(Finding(
                    "GRAPH-02", "error", graph_rel, (e.line,),
                    f"edge condition uses unknown outcome value '{m.group(1)}'",
                    f"Edge {e.src} -> {e.dst}: condition=\"{cond}\"",
                    f"Valid outcomes: {sorted(VALID_OUTCOMES)}.",
                ))

    # load prompt files (@refs) — GRAPH-06
    for name, node in sorted(nodes.items()):
        prompt = node.attrs.get("prompt", "")
        if prompt.startswith("@"):
            relp = prompt[1:]
            cand = wf_root / relp
            if not cand.exists():
                findings.append(Finding(
                    "GRAPH-06", "error", graph_rel, (node.attr_lines.get("prompt", node.line),),
                    f"node '{name}' references missing prompt file '{relp}'",
                    suggestion="@prompt paths resolve relative to the graph file "
                               "(docs/fabro/prompts.md L39-49).",
                ))
            else:
                text = _read(cand)
                prompts[name] = text
                prompt_lines[name] = text.splitlines()
                node.attrs["_prompt_rel"] = str(cand.relative_to(fp.root))

    # GRAPH-03: label routing contract between prompt JSON and edge labels
    for name, node in sorted(nodes.items()):
        if name not in prompts:
            continue
        if node.attrs.get("output_schema") != "routing":
            continue
        declared = prompt_contract_labels(prompts[name])
        edge_labels = [e.label for e in outgoing.get(name, [])]
        for label, pline in declared:
            if label not in edge_labels:
                findings.append(Finding(
                    "GRAPH-03", "error", graph_rel, (node.line,),
                    f"prompt of '{name}' declares preferred_next_label \"{label}\" with no matching edge label",
                    f"Declared at {node.attrs.get('_prompt_rel', '?')}:{pline}; "
                    f"outgoing edge labels: {edge_labels or 'none'}.",
                    "preferred_next_label must match an outgoing edge label exactly, or "
                    "routing falls through to the unconditional edge.",
                ))
        conditional = [e for e in outgoing.get(name, []) if "condition" in e.attrs]
        unconditional = [e for e in outgoing.get(name, []) if "condition" not in e.attrs]
        for e in conditional:
            if ("outcome=failed" in e.attrs.get("condition", "")
                    or "outcome=" not in e.attrs.get("condition", "")):
                continue  # engine-failure routes are not model-routed
            if e.label and e.label not in {l for l, _ in declared}:
                findings.append(Finding(
                    "GRAPH-03", "warn", graph_rel, (e.line,),
                    f"edge '{e.label}' ({e.src} -> {e.dst}) is condition-routed but never declared as preferred_next_label",
                    "Acceptable for engine conditions, but the prompt should still name the "
                    "label so model-driven routing and condition routing agree.",
                ))
        if name not in COMMAND_SHAPES and node.attrs.get("shape") not in COMMAND_SHAPES:
            if not unconditional:
                findings.append(Finding(
                    "GRAPH-03", "warn", graph_rel, (node.line,),
                    f"node '{name}' has no unconditional fallback edge",
                    "If no condition matches and no label matches, the run terminates "
                    "(docs/fabro/failures.md 'When failures become fatal').",
                ))

    # GRAPH-04: failure-swallowing edges into exit
    for e in edges:
        if e.dst == "exit" and "outcome=failed" in e.attrs.get("condition", ""):
            if str(nodes.get(e.src).attrs.get("goal_gate", "")).lower() == "true" if nodes.get(e.src) else False:
                findings.append(Finding(
                    "GRAPH-04", "pass", graph_rel, (e.line, e.line_end),
                    f"failure exit edge of '{e.src}' is guarded by goal_gate=true",
                    "A genuine failure reaching exit fails the goal gate and thus the run "
                    "(docs/fabro/outcomes.md L108-119) — correct pattern.",
                ))
            else:
                findings.append(Finding(
                    "GRAPH-04", "info", graph_rel, (e.line, e.line_end),
                    f"genuine failures of '{e.src}' exit the run as success (fail-open)",
                    f"Edge '{e.label}' ({e.src} -> exit) catches outcome=failed. After "
                    f"output_retries are exhausted, malformed JSON or provider errors also end as "
                    f"failed (docs/fabro/failures.md retry layers) — the run then reaches exit and "
                    f"is reported successful. goal_gate=true is NOT an option for nodes that a "
                    f"normal run may never visit: unvisited goal-gate nodes fail the whole run "
                    f"at exit (engine semantics).",
                    "Acceptable when the tracker keeps the truth (failed seed stays open, the "
                    "next run re-enters the cycle) — document that trade-off at the edge. "
                    "Alternatively route genuine failures to an always-visited node that can "
                    "distinguish them via context flags.",
                ))

    # GRAPH-09: agent nodes whose unconditional verdict edge also catches genuine failures
    for name, node in sorted(nodes.items()):
        if name not in prompts or node.attrs.get("output_schema") != "routing":
            continue
        declared = dict(prompt_contract_labels(prompts[name]))
        has_failed_edge = any(
            "outcome=failed" in e.attrs.get("condition", "") for e in outgoing.get(name, []))
        uncond = [e for e in outgoing.get(name, []) if "condition" not in e.attrs]
        if not has_failed_edge and uncond:
            verdict = next((e for e in uncond if e.label in declared), None)
            if verdict:
                findings.append(Finding(
                    "GRAPH-09", "info", graph_rel, (verdict.line, verdict.line_end),
                    f"genuine failures of '{name}' route through the unconditional verdict edge '{verdict.label}'",
                    "The prompt maps every verdict to outcome=succeeded and this node has no "
                    "outcome=failed edge. A crashed node (schema exhaustion after "
                    "output_retries, provider death) therefore looks like the verdict "
                    f"'{verdict.label}' to downstream nodes — with its context keys (e.g. "
                    "review_feedback) missing.",
                    "Acceptable if bounded by max_visits; otherwise add an explicit failure "
                    "route (condition=\"outcome=failed\") so errors fail loudly instead of "
                    "looping as verdicts.",
                ))

    # GRAPH-03 pass note: every declared label matched an edge
    for name, node in sorted(nodes.items()):
        if name not in prompts or node.attrs.get("output_schema") != "routing":
            continue
        declared = prompt_contract_labels(prompts[name])
        edge_labels = [e.label for e in outgoing.get(name, [])]
        if declared and all(l in edge_labels for l, _ in declared):
            findings.append(Finding(
                "GRAPH-03", "pass", graph_rel, (node.line,),
                f"routing contract of '{name}' is consistent ({len(declared)} labels match edges)",
            ))

    # GRAPH-05: safeguards
    if "max_node_visits" not in graph_attrs:
        findings.append(Finding(
            "GRAPH-05", "warn", graph_rel, (),
            "graph sets no max_node_visits — cycles are unbounded",
            suggestion='Add graph [max_node_visits=…] (docs/fabro/failures.md "Node visit limits").',
        ))
    for name, node in sorted(nodes.items()):
        shape = node.attrs.get("shape")
        if shape in COMMAND_SHAPES:
            if "timeout" not in node.attrs:
                findings.append(Finding(
                    "GRAPH-05", "warn", graph_rel, (node.line,),
                    f"command node '{name}' has no timeout",
                    suggestion="Set timeout=… so a hanging gate cannot stall the run "
                               "(stall watchdog defaults to 30 min).",
                ))
            if "max_retries" not in node.attrs and "retry_policy" not in node.attrs:
                findings.append(Finding(
                    "GRAPH-05", "info", graph_rel, (node.line,),
                    f"command node '{name}' relies on default retries (3)",
                    "Deterministic gates usually want max_retries=0 so a red gate routes "
                    "immediately instead of re-running.",
                ))
        if node.attrs.get("prompt") and "output_retries" not in node.attrs:
            findings.append(Finding(
                "GRAPH-05", "info", graph_rel, (node.line,),
                f"prompt node '{name}' sets no output_retries",
                suggestion="output_retries=2 gives the model a repair round for malformed JSON.",
            ))
    if not any(str(n.attrs.get("goal_gate", "")).lower() == "true" for n in nodes.values()):
        findings.append(Finding(
            "GRAPH-05", "info", graph_rel, (),
            "no goal_gate node — exit does not verify that the gate/review actually passed",
            "With routing-only safety, a broad failure edge can reach exit 'successfully'. "
            "goal_gate=true on the tester/reviewer makes a failed pass fail the run at exit "
            "(docs/fabro/outcomes.md L108-119).",
        ))

    # GRAPH-08: reachability
    reach: set[str] = set()
    stack = ["start"]
    while stack:
        cur = stack.pop()
        if cur in reach:
            continue
        reach.add(cur)
        for e in outgoing.get(cur, []):
            stack.append(e.dst)
    for name in nodes:
        if name not in reach and name not in ("start", "exit"):
            findings.append(Finding(
                "GRAPH-08", "error", graph_rel, (nodes[name].line,),
                f"node '{name}' is unreachable from start",
            ))
    if "exit" not in reach:
        findings.append(Finding(
            "GRAPH-08", "error", graph_rel, (),
            "exit node is unreachable — the run can never complete",
        ))

    # GRAPH-07: template variables (docs/fabro/prompts.md L51-70)
    for name, text in prompts.items():
        rel = nodes[name].attrs.get("_prompt_rel", f"{rel_dir}/<prompt>")
        for i, line in enumerate(prompt_lines[name], 1):
            for m in re.finditer(r"{{\s*(\w+)(?:\.(\w+))?\s*}}", line):
                var, sub = m.group(1), m.group(2)
                if var == "goal" and "goal" not in graph_attrs:
                    findings.append(Finding(
                        "GRAPH-07", "warn", rel, (i,),
                        "prompt uses {{ goal }} but the graph sets no goal attribute",
                    ))
                if var == "inputs":
                    findings.append(Finding(
                        "GRAPH-07", "info", rel, (i,),
                        f"prompt uses {{{{ inputs.{sub} }}}} — verify [run.inputs] defines it",
                        "Undefined template variables render empty and fail run-style commands "
                        "(docs/fabro/prompts.md L70).",
                    ))

    # context-key flow (PROMPT-02): consumer keys must be produced upstream
    produced: dict[str, str] = dict.fromkeys(KNOWN_CONTEXT_KEYS_PRODUCED, "<engine>")
    order: list[str] = []
    seen: set[str] = set()
    stack = [("start", 0)]
    # simple BFS layering for "upstream" approximation
    layer: dict[str, int] = {"start": 0}
    queue = ["start"]
    while queue:
        cur = queue.pop(0)
        for e in outgoing.get(cur, []):
            if e.dst not in layer:
                layer[e.dst] = layer[cur] + 1
                queue.append(e.dst)
    producer_layer: dict[str, int] = {k: -1 for k in produced}
    for name, text in prompts.items():
        for key in set(context_update_keys(text)):
            produced.setdefault(key, name)
            producer_layer[key] = min(producer_layer.get(key, 99), layer.get(name, 99))
    consumers = {
        "review_verdict": re.compile(r"\breview_verdict\b"),
        "review_feedback": re.compile(r"\breview_feedback\b"),
        "current_seed_id": re.compile(r"\bcurrent_seed_id\b"),
        "current_seed_title": re.compile(r"\bcurrent_seed_title\b"),
        "current_seed_brief": re.compile(r"\bcurrent_seed_brief\b"),
        "implementation_summary": re.compile(r"\bimplementation_summary\b"),
    }
    for name, text in prompts.items():
        rel = nodes[name].attrs.get("_prompt_rel", f"{rel_dir}/<prompt>")
        for i, line in enumerate(prompt_lines[name], 1):
            for key, rx in consumers.items():
                if key in context_update_keys(text) and name == produced.get(key):
                    continue  # this node produces it
                if rx.search(line) and i > 30:  # skip own contract blocks near examples
                    if key not in produced:
                        findings.append(Finding(
                            "PROMPT-02", "error", rel, (i,),
                            f"prompt of '{name}' reads context key '{key}' that no node ever writes",
                            suggestion="Add the key to a producer's context_updates contract.",
                        ))
                    elif layer.get(name, 0) < producer_layer.get(key, 99):
                        findings.append(Finding(
                            "PROMPT-02", "warn", rel, (i,),
                            f"prompt of '{name}' reads '{key}' before its producer '{produced[key]}' can run",
                        ))
    return findings, prompts


# --------------------------------------------------------------------------
# script + prompt heuristics
# --------------------------------------------------------------------------

def check_scripts(wf_root: Path, rel_dir: str, files: list[Path], fp: Fingerprint,
                  allowed_tools: set[str]) -> list[Finding]:
    findings: list[Finding] = []
    evidence_script: Path | None = None
    scoped_lines: list[tuple[str, int]] = []
    for f in files:
        rel = str(f.relative_to(fp.root))
        lines = _read(f).splitlines()
        for i, line in enumerate(lines, 1):
            # SCRIPT-01: checkpoint base detection
            if re.search(r'--grep=.*Fabro-(Completed|Run)', line):
                findings.append(Finding(
                    "SCRIPT-01", "warn", rel, (i,),
                    "checkpoint base detection greps the whole history — merged prior runs poison the base",
                    "`git log --grep=Fabro-…` lists checkpoint commits of ALL runs reachable "
                    "from HEAD. When an earlier run branch was merged into the base branch, "
                    "the oldest match predates this run and the evidence diff includes "
                    "unrelated commits (docs/fabro/checkpoints.md L38-52).",
                    "Scope the grep to this run's id, e.g. derive it from the run branch "
                    "name (`fabro/run/<id>`) and grep the commit subject `fabro(<id>):`.",
                ))
            if "fabro/run/" in line:
                scoped_lines.append((rel, i))
            # SCRIPT-05: per-file diff filters that silently drop deletions
            m5 = re.search(r"--diff-filter=([A-Za-z]+)", line)
            if m5 and "D" not in m5.group(1) and "diff" in line:
                findings.append(Finding(
                    "SCRIPT-05", "warn", rel, (i,),
                    f"diff-filter={m5.group(1)} excludes deletions from the per-file diffs",
                    "Deleted files appear only in the --stat overview, never in the per-file "
                    "diff body the reviewer reads. A seed whose acceptance criterion is "
                    "'file X is gone' cannot be verified from the evidence.",
                    "Use --diff-filter=ACMRTD (or drop the filter) so deletions are shown.",
                ))
            # SCRIPT-01b: empty-evidence fallback (skip when a marker exists)
            file_text = "\n".join(lines)
            has_marker = re.search(r"NO RUN BASE|unreliable", file_text, re.I)
            if not has_marker and re.search(r"is-empty", line) and "HEAD" in "\n".join(lines[max(0, i - 4):i + 5]):
                findings.append(Finding(
                    "SCRIPT-01", "info", rel, (i,),
                    "fallback base=HEAD yields an empty diff when no run checkpoints exist",
                    "In-place runs without a fabro/run/* branch (checkpointing disabled, "
                    "dry-run) produce base=HEAD and an empty evidence diff. The reviewer "
                    "prompt says to distrust unverifiable claims, but the capture looks "
                    "normal — easy to approve by accident.",
                    "Print an explicit marker (e.g. 'NO RUN BASE — evidence unreliable') "
                    "when the base falls back, so the reviewer can react.",
                ))
            # SCRIPT-02: bounded diff — pass when disclosed to the reviewer
            file_l = "\n".join(lines).lower()
            disclosed = re.search(r"truncat|budget|omitted|unseen|not shown", file_l)
            if (re.search(r"100000|100_000|100k", line, re.I)
                    and not disclosed
                    and "truncat" in "\n".join(lines[max(0, i - 2):i + 2]).lower()):
                findings.append(Finding(
                    "SCRIPT-02", "info", rel, (i,),
                    "evidence diff is truncated at 100k chars without telling the reviewer",
                    "Beyond the cut the reviewer is blind; large diffs silently lose their tail.",
                    "Emit diff --stat plus per-file diffs for changed files, or state the "
                    "truncation in the prompt so the reviewer treats it as partial evidence.",
                ))
            if (re.search(r"100000|100_000|100k", line, re.I) and disclosed
                    and not any(f.rule == "SCRIPT-02" for f in findings)):
                findings.append(Finding(
                    "SCRIPT-02", "pass", rel, (i,),
                    "evidence diff budget is disclosed to the reviewer (truncation is explicit)",
                ))
            # SCRIPT-04: just recipes referenced by workflow assets must
            # exist. Only real invocations count (line-initial `just x` or a
            # script="just x" attribute); mid-sentence English prose such as
            # "just the brief" is not a recipe call.
            stripped = line.lstrip()
            is_call = stripped.startswith("just ") or 'script="just ' in line
            if is_call:
                for m in re.finditer(r"\bjust\s+([A-Za-z0-9_-]+)", line):
                    recipe = m.group(1)
                    if recipe in ("--list", "--summary", "--choose", "-l"):
                        continue
                if fp.just_recipes and recipe not in fp.just_recipes:
                    findings.append(Finding(
                        "SCRIPT-04", "error", rel, (i,),
                        f"workflow references `just {recipe}` but the justfile defines no such recipe",
                        "The gate bridge fails with exit != 0 in every host project that does "
                        "not define this recipe.",
                        "Document the required recipes (contract) in the workflow README and "
                        "fail with a clear message when missing (e.g. `just --summary | grep …`).",
                    ))
            if f.name.endswith("evidence.nu"):
                evidence_script = f
    used_recipes: set[str] = set()
    for f in files:
        for m in re.finditer(r"\bjust\s+([A-Za-z0-9_-]+)", _read(f)):
            if m.group(1) not in ("--list", "--summary", "--choose", "-l"):
                used_recipes.add(m.group(1))
    if used_recipes and fp.just_recipes and used_recipes <= set(fp.just_recipes):
        findings.append(Finding(
            "SCRIPT-04", "pass", "justfile",
            (min(fp.just_recipes[r] for r in used_recipes if r in fp.just_recipes),),
            f"all just recipes referenced by the workflow exist ({', '.join(sorted(used_recipes))})",
        ))
    if scoped_lines and not any(f.rule == "SCRIPT-01" and f.severity == "warn" for f in findings):
        rel0, i0 = scoped_lines[0]
        findings.append(Finding(
            "SCRIPT-01", "pass", rel0, (i0,),
            "checkpoint base detection is scoped to this run (run branch name)",
        ))
    return findings


def check_gate_bridge(wf_root: Path, rel_dir: str, nodes: dict[str, Node], edges: list[Edge],
                      fp: Fingerprint, allowed_tools: set[str]) -> list[Finding]:
    """SCRIPT-03: the gate bridge assumes the project gate covers untracked files."""
    findings: list[Finding] = []
    gate_recipes: set[str] = set()
    for name, node in nodes.items():
        script = node.attrs.get("script", "")
        for m in re.finditer(r"\bjust\s+([A-Za-z0-9_-]+)", script):
            gate_recipes.add(m.group(1))
        head = script.strip().splitlines()[0].strip() if script.strip() else ""
        m = re.search(r"([\w./-]+\.(?:nu|sh|py|ts|js))\b", head)
        if m:
            for cand in (fp.root / m.group(1), wf_root / m.group(1)):
                if cand.exists():
                    for mm in re.finditer(r"\bjust\s+([A-Za-z0-9_-]+)", _read(cand)):
                        gate_recipes.add(mm.group(1))
                    break
    if not gate_recipes:
        return findings
    # find recipe bodies in justfile (naive: recipe line + following indented block)
    justfile = None
    for cand in ("justfile", "Justfile"):
        if (fp.root / cand).exists():
            justfile = fp.root / cand
            break
    if not justfile:
        return findings
    lines = _read(justfile).splitlines()
    for recipe in sorted(gate_recipes):
        start = fp.just_recipes.get(recipe)
        if start is None:
            continue
        body: list[str] = []
        for line in lines[start:start + 15]:
            body.append(line)
        body_text = "\n".join(body)
        target = None
        m = re.search(r"(\S+\.(?:nu|sh|py))\b", body_text)
        if m and (fp.root / "scripts" / m.group(1)).exists():
            target = fp.root / "scripts" / m.group(1)
        if target:
            gate_text = _read(target)
            rel = str(target.relative_to(fp.root))
            has_tracked = "ls-files" in gate_text
            has_worktree = re.search(r"git status|untracked", gate_text)
            gate_line = next((i for i, l in enumerate(gate_text.splitlines(), 1)
                              if "ls-files" in l), 1)
            if has_tracked and not has_worktree:
                findings.append(Finding(
                    "SCRIPT-03", "warn", rel, (gate_line,),
                    "project gate checks only tracked files — untracked artifacts pass green",
                    "Fabro stages ALL worktree changes into the checkpoint commit after the "
                    "node completes (docs/fabro/checkpoints.md L46-50). An untracked binary "
                    "left by the implementer passes `git ls-files`-based checks, then rides "
                    "into the run branch via the next checkpoint.",
                    "Extend the gate to the working tree (e.g. also scan `git status --short` "
                    "entries and untracked files > 1 MB), or let the reviewer prompt treat any "
                    "untracked binary in `git status` as a deviation.",
                ))
    return findings


def check_prompts(wf_root: Path, rel_dir: str, nodes: dict[str, Node],
                  prompts: dict[str, str], fp: Fingerprint,
                  scripts_text: dict[str, str]) -> list[Finding]:
    findings: list[Finding] = []
    for name, text in prompts.items():
        rel = nodes[name].attrs.get("_prompt_rel", f"{rel_dir}/<prompt>")
        lines = text.splitlines()
        uses_goal = any("{{ goal }}" in l for l in lines)
        has_guard = any("user-provided data" in l for l in lines)
        if uses_goal and not has_guard:
            findings.append(Finding(
                "PROMPT-01", "warn", rel, (),
                f"prompt of '{name}' expands {{{{ goal }}}} without an injection guard",
                suggestion="State that the goal block is user-provided data, not instructions.",
            ))
        # PROMPT-03: planner closing seeds without review
        for i, line in enumerate(lines, 1):
            if re.search(r"close that seed|close the seed.*instead", line, re.I) and \
               re.search(r"sanity-check|already visibly satisfied|stale", "\n".join(lines[max(0, i - 6):i]), re.I):
                findings.append(Finding(
                    "PROMPT-03", "warn", rel, (i,),
                    "Planner may close seeds on visual inspection — bypasses gate and reviewer",
                    "The stale-tracker shortcut closes a seed because the work 'looks "
                    "complete', without the quality gate or the read-only reviewer seeing it. "
                    "Half-finished work can be closed silently.",
                    "Route suspected-stale seeds through the normal loop: re-claim, let the "
                    "implementer verify, let the reviewer approve — or require the evidence "
                    "capture (diff since base) as proof before closing.",
                ))
                break
        # PROMPT-04: reviewer ground truth
        if re.search(r"read-only|without tools|no tools", text, re.I) and \
           re.search(r"verify everything against it|nothing else", text, re.I):
            evidence_txt = "\n".join(scripts_text.values())
            # satisfied when the capture emits the seed description at all:
            # `sd show <id>` or `sd list --format json` + `.description` use
            spec_emitted = (
                "sd show" in evidence_txt
                or ".description" in evidence_txt
                or "spec" in evidence_txt.lower() and "authoritative" in evidence_txt
            )
            if evidence_txt and not spec_emitted and "sd list" in evidence_txt:
                anchor = next(
                    (i for i, l in enumerate(lines, 1)
                     if re.search(r"ground truth|current_seed_id", l)),
                    next((i for i, l in enumerate(lines, 1) if "seed" in l.lower()), 1))
                findings.append(Finding(
                    "PROMPT-04", "warn", rel, (anchor,),
                    "reviewer judges against the Planner's brief, not the seed itself",
                    "The reviewer is tool-less by design (good), but the evidence capture only "
                    "runs `sd list --format json` — the full seed description (`sd show`) is "
                    "never captured. If the Planner's brief drifts from the seed spec, the "
                    "reviewer cannot notice.",
                    "Extend the evidence script to also emit `sd show <current_seed_id>` (or "
                    "all in_progress seeds) so the reviewer sees the authoritative spec.",
                ))
        if uses_goal and has_guard:
            findings.append(Finding(
                "PROMPT-01", "pass", rel, (),
                f"prompt of '{name}' guards the {{{{ goal }}}} block against injection",
            ))
    return findings


# --------------------------------------------------------------------------
# repo wiring
# --------------------------------------------------------------------------

def check_repo(fp: Fingerprint, allowed_tools: set[str]) -> list[Finding]:
    findings: list[Finding] = []
    ptoml = fp.root / ".fabro/project.toml"
    if ptoml.exists():
        text = _read(ptoml)
        for i, line in enumerate(text.splitlines(), 1):
            m = re.search(r'"(?:just\s+)?(\w[\w-]*)(?:\s.*)?"\s*$', line)
            if "script" in line:
                mm = re.search(r'script\s*=\s*"(.+)"', line)
                if mm:
                    cmd = mm.group(1).strip()
                    first = cmd.split()[0]
                    if first == "just":
                        recipe = cmd.split()[1] if len(cmd.split()) > 1 else ""
                        if recipe and fp.just_recipes and recipe not in fp.just_recipes:
                            findings.append(Finding(
                                "REPO-01", "error", ".fabro/project.toml", (i,),
                                f"run.prepare references `just {recipe}` but the justfile has no such recipe",
                            ))
    # REPO-02: provisioning contract for allowed tools
    provisions = set()
    for f in ("scripts/bootstrap.nu", "scripts/bootstrap.sh"):
        p = fp.root / f
        if p.exists():
            provisions |= set(re.findall(r"\b(just|sd|ml|nu)\b", _read(p)))
    provisions |= set(fp.mise_tools)
    missing = [t for t in sorted(allowed_tools) if t not in provisions]
    for tool in missing:
        findings.append(Finding(
            "REPO-02", "warn", ".fabro/workflows", (),
            f"workflow tool contract '{tool}' is not provisioned anywhere in this repo",
            "The workflow is agnostic, but it assumes just/ml/sd exist in the sandbox. "
            "A host project copying the workflow must provision them itself.",
            "Document the tool contract (just + sd + ml must be on PATH) next to the "
            "workflow, e.g. a README in the workflow directory.",
        ))
    if not missing and allowed_tools:
        findings.append(Finding(
            "REPO-02", "pass", ".fabro/workflows", (),
            f"tool contract ({', '.join(sorted(allowed_tools))}) is provisioned (mise/bootstrap)",
        ))
    return findings


# --------------------------------------------------------------------------
# orchestration + rendering
# --------------------------------------------------------------------------

def _collect_workflow(
    root: Path, wf_name: str, fp: Fingerprint,
    allowed_tools: set[str], platform_tools: set[str],
) -> list[Finding]:
    wf_root = root / ".fabro/workflows" / wf_name
    rel_dir = f".fabro/workflows/{wf_name}"
    findings: list[Finding] = []

    if not wf_root.exists():
        return [Finding("REPO-00", "error", rel_dir, (), f"workflow directory does not exist")]

    fab = wf_root / "workflow.fabro"
    if not fab.exists():
        return [Finding("GRAPH-00", "error", f"{rel_dir}/workflow.fabro", (),
                        "workflow.fabro missing")]

    # workflow.toml wiring
    wtoml = wf_root / "workflow.toml"
    if wtoml.exists():
        t = _read(wtoml)
        m = re.search(r'graph\s*=\s*"([^"]+)"', t)
        if m and not (wf_root / m.group(1)).exists():
            findings.append(Finding(
                "GRAPH-06", "error", f"{rel_dir}/workflow.toml", (1,),
                f"workflow.toml points to missing graph file '{m.group(1)}'",
            ))
    else:
        findings.append(Finding(
            "GRAPH-06", "warn", f"{rel_dir}/workflow.toml", (),
            "no workflow.toml next to the graph",
        ))

    nodes, edges, graph_attrs = parse_dot(_read(fab))
    files = sorted(p for p in wf_root.rglob("*") if p.is_file() and p.suffix in (".md", ".nu", ".toml", ".sh", ".py"))
    files.append(fab)

    findings += check_agnosticity(wf_root, rel_dir, files, fp, allowed_tools, platform_tools)
    findings += check_script_attrs(wf_root, rel_dir, nodes, fp, allowed_tools, platform_tools)
    graph_findings, prompts = check_graph(wf_root, rel_dir, nodes, edges, graph_attrs, fp)
    findings += graph_findings
    findings += check_scripts(wf_root, rel_dir, files, fp, allowed_tools)
    findings += check_gate_bridge(wf_root, rel_dir, nodes, edges, fp, allowed_tools)

    scripts_text = {
        str(p.relative_to(root)): _read(p)
        for p in files if p.suffix in (".nu", ".sh", ".py")
    }
    findings += check_prompts(wf_root, rel_dir, nodes, prompts, fp, scripts_text)
    return findings


def _git_info(root: Path) -> tuple[str, str]:
    """Return (branch, short_sha) of the scanned repo, best effort."""
    def _git(*args: str) -> str:
        try:
            r = subprocess.run(
                ["git", "-C", str(root), *args],
                capture_output=True, text=True, timeout=10, check=False,
            )
            return r.stdout.strip() if r.returncode == 0 else ""
        except Exception:
            return ""
    branch = _git("rev-parse", "--abbrev-ref", "HEAD") or "detached"
    sha = _git("rev-parse", "--short", "HEAD") or "unknown"
    return branch, sha


def save_report(
    report: str,
    root: str | Path,
    workflow: str | None,
    report_dir: str | Path = "docs/reviews",
) -> Path:
    """Persist a markdown report under `<root>/<report_dir>/`.

    Filename: `<workflow|all>-review-<shortsha>.md`. A header comment records
    the reviewed commit so the report stays attributable when the branch moves.
    Returns the path of the written file.
    """
    root_path = Path(root).resolve()
    branch, sha = _git_info(root_path)
    from datetime import date
    today = date.today().isoformat()

    # Visible date line under the title, plus a machine-readable comment.
    lines = report.split("\n")
    meta = f"> Reviewed: **{today}** \u00b7 `{branch}@{sha}`"
    if lines and lines[0].startswith("# "):
        lines[2:2] = [meta, ""]
        report = "\n".join(lines)

    header = f"<!-- reviewed commit: {branch}@{sha} ({today}) -->\n\n"
    out_dir = root_path / report_dir
    out_dir.mkdir(parents=True, exist_ok=True)
    name = f"{workflow or 'all'}-review-{sha}.md"
    out_path = out_dir / name
    out_path.write_text(header + report + "\n", encoding="utf-8")
    return out_path


def run(
    root: str = ".",
    workflow: str | None = "develop",
    allowed_tools: tuple[str, ...] = ("just", "ml", "sd"),
    platform_tools: tuple[str, ...] = ("git", "fabro"),
    min_severity: str = "info",
    format: str = "markdown",
    report_dir: str | None = "docs/reviews",
) -> str:
    """Review Fabro workflow assets for weaknesses, gaps and agnosticity leaks.

    Parameters
    ----------
    root:
        Repository root to scan (default: current directory).
    workflow:
        Workflow name under `.fabro/workflows/` to review (default: "develop").
        Pass None to review every workflow in the repo.
    allowed_tools:
        Tooling the workflow is allowed to reference (default: just, ml, sd).
    platform_tools:
        Platform substrate that is always allowed (default: git, fabro —
        Fabro checkpoints are plain Git commits, docs/fabro/checkpoints.md).
    min_severity:
        One of "error", "warn", "info", "pass".
    format:
        "markdown" (report) or "json" (machine-readable findings).
    report_dir:
        Directory (relative to `root`) where the markdown report is saved as
        `<workflow>-review-<shortsha>.md`. Default: "docs/reviews".
        Pass None to skip saving (json format never saves).

    Returns
    -------
    str
        Markdown report or JSON array of findings. Every finding carries a
        rule id, severity, repo-relative path and line numbers. A saved
        markdown report ends with a "Saved to:" footer naming the file.
    """
    root_path = Path(root).resolve()
    fp = build_fingerprint(root_path)
    allowed, platform = set(allowed_tools), set(platform_tools)

    names = (
        [workflow] if workflow
        else sorted(p.name for p in (root_path / ".fabro/workflows").iterdir() if p.is_dir())
    )
    findings: list[Finding] = []
    for wf in names:
        findings += _collect_workflow(root_path, wf, fp, allowed, platform)
    findings += check_repo(fp, allowed)

    # dedupe identical findings
    seen: set[tuple] = set()
    unique: list[Finding] = []
    for f in findings:
        key = (f.rule, f.path, f.lines, f.title)
        if key not in seen:
            seen.add(key)
            unique.append(f)

    threshold = SEVERITY_ORDER[min_severity]
    if format == "markdown":
        # passes are the verification summary — always shown in reports
        unique = [f for f in unique if SEVERITY_ORDER[f.severity] <= max(threshold, 3)]
    else:
        unique = [f for f in unique if SEVERITY_ORDER[f.severity] <= threshold]
    unique.sort(key=lambda f: (SEVERITY_ORDER[f.severity], f.path, f.lines and f.lines[0] or 0))

    if format == "json":
        return json.dumps([asdict(f) | {"lines": list(f.lines)} for f in unique], indent=2)

    counts = {s: sum(1 for f in unique if f.severity == s) for s in ("error", "warn", "info", "pass")}
    out = [f"# reviewer-agent report — {root_path.name}", ""]
    scope = workflow or "all workflows"
    out.append(f"Scope: `{scope}` · allowed tools: `{', '.join(sorted(allowed))}` · "
               f"platform: `{', '.join(sorted(platform))}`")
    out.append("")
    out.append(f"**{counts['error']} errors · {counts['warn']} warnings · {counts['info']} info**")
    out.append("")
    if counts["error"] == 0 and counts["warn"] == 0:
        out.append("No errors or warnings found.")
    for f in unique:
        if f.severity != "pass":
            out.append(f.render_md())
    passes = [f for f in unique if f.severity == "pass"]
    if passes:
        out.append("## Verified (pass)")
        out.append("")
        for f in passes:
            extra = f" — {f.detail}" if f.detail else ""
            out.append(f"- `{f.ref()}` {f.title}{extra}")
    report = "\n".join(out)
    if format == "markdown" and report_dir:
        out_path = save_report(report, root_path, workflow, report_dir)
        report = f"{report}\n\nSaved to: `{out_path.relative_to(root_path)}`"
    return report


# --------------------------------------------------------------------------
# CLI
# --------------------------------------------------------------------------

def cli(argv: list[str] | None = None) -> int:
    """Console entry point: review a repo and save the report."""
    import argparse
    ap = argparse.ArgumentParser(
        prog="reviewer_agent",
        description="Static reviewer for Fabro workflow assets.",
    )
    ap.add_argument("--root", default=".", help="Repository root to scan")
    ap.add_argument("--workflow", default="develop",
                    help="Workflow name under .fabro/workflows/ (default: develop; 'all' for every workflow)")
    ap.add_argument("--allowed-tools", default="just,ml,sd",
                    help="Comma-separated tools the workflow may reference")
    ap.add_argument("--platform-tools", default="git,fabro",
                    help="Comma-separated always-allowed platform substrate")
    ap.add_argument("--min-severity", default="info",
                    choices=["error", "warn", "info", "pass"])
    ap.add_argument("--format", default="markdown", choices=["markdown", "json"])
    ap.add_argument("--report-dir", default="docs/reviews",
                    help="Directory (relative to root) for the saved report; 'none' disables saving")
    args = ap.parse_args(argv)

    workflow = None if args.workflow == "all" else args.workflow
    report_dir = None if args.report_dir == "none" else args.report_dir
    report = run(
        root=args.root,
        workflow=workflow,
        allowed_tools=tuple(t.strip() for t in args.allowed_tools.split(",") if t.strip()),
        platform_tools=tuple(t.strip() for t in args.platform_tools.split(",") if t.strip()),
        min_severity=args.min_severity,
        format=args.format,
        report_dir=report_dir,
    )
    print(report)
    return 0
