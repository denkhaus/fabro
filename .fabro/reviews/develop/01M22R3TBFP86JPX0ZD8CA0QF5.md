# Improve review — run 01M22R3TBFP86JPX0ZD8CA0QF5

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (9.2 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-09 09:46+0000 by revisor `fabro_ask`

---

All facts below are from this run's event history (stage transcripts, tool calls, billing, timings), the run's single warn-level log line, and the journal entries the stages wrote. Run totals for scale: 9 m 15 s wall, $0.444 LLM cost — planner 94.5 s/$0.103, implementer 378.6 s/$0.295 (66 %), tester 5.1 s/$0, evidence 0.4 s, reviewer 45.2 s/$0.047, closeout 0.3 s.

## Recommendations, by expected impact

**1. Inline the evidence capture for the reviewer — raise `preamble_budget_kb` 24 → 32 in `.fabro/workflows/develop/workflow.fabro`** *(graph design; open seed fabro-8d2c, or better fabro-e4c4: deliver evidence via the node `output_schema`)*
What happened: the evidence capture was 15,263 bytes — *under* the reviewer node's `preamble_inline_max_kb=16` — yet it arrived demoted to "Output (15.4 KB; full value: …/blobs/….json) Preview:" (stage.prompt, seq 320), because the aggregate 24 KB graph budget was exceeded by prompt (~13.6 KB) + tester section + context table. The reviewer's very first tool call (seq 330) was reading that blob — 15.7 KB poured back into the context that was just demoted out of it — on a first LLM call that also paid full input price (12,302 tokens, 0 cache). The reviewer journaled this exact painpoint verbatim. This is the second consecutive budget raise that still didn't fix it (graph comment: window at 1.9 %, "the blob detour is the constraint").
Expected effect: removes one tool round-trip + one full-price LLM turn per review, and eliminates the preview-looks-truncated ambiguity the reviewer prompt currently spends two paragraphs defending against.

**2. Make the `fs_hide` denial message name the shell bypass** *(error handling; engine-side, open seed fabro-d02d — fallback: move the rule out of planner.md step-3 fine print into the sd command table)*
What happened: planner seq 49–50: `read_file` on `closeout.nu` → "path … is hidden … and behaves as if it did not exist"; seq 54: `shell cat` succeeds. One failed call + one LLM round (~5 s) burned discovering a denial the prompt already warns about — the instruction exists but is buried. Platform-file basis checks are this tracker's most common seed class, so this tax repeats.
Expected effect: the denial itself teaches the workaround; one fewer failed tool call and LLM round per platform-seed planning pass.

**3. Turn the nushell lessons this run paid for into a skill the implementer actually loads** *(tool usage / knowledge delivery; add `.fabro/skills/nushell-scripts/SKILL.md` — discovery already scans `/workspace/fabro/.fabro/skills` — plus one line in `.fabro/workflows/develop/prompts/implementer.md` step 2)*
What happened: the implementer (69 % of wall, 66 % of cost for a 102-line insert-only change) spent three probe turns reverse-engineering nu 0.115 semantics (`complete` is external-only — failed twice, seq 170/176; `source` const resolution; `def main` auto-invoke collision, seq 182; its two heaviest reasoning turns, 1,254 + 2,437 reasoning tokens ≈ $0.029) plus one parse failure from parenthesized prose in `$"…"` (seq 194 → fix seq 199). All of it is now recorded as mx-84a287 in `.mulch/expertise/workflows.jsonl` — but `agent.memory.loaded` shows only `AGENTS.md` is injected into sessions, so the *next* script seed will re-derive all of it.
Expected effect: next `.nu`-touching seed skips the probe/debug cycle — conservatively 1–2 min and $0.03–0.08 per run, and fewer failed writes.

**4. Briefs must prescribe outcomes, not unverified language mechanics** *(prompting; `.fabro/workflows/develop/prompts/planner.md`, step 7)*
What happened: the brief's criterion (3) literally prescribed "wrap in `do { ... } | complete`" — a construct that errors in nu 0.115 when the block returns a value. The implementer had to discover the prescription was wrong, deviate (per-git-call wrappers + outer `do -i`), and journal the deviation. The requirement ("must NEVER fail or block the close") was correct; the mechanism was not, and the planner had no way to verify it.
Change: add to step 7: "state acceptance criteria as outcomes; annotate any code-snippet prescription in the seed body as advisory unless you have verified it against the current tree."
Expected effect: prevents brief-induced debug cycles on script seeds and keeps the reviewer's PASS/FAIL lines outcome-aligned.

**5. Share a prompt-cache prefix across stage sessions** *(cost; engine, open seed fabro-944d)*
What happened: every stage session starts cold — `cache_read_tokens=0` on each first call (planner 12,271 in, seq 30; implementer 12,147, seq 114; reviewer 12,302, seq 329) and `cache_write_tokens=0` in *every* billing record in the run. The three cold opens alone billed ≈ $0.053 (~12 % of run cost) for content that is near-identical across sessions (system + tools + the 24 KB `AGENTS.md` memory block, ~5.7–6 k tokens of it, in all three).
Expected effect: a stable shared prefix (system/tools/AGENTS.md) makes stage openings cache hits — ~$0.05/run here, compounding on multi-cycle runs where every revisit re-pays it.

**6. Kill planner reconnaissance sprawl: top-N `sd ready` + a repo-layout map** *(tool usage; `.fabro/workflows/develop/prompts/planner.md` — open seeds fabro-c3b4 and fabro-3d2d)*
What happened: (a) `sd ready --assignee fabro --limit 200` returned **23,536 bytes / 175 seeds** (seq 32); the planner needed only the top of the priority list, yet the whole firehose rode in context for the session (conversation grew 3.5 k → 16.2 k tokens). (b) Three calls to locate the closeout node inside its *own* workflow (seq 71 grep `workflow.toml` → empty; seq 77 ls + greps for `.dot`/`.gv` → nothing; seq 83 finally greps `workflow.fabro`).
Change: pipe the ready listing through `head -40` (priority-ordered anyway), and add a 6-line layout map (graph = `workflow.fabro`, scripts = `…/scripts/`, prompts = `…/prompts/`) at the top of planner.md.
Expected effect: ~3 fewer tool calls (~15 s, ~$0.02) and ~20 KB less context per planning pass.

**7. Downgrade the by-design `preamble_allow_keys` WARN to info** *(UX/log hygiene; engine, open seed fabro-8275)*
What happened: the run's only warn-level log line is "preamble_allow_keys entry absent from context node=implementer key=output.gate_known_bug_hits" (09:33:03) — fired on a green first pass where that key *legitimately* doesn't exist (only gatebounce produces it on a red bounce).
Expected effect: one-line change; zero-noise logs so a genuine warning can't hide in per-run WARN spam.

**What already worked (no change):** the journal contract was honored by all three agent stages; the gate self-scoped to 5.1 s ("no crates touched"); closeout ran deterministically in 0.3 s; PR #80 was created cleanly at 09:40:36 (seq 373) — I found no PR-postlude failure in this run's events, so I make no recommendation there despite open seed fabro-6a5a.
