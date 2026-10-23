# Revision — run 01M5KCXWDYM96AVJM95S51A1Y3

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M5KCXWDYM96AVJM95S51A1Y3.md
- items filed: none — tracker binding outage (7th consecutive revisor context): `fabro_work_list` and `fabro_work_create` both fail with "no work project is bound to target repository denkhaus/fabro"; run verdict is healthy (self-closed fabro-1923, landed bd02d847e / PR #477 on `origin/denkhaus`), and 10 of 12 ask recommendations were duplicate_of existing seeds at analyze time
- balance: 0 non-exempt items filed / 0 — no credit this pass (no same-pass stale/superseded closes possible; `fabro_work_close` dead under the same outage)
- basis: run 01M5KCXWDYM96AVJM95S51A1Y3, workflow version absent, commit 854f2c4bac6cc5b4c8aee876071af7d5238b5474
- revised_at_commit: 854f2c4bac6cc5b4c8aee876071af7d5238b5474 (ADR-0015: engine drift signal for later judgement)

## Findings

**1. Closeout none-marker filter misses "none — <prose>" payloads** — overflow

- overflow: Closeout none-marker filter misses "none — <prose>" payloads — extend the skip filter in `.fabro/workflows/develop/scripts/closeout.nu` (the fabro-7ece fix landed an exact `== "none"` match at line ~340, yet junk seed fabro-30a5 was still filed by this run's closeout from a payload reading "none — the open dependency fabro-4653 …"): skip deferred text that begins with `none` followed by punctuation/prose, add smoke assertions for that payload shape in `closeout-smoke.nu`, and close the junk seed fabro-30a5 as a duplicate of the already-open fabro-4653; effect: no junk rows polluting every future `seeds ready`/preflight scan. Cross-ref: closed fabro-7ece (same file/mechanism — extension, not supersession; it is already closed), open fabro-db25 (closeout materiality bar, complementary). Original finding-3 framing (skip `deferred-action: none` markers) was duplicate_of fabro-7ece; only the prose-payload extension survives.

**2. Briefs must adjudicate named preconditions** — overflow

- overflow: Briefs must adjudicate named preconditions — add to the brief-writing step of `.fabro/workflows/loop/prompts/planner.md` (mirror in develop's planner.md) one met/waived bullet per seed id the claimed body names as a precondition (evidence: implementer journal of this run — seed fabro-1923's action was conditional on phase-3 rewrites with fabro-4653 still open, and "the precondition judgment fell to the implementer" because the planner had adjudicated it only as a journal observation); effect: claim-eligibility decisions stop being re-derived downstream by the implementer. Adjacent, not duplicates: fabro-1314 (briefs prescribe outcomes), fabro-a18d (observations become brief bullets) — different mechanisms.

Duplicates dropped at analyze time (recorded in the review and the analyze journal): fabro-c3b4, fabro-a827, fabro-ab38, fabro-17fb, fabro-eaa0, fabro-b529, fabro-1cde, fabro-b769, fabro-801e, fabro-7808.

Needs-user retry (exempt from balance): the capability-affecting "Loop reviewer node cannot read the evidence blob" item (ADR-0019) was attempted via `fabro_work_create` this pass and failed on the same tracker binding outage — it remains an open, unconsumed overflow at `.fabro/revisions/01M5K62554W7YCVZ7W4AT8YYWK.md` line 26; no `overflow-dup` line needed (the original entry stays the single open theme entry).
