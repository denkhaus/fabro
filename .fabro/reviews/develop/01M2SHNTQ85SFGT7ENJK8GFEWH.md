# Improve review — run 01M2SHNTQ85SFGT7ENJK8GFEWH

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (23.3 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-18 06:47+0000 by revisor `fabro_ask`

---

All evidence below is from this run's events, stage journals, and the tracker at `.seeds/issues.jsonl`. Run shape for context: **23.1 min wall, $1.273 total, 0 retries, first-pass approval**. Stage split (from run events / conclusion): preflight 11.5s → planner 113s/$0.133 → **implementer 1,170s/$1.089 (84% of wall, 86% of cost; 72 tool calls)** → tester 28s → evidence 0.3s → reviewer 46s/$0.051 → closeout 0.4s. The loop's core design worked (crate-scoped verify pre-warmed the gate: 28s vs the ~15m cold worst case documented in `workflow.fabro`; the brief's labeled hypothesis was reused nearly verbatim). The recommendations target what actually hurt:

## 1. Reviewer evidence capture blob-ref'd at 16.9 KB — raise the per-node inline cap → **fabro-cf3e** (open; amended to ≥40 KB)
- **Node/file:** `reviewer` node in `.fabro/workflows/develop/workflow.fabro`, `preamble_inline_max_kb=16` → 32–40.
- **What happened:** evidence output measured 16,765 bytes — 381 bytes over the 16,384-byte cap — so the capture arrived as a blob ref (`/tmp/sandbox-driver/runtime/blobs/d09af0f7…`). The reviewer's own journal painpoint: "every review pays this paging tax… an unread blob ref is grounds for Verification blocked." Reviewer burned 1 of its 3 tool calls reading the blob (46s stage).
- **Expected effect:** capture renders inline; one fewer tool round per review (every run), and the unread-blob → Verification-blocked → re-capture bounce class (a full extra evidence+reviewer cycle, ~2–4 min) disappears. Composable with **fabro-c1bb** (drop `current_seed_brief` from reviewer `preamble_allow_keys` — in this run the reviewer preamble carried both the ~3 KB brief and the capture, which itself embeds the seed spec).

## 2. verify.nu missed inline `#[cfg(test)]` tests — implementer had to run the full suite manually → **fabro-2f70** (open)
- **File:** `scripts/verify.nu`, `is-test-file` (~line 24).
- **What happened:** implementer journal painpoint (verbatim): verify "classified test-file-touched as empty even though in-file #[cfg(test)] tests were edited in fabro-workflow — the full-suite rule silently fell back to a compile check; I ran `cargo nextest run -p fabro-workflow` manually (1540 pass)." This seed wrote +142 lines of table-driven tests inside `mod tests`, and the dispatcher didn't see them.
- **Expected effect:** the mechanical verify derives test-file-touched from `#[cfg(test)]` diff hunks, so the implementer-side test signal fires on the repo's dominant test convention instead of silently degrading — and the manual full-suite workaround (a policy violation the implementer had to justify) stops being necessary.

## 3. Planner drank the 200-row `sd ready` firehose the preflight had already summarized → **fabro-c3b4** (open)
- **File:** `.fabro/workflows/develop/prompts/planner.md` command table.
- **What happened:** the preflight verdict table (top-5, ranked, with verdicts) was already inline in `## Context`, yet the planner's first tracker call was `sd ready --assignee fabro --limit 200` → **29,222 bytes / 200 rows** (event seq 37–39) to confirm the #1 candidate; `fabro_runs_list` added another ~13 KB / 51 runs (seq 50). Planner stage: 113s, $0.133, 46.5k input tokens.
- **Expected effect:** top-N view (`--limit 10`) + `created_since` on `fabro_runs_list` cuts ~40 KB of context per planning pass and 1–2 early turns; the preflight table already answers "which seed is next."

## 4. Preflight found fabro-90ae duplicate but closed nothing — extend the mechanical close beyond the top candidate → **new seed**
- **File:** `.fabro/workflows/develop/scripts/planner-preflight.nu` (the close arm that fires only for the TOP candidate; `closed: {"seed": null}` despite a `duplicate` verdict with sha `4f8a254` for candidate #3, fabro-90ae).
- **What happened:** the planner journaled (observation #2): "fabro-90ae was preflight-classified duplicate (landed via #232) yet remains open — the preflight's mechanical superseded-close did not fire; a later pass or the user should close it." No later pass will: every run re-reports this, and the planner also burned a tool round reading the preflight script's 80-line header (seq 40–42) to learn what `duplicate` means — a one-line verdict legend in `output.preflight` would have answered that inline.
- **Change:** apply the existing closure-note + superseded-close to *any* candidate with an unambiguous `duplicate` verdict (not just top), and emit a verdict legend field in the report.
- **Expected effect:** stale duplicate rows converge mechanically instead of recurring in every run's journal; planner skips the script-source read.
- **New-seed justification:** searched the tracker — fabro-a81d (write-pair chaining), fabro-2ade (parse bug), fabro-2afd (degraded-mode mandate), fabro-22fa (closeout sweep) all touch adjacent mechanisms; none extends the preflight's mechanical close to non-top candidates or adds a verdict legend.

## 5. In-flight check ran against null PR states again → **fabro-4bb7** (open)
- **What happened:** `fabro_runs_list` (seq 50) returned `pull_request.state: null` for terminal runs #233, #227, #225; the planner's observation #3 names it: "null-state PRs remain the fabro-4bb7 projection gap," and its reasoning trace shows the wasted adjudication ("PR state null is odd… but seed 0a4c is about the engine fix, not landed").
- **Expected effect:** backfilled states make the in-flight exclusion decisive instead of a journaled degraded mode; less planner reasoning per pass around ambiguity that is mechanically fixable.

## 6. Prompt mandates `read_file` on an fs_hide-bound path — guaranteed denial on every Rust stage → **new seed**
- **File:** `.fabro/workflows/develop/prompts/implementer.md` and `reviewer.md`, "Rust work — read the vendored style guide FIRST (hard gate)" / "RUST STANDARDS AXIS" sections.
- **What happened:** the implementer's literal first action per the hard gate — `read_file .fabro/skills/rust-style-guide/SKILL.md` — was **denied**: "path … is hidden from this stage by fs_hide" (event seq 101, `error_kind: denied`), followed by the shell fallback (seq 108). The instruction names the wrong tool for a path the same node's `fs_hide=".fabro/**,…"` always hides; this fires twice per Rust run (implementer + reviewer).
- **Change:** one sentence in both prompts: read the guide via shell (`sed -n`/`cat`) — it lives under fs_hide; never `read_file` it.
- **Expected effect:** −2 wasted error/recovery rounds per Rust run; the "hard gate" stops teaching the model the wrong affordance.
- **New-seed justification:** fabro-e6c7 covers briefs *targeting* hidden paths (implementation targets) and fabro-81b7 covers *enforcing* the guide load; no existing seed fixes the standing style-guide instruction that itself names `read_file` on an fs_hide-bound path.

## 7. 20-minute implementer stage with zero interim signal → **fabro-b769** (open)
- **What happened:** the implementer ran 1,170s — 84% of the run — with notifications configured only for `run.completed`/`run.failed` (run settings). Nothing surfaces progress or a stall until terminal; the graph's own `stall_timeout` is 63m, so a wedged stage could sit silent for an hour.
- **Expected effect:** a long-stage heartbeat (e.g. Slack ping at a threshold) gives the user a mid-run signal exactly where 86% of cost concentrates — the difference between watching and discovering after the fact.

## 8. New routing heuristic shipped this run is a trap for future schema authors → **new seed** (lowest urgency, doc/lint only)
- **What happened:** the landed fabro-0a4c fix means any file schema whose payload contains `context_updates` (or any routing-named top-level field) is now routing-active — the implementer's `implementation_summary` flags this MATERIAL SEMANTIC RISK explicitly, and its journal observation #2 says it's "worth knowing when authoring future file schemas." Nothing in the workflow-asset docs records it.
- **Change:** one paragraph in the schema-authoring guidance (plus optionally a `prompt-lint.nu` warning when a file schema defines routing-named fields) stating the `contains_routing_field` contract.
- **Expected effect:** the next `@schemas/*.json` author doesn't silently opt into routing semantics.
- **New-seed justification:** the risk was discovered *during* this run (mx-e604ec) and exists in no seed — fabro-7028/fabro-e4c4 cover the output-key mismatch class, not the routing-field activation heuristic.

**Not recommended (checked, no evidence this run):** changes to the gate-bounce node, cycle guards, or reviewer fast-paths — none were exercised (gate green first try, single review cycle, 0 retries); their existing coverage in the graph comments already reflects prior runs, not this one.
