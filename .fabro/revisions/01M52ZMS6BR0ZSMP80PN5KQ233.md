# Revision — run 01M52ZMS6BR0ZSMP80PN5KQ233

- status reviewed: succeeded (10m05s wall, $0.61, 6 stage retries) — duplicate run: claimed seed fabro-828c was already closed on the base branch (PR #405 landed the same surface at 5254889feb; seeds-close commit 45a6f2372 closed it citing the PR #412 duplicate); presented as a duplicate, never a healthy pass
- review: .fabro/reviews/develop/01M52ZMS6BR0ZSMP80PN5KQ233.md
- seeds filed: none — zero balance credit this pass (closeout-race count 0, no same-pass stale/superseded closes); all surviving findings journaled as overflow or overflow-dup below
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M52ZMS6BR0ZSMP80PN5KQ233, workflow version absent, commit 25bc91f5d34d2d57d1c5cba050802d247a977b09
- revised_at_commit: 25bc91f5d34d2d57d1c5cba050802d247a977b09 (ADR-0015: engine drift signal for later judgement)

## Findings

**1. CREATE_INTENT verb-proximity regex bounces creation-type claims** — overflow to journal (no balance credit this pass):

- overflow: Accept a `(create)`/`(new)` suffix directly after the path in creation-named? — in `.fabro/workflows/develop/scripts/anchor_check.nu` (CREATE_INTENT at line 205, creation-named? at 228-238), also accept a literal `(create)`/`(new)` suffix token immediately after the path, or bound the `\W*` gap so "created: scripts/up.nu (create) and scripts/install-url.nu" windows every listed path; effect: creation-type seeds (the lane's most common class) claim on planner visit 1 instead of visit 4 — this run's 3 claim_check bounces cost ~68s / ~$0.11 of planner time plus 4 checkpoint commits (~11% of run wall). Distinct from closed fabro-f2eb (moved the window to the shared seam) and open fabro-e8ae (one-char truncation).

**2. Loop gate runs lint-nu + prompt-lint twice inside one gate pass** — overflow-dup: delete the duplicated lint sections from the loop gate (open in `.fabro/revisions/01M43J0MSS6HK1N8396YQAADFT.md`); this run's tester@1 12.3KB gate output shows the full lint block duplicated between `just validate-workflows` (loop-gate.nu:83) and `battery-runner all` (loop-gate.nu:84). No new overflow entry.

**3. Loop implementer probes must never invoke pipeline recipes** — overflow to journal (no balance credit this pass):

- overflow: Forbid pipeline recipes in loop implementer probes — add a rule to `.fabro/workflows/loop/prompts/implementer.md`: probes use synthetic lock holders and never invoke pipeline recipes (`just up`/`build-image`); effect: a contention probe that killed its background holder ran the entire `just up` pipeline (cold cargo compile ~38s, bun build, docker failure) inside the run worktree — the implementer stage was the run's costliest at 325.7s / $0.282 with visible probe thrash. Complements the open probe-discipline overflow in `.fabro/revisions/01M52XXVC22TFSNQ6B34W6V83H.md` (nesting/shell quoting, different rule) and open fabro-d19e (nu-semantics skill).

**4. Loop reviewer preamble truncates the evidence diff at 16KB** — overflow to journal (no balance credit this pass):

- overflow: Raise the loop reviewer node's preamble_inline_max_kb 16 → 32 — in `.fabro/workflows/loop/workflow.fabro` (graph already carries `x.preamble_budget_kb=48`), or have `.fabro/workflows/develop/scripts/evidence.nu` suppress the static ~40-line prompt-lint notes/warnings block from recorded-check captures (compounds with finding 2); effect: the reviewer sees the full seed diff inline instead of truncating mid-tracker-diff at 43.3KB and recovering via 3 read_file calls (this run approved against a recovered view — a verification-gap risk). Sibling of open fabro-cf3e (develop reviewer node, same mechanism, different file); distinct from open fabro-020b (loop-churn caps — this run churn was 0 and it still truncated) and the open caps overflow in `.fabro/revisions/01M40KWWZ4M4P396PGCVH08Q6G.md` (develop graph).

**5. Loop lane `[run.agent] fabro_tools = true` enable** — duplicate_of: fabro-d62c (filed by the previous revisor pass at 2026-10-16 19:03, capability-affecting, needs-user); fabro-9b84's evidence append names the same suppressor. No new filing.

**6. Land the nushell-semantics skill (fabro-d19e)** — duplicate_of: fabro-d19e (open; this run's probe failures — `complete ignore` parse error, bare `^flock` exit 99, holder killed by `complete` — are exactly its two rules). No new filing; fresh evidence strengthens it.
