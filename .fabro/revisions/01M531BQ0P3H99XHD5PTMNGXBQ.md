# Revision — run 01M531BQ0P3H99XHD5PTMNGXBQ

- status reviewed: succeeded (288.7s wall, $0.3212, PR per run branch, claimed seed fabro-4cc3 — healthy pass, clean dup-run-check, closeout-race count 0)
- review: .fabro/reviews/develop/01M531BQ0P3H99XHD5PTMNGXBQ.md
- seeds filed: none — healthy run; all surviving findings already owned by open seeds or open overflows
- balance: 0 non-exempt seeds filed / 0 — no credit this pass (no same-pass stale/superseded closes; closeout-race count 0)
- basis: run 01M531BQ0P3H99XHD5PTMNGXBQ, workflow version absent, commit a79e4a221fd6edc8b511f61ff6f02fc9c38c5e3e
- revised_at_commit: a79e4a221fd6edc8b511f61ff6f02fc9c38c5e3e (ADR-0015: engine drift signal for later judgement)

## Findings

**1. Preflight/claim-check anchor parity + false in_flight** — duplicate_of: fabro-e8ae (anchor seam) and fabro-ab93 (in_flight keyed on the run actually holding the seed). No new filing; fresh evidence strengthens both.

**2. Enable run-level fabro tools for the loop planner** — duplicate_of: fabro-d62c (filed by the previous revisor pass at 2026-10-16 19:03, capability-affecting, needs-user). No new filing.

**3. Port the `rg -rn` and English-response one-liners into the loop prompts** — duplicate_of: fabro-6997 and fabro-eaa0 (loop-lane prompts to carry the same lines when landing). No new filing.

**4. Cap tracker-row diff context in `evidence.nu`'s seed-work section** — overflow to journal (no balance credit this pass):

- overflow: Cap tracker-row diff context in evidence.nu — in `.fabro/workflows/develop/scripts/evidence.nu`, render the `.seeds/issues.jsonl` diff with `-U0` (or filter context hunks to rows naming the current seed id), keeping full diffs for real code files; effect: evidence captures drop from ~47KB to a few KB, the reviewer preamble stays inside the 16KB inline cap, and blob-paging detours disappear (basis: run 01M531BQ0P3H99XHD5PTMNGXBQ evidence@1 46,881 bytes inlining three unrelated multi-KB seed bodies). Distinct from open fabro-020b (caps the loop-churn section only) and from the open lockfile-elision overflow in `.fabro/revisions/01M2ZYVQ7CBSY3T0FEYBE9JK74.md` (different mechanism: `*.lock` elision); extend fabro-020b or file a sibling when landed.

**5. Loop gate runs lint-nu + prompt-lint twice** — overflow-dup: delete the duplicated lint sections from the loop gate (open in `.fabro/revisions/01M43J0MSS6HK1N8396YQAADFT.md`); this pass's evidence (tester@1 19.2s, 12,292-byte output with the full lint block duplicated between tiers 1 and 2) confirms the same change. No new overflow entry.

**6. Stop pouring the 65-row ready firehose into the loop planner** — duplicate_of: fabro-c3b4 (extend to the loop planner prompt when landing). No new filing.

**7. Superseded-close fabro-c0c2 (duplicate of landed fabro-4cc3)** — operational, no seed: the next loop planner performs the sanctioned superseded-close; fabro-ab38 owns the durable automated sibling sweep. No new filing.

**8. Mid-run heartbeat for long agent stages** — duplicate_of: fabro-b769. No new filing.
