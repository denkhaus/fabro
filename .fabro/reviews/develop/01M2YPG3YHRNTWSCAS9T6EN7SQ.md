# Improve review — run 01M2YPG3YHRNTWSCAS9T6EN7SQ

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (15.2 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-20 06:22+0000 by revisor `fabro_ask`

---

Recommendations for the develop workflow, ordered by expected impact. All evidence is from run `01M2YPG3YHRNTWSCAS9T6EN7SQ` (seed fabro-7aac, ~15 min wall, **$1.11 total**: planner 217s/$0.406, implementer 586s/$0.666, reviewer 32s/$0.037, tester 23.8s, 0 retries — from run events/projection).

**1. Stop the closeout residual sweep from spawning cosmetic dev cycles — implement open seed fabro-db25 (arm 1).**
What happened: this run's closeout filed **fabro-e557**, an open P2 bug whose entire body is "awkward mid-sentence line breaks — cosmetic only, prompt-lint green" (from the closeout diff / tracker). That seed will consume a future run (~$1.1 / ~15 min by this run's own economics) to reflow six lines. This is the exact self-perpetuating generator fabro-db25 arm 1 was filed to kill.
Change: `.fabro/workflows/develop/scripts/closeout.nu` — in `sweep-reviewer-findings`/`is-nonblocking`, file only findings with a Defect+Target pair; cosmetic nits stay journaled.
Effect: eliminates one recurring ~$1/dev-cycle per non-defect reviewer observation; fabro-e557 itself becomes closeable as cosmetic.

**2. Park the two externally-blocked planner candidates — new seed (mirrors the fabro-a285 pattern used for fabro-d810).**
What happened: the planner's top two candidates were unimplementable in-repo: fabro-af22 (fix lives in the pebble dependency behind unmerged upstream PR fabro-sh/fabro#784 — verified via `git ls-remote` + `git fetch`/`merge-base`, seq 51–76) and fabro-23d4 (needs an `sd note` subcommand the external seeds CLI v0.5.15 lacks, seq 77–88). That adjudication cost ~85s wall and ~$0.20 — **~18% of total run cost** — and will repeat on every develop lap until upstream moves. The planner journaled exactly this ("should arguably be parked/marked blocked-external like fabro-a285"), but journal-only proposals don't exist in the tracker by the loop's own rule.
Change: tracker hygiene — `sd block` (or label `blocked-external`) fabro-af22 and fabro-23d4 with the infeasibility facts already in this run's planner journal.
Effect: removes ~$0.15–0.20 and ~1.5 min from every subsequent develop run; the planner starts at an actionable candidate.
*New-seed justification: no open seed parks these two; a285's park was a one-off for fabro-d810 and the recurring-parking action exists only in this run's journal.*

**3. Stop blob-ref'ing the evidence capture — implement open seed fabro-cf3e (raise reviewer `preamble_inline_max_kb` 16 → 32).**
What happened: the evidence capture was 19.1 KB (from `evidence@1` script_timing), which exceeded the reviewer's 16 KB per-key inline ceiling, so it arrived as a blob ref — the reviewer's single tool call of the whole pass was paging that blob (`read_file` ×1, from stage agent stats). The 48 KB graph budget (fabro-1e9f) intended inline captures; the per-key ceiling still demotes ~19 KB ones, and every demotion re-opens the "unread blob ref" rejection risk the reviewer prompt explicitly polices.
Change: `.fabro/workflows/develop/workflow.fabro`, reviewer node: `preamble_inline_max_kb=32`.
Effect: typical 15–30 KB captures render inline; zero blob round-trips per review.

**4. Give the implementer a nushell skill — implement open seed fabro-d19e (companion: fabro-866a).**
What happened: the implementer ran 52 shell calls (5 errors) with 499s inference vs 86s tool time on a pure-nu seed, and its journal records that a misleading nu parse error ("expected valid variable name" pointing at the wrong token) "cost 4 diagnostic calls". The pitfalls were appended to expertise record mx-93569c — but only `rust-style-guide` is discoverable as a skill, so the next nu seed re-derives them again.
Change: add `.fabro/skills/nushell-scripts/` (pitfalls 1–6 of mx-93569c as the seed content); the implementer node already has `skills="discover"`.
Effect: fewer diagnostic round trips on every loop-asset/nu seed — the implementer is 60% of run cost, and nu seeds are this loop's most common kind. fabro-866a (chain recon+verify calls, open, was candidate #4 in this run's preflight) compounds it.

**5. Fix painpoint misfiling — implement open seed fabro-21c0 (port the workaround-is-a-painpoint clause to implementer.md).**
What happened: two genuine loop-asset frictions — the prompt-lint run-id-literal rework and the `ml prune` near-miss — landed in the implementer's journal **observations**, not `painpoints` (from the implementer journal record). The reviewer prompt already carries the "a workaround you performed is a painpoint, not an observation" clause; the implementer prompt doesn't. Misfiled friction never reaches the revisor, so it never becomes a seed.
Change: `.fabro/workflows/develop/prompts/implementer.md`, journal section — add the one-line workaround-is-a-painpoint clause (fabro-21c0).
Effect: the improve loop actually sees loop-asset friction like the prompt-lint rule; cheap one-line prompt change.

**6. Make destructive `ml` subcommands safe by default — new seed.**
What happened: a bare `ml prune` (meant as a help read) soft-archived **~30 expertise records repo-wide**; recovery required `git checkout -- .mulch/` plus manually re-applying the intended amendment (implementer journal, observation 3). One mistyped command nearly destroyed the loop's accumulated lessons — a real error-handling gap, this time caught only because the implementer noticed.
Change: wrapper or upstream `ml` change — `ml prune` requires an explicit `--yes`/`--dry-run` (default dry-run), same shape as `sd close`'s mandatory `--reason`.
Effect: eliminates a silent mass-destruction class on every implementer pass that touches lessons.
*New-seed justification: checked open seeds — fabro-b94d (mulch.config mutation) and fabro-8d81 (mx-id printing) don't cover destructive-subcommand guarding.*

**7. Seed-intake lint: mechanism citations must reference landed seeds — extend open seed fabro-3839 with an arm.**
What happened: fabro-7aac's body said "extend the fabro-534e mechanism", but fabro-534e is still open and unlanded; the planner had to probe `closeout.nu`, discover the real pattern (fabro-22fa), and rewrite the whole seed body via `sd update --description` before claiming (planner transcript seq 94–97, journal observation 3). The correction machinery worked, but the bad citation was filed on 2026-09-16 and burned planner probes four days later.
Change: add an arm to the filed-seed lint (fabro-3839's surface): a body phrase "extend the <fabro-id> mechanism" fails lint unless that seed is closed.
Effect: no implementer ever re-hits an unlanded-mechanism contradiction; saves the ~1-minute planner correction lap per occurrence.

**UX, one-liner:** open seed fabro-1409 (per-stage cost/time table + painpoint digest in the PR body) is well-grounded here — this run's 60/37/3% cost skew between implementer/planner/reviewer is invisible in PR #316's surface; I could only see it by reading run events.

Not verifiable from this run: the gatebounce, deadlock-exit, and Verification-blocked paths were never exercised (first-pass green, 0 retries), so no recommendation about them is evidence-backed here.
