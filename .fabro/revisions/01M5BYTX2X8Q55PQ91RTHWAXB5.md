# Revision — run 01M5BYTX2X8Q55PQ91RTHWAXB5

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M5BYTX2X8Q55PQ91RTHWAXB5.md
- seeds filed: none — healthy-plus run, 0 balance credit this pass; surviving findings journaled as overflow
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M5BYTX2X8Q55PQ91RTHWAXB5, workflow version absent, commit 664581339df9ed8de474a575b22eb6d9271ae3e0
- revised_at_commit: 664581339df9ed8de474a575b22eb6d9271ae3e0 (ADR-0015: engine drift signal for later judgement)

## Findings

**1. Top-N ready view for the planner** — duplicate_of: fabro-c3b4 (open, @loop): planner read a truncated 58-seed firehose and re-derived priority ordering in prose; fabro-c3b4 already names the top-N candidates view.

**2. Long-stage heartbeat** — duplicate_of: fabro-b769 (open, @loop): notifications fire only on `run.completed`/`run.failed`; 3-minute implementer/planner laps run silent. fabro-b769 already names the stage-progress notify.

**3. Nushell skill with probe patterns** — duplicate_of: fabro-d19e (open, @loop): implementer burned two failed nested-quoting probe iterations; fabro-d19e already carries the skill.

**4. Invert `spec-named-loop-paths` to an editable-roots allow-list** — overflow

- overflow: Invert `spec-named-loop-paths` from deny-list drops to an editable-roots allow-list — in `scripts/qualitygate.nu` (~line 346) keep only paths inside the implementer's actual fs-write roots instead of filtering known-bad prefixes, and add a fixture arm in `.fabro/scripts/seed-spec-paths-fixtures.nu` asserting a provenance-only body yields "tier idle"; effect: eliminates the burned-implementer-lap class (this run: tester RED on `.fabro/journal/01M59JSC3MKGRCH5PMR53XHADZ.jsonl` parsed from the seed's `Origin:` line forced implementer@2 143.4s/$0.162 + a second tester pass — ~29% of the $0.558 budget; second piecemeal drop after closed fabro-48f3). Distinct from the open overflow "Fix seed-spec-paths extraction in the loop gate" (`01M59JSC3MKGRCH5PMR53XHADZ.md`, loop-gate.nu correction-line citations — different script/mechanism).

**5. Aggregate static lint notes/warnings into per-class roll-ups** — overflow-dup: Silence by-design prompt-lint warnings in the qualitygate (open in `01M2XQJC3TWRMJ1128WRXRQYXH.md`) — same lint-noise theme (date-pin nag overlaps); roll-up-with-counts in `.fabro/scripts/prompt-lint.nu`/`.fabro/scripts/battery-runner.nu` is a complementary mechanism, consolidatable as arms when that entry is filed.

**6. Make prompt-lint's exit-2 "scope broken" a teaching error** — overflow

- overflow: Make prompt-lint's exit-2 "scope broken" a teaching error — in `.fabro/scripts/prompt-lint.nu` have the scope check print which expected dirs/files are missing and document the temp-root fixture contract in the `.fabro/scripts/prompt-lint-fixtures.nu` header; effect: one-shot fixture authoring instead of exit-2 trial-and-error (this run's implementer had to reconstruct the whole conductor/develop/loop/revisor layout under `mktemp -d` to discover the implicit contract). Not covered by fabro-d19e (nu semantics, not this fixture contract); no open overflow names it.

**7. Live-schema tool-call examples for `fabro_run_set_title`/`fabro_run_search` in the planner prompt** — overflow

- overflow: Add live-schema tool-call examples for `fabro_run_set_title`/`fabro_run_search` in `.fabro/workflows/loop/prompts/planner.md`, as input to open fabro-f18a whose schema lint then keeps them honest; effect: removes 1–2 retry turns per planning lap (this run's planner stats: `fabro_run_search: 2 calls, 1 error`, `fabro_run_set_title: 2 calls, 1 error` in the most expensive non-implementer stage, 150.1s/$0.183). Cross-ref fabro-f18a (open, @loop) — the examples are its input, the lint is its deliverable; no open overflow names the prompt-side examples.
