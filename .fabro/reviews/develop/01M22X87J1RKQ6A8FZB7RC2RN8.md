# Improve review — run 01M22X87J1RKQ6A8FZB7RC2RN8

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (15.6 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-09 11:21+0000 by revisor `fabro_ask`

---

All evidence gathered. Here are the recommendations, grounded in this run's events, timings, and journals, ordered by expected impact.

**Run economics (from run conclusion + stage events):** 15.4 min wall, $0.386 total. Implementer = 706 s wall (76% of run) and $0.266 (69% of cost). Everything below targets measured waste in those numbers.

---

**1. Warm the Rust build cache in the toolchain image — the implementer's cargo cold-start is half the run's wall.**
- **Change:** Adopt open seed fabro-fe15 (`.fabro/Dockerfile` / engine-managed cargo cache mount; needs the pending ADR-0019 user approval).
- **Evidence (run events):** implementer's `cargo nextest run -p fabro-workflow` ran 11:06:45→11:11:52 (~5.1 min cold compile + 1530 tests, seq 218), then `fmt && clippy` 11:11:52→11:14:26 (~2.5 min, seq 224). That's ~7.6 of the implementer's 11.8 minutes on a cold `target/`. The tester then ran the same crates in **45 s** (vs the graph's ~15 min cold worst-case note) purely because the implementer warmed the cache.
- **Expected effect:** ~6–8 min off every run's wall (~40–50%), and real headroom under the tester's 20 m timeout on gate-red bounces.

**2. Add an rg flag-discipline line to the implementer prompt — and correct the false "sandbox rg is unreliable" journal lore this run produced.**
- **Change:** One line in `.fabro/workflows/develop/prompts/implementer.md`: `rg -r <text>` REPLACES matches — never write `rg -rn`; `-n` alone is the line-number flag.
- **Evidence (run events seq 140, 158):** the implementer itself ran `rg -rn "is_engine_stamped_key"` and `rg -rn "pub fn is_success"`. rg parsed `-r n` as "replace matches with literal `n`", producing `pub fn n(key: &str...)` and `nful(self)` — the exact "mangling" it then journaled as *"rg in this sandbox mangled matching output… grep/rg output here cannot be trusted verbatim"* (implementer journal, `.fabro/journal/01M22X87J1RKQ6A8FZB7RC2RN8.jsonl`). That observation is a misdiagnosis now sitting in durable journal data; it cost 2 extra diagnostic rounds and, worse, could seed wrong platform lore via the improve loop.
- **Expected effect:** eliminates this error class and the diagnostic rounds; prevents a false platform claim from being acted on by future revision seeds.

**3. Expose the per-run seed id in `fabro_runs_list` output — the in-flight-PR guard burned 4 tool calls reverse-engineering it.**
- **Change:** Engine-side, extend the `fabro_runs_list` run projection to include the run's `current_seed_id` (the planner's guard at `planner.md` step 4 currently tries to "extract the seed id from each run's goal text" — impossible, since every develop run shares one generic goal). Interim one-liner in `planner.md` step 4: read it from `.fabro/journal/<run_id>.jsonl` directly.
- **Evidence (run events seq 44–59):** planner saw PR #78 open, couldn't map it to a seed, and spent `ls | grep` on the journal dir, a grep whose regex `fabro-[a-z0-9]{4}` matched junk (`fabro-jour` ×7), and a `head -c 3000` of another run's journal before concluding fabro-fc1b — ~3 extra LLM rounds inside its 80 s inference.
- **Expected effect:** −3 tool calls and −2 LLM rounds per planner pass whenever any PR is open; removes a fragile grep heuristic that could one day mis-skip a valid candidate (or miss a real double-pick).

**4. Raise the graph preamble budget 24→32 KB so the evidence capture stops getting blob-ref'd under the reviewer's own 16 KB ceiling.**
- **Change:** `preamble_budget_kb=24` in `.fabro/workflows/develop/workflow.fabro` (open seed fabro-8d2c), or an inline floor for contract-critical keys (fabro-b568).
- **Evidence (reviewer prompt, seq 291):** the evidence capture was **11.6 KB** — under the reviewer node's `preamble_inline_max_kb=16` — yet still demoted to a blob ref because the *aggregate* 24 KB budget was consumed by the tester section + brief + summary. The reviewer had to burn a tool round reading `/tmp/fabro/runtime/blobs/b0cc….json` (read_file invoked, 12 ms) — and the prompt carries ~2 paragraphs of blob-ref handling instructions for this recurring case.
- **Expected effect:** reviewer judges from inline context every run; zero tool detours; smaller reviewer prompt. Recurring, if small (~1 round + ~1.4 KB prompt per review).

**5. Make the "workaround = painpoint" rule verifiable — this run's reviewer performed a blob-read workaround and still reported `painpoints: []`.**
- **Change:** Engine-side auto-stamp: when a reviewer-stage tool call reads a blob path, append a painpoint stub to its journal (or lint the mismatch at stage completion — conveniently, the very `ContextKeyOmitted` lint machinery this run built). Alternatively, harden `reviewer.md`'s journal section with the example "blob ref you paged through → painpoint entry".
- **Evidence:** `reviewer.md` explicitly says "a workaround you performed is a painpoint, not an observation," yet reviewer@1's journal (run events seq 309) shows `painpoints: []` while its tool log shows the blob read. The friction-recollection channel — the input to the platform improve loop — silently dropped a real, recurring friction event.
- **Expected effect:** the improve loop actually sees recurring evidence-pipe friction instead of depending on model self-report that demonstrably failed here.

**6. Silence the by-design `output.gate_known_bug_hits` absence warning — it fires in every green run.**
- **Change:** Open seed fabro-8275: remove `output.gate_known_bug_hits` from the implementer node's `preamble_allow_keys` on non-bounce visits (or make the fidelity lint conditional on the gatebounce node having run).
- **Evidence (worker log, exactly one warn line):** `preamble_allow_keys entry absent from context node=implementer key=output.gate_known_bug_hits` — the key only exists on gate-red bounces by design, so a green first-visit run always warns.
- **Expected effect:** warn-level logs regain signal (one fewer guaranteed-false warning per run); real envelope drift becomes visible.

**7. Cut planner/implementer reconnaissance waste: top-N `sd ready` view + repo-layout map.**
- **Change:** (a) planner.md step 1: cap the listing (`… | head -40`) per open seed fabro-c3b4; (b) add the module map to `AGENTS.md`/planner prompt per open seed fabro-3d2d.
- **Evidence (run events):** planner's first `sd ready` returned 24,445 bytes of stdout, `stdout_truncated: true`, 181 seeds (seq 32) — it needed the top High band. Implementer guessed a wrong module path (`lib/components/fabro-workflow/src/context/keys.rs`, seq 129 — doesn't exist; the keys module lives in `lib/foundation/fabro-types/src/context_keys.rs`) and spent 2 extra probes finding `is_engine_stamped_key`.
- **Expected effect:** ~1–2 fewer tool rounds per stage per run and less context churn; small dollars, but it compounds across every run of this loop.

---

**What worked and needs no change (from run events):** the crate-scoped implementer verification → warm-gate handoff (tester 45 s), the deterministic closeout (0.3 s, `current_seed_id` via stdin), the gatebounce/deadlock guards stayed unvisited (green path), and the fresh `ContextKeyOmitted` lint shipped clean through gate + first-pass review. The levers with real headroom are the cold build (rec 1) and the small, recurring reconnaissance and evidence-pipe frictions (recs 2–7).
