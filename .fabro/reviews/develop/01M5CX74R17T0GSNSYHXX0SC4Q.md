# Improve review — run 01M5CX74R17T0GSNSYHXX0SC4Q

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (5m17s, revisor pass — reason and cost in run detail)
- generated: 2026-10-20 15:18+0000 by revisor `fabro_ask`

---

Recommendations for the `Loop` workflow, ordered by expected impact. All evidence is from this run (01M5CX74R17T0GSNSYHXX0SC4Q: claimed fabro-bdff, 317s wall, $0.282 total; planner $0.133/112.5s, implementer $0.120/132.3s, reviewer $0.030/19.8s — together ~97% of run cost). Dedup searches run against the tracker: `brief acceptance criteria mechanism`, `cache`, `prompt-lint warnings date pin`, `title`, `brief`, `date pin`, `nushell`, `ready firehose`, `substring collision name filter`, `probe timeout nextest list`, `fabro-97fd`, `deviation implementer brief`.

**1. Claim from a top-N view instead of the 56-seed firehose — seed fabro-c3b4 (open, @loop)**
Evidence: after `preflight` had already narrowed the loop queue to 5 ranked candidates, the planner ran `seeds ready --assignee loop --limit 200` and ingested 8,105 bytes / 56 rows (planner transcript, seq 92); it then claimed row 1. The redundant firehose fed the planner's ranking turns — planner input was 30,282 tokens for a 1,234-token brief at `reasoning_effort=low`.
Change: `prompts/planner.md` step 1–2 — consume preflight's candidate table plus a top-N (≈5) `seeds ready` slice; widen only when all N are skipped.
Effect: cuts planner conversation tokens and ranking turns in the run's most expensive stage; direct cost/latency reduction every scheduled run.

**2. Ship the nushell-scripts skill — seed fabro-d19e (open, @loop)**
Evidence: implementer made 3 of 7 shell calls error while re-deriving nu 0.115 semantics in `/tmp/probe-bdff` (`assert equal $pos 12` parse mismatch → `$m.ran.0` cell-path error → list-literal splitting of `(expr) == 12`), needing 4 probe attempts (evidence "recorded checks", re-runs 1–4). Its own journal: "a quoting/parsing note worth remembering for future nu probes."
Change: implement fabro-d19e; list the skill in the `implementer` node's `use_skill` guidance (`prompts/implementer.md`).
Effect: removes the recurring probe-fail→retry loop in the longest stage (132s wall); fewer wasted LLM turns per loop run.

**3. Bound the in-flight check + fix the fabro_run_search recipe — seed fabro-6b58 (open, @loop)**
Evidence: planner's first `fabro_run_search` call failed by guessing an invalid enum (`status: ["queued","running"]` → "unknown run status `queued`", seq 141); the unfiltered retry returned 20 runs including **this run itself** (no self-exclusion) and 18 old succeeded runs it had to eyeball.
Change: `prompts/planner.md` step 5 — state the valid status filter values verbatim (like the seeds CLI table already does), add a `created_since` window, and exclude this run's own id.
Effect: one correct call instead of two; eliminates the false "self in-flight" hazard.

**4. Make brief mechanism-divergence visible — NEW SEED (justification below)**
Evidence: the brief's bullet 1 prescribed adding `--no-tests fail` (planner probe-verified the flag exists); the shipped diff contains **no such flag** — the implementer silently substituted a Summary-line parser + `ran < names` guard (evidence diff: the `cargo nextest run` invocation line unchanged). Loop Gate and reviewer both went green; nothing flagged the substitution.
Change: `prompts/planner.md` bullet-class marker (`required-literal:` vs `acceptance:`) + a matching tier in `.fabro/workflows/loop/scripts/loop-gate.nu` that greps the run diff for each `required-literal`, or bounce to implementer on miss.
Effect: silent mechanism substitution becomes a deterministic RED (or a journaled deviation) instead of passing unnoticed — this run it was benign, but the gate cannot tell.
New-seed justification: searches `brief acceptance criteria mechanism`, `deviation implementer brief`, and the full `brief` sweep found no open seed; closed fabro-1314 ("briefs must prescribe outcomes, not mechanics") covered only the planner-side phrasing rule, and this run shows both halves still open (brief prescribed a mechanism; divergence unchecked).

**5. Shared prompt-cache prefix across stages — seed fabro-944d (open, @fabro, engine)**
Evidence: reviewer ran with `cache_read: 192` tokens vs planner 307,008 and implementer 241,664, and `cache_write: 0` in all three stages — the reviewer paid full input price ($30.1k micros) for a 598-token verdict (stage usage, run events).
Change: engine-side per fabro-944d — stable cacheable prefix (system prompt, memory, skills) ahead of per-stage variable preamble.
Effect: reviewer-stage input cost drops to cache-read rates on every run; at ~$0.03/run × scheduled frequency it compounds.

**6. Surface the 100-char title cap in the planner prompt — seed fabro-b529 (open; engine-side relief)**
Evidence: planner's first `fabro_run_set_title` (137 chars) was rejected — "run title must be at most 100 characters" (seq 166) — then succeeded at 89 chars; the RUN TITLE section in `planner.md` never states the cap.
Change: add "≤100 characters" to the RUN TITLE line in `prompts/planner.md`; pair with fabro-b529's engine-side cap relief.
Effect: one-call titles, no rejected tool call per run.

**7. Triage Loop Gate warning noise — NEW SEED (justification below)**
Evidence: the tester's "green" output carried 35 warnings this run, all recurring and benign: 17 date-pin warnings ("older than 45 days — still load-bearing?") and 18 routing-named property warnings — the same five routing keys × four schema files, re-emitted every run (tester@1 output).
Change: `.fabro/scripts/prompt-lint.nu` — support a reviewed/waived marker for date pins and collapse the routing-named notice to one summary line per file.
Effect: gate output becomes signal; a future real hygiene warning won't drown in 35 known-benign ones.
New-seed justification: searches `prompt-lint warnings date pin` and `date pin` returned nothing on warning triage (only unrelated fabro-895d); open fabro-0d16 covers a different prompt-lint defect (marker name drift).

**8. Prescribe `nextest show-config`, never `list`, for planner probes — seed fabro-e137 (open, unassigned)**
Evidence: planner journal — "`cargo nextest list -p <crate>` builds test binaries and blew a 10s timeout on a cold cache," forcing the brief to fall back to the flag-form fix; also the planner's 1 errored shell call.
Change: adopt fabro-e137's rule in `prompts/planner.md` step 8 probe guidance.
Effect: planner probes stop paying test-binary build costs on cold caches.

Note: the reviewer's substring-collision false-negative on the new `ran < names` guard is already tracked — closeout filed it as **fabro-97fd** (open, @loop) — so no new filing is needed there. One thing I could not inspect: per-token pricing breakdowns beyond the catalog micros totals, so dollar estimates for #1/#2/#5 are proportional inferences from the stage usage numbers, not priced deltas.
