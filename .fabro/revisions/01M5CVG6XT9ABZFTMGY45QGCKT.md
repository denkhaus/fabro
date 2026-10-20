# Revision — run 01M5CVG6XT9ABZFTMGY45QGCKT

- status reviewed: succeeded (559s, $0.527, healthy green pass: fabro-7f58, PR unmerged, tracker OPEN — dup-run-check clean with `--self 01M5CVG6XT9ABZFTMGY45QGCKT`, closeout-race count 0 at file time)
- review: .fabro/reviews/develop/01M5CVG6XT9ABZFTMGY45QGCKT.md
- seeds filed: none — healthy run, zero balance credit this pass (no same-pass stale/superseded closes)
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M5CVG6XT9ABZFTMGY45QGCKT, workflow version absent (run journal not present in revisor checkout; `fabro ps` unavailable — same gap as pass 1), commit a0b531148a1f13db259cd0eb81882d5d2c6c9205
- revised_at_commit: a0b531148a1f13db259cd0eb81882d5d2c6c9205 (ADR-0015: engine drift signal for later judgement)

## Findings

**1. verify.nu silent false-green exit contract + test-fn-names blank-line drop (same file, consolidated)** — overflow
- overflow: fix `scripts/verify.nu` exit contract and test-fn-names attribute window — exit 1 on every FAIL arm (fmt/clippy/compile/nextest incl. `full-crate-suite`'s `return false` path at lines 219–283), drop the dead `$env.LAST_VERIFY_FAIL`, mirror the nonzero-exit contract in develop's `prompts/implementer.md` step 4; and in `test-fn-names`, clear `$attrs` only on non-attribute non-blank lines so `#[test]` + blank line + `fn` is not silently dropped from the name filter; effect: callers trusting exit status stop reading red as green, and the narrowed tier this run shipped cannot under-cover test names (edge found by the run's reviewer in code that landed minutes ago). Dedupe this pass: `seeds search verify` / `seeds search exit` — no seed touches verify.nu's exit contract or the filter window; closed fabro-7f58 is the narrowing itself.

**2. Move the seed-spec-paths check into the implementer's cheap lane** — overflow
- overflow: kill the tester spec-paths bounce — extend `.fabro/workflows/develop/scripts/claim-check.nu` (already parses the seed body's paths) to emit the spec-named path list into context, and add one line to `.fabro/workflows/loop/prompts/implementer.md` to diff that list against `git diff --name-only` before routing Implemented; effect: the bounce class that cost this run implementer@2 (98.5s, $0.115 = 22% of run cost) plus a second full tester run (14.9s) for one mirror line disappears and the battery runs once. Dedupe this pass: only spec-paths seeds `fabro-8a60`/`fabro-48f3` exist, both closed and gate-side; open overflows "Fix seed-spec-paths extraction in the loop gate" (`01M59JSC3MKGRCH5PMR53XHADZ.md`) and "Invert spec-named-loop-paths" (`01M5BYTX2X8Q55PQ91RTHWAXB5.md`) are different mechanisms (correction-line citations, editable-roots).

**3. fabro-7f58's run evidence strengthens open seeds — claimed, not re-filed**
- Rec 2 (nushell-semantics skill: 5 probe attempts, 4 failed, inside the stage consuming 74% of run cost) — duplicate_of: fabro-d19e (open, @loop)
- Rec 5 (`fabro_run_search` rejected status `queued`, 2 of 3 calls errored) — duplicate_of: fabro-1cde (open); its planner-prompt arm (inline valid enum literals, title-length hygiene) is already carried by the open overflow "Planner title/search one-shot hygiene" (`01M5CR2B9DKQ0ZRXKD3DZSPMP0.md`)
- Rec 7 (278s implementer stage with no user-visible signal) — duplicate_of: fabro-b769 (open, @loop)

**4. prompt-lint noise (routing-named markers, date pins, fixture-id notes)** — overflow-dup: Silence by-design prompt-lint warnings in the qualitygate (open in 01M2XQJC3TWRMJ1128WRXRQYXH.md) for the routing-named/date-pin arms; overflow-dup: Batch prompt-lint synthetic-id fixture notes to one summary line (open in 01M5CPBDF5PNC8EGDK805SVW10.md) for the fixture-note arm — this run's evidence (25 identical routing-named warnings + 19 date-pin warnings + repeated fixture-id notes ≈ 9KB per tester run, repeated 4× per run incl. the 37KB evidence capture) strengthens both open entries; no second canonical entry per ADR-0022 rule 3.
